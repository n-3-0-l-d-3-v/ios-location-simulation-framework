# Development

Requirements: Rust ≥ 1.75 (`rustup`), with `rustfmt` and `clippy`. No Apple
tooling is needed for the simulation core.

| Task | Command |
|---|---|
| All tests | `./Scripts/test.sh` |
| One module | `cargo test -p locsim-core geographic` |
| Property tests only | `cargo test -p locsim-core --test geographic_props` |
| Lint | `./Scripts/lint.sh` |
| Pre-commit gate | `./Scripts/validate.sh` |

## Conventions

- Units are in names: `_m`, `_mps`, `_deg`, `_dps`, `_s`. Angles are degrees at
  public APIs, radians only inside geodesy functions.
- No silent defaults for critical parameters: parameter structs have no
  `Default`; validation returns every problem as a `ConfigError { field, reason }`.
- No new dependency in `locsim-core` without a written justification in the
  crate's `Cargo.toml`. Randomness comes only from `rng::Rng` (seeded).
- Property tests use fixed seeds and print the failing case index.
- Workflow per ticket: implement → test → `validate.sh` → commit and push each
  logical change → update `PROGRESS.md` and the context documents. The full
  rules are contract C17 in
  [PROJECT_CONTEXT/ENGINEERING_CONTRACTS.md](PROJECT_CONTEXT/ENGINEERING_CONTRACTS.md).
- The declared minimum Rust version (1.75) has not been tested; development
  has only used 1.98.1.
