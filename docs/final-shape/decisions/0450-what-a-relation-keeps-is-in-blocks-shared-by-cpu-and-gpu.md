# 0450. What a relation keeps is in blocks, the same for the CPU and the GPU

Date: 2026-11-19. Status: decided (user, 2026-11-19, on Q261); built in steps below.

## Intro

A join reads relations. Around the relations, the engine keeps things it derived from them so it need not derive
them again:
- indexes and sorted orders;
- statistics and count tables;
- the results of text tests and text functions for each symbol;
- the device's handles to all of these.

Today most of these are Rust structures in the heap: maps, vectors and `Rc`/`Arc` graphs. The CPU keeps one set on
each relation (`setfix::InRel`: `idx`, `chains`, `dead`, `counted`, `tables`, `keyed`, `vers`). The GPU keeps
another, per thread (`setfix::device::RESIDENT`). The same index can exist twice: once in the relation's block,
which the GPU reads, and once as the CPU's hash table (`Csr`).

Q261 found what that costs. The device's per-thread cache holds a handle to each relation it read in place, and the
handle keeps the relation's pages. A chunk's device strata run in pool jobs on about 18 threads, and each thread lets
go only at its next device stratum. So a chunk's memory is freed by the threads' timing, and the chunks that run at
once differ from run to run.

Asked why the device caches relations at all, the user set the rule (2026-11-19):

> "Why are we caching relations? Aren't these available in the block they were read from? ... If we do need a cache,
> there should be one. It should be the same across CPU and GPU, have a bound by LRU (like the rolling LRU cache we
> designed for the GPU), and not contain stuff we already have elsewhere."

> "the requirement is also that we remove ALL pointer-chasing Rust data-structures, and use ONLY block
> data-structures shared between the CPU and the GPU"

## Decision

1. **Data a relation has is read where it is.** A relation's rows, dead rows, indexes and sorted orders are in its
   block (0325, 0353). Every reader, the CPU's or the GPU's, reads them there, through the reads written once (0310).
   No reader keeps a handle, copy or table of them apart.
