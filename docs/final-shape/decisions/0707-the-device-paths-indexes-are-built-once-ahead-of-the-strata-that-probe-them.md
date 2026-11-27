# 0707: The device path's indexes are built once, ahead of the strata that probe them

Status: decided (2026-11-26), by the user: "If joins need indexes, then yes the planner should require that they be
built first", then "go ahead with the planner change. It needs to account for index's cost and build them as part of
the execution plan". Being built. The device path's half of decision 0706's "Indexes: declared, planned, built ahead,
shared" (the django-bugs line, the CPU walker's): one catalogue of indexes when the two lines merge. Follows 0353
(blocks never copied) and 0478 step 3 (a view's indexes in its own home).

## What was there

Each stratum the device takes (its joins run by the device, or by its join program on the host: `RERAIL_GPU=cpu`)
builds the index of each relation its stages probe, in its own batch, unless a committed batch built it before. The
data layer's 57 rules over corpus0's first 50 files, on the host's join program, built indexes of 134 M rows (each
build counted as its batch ended) where about 15 M would build each once. By what each built (a trace of the builds):

| Built again, because | Rows built | Once each |
|---|---|---|
| `within` and `docof` read as a copy each stratum (0353 step 6's upload), its index built in the copy | 84.7 M | 4.0 M |
| A view's home out of room (4 keys; a placed relation 1), the index built in a home of the batch's own | 16.6 M + 7.9 M | |
| Concurrent strata each building what no committed batch had, one given back | 12.2 M | |
| A recursive stratum's own relation, built again as it grows each round | 13.9 M | 3.5 M |

## The decision

1. **A relation a stratum probes is read where it lies, never copied for it.** The two copies that cost most go:
   - `within` with tape values admitted into its chunk (`values::Layered`): solved on the host, where no device
     solves it, into one relation in its block, as the device's solve writes it (`Within::solved_on_host_with`: its
     batch's rows, then each value's). Before, only the device solved a layered axis; on the host each stratum
     uploaded every row.
   - A placed relation (`docof`, `line`: the `Loc` view's rows, then `Extra`'s spans) with tape values admitted: a
     `PlacedDir` over the values' directory, the base's spans past its rows (`places::Over`), kept by the layered
     source so its indexes are found again. Before, a layered placed view was refused in place, and each stratum
     uploaded its 104,163 rows for the 16 rows of its values.
2. **A view's home has room for the keys its probes ask, and keeps them** (`rows::IndexHome`). A probe by a key past
   its room makes the home again with room for twice as many (at most a relation's 16), in its source's place, so
   every later stratum finds the index built there. Before, each such probe built its index in a home of its own,
   found by no one after (`kind` by its fifth key: 80 times).
   *Merged (2026-11-26) into the spike-shared-caches line:* the room is the hub's instead (roll-up 126): a view's
   home is made once with room for a relation's 16 added indexes (`Part::for_device_indexed(0, rows, ADDED_INDEXES)`),
   address space alone, its pages taken as they are written. No home is made again, so `rows::IndexHome` and the
   source's `grows` are gone; what point 2 asks (every later stratum finds the index where it was built) holds.
3. **The planner builds a level's indexes before its strata run** (`indexes_first`): the indexes its seed joins'
   device stages probe, in one batch of the device's, done before any of the level's strata (the level's waves and
   the issued components, where the count tables and the level's joins are prefetched). Each stratum finds each
   built. A relation its stratum would upload is left to it.

Not yet, and the next steps:

- **The cost.** The order search does not price an index: a probe by a key with no index costs what one with an
  index costs. 0706 §3 (`Available`: built, scheduled with its cost shared by its known readers, or absent) is the
  form; on the device path a probe has no fallback without its index, so the price is the build, shared by the
  level's readers of the key, and an order whose key no other reader shares pays it whole.
- **Recursive strata and later levels.** The build ahead covers seed joins; a recursive stratum's delta joins, and a
  stratum issued in a later group than the level's build, still build their own (4.3 M rows built twice remain).
- **A recursive relation's own index**, built again each round (13.9 M rows): its index grown with its rows, as a
  live relation's is (`try_reserve_index_live`).

## Measured

The 57 data-layer rules over corpus0's first 50 files, cold, `RERAIL_GPU=cpu`, this Mac (load 12–25): 527 outputs
and 48 findings equal to the scalar joins' after each step.

| Step | Instructions | Rows indexed | Of them built twice, given back | Uploads |
|---|---|---|---|---|
| Before (3811351d67) | 634–671 G | 134 M | 3.6 M | 1.34 GB |
| 1, the two copies gone | 560 G | 45 M | 3.3 M | 5 MB |
| 2, homes that grow | 520 G | 43 M | 12.2 M | 5 MB |
| 3, indexes first | 521 G | 30 M | 4.3 M | 5 MB |

Step 2's homes made the concurrent strata's builds of one index meet in one home, so the second is given back
where it was a home of its own before; step 3 builds them first. The run's instructions did not move with step 3
past the noise (one run each, the Mac at a load of 12–25).

Tests: `a_chunks_axis_and_places_with_a_value_admitted_are_read_where_they_lie_on_the_host` (tests-lang): a chunk with
a tape value admitted, its sources layered as the run layers them, gives `within` solved on the host into one relation
in its block, every pair the value's too, and `docof` a placed read over the values' directory, every row, with a home
its source keeps.

Open, seen while testing this (both on the head before it too):

- `cpu_and_metal_read_the_same_tape_values` (tests-corpus) breaches the one-launch check on the CPU device: the HTML
  file's scripts' `docof` and `line` (42 rows) are not in `views_in_place` (no tape of their own), so the stratum makes
  each with `view_expand` (copy 0353 step 4), a launch past its program.
- One cold run of the 50 files stopped making progress after 266 strata (killed at 300 s); the next, the same binary,
  ran through in 15 s.
