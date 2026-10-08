# Compatibility

Status vocabulary: **Verified** (run on that exact combination, results
recorded here), **Partially Verified**, **Untested**, **Unsupported**.
A row is never upgraded on the basis that it "should work".

## Simulation core (`locsim-core`)

| Host | Toolchain | Status | Notes |
|---|---|---|---|
| Windows 11 x86_64 | rustc 1.98.1 | Verified | Full test suite, 2026-10-08 |
| macOS (arm64 / x86_64) | — | Untested | |
| Linux x86_64 | — | Untested | |
| iOS arm64 (`aarch64-apple-ios`) | — | Untested | Not yet cross-compiled |

## Delivery on Apple platforms

No platform adapter exists yet (ticket T11), so every row is Untested.

| Device | SoC | iOS | Arch | Simulator | Physical device | Jailbreak required | Known limitations | Status |
|---|---|---|---|---|---|---|---|---|
| iOS Simulator (any) | host | — | arm64/x86_64 | planned: `simctl location`, GPX | n/a | No | Affects the whole simulator, not one app | Untested |
| Development device | any | — | arm64 | n/a | planned: Xcode GPX / in-app provider injection | No | In-app injection only affects the app built with the adapter | Untested |
| Research device (jailbroken) | — | — | arm64/arm64e | n/a | not designed yet | Yes | No hardware available to the project so far | Untested |
