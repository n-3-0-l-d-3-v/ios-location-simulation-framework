#!/usr/bin/env bash
# Builds the C harness against the built locsim-ffi library and runs it.
#
#   crates/locsim-ffi/harness/run.sh [msvc|mingw]... (default: both)
#
# For the debug and the release build of the library, and for each
# compiler named:
#   1. the exports of the DLL must be exactly the functions the header
#      declares;
#   2. harness.c is compiled as C11 with warnings as errors, and linked
#      against the DLL's import library;
#   3. the harness runs, and must exit 0;
#   4. the samples it wrote must equal, byte for byte, what the Rust
#      supervisor produces without the C interface (examples/reference.rs).
# With MSVC the harness is also linked against the static library, and the
# exports are listed a second time with dumpbin. The harness is compiled
# with /MD, the C runtime the Rust library uses.
#
# Fails if a compiler that was asked for is not found: nothing is skipped.
#
# Windows only (Git Bash or MSYS2): this is the one platform it has run on.
# "mingw" means a C caller compiled by MinGW-w64 gcc, linked to the library
# as built by the MSVC Rust toolchain; the library itself is not built with
# MinGW here.
set -euo pipefail
cd "$(dirname "$0")/../../.."

root="$(pwd)"
ffi="$root/crates/locsim-ffi"
scenarios="$root/Examples/Scenarios"
work="$root/target/ffi-harness"
compilers=("$@")
[ ${#compilers[@]} -eq 0 ] && compilers=(msvc mingw)

vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
vcvars=""
find_msvc() {
  [ -x "$vswhere" ] || { echo "MSVC: vswhere.exe not found" >&2; exit 1; }
  local install
  install="$("$vswhere" -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | tr -d '\r')"
  vcvars="$install\\VC\\Auxiliary\\Build\\vcvars64.bat"
  [ -n "$install" ] && [ -f "$(cygpath -u "$vcvars")" ] || { echo "MSVC: vcvars64.bat not found" >&2; exit 1; }
}

# Runs one command line in the MSVC x64 environment.
in_msvc() {
  local script="$work/msvc_$RANDOM.bat"
  printf '@echo off\r\ncall "%s" >nul\r\nif errorlevel 1 exit /b 1\r\n%s\r\n' "$vcvars" "$*" > "$script"
  cmd.exe //c "$(cygpath -w "$script")"
}

# The names a header declares, and the names a DLL exports, one per line.
declared() { grep -oE 'locsim_[a-z_]+\(' "$ffi/include/locsim.h" | tr -d '(' | sort -u; }
exported() { objdump -p "$1" | sed -n '/\[Ordinal\/Name Pointer\] Table/,/^$/p' | awk '$1 == "[" || $1 ~ /^\[[0-9]/ {print $NF}' | sort -u; }

mkdir -p "$work"
declared > "$work/declared.txt"
echo "== the header declares $(wc -l < "$work/declared.txt") functions"

for profile in debug release; do
  echo "== library: $profile"
  if [ "$profile" = release ]; then cargo build -q -p locsim-ffi --release; else cargo build -q -p locsim-ffi; fi
  lib="$root/target/$profile"

  exported "$lib/locsim_ffi.dll" > "$work/exported_$profile.txt"
  if ! diff "$work/declared.txt" "$work/exported_$profile.txt"; then
    echo "FAILED: the DLL's exports differ from the header's declarations ($profile)" >&2
    exit 1
  fi
  echo "   exports: exactly the $(wc -l < "$work/exported_$profile.txt") declared functions"

  if [ "$profile" = release ]; then
    cargo run -q -p locsim-ffi --release --example reference -- "$scenarios" > "$work/expected_$profile.txt"
  else
    cargo run -q -p locsim-ffi --example reference -- "$scenarios" > "$work/expected_$profile.txt"
  fi

  for compiler in "${compilers[@]}"; do
    out="$work/$profile-$compiler"
    rm -rf "$out"; mkdir -p "$out"
    cp "$lib/locsim_ffi.dll" "$out/"
    case "$compiler" in
      msvc)
        find_msvc
        # The same question as above, asked of Microsoft's own tool.
        in_msvc "dumpbin /nologo /exports \"$(cygpath -w "$lib/locsim_ffi.dll")\"" | tr -d ''           | awk '$1 ~ /^[0-9]+$/ && $2 ~ /^[0-9A-F]+$/ && NF >= 4 {print $4}' | sort -u > "$out/dumpbin.txt"
        diff "$work/declared.txt" "$out/dumpbin.txt" || { echo "FAILED: dumpbin disagrees ($profile)" >&2; exit 1; }
        echo "   msvc: dumpbin lists exactly the $(wc -l < "$out/dumpbin.txt") declared functions"
        echo "   msvc: compile, link to the DLL, run"
        in_msvc "cl /nologo /std:c11 /W4 /WX /O2 /MD /I \"$(cygpath -w "$ffi/include")\" /Fo\"$(cygpath -w "$out")\\\\\" /Fe\"$(cygpath -w "$out/harness.exe")\" \"$(cygpath -w "$ffi/harness/harness.c")\" /link \"$(cygpath -w "$lib/locsim_ffi.dll.lib")\""
        "$out/harness.exe" "$scenarios" "$out/samples.txt"
        diff -q <(tr -d '\r' < "$work/expected_$profile.txt") <(tr -d '\r' < "$out/samples.txt")
        echo "   msvc: samples equal the Rust reference"

        echo "   msvc: link to the static library, run"
        natives="$(cargo rustc -q -p locsim-ffi $([ "$profile" = release ] && echo --release) --crate-type staticlib -- --print native-static-libs 2>&1 | sed -n 's/.*native-static-libs: //p' | tail -1)"
        in_msvc "cl /nologo /std:c11 /W4 /WX /O2 /MD /I \"$(cygpath -w "$ffi/include")\" /Fo\"$(cygpath -w "$out")\\\\static_\" /Fe\"$(cygpath -w "$out/harness_static.exe")\" \"$(cygpath -w "$ffi/harness/harness.c")\" /link \"$(cygpath -w "$lib/locsim_ffi.lib")\" $natives"
        mkdir -p "$out/static"
        cp "$out/harness_static.exe" "$out/static/"
        "$out/static/harness_static.exe" "$scenarios" "$out/static/samples.txt"
        diff -q <(tr -d '\r' < "$work/expected_$profile.txt") <(tr -d '\r' < "$out/static/samples.txt")
        echo "   msvc static: samples equal the Rust reference"
        ;;
      mingw)
        command -v gcc >/dev/null || { echo "MinGW: gcc not found" >&2; exit 1; }
        echo "   mingw: compile, link to the DLL, run"
        gcc -std=c11 -Wall -Wextra -Werror -pedantic -O2 -I "$ffi/include" \
          -o "$out/harness.exe" "$ffi/harness/harness.c" "$lib/locsim_ffi.dll.lib"
        "$out/harness.exe" "$scenarios" "$out/samples.txt"
        diff -q <(tr -d '\r' < "$work/expected_$profile.txt") <(tr -d '\r' < "$out/samples.txt")
        echo "   mingw: samples equal the Rust reference"
        ;;
      *) echo "unknown compiler: $compiler" >&2; exit 2 ;;
    esac
  done
done
echo "== all harness runs passed"
