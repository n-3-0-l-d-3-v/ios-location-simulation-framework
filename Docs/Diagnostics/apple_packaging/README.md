# Apple packaging experiment

An experiment in T11B readiness: whether the existing C interface can be
packaged for Xcode and consumed by a Swift test target running in an iOS
Simulator. **It is not acceptance evidence for T11B.** The Swift consumer
is a fixture with four tests; it is not the adapter, there is no app, and
nothing touches Core Location or GPX.

| File | What it is |
|---|---|
| `run-38237842696.log` | The whole log of the workflow run this page describes, colour codes removed |
| this file | What was run and what it showed. Written by hand from that log |

## What was run

Workflow `apple-probe.yml`, job `packaging`, started by hand, on
2026-10-12: run
[38237842696](https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework/actions/runs/38237842696),
commit `e5e3100`. An earlier run of the same steps,
38237140683 at `db60a3b`, gave the same results; two faults in its printed
diagnostics were corrected between the two, and nothing that was tested.

| | |
|---|---|
| Runner | GitHub-hosted `macos-15`, image `20260907.0337.1`, macOS 15.7.9 (24G830), arm64 |
| Xcode | 16.4 (16F6), pinned by `DEVELOPER_DIR`; Swift 6.1.2; SDK `iPhoneSimulator18.5.sdk` |
| Rust | 1.98.1 (LLVM 22.1.8), pinned by `RUSTUP_TOOLCHAIN` |
| Target | `aarch64-apple-ios-sim`, release profile, `panic="unwind"` |
| Simulator | iPhone 16 Pro, runtime iOS 18.6 (22G86) |

## Packaging route

`Scripts/apple/build-xcframework.sh`:

1. `cargo rustc -p locsim-ffi --target aarch64-apple-ios-sim --release
   --crate-type staticlib`. The manifest is unchanged; `staticlib` is a
   crate type it already declares.
2. A staging directory with `locsim.h`, copied and compared byte for byte
   with the source, and `Apple/LocsimFFI/module.modulemap`.
3. `xcodebuild -create-xcframework -library … -headers …` into
   `Apple/PackagingProbe/Artifacts/LocsimFFI.xcframework`, removing any
   earlier one first.
4. A copy of `Examples/Scenarios/walking.json` for the tests to read.

Everything written is ignored by git. `Apple/PackagingProbe` is a Swift
package with a binary target at that path, a one-function library target
and a test target.

Alternatives considered, none of them tried:

| Route | Why not chosen for the experiment |
|---|---|
| Static library and header added to an Xcode project by search paths | Works without SwiftPM, but every consumer repeats the paths and nothing checks that the slice matches the destination |
| Dynamic framework in the XCFramework (`cdylib`) | Needs an install name, embedding and, on a device, signing. More to get wrong, and nothing here needs a dynamic library |
| SwiftPM with a C target that wraps a prebuilt library | SwiftPM does not build Rust; this ends at a binary target anyway |
| A generator (UniFFI, cbindgen, swift-bridge) | Decision D18 fixed a hand-written C interface with no generator and no dependency |
| Checking the XCFramework in, or downloading a released one | The task forbids relying on an artifact that is not generated from the checkout, and nothing is distributed |

## Results, kept apart

| Stage | Result | Evidence in the log |
|---|---|---|
| Clean checkout, nothing generated | The package does not resolve: `local binary target 'LocsimFFI' … does not contain a binary artifact`, exit 74 | step "Negative 1" |
| Build of the static library | Completed in 15 s. `native-static-libs: -lSystem -lc -lm`. 6 398 168 bytes, arm64, `minos 14.0`, platform 7 (iOS Simulator) | step "Build the static library…" |
| Packaging | One slice, `ios-arm64-simulator`. Library SHA-256 `6a3d6304…921fe`, header `8834ccab…d922`. The same two digests came out of both runs | same step |
| Header against exports | 15 `locsim_` symbols, equal to the header's declarations | step "Symbols…" |
| **Compile** | 18 Swift compile steps. One warning in the whole build, from `appintentsmetadataprocessor` ("Metadata extraction skipped"), not about the library. The header was imported unmodified as module `LocsimFFI` | step "Stage 1 and 2" |
| **Link** | Two link steps; the test bundle linked with `-llocsim_ffi`. No linker diagnostic of any kind | same step |
| **Run** | 4 tests executed in the Simulator, 0 failures. The log line written by the tests: `os="Version 18.6 (Build 22G86)" model=iPhone17,1 runtime=18.6 abi=1` | step "Stage 3 and 4" |
| Damaged library, earlier build products present | The build fails at the link: `ld: unknown file type in '…/liblocsim_ffi.a'`, exit 65 | step "Negative 2" |
| Removed XCFramework, earlier build products present | The package does not resolve, exit 74 | step "Negative 3" |
| Regenerated | The four tests pass again | step "After regeneration…" |

