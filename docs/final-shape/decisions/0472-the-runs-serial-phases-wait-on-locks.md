# 0472. The run's serial phases wait on locks the pool's threads share

Status: accepted (2026-11-19). Follows 0465 §1 (Q284's second item) and 0471.

## Intro

0465 found the run's own thread spending 8–10 s of all's 19–23 s in serial phases: `cross` 2.7–5.1 s, of which
`public_best_len` and `public_best_route` were 1.4 s off a core; `stats` 2.1–2.5 s; `combine` 2.2–3.0 s. The user:
"please do all 3. Our parsing should be lightning fast".

Sampled on all, native, on an M1 Max (`sample`, 8 s from the 13th second, the run's `stats` and `cross`):

- The run's thread waited in `run_join_in_each`'s morsels (1,062 samples of 5,526): the strata ran on the pool, and
  it waited for them.
- The pool's ten threads spent 7,708 samples of 41,750 in `__psynch_mutexwait` and `__psynch_mutexdrop`, all in
  `setfix::scalars::Pool::evaluate`. That is the memo of an aggregate's groups (`l = min k : { public_len(o, n, k) }`),
  probed once a row, from every thread, behind one `Mutex`, twice a miss.

## Decision

1. **The scalar pool is 32 shards by the group's key**, each a `parking_lot::RwLock`. A key's memo lives in its
   shard, with the shard's own epoch.
   - A hit takes the read lock only. Its lookup count is an atomic.
   - A miss takes the write lock, as before, folds the group outside it, then takes the write lock to insert.
   - Each shard knows every bag's preferred value, written by `register`, whose proof runs once.
   - `Pool::memos` gives each bag's memo, every shard's summed, for the profile's explanation and the tests.

2. **A tape's room is placed without the residency's lock.** A slab's room is a compare-and-swap bump of its block,
   so `tape_room` places a tape in the slab tapes go to now (`TAPE_SLAB`, under a read lock). It takes the
   residency's lock only to begin the next slab. `TAPE_SLAB` holds that slab, so it is not retired while tapes go to
   it, and `clear` drops it.
3. **`Grammar::embedded`'s cache is read-locked** for a grammar built already: every parse worker asks it for each
   document.

## Measured

All, `internal_dead_code`, native, an M1 Max farm host. The rows were ed502024 in every run.

| | Before | After |
|---|---|---|
| `public_best_len` | 0.90 s wall, 3 ms of the run's CPU | 0.16 s |
| `public_best_route` | 0.88 s | 0.15 s |
| `cross` | 6.2 s wall, 290,583 involuntary switches | 3.6–3.7 s, 48,628–49,725 |
| the pool's mutex samples in `scalars::Pool` | 7,708 | none listed |
| the run's CPU | 61.5–62.8 s | 53.7–54.2 s |
| wall | 18.8–21.6 s at load 8–10 | 16.6–21.5 s at load 12–14 |

The scalar tests (`-p rerail-logic`, 11) pass.

The parse window (sampled 3–11 s, native all, two runs each, alternating), the pool's samples in mutex waits:

| | Before | After |
|---|---|---|
| all of them | 2,257 and 3,209 | 1,358 and 1,616 |
| `slabs::tape_room` and `Grammar::embedded` | 743 and more | none listed |
| the store writer's state (`ContentStore::gather`, `pending`) | 1,034–1,362 | 881–1,350 |

The parse workers' CPU is the same (8.3–8.5 s): a thread parked on a lock takes no CPU, so this is parallelism. The
host's load was 13–33, so the walls say nothing. The residency's and the tapes' placement tests (36) pass.

## What is next

- **The store writer's one lock in the parse.** Every tape a worker queues (`queue_source_and_tape` into the
  run's one gather) and every look-up of a queued tape (`pending`) takes the writer's state `Mutex`. A gather per
  worker would remove it, but the other threads' look-ups of a queued tape must still find it.
- **The parse workers' CPU** is W's reference graph (0466): 6.5 s of the 8.4 s.
- `cross`' largest strata now run on a core: `reach_edges` 0.69 s and `flow0` 0.63 s, CPU nearly all.
- `stats` is 1.0 s (it was 1.6–2.5 s), its waits now small.
