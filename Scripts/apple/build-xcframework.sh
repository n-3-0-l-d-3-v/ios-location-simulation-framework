#!/usr/bin/env bash
# EXPERIMENTAL (T11B readiness). Builds locsim-ffi as a static library for
# one Apple target and packages it, with the public header and a module
# map, as LocsimFFI.xcframework inside Apple/PackagingProbe. Also copies the
# scenario that the tests of the probe read.
#
# Everything it writes is generated and ignored by git. It needs macOS,
# Xcode and a Rust toolchain with the target installed. It has only been
# run where Docs/Diagnostics/apple_packaging says it was.
#
#   Scripts/apple/build-xcframework.sh
#
# LOCSIM_APPLE_TARGET  the Rust target (default aarch64-apple-ios-sim)
# LOCSIM_PROFILE       debug or release (default release)
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
target="${LOCSIM_APPLE_TARGET:-aarch64-apple-ios-sim}"
profile="${LOCSIM_PROFILE:-release}"
probe="${root}/Apple/PackagingProbe"
output="${probe}/Artifacts/LocsimFFI.xcframework"
fixtures="${probe}/Tests/LocsimPackagingProbeTests/Fixtures"
header="${root}/crates/locsim-ffi/include/locsim.h"

case "${profile}" in
  release) profile_flag="--release" ;;
  debug) profile_flag="" ;;
  *) echo "LOCSIM_PROFILE must be debug or release" >&2; exit 2 ;;
esac

cd "${root}"
echo "== toolchain"
rustc -vV
xcodebuild -version

echo "== build: locsim-ffi, staticlib, ${target}, ${profile}"
# The panic contract of the C interface holds only for a library that
# unwinds. This is what the target does unless a profile says otherwise.
echo "panic strategy of the target: $(rustc --print cfg --target "${target}" | grep "^panic=")"
grep -n "panic" Cargo.toml || echo "no panic setting in the workspace manifest"
# The manifest is not changed: staticlib is a crate type it declares.
# native-static-libs is what the final link must add; rustc prints it as a
# note when it compiles.
# shellcheck disable=SC2086
cargo rustc -p locsim-ffi --target "${target}" ${profile_flag} --crate-type staticlib -- --print native-static-libs
library="${root}/target/${target}/${profile}/liblocsim_ffi.a"
ls -l "${library}"
lipo -info "${library}"

echo "== stage: the header, unmodified, and the module map"
stage="$(mktemp -d)"
trap "rm -rf \"${stage}\"" EXIT
mkdir "${stage}/Headers"
cp "${header}" "${stage}/Headers/locsim.h"
cp "${root}/Apple/LocsimFFI/module.modulemap" "${stage}/Headers/module.modulemap"
cmp "${header}" "${stage}/Headers/locsim.h"

echo "== package"
# Never reuse an artifact from an earlier run.
rm -rf "${output}"
mkdir -p "$(dirname "${output}")"
xcodebuild -create-xcframework \
  -library "${library}" -headers "${stage}/Headers" \
  -output "${output}"
(cd "${output}" && find . -type f | sort)
plutil -p "${output}/Info.plist"
packaged="$(find "${output}" -name liblocsim_ffi.a)"
cmp "${library}" "${packaged}"
cmp "${header}" "$(find "${output}" -name locsim.h)"
shasum -a 256 "${packaged}" "${header}"

echo "== fixture for the tests of the probe"
rm -rf "${fixtures}"
mkdir -p "${fixtures}"
cp "${root}/Examples/Scenarios/walking.json" "${fixtures}/walking.json"
cmp "${root}/Examples/Scenarios/walking.json" "${fixtures}/walking.json"

echo "packaged: ${output}"
