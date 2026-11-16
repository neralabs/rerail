# 0230: A reached stratum pays for what moved, not for its size

Status: built (2026-11-14), in steps. `runs.rs` (`Run::galloped`), `plan.rs` (`cyclic_relations`,
`Fx::stats_changed`), `setfix.rs` (`live_changed`).

## Intro

In a session, each stratum an edit reaches is set up, planned, run and closed. After decision 0217, a one-file
edit's joins were a small part of that. O125 step 14 (one file) reaches 190 strata. Its cross part spent 0.14 s on
the main thread and 0.17 s in the stream jobs, and the joins took 19 ms of the jobs' time.

Several of the fixed costs followed the size of a stratum or of its relations, not what changed in it:

- **Close.** A relation's indexes are merged to one run when its component closes (`FROZEN_RUNS`). A maintained
  relation that started from 1 M kept rows and gained 25 merged all 1 M again, a row comparison per row, for each
  index.
- **Setup.** `cyclic_relations` walked the stratum's graph from each relation, over string sets. The 220-relation
  stratum of the `acls`/`ainst`/`alook` family walked it 220 times.
- **Live statistics.** After each chunk, every relation of the component reported its statistics, and the planner
  updated its maps for each column. In a wave of a large component, most relations gained nothing.
- **Index report.** `probed::note(&format!(…))` built a string per relation on each statistics call, even with the
  report off.

## What changed

1. **A small run gallops into a large one.** When one run has under 1/16 of the other's ids, each small id is
   placed by a binary search, and the large run's stretches between are copied whole. The order is the walking
   merge's: both are total orders, ties broken by id. `RERAIL_GALLOP=off` walks as before.
2. **Cyclic relations in one pass.** Tarjan's algorithm over the same edges. A relation is cyclic where it is in a
   strongly connected part of more than one relation, or reads itself, exactly the relations the walk found.
3. **Live statistics of what moved.** `live_changed` reports a relation only when its rows, its delta's start or
   its delta's end differ from the caller's last reading. Those are all a relation's statistics come from: its
   sketches change only as rows are pushed. The per-chunk loop keeps the readings. A drive that returns and comes
   back starts them anew, so its first reading reports every relation. `RERAIL_LIVE_ALL=on` reports all each time.
4. **The index report's strings** are built only when the report is on.

## Measured

O125 replay, steps 12–16 (five sessions), `--profile profiling` build, unprofiled, two rounds each. Setting A:
`RERAIL_GALLOP=off RERAIL_LIVE_ALL=on`. Steps 1 and 2 of the list are in both runs. Every run ended "same as
full".

| step | files | cross before (G) | cross after (G) |
|---|---|---|---|
| 12 | 9 | 11.57 | 11.43 (−1%) |
| 13 | 2 | 6.45 | 6.13 (−5%) |
| 14 | 1 | 3.52 | 3.28 (−7%) |
| 15 | 1 | 4.19 | 3.96 (−6%) |
| 16 | 2 | 4.56 | 4.34 (−5%) |

## Still open

A sample of those five sessions' stream jobs puts the joins at about half their work. The fixed cost left, in
order:

- planning, about 12%: a session's delta rules are first seen in it (2,887 first-sight plans in session 14). The
  search's SipHash lookups are not the lever: the planner's per-step maps (`card`, `delta_rows`, `fan`, `obs`, the
  search's `kinds` and `best_of`) moved to Fx measured within noise (steps 12–16, two rounds), and were left as
  they were. The count of first-sight plans is.
- opening: 1.3% of the whole step-14 sample, not the 5% first read; `Rel::based` is 6 of its 88 samples. Most of
  it was a syntax view's constant filter scanning through the trait's per-row default, a document search per row:
  the view's own document-by-document scan had been left outside its `Source` impl by a merge. Fixed in
  712e08427 (sessions within 1%, process −1%). A filter on a column no index answers still reads every node of
  the batch each session.
- the main thread's placement of each reached stratum (`form`, 50 µs each).

## Tests

- `a_galloped_merge_is_the_walking_merge` (`runs.rs`): the same ids, tags, prints and fences, for tagged, prefix
  and printed orders, the small run on either side. It fails with the large run's tail left out.
- `cyclic_relations_are_those_that_reach_themselves` (`plan.rs`): Tarjan's against the walk, over 60 generated
  programs.
- Logic crate: `setfix`, `plan::`, `vtab`. Engine: `logic::plan::tests`, `session`, `waves`, `corpus`.
