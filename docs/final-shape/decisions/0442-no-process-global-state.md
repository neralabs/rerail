# 0442. No process-global state: the run, the connection, and a shared object the caller passes

Status: accepted (2026-11-19). The user: "we want the engine to be re-entrant, multi-thread possible and to not have
global state"; "How can they stay global when multiple runs could be happening at once?"; and on what is shared by
design: "things that are shared by design need to come from an explicit "shared by design" object that is passed.
Otherwise we can't maintain security boundaries."

## Intro

The engine kept much of its state in statics: 563 non-constant statics in its source (test files left out), beside
its thread-locals. Two runs at once in one process collide in many of them: one tape for the native extension,
results by name, phase 1's batch, testing mode's store, a store's stored tables dropped by another run's start,
measurement sums. Settings read once a process go stale when a caller changes the environment between commands.
And what is shared on purpose (a store's writer, a learned cost model, the executors, the interners) was reachable
by any caller in the process, so two callers of different trust could not be kept apart.

## Decision

1. **The test is two runs at once, and two callers of different trust**, not whether a thing is the process's by
   nature.
2. **Where state lives:**
   - **The run** (`logic::run::Run`, decision 0441): a run's results, batches, settings (read as it starts) and
     measurements. Made by the outermost run's scope, carried to its jobs, returned in its record.
   - **The connection**: a connection's own state (components, derived and input relations, temporary tables),
     on the connection's object, not in a process map keyed by connection.
   - **The shared object** (`Shared`): what is shared by design: each store's resources (its writer, handles,
     atoms, frames, observers, journal, symbols), the learned models and their queues, the executors, the
     interners, the caches 0104 has not yet moved into facts. A caller makes it and passes it in (`tools::cli_in`,
     the Node addon, the reactor, a test). A run takes it at its scope and reaches it through the run, never through
     a static; a run with none is refused, so there is no implicit process default. Two callers with two shared
     objects reach nothing of each other's.
3. **What may stay static:** constants; code a crate above registers at load (a function pointer, a device); a
   value computed once from the engine's own files (a regex of a literal, core.rr's declarations). A thread's own
   scratch stays a thread-local (`run_state::STATE`, class Thread).
4. **The guard.** `scripts/check/check-no-global-state.py` (pre-commit) lists every non-constant static in the
   engine's source against `scripts/check/global-state.txt`, each by its class: `stays`, `run`, `connection`,
   `shared`, or `review` (not yet classified by hand). A static not listed is refused; one gone must be cut. Every
   class but `stays` only shrinks.

## Order

1. Settings, logs and measurement sums into the run (`run::Local`).
2. The run's results and batches (the native extension, results by name, phase 1's batch, testing mode's store,
   stored tables, frame counts).
3. The profile into the run.
4. Per-connection state onto the connection.
5. `Shared`: made by the caller, passed through every entry point, carried by the run; the store's resources,
   models, executors, interners and caches into it.
6. The `review` entries, classified by hand and moved.

## Open

- The baseline (2026-11-19): 563 statics: 78 stay, 109 run, 9 connection, 41 shared, 326 review.
- Thread-locals of class Carried are the run's context in thread-locals: they fold into `Run` once it holds the rest.
- Step 3 (the profile), as done:
  - A run's records are the run's (`profile::Records`, a run local). Each thread's shard of them is listed in the run
    it records for. Outside a run nothing keeps them: a test that reads a profile holds a run (`run::scope`, or
    `profile::test_run` in the crates below `logic`).
  - The run thread's phases are the run's. The command's clocks are the command's thread's (`COMMAND`, Outermost).
  - Whether to profile, and where to write the report, are the caller's: thread-locals carried to its runs' jobs
    (`ON`, `DIR`). `profile::carried` carries the run and the switch into a pool thread's item too.
  - The process lock that let one profiled run at a time (`one_run`) is gone. A corpus run holds its run's records
    (`hold_records`); one nested in it neither resets nor writes them.
  - The threads' CPU accounts (`THREADS`, `EXITED`, `EXITED_GAPS`) are the pool's, which is shared by design. They
    move with the pool into `Shared` (step 5).
  - Not done yet: `WAVES` (the process's count of waves, for a thread with no run's context) and `WARM_FAULTS` (the
    process's exit status under `RERAIL_WARM=list`).
