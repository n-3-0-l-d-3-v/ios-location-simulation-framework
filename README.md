# iOS Location Simulation Framework

A synthetic location framework for authorized privacy testing, application
development and controlled research. It generates a coherent, deterministic,
validated location stream (fixed point, bounded random walk, walking, driving,
orbit, route replay) and delivers it through a replaceable platform adapter.

**Status: simulation core, scenario documents and persistence only.**
Tickets T00–T09 are done: the Rust core generates, validates and emits a
consistent synthetic location stream; scenarios can be imported from and
exported to strict, versioned JSON (`crates/locsim-scenario`, examples in
`Examples/Scenarios/`); a scenario and the last emitted sample can be stored
in files with a digest and replaced atomically (`crates/locsim-store`). A
stored simulation cannot be resumed, only started again. All of it is tested
on one Windows desktop host. There is no iOS code yet — no C ABI, no Swift
adapter, no app — and nothing has been run on an iPhone or the iOS Simulator.
See [PROGRESS.md](PROGRESS.md), [Docs/COMPATIBILITY.md](Docs/COMPATIBILITY.md)
and, for the full project context and how to resume work in a new session,
[Docs/PROJECT_CONTEXT/README.md](Docs/PROJECT_CONTEXT/README.md).

## Design in one paragraph

The simulation core is a dependency-free Rust crate (`crates/locsim-core`)
that needs no Apple tooling to build and test (verified on Windows only so
far; macOS and Linux are untested). iOS is
planned to be reached through a thin C ABI and a Swift adapter that converts
the core's `SyntheticLocation` into `CLLocation` (ticket T11, not started);
no simulation mathematics will live on the platform side. See [Docs/ARCHITECTURE.md](Docs/ARCHITECTURE.md).

## Out of scope

This project does not implement, and will not accept, mechanisms meant to hide
the framework or a jailbreak from security software, bypass application
integrity checks, or defeat third-party anti-spoofing systems.

## Quick start

    ./Scripts/validate.sh    # fmt + clippy + all tests

See [Docs/DEVELOPMENT.md](Docs/DEVELOPMENT.md).

## License

MIT — see [LICENSE](LICENSE).
