# 0394. No runaway: a width cap per relation, and a memory ceiling

Status: accepted (2026-11-18; the user: "find a way to make it impossible for runaway memory use to happen").

## Intro

A rule whose head is huge fills memory until the machine gives out. Ruff's SLF001 as written ran past 56 GB on
1,644 PostHog files (0393). Runaways like this had crashed the development machine twice, so every run there went
through an outside watchdog.

Replanning (0203 §3a) cannot help: no join order shrinks a head. 0393 bounds the one shape we know (a node
product), but other rules can still blow up. The engine itself has to refuse to grow without limit, and say
which rule was growing.

## Decision

1. **A width cap per derived relation.** `RERAIL_WIDEST_ROWS` (default 50,000,000; `off` or 0 for none) is the
   most rows a derived relation may hold. A join walk checks it as it makes each head row (`setfix`'s `enter`).
   Parallel morsels check it again when their rows merge, and edge rules check it too. The check counts the
   relation's rows plus the wave's new rows, some of which may repeat, so it is an upper bound.
2. **What happens at the cap**, by `RERAIL_WIDEST`:
   - `stop` (the default): the run stops with an error. The error names the relation, then each rule deriving
     it, written in the order it ran with the rows into and out of each step, then what to do: bound it by a
     join, raise the cap, or cut.
   - `cut`: the relation keeps its rows, and the probe that reached the cap derives nothing more this run. The run
     goes on. Its results are incomplete, so the command fails at its end, naming each relation cut: `tools::cli_in`
     returns the message beside the result, and the binary and the reactor exit with code 3.
3. **A memory ceiling per run.** Each outermost run watches its own ceiling: a thread of the run's reads resident
   memory every 50 ms. Past `RERAIL_MEMORY_MAX_MB` (default: three quarters of the machine's memory; `off` for
   none), the run is told to stop. Its walks and waves stop with an error (`OverMemory`) that names the widest
   derived relations it held (those past a million rows). Resident memory is the process's, so a run beside
   another stops when the two together pass its ceiling.
4. **A backstop for the paths that do not stop when told.** The binary's `profile::watch` reads resident memory
   too. Once it has stayed past the ceiling for 2 s after the run was told to stop (a SQL stratum's insert, the
   device, a cache), it dumps the run (as a deadline does) and exits with code 3. It reads no engine state.
5. **All of it is the run's** (the user, 2026-11-19: "we want the engine to be re-entrant, multi-thread possible and
   to not have global state"). The settings, the cut list, the widest relations and the memory watch are a
   `width::Run`, made by the outermost run's scope (`width::scope`: a command, a corpus run or a program run,
   whichever starts first on the thread) and carried to its jobs (`run_state`). A nested run joins it. The record
   comes back with the result (`Outcome::limits`, `corpus::Report::limits`, `cli_in`'s message). Two runs on two
   threads have their own; nothing is process state, and a test sets a run's limits by holding its scope.

`--memory-mb` (the budget chunks and residency plan by) is unchanged. The cap and the ceiling are limits, not
plans.

## Tests

- `width::tests`: the cap's setting, and its error's text; each run has its own record, and a nested run shares
  its outer run's; runs on two threads keep apart; the memory watch stops a run past its ceiling, naming the
  widest relation.
- `plan::tests::a_relation_past_the_width_cap_stops_the_run_or_is_cut`: a 100 × 100 product under a 1,000-row
  cap, run with native strata.
  - Stopped, the error names `pairs` and its rule.
  - Cut, the run keeps between 1 and 1,000 rows and reports `pairs`.

## Open

- The SQL strata (modes without `program`) have no row cap, and do not read the run's stop: only the binary's
  backstop bounds them. A SQLite progress handler reading the run's stop would end that.
- A cut probe stays cut for the rest of the run. Its relation is not marked incomplete for the rules that read
  it: the exit code is the signal.
