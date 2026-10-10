# Progress

Ticket-level log and the source of truth for which ticket is current. For the
long-lived project context, the invariants to preserve and how to resume in a
new session, start at [Docs/PROJECT_CONTEXT/README.md](Docs/PROJECT_CONTEXT/README.md).

## Current Ticket
T10 — Health system (not started)

## Status
T00–T09 complete and tested on Windows. No iOS code exists yet.
Repository: https://github.com/n-3-0-l-d-3-v/ios-location-simulation-framework

## Completed
- T00 Repository bootstrap.
- T01 Domain model.
- T02 Geographic engine (Vincenty, ENU/ECEF, seeded PRNG).
- T03 Fixed location engine (scheduler, validator, provider).
- T04 Noise engine.
- T05 Movement engine (circular, steered: random walk / walking / driving).
- T06 Route engine (ECEF spline, rigorous admission, RouteModel).
- T07 Consistency engine.
- T08 Scenario system (`crates/locsim-scenario`: strict, versioned JSON).
- T09 Persistence. Details of T00–T08 are in
  `Docs/PROJECT_CONTEXT/TICKET_HISTORY.md`. T09 delivered:
  - **Scope decided by the project owner:** (A) scenario persistence and
    (B) a last-known-output record. (C) checkpoint/resume is deferred to a
    separate ticket that nobody owns yet. "Configuration" and "preferences"
    from the ticket text are not stored: no such type exists.
  - `locsim-scenario` gained the text form of the second record:
    `LastKnown`, `export_last_known` / `import_last_known` (record version
    1, as strict as the scenario codec) and `scenario_fingerprint` (FNV-1a
    64 of the exported text).
  - New crate `crates/locsim-store`, standard library only:
    - `Store::open(dir)`, `save_scenario` / `load_scenario`,
      `save_last_known` / `load_last_known`, `stale_temporaries` /
      `remove_stale_temporaries`.
    - Files are an envelope: `locsim-store 1`, `payload-sha256`,
      `payload-bytes`, an empty line, then the payload byte for byte. The
      digest is verified before the payload is interpreted. SHA-256 is
      implemented in the crate.
    - Atomic replacement: temporary file in the same directory, write,
      sync, read back and compare, rename, directory sync where possible.
    - `StoreError`: not a directory; I/O with the failed operation;
      invalid value (nothing written); corrupt (envelope / not UTF-8 /
      document refused; file untouched); verify mismatch; temporary-name
      collision. A missing file is `Ok(None)`.
  - `locsim-core` is unchanged. No engine, gate or domain code was touched.

## Current Work
- None in flight.

## Tests
Last full run (2026-10-10, Windows 11, rustc 1.98.1, commit `29c8342`):
**431 passed, 0 failed.** `cargo fmt --check` clean;
`cargo clippy --all-targets --all-features -- -D warnings` clean.

| Crate | Suite | Tests |
|---|---|---|
| `locsim-core` | Unit (in `src/`) | 202 |
| `locsim-core` | nine integration suites (unchanged since T07) | 71 |
| `locsim-scenario` | Unit (in `src/`) | 56 |
| `locsim-scenario` | `examples` | 11 |
| `locsim-scenario` | `last_known` | 5 |
| `locsim-scenario` | `malformed` | 15 |
| `locsim-scenario` | `round_trip` | 2 |
| `locsim-store` | Unit (in `src/`) | 38 |
| `locsim-store` | `scenario_store` | 15 |
| `locsim-store` | `last_known_store` | 10 |
| `locsim-store` | `platform` | 4 on Windows (3 more under `cfg(unix)`, never run) |
| `locsim-store` | `crash` | 2 |

T09 figures, printed by the tests (`cargo test -p locsim-store -- --nocapture`):
- Damage: 31 786 single-character changes to the eight example scenarios.
  As bare documents 3 340 are accepted, 1 357 of them as a *different valid
  scenario*. Inside the envelope: none. For the last-known record: 19 505
  changes, 942 accepted bare, none stored.
- Every truncation and every single bit flip of a stored file is detected.
- Fault injection: a save made to fail at each of seven steps (create,
  partial write as a full disk, sync, read back, read back differing, read
  back short, rename) leaves the old record byte-identical and no temporary
  file; with the temporary file made unremovable, the original error is
  still reported with the leftover named.
- Killed writer: 40 writer processes killed at random moments; the record
  was whole and valid every time; 5–10 kills per run landed inside a save.
- Concurrent reading on Windows: 240 saves from four threads, about 2 200
  loads by a reader, every one a whole record, none missing, none refused.