2. **What is derived once and kept goes in a block structure the CPU and the GPU both read.**
   - A result that belongs to a relation (an index, a count table, a column's statistics) goes in the relation's
     block.
   - A result for each symbol (a text test's verdict, a text function's output) goes in the one cache. That is
     0353's rolling cache: generations of a hash index and an arena, kept with atomics, bounded by LRU, its reads
     written once in CLIF.
3. **No pointer-chasing Rust structure holds the engine's data.** No `HashMap`, `Vec` of entries, or `Rc`/`Arc`
   graph keeps a relation's rows or anything derived from them, on either side. A thread-local cache of such data is
   refused.
4. **The cache holds nothing held elsewhere.** It holds no relation, no index and no copy of a block. A copy that
   still exists (0353's list) is not cached; it goes with its 0353 step.

## Requirements, each locked by a test

- **R1** The device holds no handle to a relation read in place between strata. A relation the engine dropped holds
  no block, on any thread, once its last stratum is done.
- **R2** A relation's index that the GPU built is the one the CPU probes, and the reverse: one index in the block,
  found by both.
- **R3** A text test's verdict for a symbol, made by either side, is found by the other in the one cache. The cache
  stays within its bound and keeps what is read (LRU).
- **R4** A view's index is in a block (the chunk's), or the view is probed through the tape. It is not in a Rust
  structure.
- **R5** On the CPU, `InRel` keeps no index, order, dead-row list, statistic, count table or key run of its own: each
  is in the relation's block.
- **R6** Every run of hogql on Metal holds the same bytes past each chunk, so the same chunks run at once (Q261).
  It gives its 1,629 findings on both bindings.
- **R7** A check refuses a new thread-local or process-wide map of relation data (as `check-no-block-copies.py`
  refuses a copy).

## Inventory (2026-11-19)

| Structure | Holds | Its home under this decision | Step |
|---|---|---|---|
| `RESIDENT` entry, a relation read in place (`DevRel::in_block`) | the relation's place and the block's GPU mapping, and its `Part` | none: made at each stratum from the block, whose mapping the block keeps (`Block::gpu_buffer`) | 1 |
| `RESIDENT` entry's sorts of such a relation | an index's number and the mapping | none: the index is in the block (`find_index`) | 1 |
| `RESIDENT` `verdicts`, `mapped` | text tests' and text functions' results for each symbol of a column | the rolling cache, by (test or functions, symbol) | 2 |
| `RESIDENT` `texts`, `unions` | symbols' names copied for the device (0353 row 5) | none: the kernels read the strings block (0353 step 6) | 2 |
| `RESIDENT` entry of a view read in place (`DevRel::view`) and its sorts | indexes the view's probes built | the chunk block, or a probe through the tape (0353 step 4) | 3 |
| `RESIDENT` entry of rows uploaded or a view made (`upload`, `expand_view`) | copies (0353 rows 4, 5) | none: removed by 0353 steps 4, 6; until then made at each stratum, never cached | 3 |
| `AUTOMATA`, `TESTS` (device) | the device's automata, by pattern | a block of compiled automata, by the pattern's hash | 4 |
| `InRel::idx` (`Csr`: hash table and ids) | the CPU's indexes, apart from a whole source's | the relation's block, the index the GPU reads (`try_reserve_index_buckets`) | 5 |
| `InRel::chains`, `sorted` | sorted orders | the relation's block | 5 |
| `InRel::dead` | the dead rows | the relation's dead-row delta in its block (`InBlock::dead`) | 5 |
| `InRel::sketches`, `measured`, `counted`, `summaries` | statistics | the relation's block | 6 |
| `InRel::tables` | count tables (0098 §1) | the relation's block | 6 |
| `InRel::keyed` | a key column's runs | the relation's block | 6 |
| `InRel::vers` | a relation's versions as read | relations in blocks | 6 |
| `GROWTH` (device) | rows out per row, by rule and stage | the joins' memo (0444), which is filed in order | 7 |
| `FLUSHED`, `SUSPENDED`, `LAST` (device) | what is in flight: waves, waiting strata, the last report | open: these are the executor's control state, not data a reader reads. Asked of the user. | — |

## Steps

1. The device makes its handle to a relation read in place, and its sorts, at each stratum from the block. A
   stratum in flight keeps its own handle until it completes. R1, R6.
2. The rolling cache holds the verdicts and the text functions' outputs; the CPU's text tests read it too. R3.
3. Views' indexes go in the chunk block, and the copies are no longer cached. R4.
4. The device's automata go in a block.
5. The CPU's indexes, orders and dead rows go in the relation's block. R2, R5.
6. The CPU's statistics, count tables, key runs and versions go there too. R5.
7. `GROWTH` joins the joins' memo. Then R7's check.

## Measured (2026-11-19, step 1)

posthog/hogql cold, Metal, profiling builds, format 15, each run with an image store of its own
(`RERAIL_IMAGE_STORE=project`), an M-series Mac at load 37-43; the two builds interleaved, three runs each; 1,629
findings in every run:

| | Instructions | Held past each growing chunk (MB) | Peak footprint |
|---|---|---|---|
| fa53bc6a6 (before) | 74.2-76.2 B | 3/15/56, 10/6/12, 69/69/24, 11/4/78, 5/13/12, 21/24/23 | 1,404-1,569 MB |
| step 1 | 74.9-76.5 B | 2, 6, 4, 4, 4, 1 in every run | 1,400-1,479 MB |

- The relations read in place found again went from about 338 a run to 92-132: each is now made from its block.
  The sorts are found in the block (`find_index`) as before. No cost was measured: an earlier, separate set of
  step 1 runs at 79-81 B was the time it ran at, not the change.
- Not yet the same chunks at once in every run (R6): a chunk's bytes written still differ on Metal (one chunk
  144-156 MB). A device run's own blocks are sized by the capacity estimates (`GROWTH`, per thread, step 7), and a
  stratum the device refuses at run time (4-29 a run, by the device room left) writes on the CPU instead.
