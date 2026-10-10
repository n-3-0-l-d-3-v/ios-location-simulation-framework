# Project context — index

This directory exists so that work on this project can continue in a brand-new
conversation (human or AI) that has none of the history. It holds the
long-lived context: what the project is, why it is built the way it is, what
must not be broken, what has been done, and how to resume.

It was written on 2026-10-09 at commit `1be8852`, after ticket T07, and
updated on 2026-10-10 at commit `fcda25c`, after ticket T10 and before T11.
**Everything here can go stale. The repository is the truth; these
documents are a map of it.**

## Reading order for a new session

| # | Document | What it gives you | Read when |
|---|---|---|---|
| 1 | [CURRENT_STATE.md](CURRENT_STATE.md) | Where things stand, the next ticket, the checklist to run before touching code | Always, first |
| 2 | [PROJECT_BRIEF.md](PROJECT_BRIEF.md) | Purpose, scope, non-goals, safety boundaries, the full ticket plan | Always |
| 3 | [ENGINEERING_CONTRACTS.md](ENGINEERING_CONTRACTS.md) | The invariants every ticket must preserve, with the source and tests that enforce them | Always, before changing code |
| 4 | [ARCHITECTURE_DECISIONS.md](ARCHITECTURE_DECISIONS.md) | Why each layer is the way it is; rejected alternatives | Before any design decision |
| 5 | [TICKET_HISTORY.md](TICKET_HISTORY.md) | T00–T10: what each did, its commits, its mistakes | When you need to know how something got here |
| 6 | [NEXT_CHAT_PROMPT.md](NEXT_CHAT_PROMPT.md) | The copy-paste prompt that starts a fresh session | To start a session |

Outside this directory:

| Document | Role |
|---|---|
| [../../PROGRESS.md](../../PROGRESS.md) | Ticket-level log: current ticket, last test run, known issues, next step. Updated at the end of every ticket. |
| [../ARCHITECTURE.md](../ARCHITECTURE.md) | The detailed technical description of every implemented module. |
| [../COMPATIBILITY.md](../COMPATIBILITY.md) | What has actually been run where. |
| [../DEVELOPMENT.md](../DEVELOPMENT.md) | Commands and conventions. |

## Which source wins when they disagree

1. **The repository itself** — code, tests, and what `git log` and a fresh
   test run say. Always.
2. **`PROGRESS.md`** for "what ticket are we on and what was the last verified
   result".
3. **`Docs/ARCHITECTURE.md`** for how a module works in detail.
4. **`ENGINEERING_CONTRACTS.md`** for what must hold. If code contradicts a
   contract, that is a bug in one of them: find out which, do not assume.
5. **`PROJECT_BRIEF.md`** for scope and safety boundaries. These do not go
   stale; they are decisions, not observations.
6. `ARCHITECTURE_DECISIONS.md` and `TICKET_HISTORY.md` are history. They
   explain; they do not override the above.
7. Anything a previous conversation *said* that is not in the repository is
   not a fact.

If you find a disagreement, fix the document (in its own commit) and say so in
your handoff. Do not rewrite git history to hide it.

## What kinds of statement are in these documents

Each document labels its claims. The labels mean:

- **Verified** — checked against the repository or by running a command, on
  the date stated. Re-verify before relying on it.
- **Reported** — a number or outcome taken from a test's printed output or
  from an earlier session's report, not re-measured when this was written.
  Reproducible, but not re-checked.
- **Decision** — something chosen. True until someone changes it on purpose.
- **Planned** — intended future work. Nothing exists yet.
- **Unverified** — believed or declared, never tested.

"Implemented" means code exists. "Tested" means automated tests pass on a
desktop host. Neither means it works on iOS. **Nothing in this project has
been run on an iPhone, an iOS Simulator, or any Apple platform.**

## Current state in five lines

- Tickets T00–T10 are complete: a dependency-free Rust simulation core
  (`crates/locsim-core`); strict, versioned JSON for scenarios and for a
  last-known record (`crates/locsim-scenario`); file storage of both with a
  digest and atomic replacement (`crates/locsim-store`); and a supervisor
  with a watchdog, bounded restart and events (`crates/locsim-health`).
- Verified at `fcda25c` on 2026-10-10 (Windows 11, rustc 1.98.1): 516 tests
  pass, `cargo fmt --check` and `cargo clippy -D warnings` are clean.
- There is no iOS code of any kind: no C ABI, no Swift, no app. Nothing has
  run on any system but one Windows machine.
- A stored or failed simulation cannot be resumed, only started again.
  There is no timer or real-time driver: everything is polled.
- **Next ticket: T11 — Platform adapter. Not started, and not completable
  on the Windows machine used so far: it needs a Mac with Xcode.**
