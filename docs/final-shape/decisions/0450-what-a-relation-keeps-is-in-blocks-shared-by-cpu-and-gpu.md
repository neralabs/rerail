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

## Built (2026-11-19, step 2e, the text functions' texts in the one cache)

- **One cache, verdicts and texts.** The store's atoms keep one rolling cache (`atoms::Kept`, was `Verdicts`), now
  of arena generations: 3 of `KEPT_ENTRIES` (65,536) entries and `KEPT_ARENA` (16 MB) each, an entry at most
  `TEXT_MOST` (4 KB) of text. A verdict entry is 32 bytes (64 aligned), a text its own length. The batch holds it
  on the device (`DevKept`), beside its atoms directory; `row_stage` takes the stage's id in it (a test's, or the
  functions').
- **A text function per row keeps its text** (`ROW_KEPT`) when its text argument is an input column's symbol, or a
  kept text of one, and its other arguments are constants: the regex replacements and every `TEXTFN_*` of a text.
  Its key is the chain of functions from the source symbol (`/dir[]`, `/rr_substr[0, 9]/sql_upper[]`: the rule's
  function names and constants, so the CPU names a chain as the device does), named in the automata block's index
  (`text_fns_id`), with the source symbol in key 7. One lane looks it up: a kept text is copied into the arena; a
  text made is written to the arena, then into an entry taken for it, and completed. A text past 4 KB, a row
  `lower` sends to the host, and a symbol the directory does not hold keep nothing.
- Test: `text_functions_keep_their_texts_in_the_one_cache` (600 names: after a device run every name's `dir` text
  and its chain's are in the cache, equal to the CPU's; a second run reads them back, the same rows). The 24 setfix
  device and `metal_join` tests pass on Metal; the CPU's verdict tests pass.
- hogql cold, Metal, profiling, format 15, load 60-70, interleaved with the build before (c7344deba), 1,629 findings
  in every run: 75.9-76.1 B against 76.0-76.4 B, peak 1,343-1,371 MB against 1,231-1,366 MB. Left out: the new
  binary's first run, which compiles the changed kernel (101 B, 43 s).
- **The CPU's text functions read and keep it too** (2026-11-19). A walk's `Step::Func` of a text function the device
  makes (`caps::device_fn`), of a symbol, its other arguments constants, carries the id of its one-step chain
  (`kept`, `text::automata::text_functions("/{name}{constants:?}")`, as the device names it). It takes the text kept
  for the symbol, else makes it (`eval`) and keeps it (`Kept::text`, `Kept::keep_text`); it then interns it as
  before. So a text either side made is the other's: a CPU's `lower` past ASCII, which the device sends to the host,
  is then found on the device. A chain on the CPU keys each step by the symbol its last step interned; the device
  keys it by the source symbol, so only one-step texts are shared.
  - Test: `a_cpu_text_function_reads_and_keeps_its_text_in_the_stores_cache` (a text put there before the run is
    the one the run takes; the others' texts are kept as it ran).
  - hogql cold, profiling, format 15, load 42-50, interleaved with the build before (640e19de6): CPU binding 84.7-
    87.4 B against 85.3-89.6 B, peak 349-372 MB against 351-367 MB; Metal 77.0-80.8 B, peak 1,292-1,434 MB; 1,629
    findings in every run.

## Built (2026-11-19, step 3: no device cache of relations)

- **A view read in place keeps its indexes in a block of its own** (R4): its relation of no columns, which the
  view's source (`syntax::Viewed`, its batch's life) makes the first time the device reads the view, and hands out
  with its device view (`DeviceView::home`). Every stratum's handle is made over it (`DevRel::view_in`), so the
  indexes any stratum built there are found (`find_index`) by the next, on any thread; they go with the batch.
- **A copy is made for its stratum, never cached**: rows uploaded from the host and views made on the device
  (0353 rows 4, 5) are made at each stratum that reads them, holding the columns it reads, their indexes built with
  them. The batch keeps each handle it is given until its work completes (`Batch::keeps`).
