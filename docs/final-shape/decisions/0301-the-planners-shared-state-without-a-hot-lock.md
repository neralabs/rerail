# 0301. The planner's shared state without a hot lock: plans published by RCU, their feedback in cells, the lines and the observed sums reduced

Status: proposed (2026-11-16), for the user's choice. Builds on 0113 (strata as jobs, `enter_plans`), 0038 (kept
plans), 0122 (planning budget), 0203 §4 (the lines run), 0116 (stored plans).

## Intro

A cold run over PostHog spends about 20 thread-seconds of kernel time in `swtch_pri`, `parking_lot`'s spin-yield
before it parks. 7 of them are readers of the run's plans (`SharedPlans`, an `RwLock<RunPlans>`) waiting behind
writers, and about 2 the keeps' observed sums (`SharedObserved`). Counted in one run (205,230 stream jobs):

| lock | shared takes (held) | exclusive takes (held) |
|---|---|---|
| plans | 1,912,458 (250,725) | at a job's end 197,124 (56,835); `lines_push` 406,554 (132,867); other 93 |
| observed | 1,142,295 (44,262) | at a job's end 294,050 (31,948); other 30 |

Of the 197,124 merges at a job's end, 227 brought a new plan (741 plans in all); the rest carried 964,095 entries
back whose plan the run held already, for their feedback: `ran` (the work the line did, 0122) and `Priced` (the
sizes it was priced at). So the plans are read 1.9 M times and written 741 times; what is written all the time
lives beside them under the same lock:

- each line's feedback, a latest value per plan;
- the lines run (`lines`), an append-only log that only `lines_take` reads, once a critical path;
- the observed sums, a commutative sum that estimates read as rates.

## Proposal

Each kind of state gets the concurrency its use has.

1. **Plans: RCU.** The run's plans are an immutable snapshot behind an `ArcSwap` (`arc-swap`, in the tree): a
   reader loads it, no lock and no wait for a writer. A new plan, or a new order for a key (a search again), is
   published copy-on-write (`rcu`), its job's new plans at once as the job ends. With 227 such ends a run, the
   copies are cheap. `stored`, `orders` and `planned_now` go with the snapshot.
2. **Feedback: a cell per plan.** A kept plan's order is immutable; its `ran` (an `AtomicU64` of the f64's bits) and
   its `Priced` (an `ArcSwapOption`) are cells in its entry, written where the line runs. That is the last writer's
   value, as the merge at a job's end gives now, but seen by every job at once, not as the writer's job ends. No
   map is written and no lock is taken.
3. **The lines run: map-reduce.** A job keeps its lines and hands them on as it ends, to a collector of their own
   (a channel, or a mutex no reader takes); `lines_take` is the reduce. The plans' readers never see it.
4. **The observed sums: map-reduce, published by RCU.** A job's sums (`OWN_OBSERVED`, as now) go to a queue as it
   ends; the run's main thread, at each chunk it finishes (`Step::Finish`) and at the cross part's levels, folds
   them in and publishes the sums (`ArcSwap`). Estimates then read sums at most a chunk old, where they are
   already other jobs' sums as of when those ended.

What stays: a job's own plans and sums are read before the run's (it sees its own writes at once), and no two jobs
plan one key (each stratum plans its own rules).

## Rejected

- **A sharded or concurrent map (`dashmap`) for all of it.** It spreads the writers over shards, but every
  feedback update still takes a write lock on a shard the readers read. The writes are the waste, not their spread.
- **Map-reduce for the feedback too.** It would delay what a job's line did until a boundary; a cell costs less
  and gives it to every job at once.
- **One snapshot for everything, published at each job's end.** 205,230 copies of the plans and sums a run.

## To check when built

- The counts again: exclusive takes of the readers' state should fall from about 900,000 a run to the new plans'
  publications (hundreds), and `swtch_pri` under the planner's locks to about none.
- How often a key gets a new order (a search again): those are publications too, counted with the new plans.
- The same findings, and the plans kept across a session's runs (`KeepPlans`, `SESSION_PLANS`) as before.
