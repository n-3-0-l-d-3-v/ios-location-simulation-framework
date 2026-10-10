# Compatibility

Status vocabulary: **Verified** (run on that exact combination, results
recorded here), **Partially Verified**, **Untested**, **Unsupported**.
A row is never upgraded on the basis that it "should work".

## Simulation core (`locsim-core`), scenario documents (`locsim-scenario`), persistence (`locsim-store`), health (`locsim-health`)

| Host | Toolchain | Status | Notes |
|---|---|---|---|
| Windows 11 x86_64 | rustc 1.98.1 | Verified | Full workspace test suite (516 tests: 273 core, 89 scenario, 69 store, 85 health) at commit `fcda25c`, 2026-10-10; with serde 1.0.229, serde_json 1.0.151 as locked in `Cargo.lock` |
| Any host | rustc 1.75 (declared `rust-version`) | Untested | The minimum version in `Cargo.toml` has never been built; only 1.98.1 has. The locked dependencies of `locsim-scenario` declare 1.71 or lower, which is a declaration, not a build |
| macOS (arm64 / x86_64) | — | Untested | |
| Linux x86_64 | — | Untested | |
| iOS arm64 (`aarch64-apple-ios`) | — | Untested | Not yet cross-compiled |

`locsim-health` has no platform-specific code: it uses no file, clock,
thread or system call, and its behaviour is a function of its inputs. That
is a reason to expect it to behave the same elsewhere; it is not a test. It
has been built and run on the Windows host above and nowhere else. Its use
from more than one thread, and its behaviour when driven by a real timer,
are Untested everywhere.

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

## Delivery on Apple platforms

No platform adapter exists yet (ticket T11), so every row is Untested.

| Device | SoC | iOS | Arch | Simulator | Physical device | Jailbreak required | Known limitations | Status |
|---|---|---|---|---|---|---|---|---|
| iOS Simulator (any) | host | — | arm64/x86_64 | planned: `simctl location`, GPX | n/a | No | Affects the whole simulator, not one app | Untested |
| Development device | any | — | arm64 | n/a | planned: Xcode GPX / in-app provider injection | No | In-app injection only affects the app built with the adapter | Untested |
| Research device (jailbroken) | — | — | arm64/arm64e | n/a | not designed yet | Yes | No hardware available to the project so far | Untested |
