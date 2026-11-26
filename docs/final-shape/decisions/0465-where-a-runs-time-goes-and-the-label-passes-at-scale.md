# 0465. Where a run's time goes, a level's phases as one persistent dispatch, and the label passes at scale

Status: analysis (audited 2026-11-23). Accepted (2026-11-19). Three measures. Follows 0475. The persistent dispatch
(§2) was built, measured and removed.

## Intro

After 0475 the label formulas were about 1% of an s1000 run, so the question was what to work on next. Three
measures would answer it:

1. **End to end:** where does a run's time go? Is it the parse and batch workers, combine or cross?
2. **The device design:** do a level's phases run faster as one persistent dispatch, with grid barriers between them,
   than as one dispatch a phase? This is the fixed-launch shape.
3. **Scale:** do the label passes grow faster than the rest of the run on a larger corpus?

The user: "please do these: [the three] — they will give us enough information to know where to go next".

The corpora:

- **s1000:** 1,000 files of the data-layer repo. The run reads 339 of them: 877,007 nodes.
- **all:** the repo's file list, 9,010 files, of which 8,890 are in the checkout. The run reads 3,089 of them:
  9,627,434 nodes, 11 times s1000.

The program was internal_dead_code. Its rows were the same as native at every step: s1000 988d8fbe, all ed502024.

## 1. Where the time goes

Measured on an M1 Max farm host (10 cores) at load 3–7, the engine at 1ccf569fe, each Metal run after a warm-up
run. On this Mac (M5 Max) the load was 12–37 and the waits were mostly "ready, no core" time: 45 s of it in one 11 s
run. Its wall times say nothing.

| | s1000 native | s1000 Metal | all native | all Metal | all Metal, pinned to the GPU |
|---|---|---|---|---|---|
| wall | 4.2 s | 4.3 s | 23.2 s | 19.2 s | 19.2–22.6 s |
| CPU | 6.5 s | 6.4 s | 77.5 s | 63.7 s | 61.4–73.0 s |
| parse workers' CPU | 1.12 s | 1.12 s | 13.9 s | 13.4 s | 13.3 s |
| batch workers' CPU | 1.41 s | 0.90 s | 15.0 s | 14.0 s | 7.7 s |
| the GPU's time | — | 56 ms | — | 4 ms | 811–891 ms |

Unpinned, the lanes' scheduler gave the GPU almost none of all's batches, at the same wall time.

The run's own thread, phase by phase (wall):

| Phase | s1000 native | all native | all Metal | What it waits on |
|---|---|---|---|---|
| before | 0.39 s | 0.40 s | 0.44 s | phase 0's include graphs, the store's writer |
| combine | 0.76 s | 2.24 s | 2.95 s | requests answered inside phase 1's fixpoint |
| wait | 1.88 s | 10.7 s | 8.9 s | `local` (4.6 s, 2.8 s Metal), `parse` (0.6–0.8 s), the stream jobs |
| stats | 0.17 s | 2.46 s | 2.34 s | 400,000–800,000 involuntary switches in 2 s |
| cross | 0.27 s | 5.08 s | 2.69 s | `reach_edges` 0.7 s, `flow0` 0.6 s, `public_best_*` 1.4 s off a core |
| write | 0.04 s | 0.51 s | 0.51 s | |

So on all:

- The workers parse at about 4.3 ms of CPU a file, and evaluate their batches for as long again on the CPU (half
  that with the GPU pinned).
- The run uses 3.3–3.6 of the 10 cores. Its threads were ready with no core for 13–48 s: the process
  oversubscribes its own cores.
- The serial part of the run's thread (combine, cross, stats, write) is 8–10 s of the 19–23 s.

## 2. A level's phases as one persistent dispatch

**What was built** (commit 2de34fb30, then removed):

- `label_phase`'s body became `phase_tile` (block, formula, phase).
- `label_phases` ran a level's every phase in one dispatch:
  - `RERAIL_PHASE_PERSISTENT=<groups>` threadgroups of 64 threads, all resident at once;
  - each group took a phase's (block, formula) tiles in turn;
  - between two phases, a grid barrier: each group added one to a counter, then polled it until every group had.
