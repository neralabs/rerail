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
| `RESIDENT` `verdicts`, `mapped` | text tests' and text functions' results for each symbol of a column | the rolling cache, by (test or functions, symbol); `verdicts` gone in 2d | 2 |
| `RESIDENT` `texts`, `unions` | symbols' names copied for the device (0353 row 5) | none: the kernels read the strings block (0353 step 6) | 2 |
| `RESIDENT` entry of a view read in place (`DevRel::view`) and its sorts | indexes the view's probes built | the chunk block, or a probe through the tape (0353 step 4) | 3 |
| `RESIDENT` entry of rows uploaded or a view made (`upload`, `expand_view`) | copies (0353 rows 4, 5) | none: removed by 0353 steps 4, 6; until then made at each stratum, never cached | 3 |
| `AUTOMATA`, `TESTS` (device) | the device's automata, by pattern | a block of compiled automata, by the pattern's hash (`text::automata`, built in 2c) | 4 |
| `InRel::idx` (`Csr`: hash table and ids) | the CPU's indexes, apart from a whole source's | the relation's block, the index the GPU reads (`try_reserve_index_buckets`) | 5 |
| `InRel::chains`, `sorted` | sorted orders | the relation's block | 5 |
| `InRel::dead` | the dead rows | the relation's dead-row delta in its block (`InBlock::dead`) | 5 |
| `InRel::sketches`, `measured`, `counted`, `summaries` | statistics | the relation's block | 6 |
| `InRel::tables` | count tables (0098 §1) | the relation's block | 6 |
| `InRel::keyed` | a key column's runs | the relation's block | 6 |
| `InRel::vers` | a relation's versions as read | relations in blocks | 6 |
| `GROWTH` (device) | rows out per row, by rule and stage | the joins' memo (0444), which is filed in order | 7 |
| `FLUSHED`, `SUSPENDED`, `LAST` (device) | what is in flight: waves, waiting strata, the last report | stays on the CPU for now (the user, 2026-11-19: "Bookkeeping / scoreboards / schedules can be on the CPU for now; if we want device-side enqueuing everywhere we will need to do so later") | — |

## Steps

1. The device makes its handle to a relation read in place, and its sorts, at each stratum from the block. A
   stratum in flight keeps its own handle until it completes. R1, R6.