**What "run" established about correctness** is exactly what the four
tests assert, and no more:

- the process is an iOS Simulator process;
- `locsim_abi_version()` equals `LOCSIM_ABI_VERSION` of the header, from
  the test target and from the library target;
- a session is created from `walking.json` with status `LOCSIM_OK` and a
  non-zero handle; destroying it returns `LOCSIM_OK`, and destroying it
  again returns `LOCSIM_ERR_INVALID_HANDLE`;
- `{}` is refused with `LOCSIM_ERR_SCENARIO`, a zero handle and a
  non-empty explanation length.

No sample was polled. Nothing was compared with `reference.rs`. The
simulation was not exercised on iOS.

## The diagnostics from `nm`

`nm -gU` on the library exits with status 1 and writes to standard error.
The listing of `locsim_` symbols is complete all the same. What the run
showed:

- The messages are `Unknown attribute kind (102)` and `(105)`, with
  `Producer: 'LLVM22.1.8-rust-1.98.1-stable' Reader: 'LLVM
  APPLE_1_1700.0.13.5_0'`: Xcode's LLVM reading something written by
  Rust's newer LLVM.
- The members named are objects of Rust's standard library and its
  dependencies. In the release library, 35 members, all of
  `compiler_builtins`. In a debug library, 49, across `std`, `core`,
  `alloc` and others. **None is an object of this project's crates.**
- A member named is an ordinary `Mach-O 64-bit object arm64` that also
  carries the sections `__LLVM,__bitcode` and `__LLVM,__cmdline`.
- With `nm --no-llvm-bc`, which turns the bitcode reader off, the exit
  status is 0, nothing is written to standard error, and the 15 symbols
  are the same.

So what `nm` cannot read is the copy of LLVM bitcode that Rust's
distributed standard library embeds beside its machine code, not the
machine code or the symbol table.

What this does and does not settle about linking:

- In this run the linker read the same archive and linked a test bundle
  that ran, with no diagnostic. That is evidence for this link: Xcode
  16.4, a debug build of the consumer, no link-time optimisation on the
  Xcode side.
- **Not tested:** a consumer built with link-time optimisation, where the
  linker would have a reason to read bitcode; a release or archive build
  of an app; App Store processing; a device build. A consumer that turns
  LTO on is the case most likely to meet the same version mismatch, and
  it is unverified.
- The cause was not removed. Nothing was stripped, and no diagnostic was
  suppressed: the symbol check reports the status of `nm` and compares
  the listing.

## Limits of this experiment

- One slice. There is no device slice (`aarch64-apple-ios`) and no Intel
  Simulator slice; a build for a generic Simulator destination notes that
  the artifact "is missing architecture(s) required by this target
  (x86_64)".
- `module.modulemap` sits at the root of the slice's `Headers`. Xcode
  copies such headers into one shared directory, so two XCFrameworks laid
  out this way in one app would collide. Not a problem for one, and not
  solved here.
- The package's `platforms: [.iOS(.v16)]` is a placeholder. The library
  was built with a minimum of iOS 14.0 by the target's default. The
  minimum iOS version of the project is not decided.
- The Simulator SDK was 18.5 and the runtime 18.6. That is the pairing
  Xcode 16.4 has on this image; it was not chosen.
- The runner is hosted. Its image, Xcode list and runtimes change without
  notice; the job fails if a pinned piece is missing.
- A test target is not an app. No app was built, signed, installed or
  launched.
