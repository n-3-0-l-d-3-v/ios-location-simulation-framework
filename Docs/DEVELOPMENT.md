# Development

Requirements: Rust ≥ 1.75 (`rustup`), with `rustfmt` and `clippy`. No Apple
tooling is needed for the Rust crates. The C harness needs Windows with
MSVC (Visual Studio's C++ tools) and MinGW-w64 gcc, and a POSIX shell (Git
Bash or MSYS2).

| Task | Command |
|---|---|
| All tests | `./Scripts/test.sh` |
| One module | `cargo test -p locsim-core geographic` |
| Property tests only | `cargo test -p locsim-core --test geographic_props` |
| Lint | `./Scripts/lint.sh` |
| Pre-commit gate | `./Scripts/validate.sh` |
| C harness (not part of the gate above) | `crates/locsim-ffi/harness/run.sh` (both compilers), or `... run.sh msvc`, `... run.sh mingw` |

## Conventions

- Units are in names: `_m`, `_mps`, `_deg`, `_dps`, `_s`. Angles are degrees at
  public APIs, radians only inside geodesy functions.
- No silent defaults for critical parameters: parameter structs have no
  `Default`; validation returns every problem as a `ConfigError { field, reason }`.
- `locsim-core` has no dependencies and stays that way. No new dependency in
  any crate without a written justification in that crate's `Cargo.toml`
  (`locsim-scenario` carries one for `serde` and `serde_json`). Randomness
  comes only from `rng::Rng` (seeded).
- The C interface (`crates/locsim-ffi`): `unsafe` is allowed in this crate
  only, each block with a `SAFETY` comment (Clippy enforces it). A test that
  creates a session first takes the `shared()` guard, or `alone()` if it
  needs to know what the registry holds; both destroy what the test created
  when they are dropped. **`cargo test` does not run the C harness**: after
  any change to the crate or to `include/locsim.h`, run
  `harness/run.sh` too. It builds the library in debug and release, checks
  that the DLL exports exactly what the header declares, compiles
  `harness/harness.c` with each compiler, runs it, and compares its samples
  with `examples/reference.rs`. It fails if a compiler is missing. Any
  change to the header needs a new `LOCSIM_ABI_VERSION`, and
  `tests/header.rs` lists every constant.
- Health: `cargo test -p locsim-health -- --nocapture` prints the measured
  figures. `tests/support/mod.rs` has a provider that follows a script, a
  real provider made to fail through `ModelFactory`, and two checks
  (`assert_consistent`, `assert_reconciles`) that every test applies after
  every step through `check`. New tests should do the same.
- Persistence: `cargo test -p locsim-store -- --nocapture` prints the
  measured figures. `tests/crash.rs` starts the test binary as a child
  process and kills it; `tests/platform.rs` has a Windows module and a Unix
  module, and only the one for the host is compiled. Keep high-volume
  corruption tests in memory: thousands of real file writes are slow.
- Scenario documents: `cargo test -p locsim-scenario`. The files in
  `Examples/Scenarios/` must stay byte-identical to their own export; a test
  checks it, and a new file there must be added to
  `crates/locsim-scenario/tests/support/mod.rs`.
- Property tests use fixed seeds and print the failing case index.
- Workflow per ticket: implement → test → `validate.sh` → commit and push each
  logical change → update `PROGRESS.md` and the context documents. The full
  rules are contract C17 in
  [PROJECT_CONTEXT/ENGINEERING_CONTRACTS.md](PROJECT_CONTEXT/ENGINEERING_CONTRACTS.md).
- The declared minimum Rust version (1.75) has not been tested; development
  has only used 1.98.1.