2. The rolling cache holds the verdicts and the text functions' outputs; the CPU's text tests read it too. R3.
   A verdict made where a reader reads it needs the symbol's name and the test's automaton in blocks, so step 2 is
   built in this order (2026-11-19):
   - **2a.** The run's pending atoms in atoms blocks (`rerail_blocks::atoms`, the stored chain's format), a chain
     of anonymous blocks each twice the last. `Interner`'s `buf`, `ends`, `hashes` and `table` go. A block wholly
     stored is dropped; its atoms are read in the store's chain.
   - **2b.** A family of reads over the atoms chain, written once in CLIF: an id's bytes, for the Rust host, the
     JIT and the kernels.
   - **2c.** The text tests' automata in a block (step 4 brought forward). Built without the CLIF read this step first
     named: the device runs a table by composing 32 bytes' transition functions across a SIMD group, which CLIF
     cannot express, and a CPU read of the table would be a loop a byte at a time, which the compiler refuses. The
     CPU runs the same lowered plans through its word-at-a-time searchers (`text::plan::Compiled`), as before; the
     table is made from those plans, and `a_tests_table_answers_as_its_matcher` keeps the two answering alike.
   - **2d.** The verdict cache: the one rolling cache, keyed by (test, symbol), filled by running the test's
     automaton over the symbol's bytes; read by the CPU's text tests and the kernels. The device's verdicts by
     column, its names uploaded (`texts`, `unions`) and `column_symbols` go.
     Chosen by the user (2026-11-19): an entry holds 64 symbols, keyed by (test, symbol >> 6): 64 verdict bits and
     a 64-bit mask of those known, so 32 bytes cover 64 symbols and neighbours share an entry's LRU life. The cache
     is the Machine's (`Shared`, decision 0442), passed in, never global; it lives across a session's runs, as
     symbols are the store's ids and a test is keyed by its automaton's slot (`text::automata`).
   - **2e.** The text functions' outputs (`mapped`), by (functions, symbol). Decided by the user (2026-11-19,
     asked why a cache's entries are fixed in size): the rolling cache's entries are of any size up to a most, each
     generation a fixed hash index and a bump arena; a text is an entry of its own length (most 4 KB a text; a
     longer one is made each time it is read, not kept). Built in two parts: the cache (below, "Built, step 2e,
     the cache"), then the texts on it.
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

## Measured (2026-11-19, step 2a)

The pending atoms in atoms blocks: hogql cold, profiling builds, format 15, load 70-81, interleaved with step 1, two
runs each on the CPU: 84.4 and 88.9 B instructions against 85.5 and 87.6 B, peak 289 and 334 MB against 298 and
339 MB; Metal 74.7 B, 1,283 MB. 1,629 findings in every run. `Interner::store_upto` no longer copies the atoms still
pending into a new buffer.

## Built (2026-11-19, step 2b)

- `rerail_blocks::atoms::AtomsDir`: a directory of runs of atoms in a block of its own (`AtomsDirRoot`, its
  entries `DirEntry`). Each entry names a run's ids, its block's first id, and spans to the block's `ends` and
  `strings`, whose offsets are in the reader's address space. It is made for the host (`|p| p as u64`) or for the
  device (each block's GPU address); nothing of an atom is copied into it.
- `atoms_len` and `atoms_at`, in CLIF (`rerail_clif::atoms_reads`): an id's run found by a binary search of the
  runs, its string's bounds from the block's `ends`. Generated as the blocks crate's Rust (`generated_reads.rs`,
  `atoms::str_in`) and MSL (`relation_reads.metal`, `clif_atoms_len`, `clif_atoms_at`), which every join kernel now
  compiles with.
- `Interner::dir` lists the store's views below `base`, then the pending blocks.
- Tests: `the_directorys_reads_find_every_atom_where_its_block_holds_it` (blocks, a run that begins past its block's
  first id), and in `a_commit_past_a_blocks_room_starts_the_next` every sampled id across a rolled-up base and pending
  blocks reads as `Interner::str` does. A kernel reading through a device directory comes with 2d, its first reader.

## Built (2026-11-19, step 2c)

- `text::automata`: one block holds every test table and replacement automaton, each written once, at a 256-byte
  offset, under an index of 16,384 slots of 64 bytes in the same block, keyed by a hash of the plans' key (or the
  pattern). A slot is published by its state (Release) and read with no lock; a refusal is kept with why.
  `text::table`'s two process-wide maps (`TABLES`, `MADE`) go, and so do the device's thread-local `AUTOMATA` and
  `TESTS` and every upload of an automaton (`DevTable::upload`, `DevBytes::upload`, `hold_table`). The kernels bind
  the block's no-copy wrap at the automaton's offset (`Pass::run_bytes_at`).
- The block is the process's, as the maps it replaces were: automata come from the rules' patterns, not from
  relations.
- Tests: `an_automaton_is_made_once_into_the_block_and_found_there`; the table and replacer tests read their bytes
  from the block; the 16 setfix device tests pass on a Mac, `text_tests_on_the_device_are_the_cpus` and
  `text_functions_on_the_device_are_the_cpus` among them.
- hogql cold, profiling, format 15, load 52-59: Metal 75.6 B against 2a's 76.0 B, peak 1,335 against 1,369 MB, no
  stratum refused; CPU 84.6 B, 348 MB. 1,629 findings in every run.

## Built (2026-11-19, step 2d, the reads and the CPU)

- `verdict_get` and `verdict_put` in CLIF (`rerail_clif::verdict_reads`), over the rolling cache's `cache_get`: an
  entry of 64 symbols of one test, its payload two masks (known to pass, known to fail), each verdict one atomic or,
  so a copy into the head may miss a verdict and never misreports one; the dead entry keeps none. The Rust backend
  gained CLIF's atomics (load, store, add, or, compare-and-swap, fence; u32s), both backends an atomic or.
- The cache is the store's atoms' (`content_store::atoms::Verdicts`, `Atoms::verdicts`): 0442's `Shared` is not on
  this branch, and the store's atoms are the per-caller state it moves there. Three generations of 32,768 entries
  (2 M symbols of a test each), made when a test first asks; `Spec::init_zeroed` writes only its header, stamps
  and guard, so pages of entries never taken are never touched (`init`'s whole fill had added 35-50 MB to hogql's
  peak, one cache per `Atoms`).
- A test's id is its plans' key named in the automata block's index (`text::table::test_id`, `automata::id`): the
  same for the CPU's matcher and the device's table, and nothing is built for it.
