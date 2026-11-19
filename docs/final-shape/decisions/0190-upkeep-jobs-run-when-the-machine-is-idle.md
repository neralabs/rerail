# 0190: Upkeep jobs run when the machine is idle, at the lowest priority, within a CPU and memory share

Status: draft (2026-11-13); a job's shape (§7, `Qos`) added 2026-11-14; the scheduler and five jobs built (§4: collection, SQLite files, search indexes,
the cost model); re-sorting stored rows (§5, O160) and trying join orders (§6, O161) designed, not built.

## Intro

The engine has work that makes later work cheaper but can wait:

- giving back what the store no longer needs (decision 0053's collection);
- keeping SQLite's files small and their statistics fresh;
- building the search indexes of stored tapes;
- training the cost model the placement reads (decision 0100 §14).

Before this decision each one ran on a timer or a counter of its own, whatever the machine was doing:

- The daemon's store thread woke every 30 s and ran up to a second of collection and a second of indexing.
- The cost statistics retrained every 2,000 samples, at most every 10 minutes, at the utility quality of service.
- Nothing ever vacuumed `costs.sqlite`. It was 176 MB, of which 47 MB (27%) were free pages, with a 50 MB
  write-ahead log, and no table had ever been analyzed.

The user asked for one pluggable set of such jobs (2026-11-13): run "when the machine is idle or close to idle,
at low priority in the background, with configurable but low CPU and memory limits and sensitive to battery
status".

This decision gives the engine **upkeep**: one scheduler (`rerail_accel::upkeep`) that any part of the engine
registers jobs with. The scheduler runs a job only when the machine allows it. Each job works in bounded steps
within a share of the CPU and memory.

## Decision

### 1. A job is a trait, registered by the part of the engine that owns the work

```rust
pub trait Job: Send {
    fn name(&self) -> String;          // "sqlite costs.sqlite"; its kind is the first word
    fn due(&mut self) -> Due;          // cheap: called at every poll
    fn threads(&self) -> usize { 1 }   // the threads a step keeps busy
    fn step(&mut self, slice: &Slice) -> Result<Step, String>;  // a bounded part of the work
}
pub enum Due { No, Idle { starve: Duration }, Urgent }
pub enum Step { More, Done }
```

- **Registering.** A job is registered with `upkeep::register(Box<dyn Job>)`, which returns a handle. Dropping the
  handle takes the job out, for example when a store is closed.
- **Starting.** The scheduler's thread is started by `upkeep::start()`, which a process that lives long enough
  calls: the daemon (when its store opens) and a run's cost statistics (`costs::start`). Registering a job never
  starts a thread, so tests and short commands keep their threads.
- **Running now.** `upkeep::run_now()` runs every due job to the end on the caller's thread. The
  `rerail-engine upkeep` command uses it.

### 2. When a job runs, and at what priority

One thread, `rerail-upkeep`, made by `exec::service` like every other thread. It runs at the background quality
of service: the efficiency cores, and throttled disk I/O.

- **Its own I/O policy would reset its QoS.** Setting the thread's I/O policy (`setiopolicy_np`) resets its
  quality of service to unspecified (measured: `0x9 → 0x0`). So no I/O policy is set; background QoS already
  throttles I/O.
- **Fallback.** Where a quality of service is refused (`EPERM`, seen under `cargo test`), the thread goes into
  Darwin's background state (`PRIO_DARWIN_BG`) instead. `upkeep::lowered()` says which one took.
- **Its jobs' own threads.** A job that starts threads (the training pool) lowers each one the same way
  (`lower_this_thread`).

Every poll (10 s on mains power, 40 s on battery) the scheduler reads the machine's state:

| Reading | How |
|---|---|
| Busy share of all cores since the last poll | `host_processor_info`, the ticks of each core |
| On battery, and the charge | IOKit's power sources (`IOPSCopyPowerSourcesInfo`) |
| Low power mode, thermal state | `NSProcessInfo` |
| The process's footprint | `proc_pid_rusage` |

From that, what may run:

| Work | Runs when |
|---|---|
| `Due::Urgent` (a store past its soft limit) | at once, at the background priority, within the CPU share |
| `Due::Idle` | the machine is idle: busy share below `RERAIL_UPKEEP_IDLE` (25%) for two polls in a row; on mains power, or on battery above `RERAIL_UPKEEP_BATTERY`'s charge (off by default); not in low power mode, not at a serious thermal state; the process's footprint within its budget (decision 0126) |
| `Due::Idle` past its `starve` | as urgent work, but only where idle work would be allowed apart from the busy share |

**Starvation bounds.** A machine that is never idle still has to collect and index. This one runs at load 45–110
on 18 cores, so its busy share is rarely below 25%. Each idle job therefore has a bound: work kept waiting past it
runs anyway, at the background priority. Two exceptions:

- A whole `VACUUM` has no bound, because it holds the file's writers for its whole length. It runs only on an
  idle machine.
- Battery and heat still hold back starved work. Only urgent work runs in those states.

**One pass.**
- The due jobs run oldest-run first.
- Each job steps until it answers `Done`, or until a poll's time has passed.
- Idle work stops after a step once the machine is busy again.
- A job that fails three times in a row is held for an hour.
- A panic is caught, and counts as a failure.

### 3. Limits and settings

**CPU.** A step is bounded in time: `Slice::until` is 200 ms away, and a job checks `Slice::over`. After a step of
wall `w`, the scheduler sleeps `w × (threads / cpu − 1)`. So a job averages at most the CPU share. This is an upper
bound: it counts a job's threads as busy for the whole step.

**Memory.** A job is given `Slice::memory` and keeps within it, for example its SQLite page cache and the samples it
reads. Idle work waits while the process's footprint is above its budget, because then the engine is busy.

| Setting | Default | Meaning |
|---|---|---|
| `RERAIL_UPKEEP` | on | `off`: no scheduler |
| `RERAIL_UPKEEP_CPU` | 25 | the share of one core the jobs average, percent |
| `RERAIL_UPKEEP_MB` | 128 (at most an eighth of the budget) | a job's memory |
| `RERAIL_UPKEEP_IDLE` | 25 | the busy share of all cores below which the machine is idle, percent |
| `RERAIL_UPKEEP_BATTERY` | off | the charge above which idle work runs on battery, percent |
| `RERAIL_UPKEEP_JOBS` | all | `sqlite,collect`: only these kinds; `-cost-model`: all but these |
| `RERAIL_UPKEEP_LOG` | off | each step to standard error |

None of these settings changes what a run outputs, so all of them are left out of the run-reuse key
(`corpus/reuse.rs`).

### 4. The jobs built

| Job | Due | Step |
|---|---|---|
| `collect <store>` | urgent past the soft limit; idle when 0053's collection is due (bound 1 h) | corpus passes until the slice is over (0053) |
| `search-index <store>` | idle while stored tapes lack an index (bound 60 s: the old 30 s wake, about) | the backlog until the slice is over |
| `sqlite <file>`, for each store and `costs.sqlite` | idle when its WAL is past 64 MB, more than 10% and 16 MB is free, or its tables have no statistics or day-old ones (bounds 1 h for space, 7 days for statistics) | one action: free pages back (`incremental_vacuum`, 4,096 pages), then a checkpoint (`TRUNCATE`); a whole `VACUUM` for a file without incremental vacuum (more than 20% free, at most 1 GB, three times its size free on the disk; it converts the file to incremental vacuum); then `ANALYZE` one table at `analysis_limit` 1000 |
| `cost-model` | idle when 2,000 samples were written since the last training and 10 minutes passed (bound 1 h) | one training on two threads at upkeep's priority; put in place and saved when it wins |

**Analyze leaves the planner's statistics alone.** The planner writes `sqlite_stat1` for its fact relations from
`fact_stats`, never from `ANALYZE` over the data (`stats/plans.rs`, docs/final-shape/02 "SQL join order"). So the
SQLite job never analyzes a table that `fact_stats` names. It analyzes only its own tables: the store's
bookkeeping, and the cost statistics.

**Where the old timers went.**
- The daemon's store thread now only renews workspace leases. Its collection and indexing are these jobs.
- The cost statistics' writer no longer retrains. A process with a saved model puts it in place and leaves the
  retraining to the job. A process with no saved model trains one at once, because the placement wants one.
- The corpus run's own collection thread (`rerail-collect`, 0053 §5) is unchanged. It belongs to the run.

**`rerail-engine upkeep [<store.sqlite>...] [--cpu <percent>] [--jobs <list>]`** runs every due job to the end now,
and prints each job's steps, wall and CPU. It is the form for a launchd job or a by-hand run.

### 5. Re-sorting stored rows for faster searches (O160, not built)

Decision 0164 keeps sorted orders in frames, compacted at a dead-row bound. Its compaction is the natural upkeep job:

- **Due:** idle when a frame's dead rows pass the bound, or a relation probed by a key has no order for it
  (0164's probe counters).
- **Step:** one frame rewritten in its key order, published by the frame's own swap.

It waits on 0164's frames being built. The search-index job above is the first instance of this kind of work: it
builds the indexes that narrow a search.

### 6. Statistics and join orders for slow joins (O161; built as decision 0203)

Decision 0203 builds this: slow joins flagged while they run, exact pair statistics, the critical path, and the
`join-search` job. What follows is the design as it was first written.

Two pieces exist.
- **The plans' statistics.** They are kept by delta (0096). Each plan's decisions are kept with the log2 buckets of
  the statistics they read (`compiled_plans`). Observed selectivity and cost per root are kept by EWMA
  (`plan_feedback`).
- **The SQL plans' reports.** `costs/sql_plans.rs` keeps each run's forecasts against its measurements, with an
  alternate forecast.

Two pieces are missing: a list of joins observed to be slow, and a way to try another order away from a run.

The job:
- **Due:** idle when a plan's measured cost per root is past its forecast by a factor (from `plan_feedback`) on
  enough samples.
- **Step:** the plan's join re-planned with the other orders the planner would weigh. Each order is priced with
  the observed statistics, and the cheapest few run over the store's own facts, under a time bound. An order that
  wins is recorded as the plan's choice (0090 §5's kept choices), keyed by the statistics' buckets as plans are.

It needs the planner to expose its alternative orders, and a bounded way to run one off the run's path. Both are
planner work.

### 7. A job's shape: its quality of service (amendment, 2026-11-14)

The user asked for a priority on each job (2026-11-14), so that one scheduler serves jobs of different shapes. A job
says its shape with `Job::qos()`:

```rust
pub enum Qos {
    Background,   // the default: §2 and §3 as written
    Utility,      // behind a run: soon, and finished before the process exits
}
```

| | `Background` | `Utility` |
|---|---|---|
| Its steps' priority | the background quality of service | the utility quality of service, set for each step on the scheduler's thread, then back |
| `Due::Idle` work | waits for an idle machine, or its `starve` | runs at once, as urgent work does |
| Battery, low power, heat | hold back all but urgent work | do not hold it back |
| The CPU share's pause | after each step | after each step on the scheduler's thread |
| At the process's end | left for the next process | finished by `upkeep::finish_soon()`, on the caller's thread, with no pause; a step the scheduler's thread is in is waited for (`STEPPING`) |

Utility work is small and wanted soon after the work that made it: a run's own leftovers. The first utility job
is `frames <store>` (decision 0227), the frames a keep decision left unreachable:

- A corpus run that collects registers it for its store, once a process (`Frames::register_once`); so do the daemon
  and the `upkeep` command.
- It is due while `corpus_decided.run` is past `corpus_decided.swept` (a column, not a table of its own: decision 0104; added on open to an older store): a point read at each poll.
- A collection pass that makes a keep decision kicks the scheduler.
- The outermost run calls `finish_soon` after its collection is joined, so a run leaves no sweep behind it.
- A process stopped before its sweep leaves it due, and the next process to register the job sweeps it.

Test: `jobs_step_to_the_end_within_the_cpu_share` (utility work runs on a busy machine, `finish_soon` finishes it
and leaves background work alone); the store's `a_keep_decision_leaves_its_sweep_to_the_frames_job`.

### 8. Upkeep yields to a run (amendment, 2026-11-14)

The user asked that background and utility work stop when a new run comes in (2026-11-14). A run is a corpus run
of the CLI (`run_handed`), or a workspace apply or index batch of the daemon. While one is under way, upkeep yields
to it:

- **No step starts.** The scheduler skips a job whose runs are active, and ends its pass after a step once a run
  has begun.
- **A step in progress stops at its next check**, and leaves its work as it was, still due for after the run.
- **`finish_soon` does nothing while a run is under way.** A run's utility work is finished by the run that ends
  last, after its guard is dropped.

How a step learns of a run:

| Run | Seen by |
|---|---|
| In this process | `upkeep::run_begins(files)`, a guard held for the run's length: a process-wide count |
| In another process, over the same file | the guard's shared `flock` on `<file>.runs.lock`; a job's `Job::file()` names the file it works on, and `runs_active(file)` probes the lock (`LOCK_EX \| LOCK_NB`, cached for 25 ms) |

The CLI's run takes the lock on its store and on the cost model's file. The daemon's runs take only the count: its
upkeep is in its own process.

Where each job stops:

| Job | Its stopping points |
|---|---|
| `collect <store>` | between passes (`Slice::over`) |
| `search index` | between batches of tapes |
| `sqlite <file>` (incremental vacuum, `VACUUM`, checkpoint, `ANALYZE`) | inside the statement: a progress handler interrupts SQLite, and the statement is rolled back |
| `frames <store>` | before it opens the store, and inside its transaction by the same interrupt, rolled back whole |
| `retrain costs` | after the comparison of the stored model, and after training; the samples' mark and the training time are left as they were |

`Slice::stopper()` is the check a step hands to work it does not run itself: a progress handler, or the training.
It is the stop flag or a run, without the slice's time bound.

**What this gives up.** A run is not slowed by upkeep that can still wait. On a machine that always has a run under
way, upkeep over a shared file (the cost model is shared by every worktree) would never run; §9 bounds that wait.

Tests: `a_run_in_another_process_is_seen_through_its_lock` (accel; `finish_soon` does nothing under a guard), the
store's `a_sweep_yields_to_a_run_over_its_store`.

### 9. Upkeep has a deadline (amendment, 2026-11-15)

§8 alone starves upkeep on a busy machine: here a run is under way nearly all the time, and the cost model's file
is shared by every worktree. The user asked for some upkeep on a deadline (2026-11-15). So due work yields to runs
only until its deadline; past it, its steps run through the runs, at their own priority and within the CPU share,
and a run does not stop them (`Slice::over` and `Slice::stopper` leave runs out).

| Work | Its deadline |
|---|---|
| `Due::Urgent` (a store past its soft limit, the frames sweep) | `RERAIL_UPKEEP_DEADLINE` (600 s) |
| `Due::Idle { starve }` | the longer of `starve` and that |
| A whole `VACUUM` (`starve` unbounded) | none: it holds the file's writers for its length, so it never runs through a run |

The deadline is counted from the job's **last finish**, not from when this process first saw the work. A CLI
process lives for one run, so a wait counted inside it would never reach any deadline. Each job over a file records
its finish in `<file>.upkeep` (`name<TAB>unix seconds`, rewritten whole and renamed into place) when a step answers
`Done`; a job that never finished counts from the file's creation. A job with no file counts from when its process
first saw it due.

A warm run finishes no job, so it writes no record (measured on dl10: a warm run left `.upkeep` as it was).

Test: `jobs_step_to_the_end_within_the_cpu_share` (accel): work over a file another process's run holds waits for
the run while the file is new, runs through it once its last finish is past the deadline, and records that finish.

## Measured (2026-11-13, debug build, load 45–50 on 18 cores)

**`rerail-engine upkeep`, on copies of the live `costs.sqlite` (176 MB) and a 186 MB content store:**

| Job | Steps | Wall | CPU | Result |
|---|---|---|---|---|
| `sqlite costs.sqlite` | 5 | 665 ms | 552 ms | 176 → 120 MB; converted to incremental vacuum; 2 tables analyzed |
| `sqlite <store>` | 82 | 51 ms | 48 ms | 31 tables analyzed (0.5 ms a table at `analysis_limit`) |
| `collect <store>` | 1 | 46 ms | 39 ms | nothing due |
| `cost-model` | 0 | — | — | not due: the saved model was 5 minutes old |

The command took 3.0 s in all, most of it the CPU share's pauses.

**The other measurements:**
- **The retraining, as a job, at upkeep's priority:** 1.02 s wall and 0.28 s CPU, at load 47 (the test
  `the_retraining_is_an_upkeep_job`). At the utility quality of service it was 0.39 s wall.
- **The scheduler's thread:** started, it ran a due job's steps on `rerail-upkeep`, at the background quality of
  service, within a second (`the_scheduler_runs_jobs_on_its_thread_at_background_priority`).
- **The machine's state as read:** 93% busy over a second while `top` showed 78% busy; on mains power, charge 100%,
  thermal state fair. These match `pmset`.

## Order of work

1. **The scheduler, its conditions and limits** (`accel/src/upkeep.rs`). *Built*, with tests for the settings, the
   gates, the steps, and the thread's priority.
2. **The store's jobs** (`content_store/upkeep.rs`: `SqliteFile`, `Collect`) and the search-index job
   (`workspace/storage.rs`). *Built*. A test gives back free pages in both vacuum modes and analyzes only the
   store's own tables.
3. **The cost model's retraining as a job** (`costs.rs`: `Retrain`, `upkeep_jobs`). *Built*.
4. **The `upkeep` command.** *Built*.
5. **A launchd agent** that runs `rerail-engine upkeep` for machines with no daemon running. Open: it needs the
   installer's paths.
6. **Re-sorting (§5, O160)** after 0164's frames; **join orders (§6, O161)** after the planner's alternatives.
