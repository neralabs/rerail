# 0204: A session is its caller's, not the process's

Status: built (2026-11-14).

## Intro

**The problem.** A corpus session (decision 0063) was one process global, `static SESSION`. That followed
from the time there was one database, which tests and the product shared. A session already owns its
own store (`session.sqlite` in its directory), its memo and its generation, so nothing needs it to be
the process's. The global had three costs:

- **Every corpus run waited for the one before.** `run_handed` took `session::current()` once and kept
  the guard to its end. That guard held the global mutex even with no session started, or with
  another thread's session. So every corpus run in a process ran one at a time: in the test binary,
  and in any product process that runs two at once.
- **Tests queued.** A test build kept a thread check, so that the other tests could run beside the
  session's thread, plus a process-wide mutex, `SESSION_TESTS`. About 80 test sites took it, whether
  they started a session or only had to not see one. The 32 session and corpus tests ran one after
  another, and that chain was about half of the library suite's wall time
  (`notes/t37-edit-loop-time.md`).
- **`full` was process-wide.** It was an `AtomicBool`, so one thread's full run made every other
  thread's runs full ones too.

**The decision.** A session is a value its caller owns: `session::Handle`. The caller binds it to its
thread for its runs (`Handle::bind`, or `session::start(dir)`, which opens and binds). A run uses the
session bound to its thread, if there is one, and holds only that session's lock. `full` is per
thread. Nothing about a session is the process's, so two sessions run side by side.

## How

- `Handle::open(dir)` opens a session: its store, memo and generation. The generation sits outside
  the lock, so a run can name its session (`generation()`) before it holds it.
- `Handle::bind()` returns a guard. While the guard lives, the thread's runs are the session's; when
  it is dropped, the thread's runs are as they were before.
- `current()` returns `None` for phase 0's runs, for full runs, and on a thread with no session bound.
  A run of no session holds nothing.
- The callers that keep a session across commands own it, as `session::Kept` (the directory
  `RERAIL_SESSION` names):
  - the browser's reactor, as instance state;
  - `repeat`, for the length of its script;
  - `tools::cli_in`, which takes the caller's `Kept`. `tools::cli` makes one for a single command.
- Tests open their own sessions (`let session = start(dir)`, `drop(session)`). `SESSION_TESTS` is gone,
  and each session test's scratch directory is unique (`fixtures::temp()`). Three tests shared two
  helpers' directories, which named only the process.
- Two counters that tests read, `maintain::strata_by_levels` and `pieces_together`, are counted per
  thread, not per process. A test reads its own runs' counts while other tests run beside it.

## Checked

- The engine's `logic::plan::corpus`, `typing::oracle_tests` and `typing::phase_tests` (129 tests),
  now run in parallel:
  - 107–123 s wall and about 215 s CPU, at load 70–90;
  - 110 pass, with 19 ignored as before.
- The CLI, and the test targets of rerail-logic, rerail-sessions and rerail-device-metal, build.

## Open

**`phase0::tests::bootstrap_resolves_as_the_builtin_reader_does` fails in about one concurrent run in
three.** It passes alone. Its graph from `derive` lacks a module's implicit `core.rr` edge and its
absent candidates, which the kept graph (`graph`) has. The whole-run cutoff was not applied in the
traced runs, so the cause is another state phase 0's runs share. Candidates:

- the shared root, `rerail-phase-0-root`;
- the default tape cache;
- the observer's journal.

Until now the global lock hid it: corpus runs never overlapped.