- The CPU's `Step::Test` of constant patterns reads the verdict, else tests the name and keeps the verdict.
- Tests: `a_verdict_kept_is_found_and_the_cache_keeps_what_is_read` (the reads, the bound, LRU),
  `a_cpu_text_test_reads_and_keeps_its_verdicts_in_the_stores_cache` (a verdict put before the run is the one the
  rule takes). The 16 setfix device tests pass on a Mac with the regenerated kernel sources.
- hogql cold, CPU, profiling, format 15, load 36-42, interleaved with 2c: 83.9 and 84.4 B against 83.9 and 84.3 B,
  peak 382 and 397 MB against 380 and 389 MB; 1,629 findings.

## Built (2026-11-19, step 2d, the device)

- A text test of constant patterns, of a symbol an input column bound, is a `row_text` stage of op `ROW_VERDICT`:
  each lane looks up a row's verdict in the store's cache, 32 rows a SIMD group; the group then walks the test's
  table over each missed symbol's bytes, read through an atoms directory made for the device's addresses (`mj::
  DevVerdicts`: its spans GPU addresses, the atoms blocks made resident by `use_resource`), and keeps the verdict
  with `verdict_put`. A symbol the directory does not hold sets the stage's counter's bit (`TEXTFN_UNHELD`) and the
  stratum runs on the host.
- The atoms blocks are `Arc<Block>` (`AtomsW`, `AtomsV`), so a batch keeps the blocks its directory reaches until
  its work completes, though the store lets a pending block go meanwhile.