- One synced save costs about 3.6 ms on this machine.
- 20 000 arbitrary last-known records round-trip bit-exactly.
- Mutation checks, all caught and restored: null speed read as zero; record
  import without the sample check; fingerprint ignoring a byte; read-back
  difference ignored; write error ignored; sync error ignored; temporary
  not removed; digest not compared; length not compared; missing file
  reported as an error; unreadable file treated as missing; invalid
  scenario written anyway.

Findings during T09:
- **Validation is not integrity.** A codec cannot tell `1.8` from `1.3`.
  About 4 % of single-character changes to a bare scenario document give a
  different valid scenario. This is why stored files carry a digest.
- **The provider is not a function of scenario and time.** It is a
  deterministic function of the scenario and the whole sequence of calls.
  Two document statements said more than that and were corrected (D4 and
  the provider section of `Docs/ARCHITECTURE.md`).
- **A read-only record cannot be replaced on Windows** (access denied at
  the rename), unlike Unix. Found by a test whose expectation was wrong.
- **The killed-writer test is weak on its own.** With the rename replaced
  by a non-atomic copy, it still passed (no kill landed in the short copy);
  the concurrent-reader test caught the defect. The step-by-step guarantee
  rests on fault injection.
- Test defects of mine, corrected against derivation: a payload length
  miscounted by one; a faulty file system also used for the check that was
  meant to be independent of it.

## Known Issues / Limitations
- **Persistence, not verified:**
  - Power loss or an operating-system crash. Nothing was tested; the
    statements about durability describe what is requested of the OS.
  - Any Unix-like system: the replacement path, directory sync and the
    three `cfg(unix)` tests have never been compiled or run.
  - Any file system but NTFS on one machine; network file systems.
- **Persistence, by design:**
  - On Windows the change of name is not confirmed durable (`DIRECTORY_SYNC`
    is `false`): after a successful save, a power loss may leave the old
    record. Closing this needs `MOVEFILE_WRITE_THROUGH`, which needs
    `unsafe` FFI or a dependency; not done.
  - Microsoft does not document the replacement as atomic. A reader may get
    an I/O error during a replacement; none was observed.
  - One writer per directory. No lock; two writing processes are unsupported.
  - No backup generation. A successful save replaces the record.
  - The digest detects accidental damage only. It is not authentication.
  - The last-known record cannot resume a run. Checkpoint/resume is not
    implemented and no ticket owns it.
  - A record held open by another Windows program without delete sharing,
    or marked read-only, makes saves fail (cleanly) until that changes.
  - Stored files are not bare JSON; a hand-edited payload fails the digest.
    To change a stored scenario: import the document, then save it.
  - `scenario_fingerprint` changes if the exported text of a scenario ever
    changes (a new schema version), so records do not match across that.
- **History:** commit `019ad0f` (T08 examples) carries the message of
  `7547565` by mistake. History is left as it is.
- Scenario documents (T08): no defaults; seeds are strings; import does not
  admit a route; validation errors have a path but no line or column; only
  schema version 1 exists, so no real migration has ever run.
- `rust-version = "1.75"` is unverified for all three crates.
- Carried over from T07 and earlier: speed is an interval mean; the first
  sample of a run has no speed or course; the noise engine computes a speed
  and course nothing uses; a provider in `Error` stays there (T10); raw GPS
  logs are usually not admissible as routes; no real-time driver; nothing
  has run on iOS.

## Compatibility
- All three crates: Verified on Windows 11 x86_64 / rustc 1.98.1 only.
- `locsim-store` on Unix-like systems: Untested, including code that only
  compiles there.
- iOS Simulator, physical devices, jailbroken devices: Untested (no adapter yet).

## Next
- T10 Health system: health state, metrics, watchdog, bounded recovery
  (for example three retries with backoff, then FAILED), structured logging
  with levels.
- What T10 can build on: `ProviderStatus` counters; the `Error` and
  `Recovering` states that exist but nothing drives; `StoreError` as a
  structured failure source; the fact, established in T09, that a run
  cannot be resumed, so "recovery" can only mean a new run.
- Decisions T10 needs: see `Docs/PROJECT_CONTEXT/CURRENT_STATE.md`.
- Gotchas:
  - Position and timestamp are authoritative; never emit speed/course from a
    model.
  - Gate is strict; engines keep `KINEMATIC_MARGIN` below limits.
  - Models receive simulated time, not wall time.
  - Gate every commit on its test result; unstage on failure.
  - Write the commit message file, read its first line back, and only then
    commit, as separate steps (`019ad0f`).
  - A file written by a shell heredoc containing backslashes or a lone
    apostrophe may not be what was typed on this machine; write such files
    with a tool, and derive expected numbers by hand.
  - Tests that write thousands of files are slow here (about 6 ms per
    write); keep high-volume corruption tests in memory.

## Working agreement
- One ticket at a time; the specification is the contract.
- One commit per logical change, each tested and pushed immediately.