- **Gone**: `RESIDENT` (the thread's cache of relations, the last thread-local of relation data on the device side),
  its `Entry`, `Owner`, `trim`, `resident_max`, `unrun`, `forget_dropped` (and the device trait's), the test hooks
  `kept_in_place`, `resident_dropped`, and the profile's "found resident" counts. `column_stats` reads a relation
  in its block in place and gathers any other's column on the host.
- Tests: `a_views_index_is_in_its_block_for_every_thread` (another thread's stratum, in another batch, finds the
  index the first built, and builds none); `a_view_made_on_the_device_is_made_at_each_stratum` (was
  `a_view_made_in_a_refused_batch_is_made_again`); R1's check in `column_stats`' test is now that nothing else holds
  the relation. The 25 setfix device and `metal_join` tests pass on Metal.
- hogql cold, Metal, profiling, format 15, load 58-84, six pairs interleaved with the build before (8596fd54c),
  1,629 findings in every run: 74.0-82.1 B (four of six 74.7-75.9 B) against 73.3-76.3 B. The cost is the views
  uploaded from the host, now made per stratum: 145 uploads, 10.2 MB in 172-197 ms of CPU in every run, against
  65-96 uploads, 5.3-7.9 MB in about 88 ms. The count is now the same in every run; before, what a thread's cache
  held decided it, by the threads' timing (R6). These uploads are 0353 row 5's copies and go with its steps.

## Built (2026-11-19, step 5a: a relation's probes by its block's index)

- **A derived relation is probed by its block's own index** (R2): `InRel::index` takes an index already built in the
  relation's block first (by the device, or an earlier probe: `InBlock::found_index`), then a kept chain (an order
  its waves made, which costs nothing), and otherwise builds one there (`InBlock::own_index`), not an order or a
  table of its own. The device's `find_index` finds what the CPU built, and the reverse. A relation with dead rows
  is indexed over its rows, dead ones too, as the device indexes it (`DevRel::dead`); every reader skips them by the
  dead bits, and checks each candidate's key, as a bucket holds other keys.
- Rows in several parts not yet merged are not merged for this (`Rows::in_one_block`): such a relation keeps its
  order, as before, so no copy is added.
- `probe_size` stays exact for a bucket (decision 0247): it counts the candidates whose key is the probe's.
- Decision 0294's scan in the next step's key order applies where that step has an order: a component relation's
  waves. An input's probes go to a hash index, for which no scan is sorted. The cut test now probes `path`'s delta.
- Not yet: the orders (`chains`, `sorted`, `Index`'s runs) are still a directory on the heap over arrays in the
  block, and `dead` is still a vector; the map of `Csr`s too.
- Tests (`setfix::block_index_tests`): `a_cpu_probe_builds_its_index_in_the_relations_block`,
  `a_cpu_probe_reads_the_index_the_device_built`, `a_probe_skips_the_dead_rows_of_the_blocks_index`,
  `a_relations_index_holds_no_table_of_its_own`. setfix's 75 CPU tests pass (on a Linux build), and the 25 device
  tests on Metal.
- Found on the way: the Linux build had been broken since 0435 step 1 (19fba5eb2a). `rerail-blocks` was a macOS
  dependency of `rerail-accel` and `rerail-facts`, and `Source::in_block` macOS only, though the CPU reads them on
  every target. They are on every target now.
- hogql cold, profiling builds, format 15, load 18-27, three pairs each interleaved with step 3's build (04a2cf4cc),
  1,629 findings in every run:

  | | Instructions | Against (step 3) | Peak | Against |
  |---|---|---|---|---|
  | CPU | 83.2-83.4 B | 84.0-84.7 B | 410-417 MB | 413-431 MB |
  | Metal | 77.6-77.9 B | 77.4-79.4 B | 1,477-1,484 MB | 1,451-1,490 MB |

  On Metal the device finds more of its indexes built: 215-248 a run against 189-229.

## Built (2026-11-19, step 7: the growths in the joins' memo; R7's check)

- **A device stage's growth** (rows out per row of its input's space, which guesses its next run's space) is in the
  joins' memo (0444), by rule and stage, not a thread-local map (`GROWTH`, gone). A chunk's strata read it as filed
  before the chunk began, never as its own or a neighbour's strata ran: a chunk's growths are kept for its filing
  (`joins::note_growth`) and filed in the chunks' order, a stage's most of the chunk standing (`Op::within`). A
  recursive component's rounds read their own rounds' growths first, in the component's order (`Own::growth`).
- **R7's check**: `scripts/check/check-no-relation-maps.py` (pre-commit, beside `check-no-block-copies.py`)
  refuses a commit that adds a `static`, thread-local or process-wide, whose type holds a collection, in the
  engine's data paths, unless it or the line above names it `state: bookkeeping`, `state: not relation data`, or
  `state (0450 step N)`.
- Tests: `a_stages_growth_is_read_as_filed_never_as_the_threads_ran` (plan.rs, beside 0444's R1);
  `a_relation_moved_for_an_index_lives_until_its_batch_is_done` (below).
- **Found while measuring: step 3 gave back a relation the GPU was building an index into.** When a relation's
  block or directory has no room for an index, `Batch::index` moves it (a view's fresh home of no columns, or the
  relation copied, 0353 row 6) and hands it back in the index (`DevIndex::moved`). `RESIDENT` held that index, and
  with it the moved relation, past the stratum; after step 3 nothing held it past the stage that encoded it. So the
  relation was released while the batch built into it, and another made in its place. Of 16 hogql runs on Metal of
  the builds with it (04a2cf4cc, 4242a36f5), one stopped at Q253's check (an index count reset, "a relation's
  descriptor overwritten while its index was built") and one gave 1,037 findings instead of 1,629. The batch now
  keeps every relation it moves until its work completes: 12 of 12 runs gave 1,629, none stopped.
- hogql cold, Metal, profiling, format 15, load 31-38, four pairs against step 5a's build, then eight runs alone:
  74.7-78.5 B (ten of twelve 74.7-76.5 B) against 75.5-76.8 B; peak 1,322-1,463 MB against 1,313-1,457 MB.
  Stages run again after an overflow: 96-103 a run, against 91-100. The growths no longer follow the threads, but
  the reruns still differ between runs (the device's room, the text arena): R6 stays open.

## Decided (2026-11-19, the user, on step 5's orders)

Asked what a relation's kept sorted orders become in its block, the user chose **one sorted order**: when a relation
is frozen into an input (`InRel::from`'s, `Rel::based`'s start), each kept order's runs are merged once into one
sorted permutation of its rows, written in the relation's block and listed in its index directory as a kind beside
the hash index (a key, and the rows in its order). The CPU reads it there, and the GPU can (a binary search). The
heap directory of an input's orders (`Index`'s `Vec<(Arc<Run>, u32)>`, `InRel::chains`) goes. A relation's waves
(`Rel::idx`, while it is derived) keep their runs until it freezes. Rejected: the runs' directory written as block
descriptors (no merge, but a format only the CPU reads), and inputs keeping no orders (a maintained relation would
sort again).


## Built (2026-11-19, step 5b: a frozen relation's orders in its block's directory)

- **The directory holds sorted orders.** `rerail_blocks::relation::Index` has three more spans (`fences`, `tags`,
  `prints`) and the chunks' bytes it owns (`held`). Two kinds join the hash index: `ORDER` (a key, and every row
  listed in its order in `rows`) and `ORDER_ROWS` (rows `lo..hi` already in the key's order, none listed).
  `Relation::try_add_order` takes an order's chunks, written already in its block. `find_order` finds one by key
  and rows, and `orders` lists them. `forget_indexes` (and so `release`) gives back the chunks an order owns. The
  directory has room for 16 entries (`ADDED_INDEXES`, was 8), since the orders share it with the hash indexes.
  The generated reads (`generated_reads.rs`, `relation_reads.metal`) were written again for the new layout.
- **At freeze** (`InRel::moved`, `in_block_orders`), each order over every row has its runs merged into one, and its
  arrays (`BlockVec`s already in the relation's block) are handed to the directory of the part holding its last rows
  (`BlockVec::into_chunk`). `InRel::chains` keeps handles only. Each handle's run reads the directory's spans where
  they lie (`Run::in_block`, a borrowed `BlockVec::shared` that holds the part and never gives the chunk back). A
  write to a borrowed array moves it to a chunk of its own first, and a clone of one is another borrow.
- **A maintained relation** (`Rel::based`) starts from the old relation's handles. Nothing is copied: its runs read
  the old relation's directory, which the handles keep. Frozen again with no new rows, such an order stays read there.
- **Not handed on** (kept as it was, in its blocks): an order over fewer rows, an order whose arrays lie in another
  block, or an order whose directory is full (its chunks are its own again, `GivenRun::into_run`). The profile
  counts each kind ("frozen orders"). On hogql: 130 in their own directory, 130 read in an older relation's, 0 kept
  as they were, in one phase. None was left outside a directory in any run.
- Tests: `a_frozen_relations_orders_are_in_its_blocks_directory` and
  `a_based_relation_reads_its_old_orders_in_the_block` (`setfix/block_index_tests.rs`).
- **Found while measuring: a directory forgotten early.** A first build forgot a relation's whole directory at
  `unindex` when no `Part` but its own held it, hash indexes included. One Metal run of three then lost a whole
  chunk's findings (1,375 of 1,629). The relation's orders are now given back only at its release, as its hash
  indexes always were. 7 of 7 runs gave 1,629 after that. **Corrected after 5e:** that loss was not this change's.
  Step 7's build (d8f645ce4), made before 5b, lost a chunk too in one Metal run of four at load 20 (1,487 findings).
  Whether the early forget added to it is not known. The loss is open (below, step 5e).
- hogql cold, profiling, format 15, three pairs against step 7's build each:
  - CPU, load 32-36: 83.1-84.0 B against 83.0-83.5 B. Even.
  - Metal, the final build, seven runs at load 19-24: 75.5-79.2 B, against 76.2-77.2 B for step 7 at load 24-27.

## Built (2026-11-19, step 5c: the known-sorted ranges in the block)

- A relation's row ranges known to be in an order (decision 0180: `runs::Sorted`) are written once at its freeze in
  its block's relation descriptor (`Relation::known`, a span of `Known { lo, hi, ncols, cols }`, given back at its
  release; `try_set_known`). `Sorted` now holds the part whose block keeps its ranges (`base`) and the ranges noted
  since (`own`, while a relation is derived). A frozen input's `own` is empty: its ranges are read in the block.
  A maintained relation (`Rel::based`) starts from the old relation's part.
- Not written (kept as they were): a key past the block's `MAX_KEY` (8), or a block with no room. The profile counts
  both ("frozen orders"). On hogql all were written: 257 relations in one phase and 18 in another, 0 not.
- Found on the way: `in_block_orders` compared a key with setfix's `MAX_KEY` (64), not the block's (8). An order of
  9 or more columns would have failed `try_add_order`'s check. It now stays as it was.
- Test: `a_frozen_relations_known_sorted_ranges_are_in_its_block`.
- hogql cold, profiling, format 15, load 24-28: CPU 83.5-83.7 B against 5b's 83.3-83.8 B; Metal 74.7-76.3 B, 1,629
  findings in each run.

## Built (2026-11-19, step 5d: the dead rows in the block)

- A relation's dead rows (bit `id`) are a block array (`BlockVec<u64>`) while it is derived (`Rel::dead`) and after
  (`InRel::dead`), not a heap vector. So are those that hold them for a step: a maintained relation's origin
  (`Origin::Based`), a mark (`RelMark`), the statistics' dead rows when they were counted, and the levels' dead rows
  at their start. At freeze (`in_block_dead`), the chunk goes to the block relation that holds the relation's last
  rows (`Relation::adopt_dead`, `own_dead`; given back at its release). From then on `InRel::dead` reads it there,
  borrowed, and every clone of it is another borrow.
- The device's dead rows (the kills delta, `Rows::in_block_dead`) read the same words when the relation is in one
  part (`Relation::try_kills_own`). Before, the bits were copied into the delta at each change. Rows in several
  parts still merge for the device (0353 row 6), and their delta still copies.
- Left as they are (counted, "frozen orders"): dead rows that lie in another block than the last part's.
- Test: `a_frozen_relations_dead_rows_are_its_blocks_and_the_devices`.
- hogql cold freezes no relation with dead rows (0 either way), so it measures no change: CPU 83.2-83.3 B against
  5c's 83.2-83.3 B at load 57-65; Metal 73.7-74.8 B, 1,629 findings in each run.

## Built (2026-11-19, step 5e: an input's probes keep no table or order of their own)

- How hogql's derived inputs were probed, by path (the profile's "probe paths", main phase): 109 with no key, 23 by
  a kept order, 43 by a sorted view, 10 by a source's own block index, 61 by a source's lookup, 20 by a selective
  lead column, 19 by a block index built, **4 by a lazily sorted order of their own and 1 by a host hash table of
  their own**. Those last two kinds were what R5 still had on the heap. Both are gone:
  - **A lazy order** (an input whose rows lie in several parts, so it has no block index) goes, once sorted, into the
    directory of its last part's block, as a frozen relation's orders do (`in_block_orders`). The probe plan keeps a
    handle.
  - **A view's index** is built in a home of its own (`view_index`): a relation of no columns and the view's row
    count in a block (`Part::for_device`, as the device's view home), its hash index built from each row's key read
    through the view (`Relation::build_index_hashed`, `InBlock::own_index_hashed`). On hogql the one host table was
    over a view of no rows. `Csr::build`, `Csr::table` and `Csr::ids` are gone: an empty view has no index.
- What `InRel::idx` keeps is now a probe plan by column set: which path, a block index's number with the part that
  keeps it, a kept order's number, a lead column. Nothing of the relation's data is in it.
- Test: `an_order_made_at_a_probe_is_in_the_blocks_directory`.
- hogql cold, profiling, format 15, load 18-23: CPU 83.2-83.9 B against 5d's 83.6-84.2 B.
- **Open: on Metal a run can lose a whole chunk's findings.** Of 3 runs of this build, 2 lost one (1,358 and 1,487
  findings). Of 4 runs each, alternating at the same load, step 7's build lost one (1,487) and 5d's none. It is older
  than 5b and is not this step's. Being looked into next.

## Built (2026-11-19, step 6a: an input's measured statistics, value counts and key runs in blocks)

- `InRel::measured` (each column's four statistics) is written once, when first asked, in a block near the rows
  (`BlockVec<[i64; 4]>`).
- `InRel::counted` (rows per value of a column the rules test by constants) is a sorted array of (value, rows) per
  column, in a block (`BlockVec<[i64; 2]>`), found by a binary search. It was a map of maps. To count, the rows' ids
  are sorted by their value read where it lies: no value is copied.
- `Keyed::runs` (a key column's ascending runs) is an array of `KeyRun { first, start, end }` in a block, shared by
  every clone of the relation. Only the list of which columns were counted (a handle each) stays on the relation.
- Test: `an_inputs_value_counts_and_statistics_are_in_a_block`.
- hogql cold, profiling, format 15: CPU 83.3-84.1 B against 5e's 83.3-84.0 B; Metal 74.6-78.1 B, 1,629 in each of 3
  runs.

## Built (2026-11-19, step 6b: the count tables in blocks)

- A count table (decision 0098 §1, `table::Table`) was a map from each key (a boxed copy of its values) to a slot of
  the histogram's counts. It is now open addressing in a block near the relation's rows. Each slot holds the key's
  first row and its rows (`[u32; 2]`), and is found by the key's hash (`rerail_blocks::key_hash`). The probe checks
  the key against that row's values, read where they lie, so no key is copied. The histogram kernel's coder gives
  each key's first row with its code.
- `InRel::tables` keeps a handle per key-column set.
- Test: `a_count_table_is_slots_in_a_block`. `a_count_many_bindings_reach_reads_its_count_table` stays ignored (the
  scalar summaries' memo answers its count first, decision 0100, open).
- hogql builds no count table. CPU 83.7-87.1 B against 6a's 84.6-88.9 B at load 39-59 (noisy). Metal 75.5-77.1 B;
  one run of 3 lost a chunk (1,487: Q262, older than this work).

## Built (2026-11-19, step 6c: the sketches' arrays in blocks)

- A column's sketch (`setfix::sketch::Sketch`, decision 0112) keeps its arrays in blocks (`BlockVec`), not on the
  heap: the HyperLogLog registers, the count sketch's counters, the small exact counts (`[value, rows]`), and a
  compact label page's counts. A sparse label set's counts were a `HashMap`. They are now open addressing in a block
  (`[value, rows]` slots, `-1` empty), starting at 128 slots and doubling to at most 2,048 (1,024 values), so a
  sketch stays within `bounded_histograms`' bound. A sketch's numbers are the same as before.
- A maintained relation (`Rel::based`) still starts from a clone of the old relation's sketches. A clone writes its
  arrays again in a block, as the heap's clone did before, because the new relation adds to them.
- Tests: `a_sketchs_arrays_are_in_blocks`; the sketch tests (`sketch::tests`, `bounded_histograms`) pass unchanged.
- hogql cold, profiling, format 15, load 27-38: CPU 83.6-84.8 B against 6b's 83.7-84.8 B; peak 372-410 MB against
  326-401 MB. Metal 74.9-78.6 B, 1,629 in each of 3 runs.

## Built (2026-11-19, step 6d: the scalar summaries in blocks; the versions)

- An input's scalar summaries (`scalars::Pool`, decision 0100) memoize each group key's summary (count, sum, min,
  max) per bag. They were a `HashMap<Vec<i64>, Summary>` a bag, each key a heap copy. They are now open addressing in
  a block (`scalars::Groups`): each slot is used, the key's values, then the summary in six words. A table doubles past
  half full, and is cleared when the relation's epoch changes, as before. The pool's map from a bag to its table
  is a handle per bag (a few per relation).
- `InRel::vers` (decision 0364 stage 2) holds handles to the relation's versions (`Old`, `Minus`, `Plus`), each an
  `InRel` whose rows, indexes, orders, dead rows and statistics are now in blocks (steps 5-6). Nothing of their data
  is in the handle, so it needs no change.
- Test: `a_bags_summaries_by_key_are_in_a_block`; the scalars tests pass unchanged.
- hogql cold, profiling, format 15, load 16-30, seven pairs against 6c: CPU 83.6-86.3 B against 83.4-85.4 B (one pair
  92.4 and 94.3 B, both high); Metal 75.9-81.4 B against 75.8 B, 1,629 in each of 5 runs.