- Gone: `Kind::Text`, mode `TEXT` (5) and its `text_pass`, the `text_verdicts` kernel, `Batch::verdicts`,
  `DevTable`, `setfix::device::verdicts` and the entries' `verdicts`. The column's names for a text test are no longer
  uploaded (`column_text` stayed for the text functions until step 2e's second part).
- `row_scatter` binds no value for a verdict stage, as for `ROW_TEST`: a first build wrote the stage's (unwritten)
  value into slot 0, so a later join on that slot, or a head reading it, saw 0 (`text_tests_on_the_device_are_the_
  cpus` and `a_stratum_run_apart_beside_an_open_wave_reads_what_the_wave_made` caught it).
- Tests: `a_text_test_on_the_device_shares_its_verdicts_with_the_cpu` (a verdict the CPU kept is the one the device
  takes; the device's verdicts, a long name's among them, are there for the CPU after; the rows keep their names);
  `every_verdict_the_device_keeps_is_the_cpus` (2,000 names read through SQL: every symbol's verdict in the cache
  is the CPU matcher's, and the head's names are the matching rows').
- hogql cold, Metal, profiling, format 15, at load 57-69, after the fix: four runs of 74.8-77.4 B, peak 1,297-1,353
  MB, against step 2c's three runs at the same load of 76.6-82.4 B, peak 1,228-1,399 MB; 1,629 findings in each (the
  new binary's first launch, 100 B and 35 s, left out). The build before the fix stopped once in five runs at the ast
  matcher wave's `BackendCapacity` (work queue, Q261 "Found"); none of the five after it did.

## Built (2026-11-19, step 2e, the cache's entries of any size)

Amends decision 0353 step 4b's rolling cache, whose entries were of one size (its log2 in the header, `SHIFT`):
that size came from the cache's first use, a 16z block decoded, and nothing else needs it.

- **Layout.** A generation is its words (its stamp, its entries taken, its arena's bytes taken `BUMP`, its writers
  `WRITERS`), its hash index (u32 slots: a tag above an entry's number plus one, as before), its entries' offsets in
  its arena by number (a u32 each), then its arena. An entry is its key, its completion and its bytes (`LEN`: header
  and payload), then its payload; entries are 64-byte aligned. The header's `MAX` (was `SHIFT`) is the most bytes an
  entry takes. `Spec::arena(gens, entries, arena, max)` makes any shape; `Spec::new` keeps the one size for its
  callers (the 16z caches, the verdicts).
- **A generation is full** when its index's entries or its arena's bytes are: the bound is in bytes too. An entry
  past `MAX` is not kept (the dead entry, a failure counted) and turns no generation. A hit in the oldest generation
  is copied with its own length.
- **No write of an older epoch lands in a newer one's entry.** The fixed size gave this by the entry's own completion
  word, which a writer of an older epoch still held. With entries of any size a new one can overlap an old writer's
  partly, so a writer counts itself in its generation from before it takes an entry to its completion, and checks
  after counting that its generation is still its epoch's. The thread that turns the head unstamps the next spare
  and clears it only with no writer in it (`cache_clear`, one thread by a compare-and-swap of its stamp); else the
  spare stays unstamped and a later full head clears it. Every loop stays bounded.
- **An open lookup** (`reads::cache_lookup` with no fill) waits for no writer. It returns the entry found (`FOUND`),
  an entry taken for the size asked that its caller writes and completes (`TAKEN`, `cache_done`), `PENDING` (another
  thread writes it), or `NOT_KEPT`, in the result's low bits. A device pass that measures a text then writes it in a
  later pass uses it. Reads `text_find`, `text_open`, `cache_done`, in CLIF; a text's key has bit 63 set, so none
  meets a verdict's in one cache.
- Test: `texts_of_any_length_are_kept_in_the_arenas_by_bytes` (lengths 0 to 200 apart and aligned; past the most not
  kept, nothing turned; pending while written; full by bytes, a text read as others come kept and one not read let
  go; a generation with a writer in it not cleared under it, and cleared once the writer is done). The verdicts' and
  the 16z caches' tests pass unchanged.
- hogql cold, Metal, profiling, load 35-44, interleaved with the build before (32cdea49d), 1,629 findings in every
  run. Format 15: 76.3-78.0 B against 76.9-80.7 B, peak 1,350-1,492 MB against 1,317-1,485 MB. Format 16z (the
  block cache's own runs): 102.9 B against 101.8 B, peak 2,297 MB against 2,232 MB; no lookup failed, none turned
  a generation, 6,155 misses against 5,898. Left out: each build's first run of each format, which compiles the
  kernels whose reads changed (180 B and 124 s; 124 B).

## Built (2026-11-19, step 2e, symbols read in place by every text kernel)

- **One directory a batch.** The atoms directory (`DevAtoms`: its block's wrap at its root, the atoms blocks' wraps
  it reaches, made resident where a kernel reads through it) is the batch's, made at the first stage that reads a
  symbol's text and made again only when the store's atoms have grown past it (the one before kept until the work
  that read it completes). `DevVerdicts` keeps the cache and the test alone.
- **Every text kernel reads a symbol's bytes through it** (`sym_text`: `clif_atoms_len`, `clif_atoms_at`): the text
  ops (`TEXTOP`), `str_cmp` and `strlen` (`STRCMP`), and every made text per row (`ROWTEXT`). A symbol the directory
  does not hold reads as empty and sets the stage's `TEXTFN_UNHELD`: the stratum runs on the host. These stages now
  each have a counter word.
- **A bound pattern holding the set separator** (`\u1f`) was refused when its column's names were uploaded; nothing
  is uploaded now, so the kernel finds it and sets `TEXTFN_HOST` on the text op's word: the stratum runs on the host,
  named (`a_bound_pattern_holding_the_set_separator_runs_on_the_host`).
- **The text functions of constant arguments run per row**, as every other text function does, a SIMD group a row
  into the batch's arena. They were made once per distinct symbol of the column (`Kind::Mapped`), over the column's
  names uploaded (`column_text`). Kept per (functions, symbol) in the rolling cache is the next part.
- Gone: `texts` (0353 row 5's names), `DevText`, `Kind::Mapped`, `TextOp`, `per_symbol`, `mapped_text`,
  `column_text`, the entries' `texts` and `mapped`, `Resident::unions`, the run's `checks`, `Batch::replace`,
  `Batch::text_map`, and the kernels `text_replace` and `text_map` with the text buffer's reads (`text_find`,
  `text_chars`).
- Tests: the 19 setfix device tests and the `metal_join` tests pass on Metal. `text_functions_on_the_device_are_the_
  cpus` (17 rules, 2,000 names, the arena filled and run again) now asserts that no names are uploaded and no
  per-symbol table is made; its rows are the CPU's.
- hogql cold, Metal, profiling, format 15, load 28-33, interleaved with the build before (d9ea6f1ec), 1,629 findings
  in every run: 75.9-77.2 B against 77.5-80.4 B, peak 1,365-1,416 MB against 1,443-1,494 MB. No stratum ran on the
  host for an unheld symbol or the separator; the refusals at run time are the device's room (Q261) in both builds.
  Left out: the new binary's first run, which compiles the changed kernels (89 B, 25 s).
