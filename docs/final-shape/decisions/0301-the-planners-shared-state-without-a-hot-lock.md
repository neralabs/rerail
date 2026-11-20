# 0301. The planner's shared state without a hot lock: plans published by RCU, their feedback in cells, the lines and the observed sums reduced

Status: decided (2026-11-16, the user: "yes, please implement and validate it"); built as in "As built" below. Builds on 0113 (strata as jobs, `enter_plans`), 0038 (kept
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

## As built

Where the build departs from the proposal, and why:

- **Plans** (`RunPlans::kept`): an `ArcSwap` of a map of `Arc<Entry>`. A job's new keys and new orders stay in its
  own plans (`OWN_PLANS`) and are published together as it ends (`RunPlans::publish`, one `rcu`; the old map's
  entries are shared, not copied). `stored` and `orders` are `ArcSwap`s of their own. `planned_now` is only
  counted for the profile, so it sits behind a mutex no planner reads under.
- **Feedback:** first built as one cell per entry, not two: an `ArcSwap<Learnt>` changed by `rcu`. That was wrong.
  Concurrent chunks run the same rules, so many threads change one plan's cell at once: the compare-and-swap
  retried, and each retry copied the entry (2,774 samples of a cold run, `Entry::update`). Corrected (2026-11-16)
  toward the proposal: each field a cell of its own, written by the last writer, with no read of the old value.
  `ran` is an `AtomicU64` of the f64's bits. `Priced` is an `Option<Arc<Priced>>` behind a `parking_lot` mutex held
  only to swap the pointer. An `ArcSwapOption`, as proposed, cost 942 samples in its store, settling the debts of
  every thread that loads it, at 330,000 stores a run. The search's findings (work per node, EXPLAIN, gap) are a
  third cell, replaced only when a put brings them. Planning hits (`plan_rule_reused`) went from 9,070 samples to
  6,427 over a whole cold run, and the feedback's writes from 3,590 to 106. Same findings.
- **Lines run:** a channel (`std::sync::mpsc`, whose sends take no lock), not a collector fed at each job's end.
  Each stratum sends its lines as it runs, and only `lines_take` receives them. Nothing reads the lines in
  between, so a job needs no buffer of its own.
- **Observed sums** (`ObservedShared`): published as an `ArcSwapOption<Observed>`. A job's sums are queued as it
  ends (`hand_in`). The fold is not done by the main thread at `Step::Finish`, which would have needed a new
  call site in the stream. Instead, the job that queues the 256th set of sums folds them all, if no fold is under
  way (`try_lock`), and a chunk's choice (`choose`) folds whatever is waiting before it reads, waiting for any fold
  under way. So an estimate read inside a job lags the run's sums by at most 255 jobs' sums, and a choice lags by
  none. A thread observing outside a job (`observing_into` with no job) folds and changes the sums under the fold
  lock at once. `choose` used to take the sums out of the run's while it priced by estimates alone. It now puts
  an empty handle in the run's place on its own thread, and leaves the run's sums as they are.

## Measured

PostHog on the M3 Max (29,885 files, `RERAIL_CHUNKS_AT_ONCE=15`). All five runs found the same 29,747 findings
(fa0efeaed6ff) as before.

| | before | after |
|---|---|---|
| exclusive takes of the readers' state (plans + observed) | about 900,000 | 698 publications (795 plans or orders) + 152 folds |
| `yield: rwlock` samples, whole cold run (`sample`, 1 ms) | 6,937 of 1.07 M | 734 of 1.32 M; most of these are the store's atom interner (`Atoms::give`, `cat_held`) and `joins::pending`, not 0301's state |
| `yield: rwlock` samples, 8 s of the waves | 930 (plans part only) | 234 |
| cold wall, unprofiled | 29.3–31.1 s | 29.8, 32.1 s (host load 8–11 from other work) |
| cold user / sys CPU | — / 72–73 s | 190–196 / 66–67 s |
| peak | 2.62 GB | 2.64–2.70 GB |
| warm | 1.05 s | 1.05–1.13 s |

The lock spin is gone, but the wall did not move: the cold run is bound elsewhere. The remaining kernel time is
`open` (23 k samples), store `pwrite`/`fsync`/`pread` (24 k) and rayon's idle spin (8 k). The leftover rwlock
yields belong to the store's interner, whose writers take a write lock per new atom; that is a separate decision.

## Publications in shards (2026-11-17)

"Copy-on-write for the few new ones" assumed few publications. A run publishes each time a stratum's entry
planned anew, and each publication copied the whole map. On B-now over 50 files, 317 programs and about 5,800
components, cloning and dropping the map was the run's top cost in samples. `Plans` is now 64 shards by key hash,
each an `Arc`'d map. A publication copies only the shards its plans fall in (`Arc::make_mut`), and readers still
take no lock. Instructions went 320 -> 316 G and cycles 110 -> 94 G (the copies were contended), with the same
findings.
