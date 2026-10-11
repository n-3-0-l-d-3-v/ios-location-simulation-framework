# Compatibility

Status vocabulary: **Verified** (run on that exact combination, results
recorded here), **Partially Verified**, **Untested**, **Unsupported**.
A row is never upgraded on the basis that it "should work". A row says what
was run: one tested configuration is not a claim about its neighbours.

## Rust crates (`locsim-core`, `locsim-scenario`, `locsim-store`, `locsim-health`, `locsim-ffi`)

| Host | Toolchain | Status | Notes |
|---|---|---|---|
| Windows 11 x86_64 | rustc 1.98.1, target `x86_64-pc-windows-msvc` | Verified | Full workspace test suite (594 tests: 273 core, 89 scenario, 69 store, 85 health, 78 ffi), 2026-10-11, code as of commit `e7fad44`; with serde 1.0.229, serde_json 1.0.151 as locked in `Cargo.lock` |
| Windows, target `x86_64-pc-windows-gnu` | — | Untested | No crate has been built with the Rust GNU toolchain |
| Windows, 32-bit | — | Untested | |
| Any host | rustc 1.75 (declared `rust-version`) | Untested | The minimum version in `Cargo.toml` has never been built; only 1.98.1 has. The locked dependencies of `locsim-scenario` declare 1.71 or lower, which is a declaration, not a build |
| macOS (arm64 / x86_64) | — | Untested | |
| Linux x86_64 | — | Untested | |
| iOS arm64 (`aarch64-apple-ios`) | — | Untested | Not cross-compiled, not even checked |

`locsim-health` has no platform-specific code: it uses no file, clock,
thread or system call, and its behaviour is a function of its inputs. That
is a reason to expect it to behave the same elsewhere; it is not a test. It
has been built and run on the Windows host above and nowhere else. A
`Supervisor` is used from more than one thread only through `locsim-ffi`,
which never lets two calls reach one at the same time. Its behaviour when
driven by a real timer is Untested everywhere.

## The C interface (`locsim-ffi`, `include/locsim.h`)

What was run is `crates/locsim-ffi/harness/run.sh`: it compiles
`harness/harness.c` as C11 with warnings as errors against the public
header, links it to the built library, runs it, and compares the samples it
received with those of the Rust supervisor byte for byte. **It is a
separate command. `cargo test` does not run it**, and a row below is not
re-verified by a passing `cargo test`.

All on Windows 11 x86_64, with the library built by rustc 1.98.1 for
`x86_64-pc-windows-msvc`, at commit `e7fad44`, 2026-10-11:

| C caller compiled by | Linked to | Library build | Status |
|---|---|---|---|
| MSVC `cl` 19.44.35228 (`/std:c11 /W4 /WX /MD`) | the DLL | debug | Verified |
| MSVC `cl` 19.44.35228 | the DLL | release | Verified |
| MSVC `cl` 19.44.35228 | the static library | debug | Verified |
| MSVC `cl` 19.44.35228 | the static library | release | Verified |
| MinGW-w64 gcc 14.2.0, MSYS2 ucrt64 (`-std=c11 -Wall -Wextra -Werror -pedantic`) | the DLL | debug | Verified |
| MinGW-w64 gcc 14.2.0 | the DLL | release | Verified |

In each of the six, the harness exited 0 (about 385 000 checks, the exact
number varying with how the threads interleave; none failed), the samples equalled the Rust reference, and the DLL exported
exactly the 15 functions the header declares (GNU `objdump`, and MSVC
`dumpbin` in the MSVC rows).

What those rows do **not** say:

| Combination | Status | Notes |
|---|---|---|
| The library built for `x86_64-pc-windows-gnu` | Untested | The MinGW rows are a MinGW-compiled *caller* of the MSVC-built library. The library has not been built with a GNU Rust toolchain |
| The header compiled as C++ | Untested | It has `extern "C"` guards; no C++ compiler has read it |
| The header's optional `LOCSIM_DLL` import macro | Untested | Never defined in a build |
| Any other C compiler, any other version of these two | Untested | |
| 32-bit Windows | Untested | |
| macOS, Linux, iOS; Swift as the caller | Untested | Nothing has been built there. The harness's threads are Win32 only and it refuses to compile elsewhere |
| The panic and poisoned-session paths, from C | Untested | Reachable only through hooks in the Rust unit tests, where they are tested |
| Two threads on one session, from C | Partially Verified | Every call returned OK or busy and wrote nothing when busy, and the sample count reconciled, in every run. The harness cannot force a collision; the exact interleavings are forced only in the Rust tests |

A static consumer on Windows must use the C runtime the library uses
(`/MD`) and link the system libraries `rustc --print native-static-libs`
lists; the script does both.

## Persistence (`locsim-store`) by file system behaviour

`locsim-store` behaves differently per operating system at exactly the
points that matter for crash safety, so its rows are kept apart.

| Behaviour | Windows 11, NTFS, rustc 1.98.1 | Unix-like (Linux, macOS, iOS) |
|---|---|---|
| Save, load, corruption detection, fault injection at every write step | Verified (commit `29c8342`) | Untested |
| Old record kept when a save fails | Verified | Untested |
| Replacement visible atomically to a concurrent reader | Partially Verified: 240 replacements with about 2 200 concurrent loads, every load a whole record, none missing, none refused. Not documented as atomic by Microsoft | Untested (specified by POSIX `rename`) |
| Record survives the writer process being killed | Verified: 40 kills, record whole every time (weak evidence on its own; see `PROGRESS.md`) | Untested |
| Record held open by another handle without delete sharing | Verified: save fails cleanly, record kept | Not applicable |
| Read-only record | Verified: save fails cleanly (access denied), record kept | Untested (expected: replaced) |
| Directory synced after the rename | Unsupported through the standard library (`DIRECTORY_SYNC == false`) | Untested; the code exists under `cfg(unix)` and has never been compiled |
| Survives power loss or an operating-system crash | Untested | Untested |
| Network or removable file systems | Untested | Untested |

The store is not reachable through the C interface.

## Delivery on Apple platforms

Nothing Apple exists yet. The C interface above is what an adapter would
call; an adapter is ticket T11B, which has not started and needs a Mac with
Xcode. **Every row is Untested, and a row may change only on a run with
Apple tooling on that combination.** A passing C harness on Windows is not
evidence for any of them.

What is planned is in-app injection: an app built with the adapter gets
synthetic locations from the framework. It is not a replacement of the
system's location for other apps.

| Device | SoC | iOS | Arch | Simulator | Physical device | Jailbreak required | Known limitations | Status |
|---|---|---|---|---|---|---|---|---|
| iOS Simulator (any) | host | — | arm64/x86_64 | planned: `simctl location`, GPX | n/a | No | Affects the whole simulator, not one app | Untested |
| Development device | any | — | arm64 | n/a | planned: Xcode GPX / in-app provider injection | No | In-app injection only affects the app built with the adapter | Untested |
| Research device (jailbroken) | — | — | arm64/arm64e | n/a | not designed yet | Yes | No hardware available to the project so far | Untested |
