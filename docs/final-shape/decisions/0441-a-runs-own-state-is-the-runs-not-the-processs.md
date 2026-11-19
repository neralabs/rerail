# 0441. A run's own state is the run's, not the process's

Status: accepted (2026-11-19; the user: "we want the engine to be re-entrant, multi-thread possible and to not have
global state", then "please move the rest of the tables that hold run data into the run").

## Intro

Some of what a run produces for its own use lived in process-wide statics or in thread-locals: the width cap's cut
list and widest relations (0394), a debugging trace of kept statistics, and the count of trees decoded from versions.
Two runs in one process (a session's commands, the browser's reactor, tests on threads) then read or reset each
other's. A thread-local counted only what the run did on its first thread: a job on another thread was missed.

## Decision

1. **A run has one object of its own: `logic::run::Run`.** The outermost run on a thread makes it (`run::scope`: a
   command in `tools::cli_in`, a corpus run, or a program run, whichever starts first). A run nested in it joins it.
   `run_state` carries it to the run's jobs on other threads (`RUN`, Carried). It ends with the outermost scope.
2. **What it holds:**
   - its limits and what it met of them (`width::Width`, decision 0394): settings, cut list, widest relations, the
     memory watch's stop;
   - the trees it decoded from versions (`Version::program`), counted on any of its threads.
3. **It comes back with the result.** The outermost scope returns a `run::Record`: `plan::Outcome::run`,
   `corpus::Report::run`, and `tools::cli_in`'s message that a command's results are incomplete. A check reads the
   record of the run it made, never a process count it reset first.
4. **What belongs to one structure lives in it.** The `RERAIL_KEPT_DOCS` trace (Q20) is the statistics keeper's
   own field, read from the setting as each run begins (`keptstats::Trace`). The device compiler's own text columns
   (decision 0307) are a parameter of `setfix::device::compile`, not a thread-local set around the call.
5. **No registry where the existing lock already orders things.** The frames job's `GONE` map (forgotten stores) is
   gone. `ContentStore::forget` takes a store's jobs out under the registry's lock (which a `due` check runs under)
   and `STEPPING` (which a step holds until its job is back in the registry). Once it returns, no job is left to
   read the store.

## Tests

- `run::tests::each_run_has_its_own_record`: runs one after another, and one nested in another, keep their own cut
  lists and decode counts; a nested run's record is its outer run's.
- `run::tests::runs_on_two_threads_keep_apart`.
- `width::tests::the_memory_watch_stops_a_run_past_its_ceiling`.
- `corpus::tests` (the maintained run that decodes no tree) reads the count from its run's report.
- `plan::tests::a_relation_past_the_width_cap_stops_the_run_or_is_cut` holds each run's scope.
- `facts_only` and `run_state`: the maps and thread-locals removed are no longer listed.

## Open

- Process state that is not run data stays listed in `facts_only` (decision 0104): settings read once, measurement
  aids (`RERAIL_PREPARED_WHY`'s file, `RERAIL_HEAP_AT`), registries filled at load (`rr::lower::ALL`, the record's
  kinds), a test's log, and caches whose every entry is the same for any run.
