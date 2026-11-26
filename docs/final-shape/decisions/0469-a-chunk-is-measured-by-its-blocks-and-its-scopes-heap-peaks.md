# 0469. A chunk is measured by what its blocks wrote and its scopes' heap at their most

Status: built (audited 2026-11-23).

Date: 2026-11-19. Status: decided (the user, 2026-11-19, on the merge of Native-and-GPU-strata d719a840ef into
investigate-bend-vs-datalog: "0435's schedule, Q203's measure"). Amends 0435 §4 and §6 (the measure); keeps 0435
R1-R6 and Q203 item 3.

## Intro

A corpus run cuts its files into chunks, and sizes each chunk from what the chunks before it took. Two branches
measured that differently:

- **0435** (Native-and-GPU-strata) cuts the chunks by a schedule (`plan/corpus/sizing.rs`, `sizing::Schedule`): the
  first chunk small, each next up to eight times the last, decided from the chunks filed before it in their order,
  so every run cuts the same chunks (R6). Its measure is what the chunk's own blocks of relations wrote
  (`rows::in_chunk_measured`, `Written::bytes`). That is the same on every run, but it leaves out the heap: a typing
  program's or a matcher's own vectors and maps, which are most of what a chunk holds on the data layer's 50
  policies (Q355).
- **Q203 item 3** (benchmarks-on-integration, b6d2c1eba0) measured a chunk by its live blocks plus its jobs' net
  heap, from jemalloc's per-thread counters, at the most seen at a block made or a job's end
  (`rows::Chunk::held_most`). That counts the heap, but its "most" is sampled across threads: when two jobs of a
  chunk run at once, which moment is seen depends on their timing, so two runs could size two chunks apart.

## Decision

The chunks are cut by 0435's schedule, and each is measured by Q203's account made the same on every run:

- **What its blocks wrote**: 0435's `Written::bytes`, as before.
- **Its scopes' heap at their most**: each scope of the chunk on a thread (its evaluation, and each job carried
  into it, `rows::enter_chunk`) resets the allocator's per-thread peak at its entry (jemalloc's
  `thread.peak.reset`) and reads it at its exit (`thread.peak.read`): the most that scope's own allocation reached,
  above what the thread held when it began. A scope nested in another (a job of another chunk run on the thread
  while it waited) is that chunk's: the outer scope's peak stops at its entry and goes on from its own allocation at
  its exit. The chunk's heap is the sum of its scopes' peaks (`Written::heap`).
- **The measure** is the two together (`Written::measure`), filed as `sizing::Measured::bytes`.

A scope's peak is a function of what the scope allocated, in order, on its own thread: what ran on other threads
meanwhile does not move it. So the measure, like 0435's, is the same on every run of a chunk (R6). The sum of the
scopes' peaks is at least what they held at once, and at most what they would hold if every peak met: an upper bound,
which is the side a memory budget errs on.

## What goes

- `host_added_mb`, the chunk's growth of the process's footprint less the device's MB (Q203 before item 3), and its
  test: the footprint is the memory line's, not a chunk's size (0435 §5). That the device's buffers are no chunk's is
  kept by `held_of` (`a_chunk_holds_what_it_handed_on_not_a_device_runs_own`).
- The chunk sizing's constants of before 0435 (`PROBE_NODES`, `CHUNK_MB`, `RUN_MB`, `CHUNK_FLOOR_MB`,
  `CHUNK_CREDIBLE_MB`, `MIN_CHUNK_NODES`) and `device_mb`: 0435's schedule replaces them.

`rows::Chunk::held_most` stays, as the profile's account of a chunk's live blocks and its jobs' net heap at a
moment, and its test (`a_chunk_holds_what_its_own_work_allocated`). Its blocks are counted by their used bytes, not
their reservations: since 0445 a block reserves 16 GiB of address space.

## Requirements, each locked by a test

- **R1** A chunk's heap is its scopes' own peaks: what it freed before its end counts at its most, a job of another
  chunk run inside it is that chunk's, and the same work measures the same (`a_chunks_heap_is_its_scopes_own_peaks`).
- **R2** 0435's R6 holds: `the_sizes_are_the_same_whatever_the_timing`,
  `a_chunk_measures_what_its_blocks_wrote_the_same_every_time`.
- **R3** Q203 item 3 holds: `a_chunk_holds_what_its_own_work_allocated`.

## Measured

`leg-083-typed` on 901 files of the data layer's repository (1,972,112 nodes, the CPU binding, the M5 at load
27-43, 2026-11-19): 8 chunks, held at the caller's most (262,144 nodes), against 80 before the merge, cut to the
8,192-node floor by the process's footprint. A typing chunk measures 932-1,043 bytes a node: about 120 in its
blocks, the rest its scopes' heap peaks (the 30 KB a node Q355 saw was the footprint's rise, not the chunk's own).
The count when the chunks begin leaves 959 MB of the 1,152 MB room; the footprint is 144-297 MB then. The per-chunk
costs went with the chunks (the statistics 4.95 to 0.02 s, the views and tables 3.73 to 0.02 s, the order probes 1.6
s to 0); the findings are the same; the peak footprint 1,342 to 1,124 MB, system time 9.0 to 4.7 s.

The chunk-size line prints the blocks' bytes and the heap's rise apart.
