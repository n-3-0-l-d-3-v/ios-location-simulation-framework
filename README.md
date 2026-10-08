# iOS Location Simulation Framework

A synthetic location framework for authorized privacy testing, application
development and controlled research. It generates a coherent, deterministic,
validated location stream (fixed point, bounded random walk, walking, driving,
orbit, route replay) and delivers it through a replaceable platform adapter.

**Status: early.** The domain model and geographic engine are implemented and
tested. Nothing has been run on an iPhone or the iOS Simulator yet — see
[PROGRESS.md](PROGRESS.md) and [Docs/COMPATIBILITY.md](Docs/COMPATIBILITY.md).

## Design in one paragraph

The simulation core is a dependency-free Rust crate (`crates/locsim-core`)
that builds and tests on Windows, macOS and Linux with no Apple tooling. iOS is
reached through a thin C ABI and a Swift adapter that converts the core's
`SyntheticLocation` into `CLLocation`; no simulation mathematics lives on the
platform side. See [Docs/ARCHITECTURE.md](Docs/ARCHITECTURE.md).

## Out of scope

This project does not implement, and will not accept, mechanisms meant to hide
the framework or a jailbreak from security software, bypass application
integrity checks, or defeat third-party anti-spoofing systems.

## Quick start

    ./Scripts/validate.sh    # fmt + clippy + all tests

See [Docs/DEVELOPMENT.md](Docs/DEVELOPMENT.md).

## License

MIT — see [LICENSE](LICENSE).
