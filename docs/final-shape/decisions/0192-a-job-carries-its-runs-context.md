# 0192. A job carries its run's context

Status: built (2026-11-13). Starts on O-run-state-threads; follows decisions 0178 and 0179.

## Intro

The engine keeps a run's context in thread-locals. That context includes:
- how deep the run is nested;
- the phases under way, and what they were handed;
- the kept plans, the observations and the derived values;
- the device binding and the planner's settings.

A nested run on another thread saw none of it. A typing run started on a pool thread would act as an outermost run:
- it would take the session's switches again;
- it would see no phase under way;
- it would plan without the run's kept plans.

So the cold batch ran its nested runs one after another on the main thread, even where they do not depend on each
other (decision 0179, "What stays").

This decision does three things:
1. It lists every thread-local with what it is.
2. It carries the run's context to every job the engine starts.
3. It adds checks so that a new thread-local cannot be added without that decision.

As a first use, the bridged typing run now runs beside phase 1.

## Decision

### Every thread-local is listed with its class

`logic::run_state::STATE` lists all 138 thread-locals outside the tests, across `native/src` and
`native/crates/*/src`. Each one has a class and a reason:

| class | what it is | count |
|---|---|---|
| Carried | the run's context: a job on another thread must see it | 23 |
| Thread | correct on any thread: caches whose entries are the same everywhere, counts of a thread's own work, its connections, codecs, device queue, open spans | 71 |
| Call | the state of one call in progress, set and read within it | 40 |
| Outermost | read only by a thread's outermost run: a session's plans and derived values between runs | 2 |
| NotYet | run context not carried yet | 2 |

The two NotYet entries are:
- `plan::PLANNED_NOW`: the profile's account of the keys planned in a run. A job's account would have to be merged
  back into its caller's.
- `choices::DURABLE`: the run's store and entries, held by the thread that opened them.

Thread-locals declared for tests alone are left out of the list. These are the ones under `#[cfg(test)]` or
`#[cfg(any(test, feature = "test-hooks"))]`, or in a test file. A test sets them on its own thread.

### A job enters its caller's context

`run_state::Carry::here()` takes the context on the caller's thread. Each module that owns carried state adds its own
parts through a `carried(&mut Capture)` function. There are three kinds of part:
- **copy** for settings: `RUNS_UNDER_WAY`, `RUN_TOKEN`, `DEPTH`, `WHOLE`, `DEVICE`, `IN_PHASE_0`, the planner's
  flags and the run's stage (below);
- **clone** for small values: the phases under way, what they were handed (now an `Arc`), and phase 1's result
  (now an `Arc`, was an `Rc`);
- **shared** for state that jobs add to:
  - the kept plans (`enter_plans`) and the observations (`enter_observed`): the job's additions are merged back as
    it ends;
  - the derived values;
  - the SQL admission ledger.

In test builds, the job also carries the store its caller's runs use (`test_store`: the store the test names, else
the one it was handed, else its own copy of the template). Without it, a test's job ran against the process's store.

`Carry::enter()` puts the context on the job's thread and returns a guard. When the guard drops, the thread gets back
what it had before. This matters because a pool thread may take another run's job while it waits.

Each of the engine's three ways of starting a job now carries the context:
- `threads::Jobs::spawn`;
- `threads::beside`;
- the native strata's jobs (`Ambient`, decision 0113 §4). `Ambient` was a hand-made carrier for five of these
  values; it is now a `Carry`.

The cost is too small to measure: instructions on `dl1` are the same within the run-to-run noise.

### A run's waves are its own

The strict mode refuses SQL inside the waves, and so does the profile's account of SQL hazards. Both used one
process-wide count of the runs that are in their waves. With two runs at once, phase 1's run began reading while the
typing run beside it was in its waves, and the strict mode stopped the process.

Each thread now has a stage, carried with its run's context:
- no run;
- a run before its first wave (set by `profile::run_begins`);
- inside the waves (set by `profile::waves`).

A run before its first wave may read, whatever other runs are doing. A thread of a run inside its waves may not. A
thread with no run's context, such as a parallel loop's item, still goes by the process-wide count. A run nested
inside a run that is in its waves stays inside the waves, as before.

### The checks

These tests live in `run_state/tests.rs`:
- **Every thread-local is listed with its class.** The test reads the source, and fails on one that is unlisted,
  gone, listed twice, or listed without a reason.
- **Every carried thread-local is taken by a carry.** The names `Carry::here()` takes must equal the Carried
  entries.
- **The context not carried yet only shrinks.** The NotYet count is pinned at 2.
- **A job enters its caller's context and leaves its own**, on a plain thread, through a pool job and through
  `beside`.

`scripts/check/check-thread-state.py` runs before every commit, from the same hook as the cache check. It refuses an
added thread-local that `STATE` does not list. It reads only the files the commit changes, so it takes well under a
second.

### The typing run beside phase 1

`run_observed` starts the bridged typing run (`bridge::facts`) as a pool job, then runs phase 1 on the caller.
Neither reads the other's results, and each writes in its own directory (`typing-<program>`, `phase-sources`). The
job's runs are nested in the caller's run, because the job carries its context.

`RERAIL_TIMELINE` shows the overlap. On the base build the two run one after the other: typing 0.706–0.760 s, then
phase 1's runs from 0.882 s. Now they run together: typing 0.673–0.743 s and phase 1's runs 0.670–0.889 s.

## Measured

`dl1`, cold (`RERAIL_TAPE_CACHE=off`, `RERAIL_SQL_STRICT=1`, 4 threads), on `--profile profiling` builds. Five
runs of each, alternated with the hub merge commit before this change (5de418d7a), at load 42–72:

| | before | after |
|---|---|---|
| wall | 1.65–5.10 s (median 1.92) | 1.54–3.34 s (median 1.69) |
| instructions | 24.0–25.4 G | 24.7–25.4 G |

The findings are identical (96 lines, md5 4b9d75bb…), including with `RERAIL_SPLIT_CHECK=1` and
`RERAIL_MAGIC_CHECK=1`. The load was high and the walls are noisy. The overlap itself is clearer in the timeline:
the typing run's 0.07–0.1 s now runs inside phase 1's span.

## What stays

- **The cross-file cone keeps whole relations.** Given the run's kept plans, the cone's job is a corpus run, and a
  corpus run makes the negation split: a relation computed only for the keys its readers reach. The cone keeps its
  strata for the next run (`cone_kept`), so after an edit a key the next run demands (an edited library's new
  import) had no rows and no delta to derive them. Before this decision the cone ran with no plans, so it never
  split. It now enters `demand::whole_relations` whenever it keeps its strata, as a session's cross-file part does,
  and takes the run's plans like every other job (O-cone-kept-plans, closed).
- **The split memo is process-wide** (`split_memo::SPLITS`). Two runs at once still give exact results: an entry is
  keyed by every input it reads. But they can push each other's entries out. The O-run-state-threads closure keeps
  this open.
- **The two NotYet entries.** A nested run on a job sees no `DURABLE` (it keeps nothing durable of its own) and adds
  nothing to the profile's planned-key account.
- **The Call class is a claim, not a check.** A call that hands its work to another thread mid-call must move its
  state to Carried. The list says which state that is, so the change has an obvious place to go.
