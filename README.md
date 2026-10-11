# iOS Location Simulation Framework

A synthetic location framework for authorized privacy testing, application
development and controlled research. It generates a coherent, deterministic,
validated location stream (fixed point, bounded random walk, walking, driving,
orbit, route replay) and delivers it through a replaceable platform adapter.

**Status: simulation core, scenario documents, persistence, health
supervision, and a C interface to a supervised session. Nothing for any
Apple platform yet.** Tickets T00–T10 are done: the Rust core generates,
validates and emits a consistent synthetic location stream; scenarios can
be imported from and exported to strict, versioned JSON
(`crates/locsim-scenario`, examples in `Examples/Scenarios/`); a scenario
and the last emitted sample can be stored in files with a digest and
replaced atomically (`crates/locsim-store`); a supervisor watches a
provider, restarts a failed run within a bound and reports health and
events (`crates/locsim-health`). A stored or failed simulation cannot be
resumed, only started again from its beginning.

Ticket T11, the platform adapter, is open and in two parts. T11A, a C
interface to one supervised session (`crates/locsim-ffi`, header
`crates/locsim-ffi/include/locsim.h`), is implemented and awaiting its
closing review; it has been called from Rust tests and from a C program
built with MSVC and with MinGW gcc. T11B, the Apple side, has not started
and needs a Mac with Xcode: there is no Swift adapter, no app, and nothing
has been built for or run on an iPhone, the iOS Simulator or a Mac.

All of it is tested on one Windows desktop host and nowhere else.
See [PROGRESS.md](PROGRESS.md), [Docs/COMPATIBILITY.md](Docs/COMPATIBILITY.md)
and, for the full project context and how to resume work in a new session,
[Docs/PROJECT_CONTEXT/README.md](Docs/PROJECT_CONTEXT/README.md).

## Design in one paragraph

The simulation core is a dependency-free Rust crate (`crates/locsim-core`)
that needs no Apple tooling to build and test (verified on Windows only so
far; macOS and Linux are untested). iOS is
planned to be reached through a thin C interface (`crates/locsim-ffi`,
ticket T11A) and a Swift adapter that converts each sample into a
`CLLocation` (ticket T11B, not started); no simulation mathematics will
live on the platform side. An app built with the adapter would receive
synthetic locations; the system's location for other apps is not replaced. See [Docs/ARCHITECTURE.md](Docs/ARCHITECTURE.md).

## Out of scope

This project does not implement, and will not accept, mechanisms meant to hide
the framework or a jailbreak from security software, bypass application
integrity checks, or defeat third-party anti-spoofing systems.

## Quick start

    ./Scripts/validate.sh                  # fmt + clippy + all Rust tests
    crates/locsim-ffi/harness/run.sh       # the C harness (Windows; not run by the above)

See [Docs/DEVELOPMENT.md](Docs/DEVELOPMENT.md).

## License

MIT — see [LICENSE](LICENSE).