- The barrier's polls were bounded. A group that ran out set a flag, and the run went again a dispatch a phase
  (`Error::GridBarrier`).
- On Apple family 7 the barrier polled with a read-modify-write, as `block_program::poll_helper` does. From family 9
  on it polled with a load.
- The scratch slots were read through `coherent(device)` pointers, since another group of the same dispatch wrote
  them.

**Measured.** The rows were the same as native at every group count that ran. `label_phase`'s encoder time on s1000,
pinned to the GPU, a run each, alternating:

| | a dispatch a phase | 128 groups | 256 | 512 | 1,024 |
|---|---|---|---|---|---|
| M5 Max, load 18–37 | 8.9–13.5 ms | 18.9 | 15.1–15.2 | 14.7–17.1 | 13.7–30.0 |
| M1 Max, load 3 | 21.8 ms | 27.6–28.6 | the barrier's polls ran out | | |

- On both machines the persistent dispatch is slower at every group count.
- The phases of a level already share one compute encoder. A dispatch boundary inside an encoder costs 5–7 µs, about
  1.1–1.5 ms over the s1000 run (0475). That is all the fold could save.
- In its place, each grid barrier waits for the slowest group's tiles. Fewer threads are in flight than in a dispatch
  over the whole grid, and every group polls one word.
- On the M1 Max, 256 groups of 64 threads were not all resident. Each barrier then spent its whole bound (2^24 polls)
  before the fallback, and the run passed the 300 s cap. A bound in polls is not a bound in time.

**Decision.** `label_phases` is removed, and a level's formulas stay a dispatch a phase in one encoder. The
fixed-launch goal holds for command buffers and encoders. At the dispatch level, a boundary inside an encoder is
cheaper than a grid barrier on these GPUs. The code is in commit 2de34fb30.

## 3. The label passes at scale

Pinned to the GPU, M1 Max:

| | s1000 | all | ratio |
|---|---|---|---|
| nodes | 877,007 | 9,627,434 | 11.0 |
| chunks | 6 | 39 | 6.5 |
| label runs (`label_parents` dispatches) | 12 | 200 | 16.7 |
| the large program's runs, files each | 6 runs; 12–130, median 36 | 283 runs; 1–186, median 6 | |
| `label_phase` dispatches | 200 | 4,769 | 24 |
| `label_phase` | 19.0 ms | 345.7 ms | 18 |
| `label_leaves` | 16.9 ms | 268.6 ms | 16 |
| `label_parents` | 6.1 ms | 71.9 ms | 12 |
| the GPU's time | 47 ms | 811 ms | 17 |
| wall | 3.8–5.5 s | 19.2–22.6 s | 4–5 |

- The label passes grow about 1.5 times faster than the nodes. The work per node does not grow.
- What grows is the runs. On all, most of the large program's runs have a few files. Where the small runs come from
  is not traced yet; phase 1's fixpoint answering its requests a few files at a time is the likely source. Each run
  pays the fixed costs of a run: its parents, its leaves, and about 25 phase
  dispatches with a floor of 5–7 µs each.
- The GPU went from 1% of the wall on s1000 to 4% on all. The run's wall grew only 4–5 times.

## What is next

In order of the time each holds on all (about 20 s):

1. **The workers' parse, about 13.5 s of CPU** (4.3 ms a file), on 3.3–3.6 of 10 cores. The thread is ready with no
   core for 13–48 s. The worker pool oversubscribes the cores, so its threads take turns rather than run together.
2. **The serial part of the run's thread, 8–10 s:**
   - `cross` 2.7–5.1 s, of which `public_best_route` and `public_best_len` are 1.4 s off a core in native;
   - `stats` 2.1–2.5 s, with 400,000–800,000 involuntary switches;
   - `combine` 2.2–3.0 s, waiting on requests inside phase 1's fixpoint.
3. **The label runs' sizes.** Batch the fixpoint's small requests into runs of the chunk's size, so that the fixed
   costs are paid once a chunk, not once a request. On all this is most of the GPU's 811 ms. First trace where the
   small runs come from.
4. **A cold run's failure.** The first Metal run of all on a fresh binary exited 1 at 12.4 s. The next ones passed.
   Its log was lost to the next run, so it is not explained.
