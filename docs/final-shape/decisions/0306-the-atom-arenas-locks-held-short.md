# 0306. The atom arena's locks held short: hashed once, written only for the new values, read in runs

Status: decided (2026-11-16, the user: "can we change the string interner to do bulk adds and take the lock for
the whole batch?"). Builds on 0279 §7 (the sharded lock), 0292 §1 (a morsel's deferred texts), 0160 (provisional
atoms).

## Intro

Once 0301 had taken the planner's locks out, the run's most-waited lock was the atom arena's (`Atoms::state`, a
`Sharded` lock: each thread reads under a shard of its own, and a writer takes every shard in order). The question
was whether new atoms should be added in batches, under one lock a batch.

They already were: `intern_all` takes a slice, and `give` takes the write lock once for all of it. What a whole-run
sample of a cold PostHog run showed (M3 Max, `incl.py` over `sample`) was this:

- **The write lock was held for work that needed no lock.** Under it, `give` hashed and looked up every value of
  the batch again, though the read pass had just found most of them. Adding the new values took 24 of `give`'s
  744 samples.
- **The writer's wait was the cost, not its work.** A writer waits for the longest reader on any shard. While it
  waits, every shard it holds already stops its readers. The longest reader was `intern_all`'s own read pass, which
  held one read lock across a whole parser batch (`Batch::build`): about 2,000 samples a run, against the writes'
  400 or so.
- **Walks in one piece interned row by row.** A walk in morsels defers its new texts and interns them at its merge
  (0292 §1). A walk in one piece took the write lock for each new text. Over PostHog that was only 4 walks and 3,823
  texts, so this was the smallest of the three.

## Decision

1. **Hash once, write only what is missing.** `intern_all` hashes each value once, outside any lock
   (`Interner::intern_h` takes the hash). The read pass keeps what it found, and `give` looks up again and adds
   only the values still missing, under the write lock. Another thread may have added one meanwhile, and the
   second look finds it.
2. **Read in runs.** The read pass looks up `READ_RUN` (256) values under each read lock, not a whole batch. A
   writer waits at most that long for this reader.
3. **A walk in one piece defers its texts as a morsel does** (`setfix::whole`). Its new texts are deferred, then
   interned with one `intern_all` at its end (`settle_deferred`). Its head rows go to a `Fresh` of its own while they
   hold deferred values, and that is then absorbed into the stratum's. The ids are the ones one walk would have
   given, in the same order. A walk run for each value (`each`) reads its collected rows as it goes, so it is left
   as it was.

## Measured

The findings were the same in every run: 29,747 (fa0efeaed6ff). Samples are whole-run `sample` counts (1 ms) of
time spent in a lock's wait.

Steps 1 and 3, on the M3 Max:

| | before | after |
|---|---|---|
| `give` (holding or waiting) | 744 | 626 |
| readers waiting (`lock_shared_slow`, every lock) | 2,513 | 1,837 |
| `cat_held` waiting | 528 | 378 |
| `with_str` waiting | 363 | 208 |

Step 2, on the M5 Max (the M3 was then short of disk):

| | before | after |
|---|---|---|
| `give` (holding or waiting) | 1,214 | 552 |
| `give` waiting for the lock | 706 | 312 |
| readers waiting (every lock) | 4,482 | 2,065 |
| `cat_held` waiting | 982 | 354 |
| `with_str` waiting | 618 | 335 |
| `intern_all` | 6,092 | 2,150 |

Both machines were busy with other work (load 15–18 on the M3, 16 then 94 on the M5), so no wall time from these
runs is evidence. The wait counts fell even though the load rose.

## Next, not done

The arena is append-only: an atom's id and text never change once given. Reads could take no lock at all: text
chunks that never move, and an insert-only hash table (open addressing over atomic slots, a new table published by
RCU when it grows). Writers would then never wait for readers. That is the end of this lock, but the arena also
holds stored segments, forks (0170) and provisional ids (0160), so it is a decision of its own.
