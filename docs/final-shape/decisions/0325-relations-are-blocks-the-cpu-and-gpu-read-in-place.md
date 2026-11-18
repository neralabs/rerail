# 0325: Relations are blocks that the CPU and the GPU read in place

Status: decided, building (2026-11-16). 0266's path step 4 ("Facts"; 0295 step 6), built in Native-and-GPU-strata.
Follows 0266 (blocks), 0289 (a chunk is a block), 0295 (the pool's tiers), 0310 (reads written once in CLIF),
0321 (placement leans to the GPU). **0353 (2026-11-17)** withdraws step 5's "the views it makes from a chunk become
relations": a view is never made, it is a read of the tape; and counts the copies left (moved indexes, fixpoint
totals, the CPU's staged heads) as 0353's to remove.

## Intro

A stratum placed on the GPU today copies its inputs to the device and its heads back:

- Each input relation is copied into a buffer of its own, column-major, live rows only (`DevRel::upload`), and
  the device builds a hash index on it (`rel_hash`).
- Each head is decoded back into the store's relation, row by row.

On PostHog's `posthog/hogql`, the copies cost more than the kernels: 286 ms of uploads and 197 ms of rows back,
against 302 ms in the kernels (0321's run). Placement priced those copies as the GPU's cost (`DEVICE_ROW_BACK`).
They are not the GPU's cost. They are a second copy of data that Apple's GPU could read where it already is: its
memory is the CPU's memory.

The user, 2026-11-16: "there should be no uploads and no reading back"; "I want the CPU and GPU to operate over the
same linear buffers"; "use the existing pre-pinning of GPU pages from the page manager"; "go ahead with step 4 here /
make sure it's done properly / for Metal / CPU JIT, make sure that you use inlining support so it's maintainable;
there is prior-art".

The pieces exist, apart:

- **The pool** (`accel::gpu::shared`, 0266 §15–16, 0295): pinned slabs, mapped once as no-copy Metal buffers,
  under one budget. `lease_block` hands out a block over a lease.
- **`rerail-blocks`' relation** (0266): rows as columns in doubling chunks, claimed by an atomic add from host or
  GPU threads; dead bits; hash indexes built at freeze; a delta read with its base (`View`); and a GPU join that
  counts, reserves and writes in place (`blocks::gpu`). Its accessors are written twice, in Rust (`rows.rs`,
  `relation.rs`) and by hand in MSL (`gpu.rs`).
- **The reads written once** (0310, `rerail-clif`): a read is a CLIF function, inlined by the JIT's pass when
  hot, and translated to Rust and to MSL. Today it holds only the tape's reads.

Nothing joins them: setfix's relations (`Rel`, `InRel`) keep their rows in `Vec<i64>` segments on the heap, so the
device can only copy them.

## Decision

Setfix's relations hold their rows in `rerail-blocks` relations, in blocks leased from the pool. The CPU and the
GPU read and write those blocks in place, through reads written once in CLIF.

1. **The reads are families** (`rerail_clif::reads`). A family is a set of CLIF reads over one base pointer:
   - the tape's (`tape_*`, 0310), which also take a view;
   - a relation's (`rows_*`, `rel_*`), whose base is the block's first byte and whose descriptor is an offset
     into it. They are: a value (`rows_get`: the chunk arithmetic, the column's width, the self-relative chunk
     reference), a value's address and width (for a writer's store), a row's dead bit, a base and delta view's
     value and liveness, and an index's group for a key's hash.

   Each family is translated by the same three backends, with the same hot and cold marks: MSL for the kernels
   (`metal_relation_reads.metal`, generated), Rust for the host (`rerail_blocks`' `generated_reads.rs`), and the
   JIT's inlining pass (`inline::reads`, over every family). `blocks::gpu`'s hand-written accessors go; `Rows::get`
   and `ColRead::get` call the generated Rust. The layout's offsets come from the `structure!`s (`offset_of!`),
   so a layout change regenerates the reads, as `TapeView`'s did in the oscar merge.
2. **Setfix's rows are a blocks relation.** `setfix::rows::Rows` holds a `rerail_blocks::relation::Relation` in a
   block leased from the pool (`lease_block`, kind `FACTS`), not `Vec<i64>` segments:
   - A closed relation (`InRel`) is a frozen block, shared by `Arc` as its segments are now.
   - A maintained run's relation is a delta of the kept one (`base_rows`, kills as dead bits), not a copy (0266
     step 4's "a maintained run is a delta of the kept block").
   - The executor's reads of a row as a slice (90 sites, `row(id) -> &[i64]`) read it through `RowRef` or into a
     row on the stack. Its value reads (`RowRef::at`) are unchanged.
   - `Rel`'s and `InRel`'s other state (orders, sketches, levels, lattices) stays theirs for now; the indexes move
     in step 5.
3. **The device reads relations in place.** A device stage binds each input's block (a no-copy buffer of its
   lease) and its descriptor's offset. Its kernels read values through the generated `rows_get` and skip dead rows
   by their bits. `DevRel::upload` and its column-major copy go.
4. **The device writes heads in place.** A head is a relation in a block. A stage counts its rows, the host
   reserves the chunks between passes (`Rows::reserve`), and the write pass claims rows with one atomic add and
   writes them (`blocks::gpu`'s protocol; a claim past the reservation adds to `needed`, and the pass runs again at
   the exact size). The host's `InRel` of the head is that block: no rows are decoded.
5. **Indexes are the relation's.** The device's probe reads the relation's index in its block (`rel_group`), made
   once by the first reader that needs it, the CPU's or the GPU's, and kept with the relation. `rel_hash` per
   upload goes. The key hash is one (`blocks::mix`), in the CPU's probe and the device's.
   - **Every device relation is a blocks relation first** (the user, 2026-11-17: "Blocks first, one form"). The
     device's own relations (the views it makes from a chunk, the fixpoint's totals and deltas, a lattice's total,
     an upload) were column-major buffers with no descriptor, so a probe of them could not read `rel_group`. They
     become relations in blocks of their own life (`Life::Device`), written through `clif_rows_at` (a group's room
     is at most a quarter of it). The column-major form (`at == 0`) goes, so every read and write of a relation on
     the device is one form; what is left at `at == 0` is words a raw mode reads (a text's names, verdict bits),
     not relations.
   - A device relation is alone in a block of its size, leased as the device's join buffers are (within its
     budget) and given back with it, as those buffers were. Shared 16 MB blocks, kept by any relation in them, had
     the budget refuse small views their room.
   - A fixpoint's total stays the device's: its rows are copied once into a run's block at the end (step 4), so
     a kept head never holds a block of the device's budget.
   - An index is added to a relation after it is made (`Relation::try_add_index`): its directory, buckets and
     rows in the relation's block, built by the host or the device, rows in id order within a bucket, so a
     probe's answer has one order. A key has up to 8 columns (`Index::key`), as the device's probes. A block with
     no room for the index has the relation copied once into one that has (as `in_block`'s merged copy is).
   - The key hash is a read written once (`rel_key_mix`), so the host's builder, the device's builder and every
     probe hash a key alike.
6. **Placement prices no copies.** The device's price loses the upload and row-back terms (`DEVICE_ROW_BACK`).
   A submission is priced per wave, not per stratum. `GPU_BIAS` is refitted on the new runs.
7. **The other row stores follow** (0266 step 4's rest): the output's rows, the frames' rows and the memo's
   entries become relations in blocks, written and read as these.

## What it keeps

- 0266: blocks are big things with everything nested, in self-relative form; a delta reads with its base.
- 0289: a chunk is a block; 0295: the pool's one budget and tiers; a deferrable block writes back from its
  mapping.
- 0310: one definition of each read; the JIT inlines a hot read, keeps a cold one a call.
- 0104: no SQL in the waves; findings stay the CPU binding's (1,629 on the light hogql run).

## Checks

- The generated reads are what the reads build (`the_*_reads_are_these`, as 0310's), and a relation read
  JIT-compiled, read in Rust and read on the device gives `Rows::get`'s values, across chunk boundaries and both
  widths.
- The device parity tests (`setfix::device::parity`, `device::tests`) pass with no `DevRel` in the path.
- The profile of the light hogql run on Metal shows no upload and no row-back spans; findings 1,629.
- Measured in retired instructions and the stratum's GPU and host times against 0321's run.

## Built

Step by step, each leaving the engine working (see the commits that name this decision).

- **Step 1** (2026-11-16): the families. `reads::Family` (`Tape`, `Relation`) says a read's pointers: the tape's
  base and view, or a relation's descriptor. `rerail_clif::relation` holds the relation's reads: `col_at` and
  `col_get` (a column resolved once, `rows::ColRead`), `rows_col`, `rows_width`, `rows_at`, `rows_get`, `rel_get`,
  `rel_dead`, `rel_group`, `rel_group_row`, all hot. Their offsets are `rerail_blocks`' `offset_of!`.
  - The Rust backend writes `rerail_blocks::generated_reads`, which `ColRead::get`, `Rows::get`, `Rows::at`,
    `Relation::dead_bit` and `Relation::group` call. Every load is an aligned word of the block, read at its
    address, unchecked.
  - The MSL backend writes `blocks/src/relation_reads.metal`. A relation's read takes a base and the descriptor's
    offset from it (`t`, `a0`): every address a read computes, or passes to another, is an offset from the one
    base. A form with the descriptor as the base pointer returned `col_at`'s address relative to the column's
    directory, not the caller's. `blocks::gpu`'s hand-written `chunk_of`, `rows_col`, `rows_at`, `rows_get`,
    `rows_put`, `dead_bit`, `rel_index`, `rel_group` and `rel_group_rows` are gone, named now as calls of the
    generated reads.
  - The JIT's pass covers every family (`READS` holds both; `jit::Reads` builds each read through it and checks
    each hot read is inlined).
  - The MSL helpers are guarded, so a kernel includes both families' reads.
  - Tests: `the_blocks_reads_are_these`, `the_blocks_kernels_reads_are_these`; `the_jits_relation_reads_are_the_hosts`
    and `the_device_reads_a_relation_as_the_host_does` (every row's two columns over many chunks, its dead bit and
    its key's group, for a base and a delta in two blocks); `the_hosts_chunk_arithmetic_is_the_reads` (the range
    walks' `chunk_of` against `rows_at`).
- **Step 2** (2026-11-16): setfix's rows are blocks relations.
  - The seam first (b84b98d3e): `RowData::row` returns a `Row`, the row's `k` values written into the caller's
    slot and read as a slice. No reference into the rows leaves them. `RowRef::Rows` reads one value of a derived
    relation by id.
  - `setfix::rows::Rows` keeps its shape (frozen segments shared by `Arc`, a tail, `MAX_SEGMENTS` merged from the
    newest), but each segment and the tail is a `rerail_blocks` relation (`Part`), read through the generated
    reads. A row's values are its relation's columns, 8 bytes each.
  - The relations live in shared FACTS blocks (`Facts`, 64 MB): many relations a block, as 0266 wants blocks
    big. A block is leased from the pool within the Metal budget (pinned, a no-copy buffer of the device's), or
    reserved anonymously where the pool has no room or there is no Metal. A block never moves, so a relation's
    address is stable while a part keeps its block; a tail whose block is full moves to another, one copy of its
    rows (`Part::grow`, as a vector's growth copied them).
  - `rerail_blocks` gains what a shared block needs: `Relation::try_create`, `Rows::try_init`,
    `Rows::try_reserve` (a full block refuses, never panics), `Rows::claim_reserved` (a lone writer's claim, no
    lock) and `Rows::reserved`.
  - Open: a block is given back when its last part goes, so a long session's kept relations hold the blocks they
    were made in, with whatever else died there. If the memory profile shows it, a freeze copies a kept segment
    out of a block that is mostly dead.
  - Built for anonymous blocks (2026-11-17, Q76): a relation's last holder gives its chunks' pages back to the OS,
    so a dead relation no longer holds memory while its block lives (about 500 MB at a 300-file cold peak).
  - Compares read the columns they compare (`RowData::value`, `cmp_ids`), not the row copied. A `Row` keeps 16
    values on the stack, and a wider one goes on the heap. On posthog/hogql the CPU binding is then level with
    step 1.
- **Step 3, flat relations** (2026-11-16): the device reads a flat input relation where it lies.
  - `InRel::in_block` gives its rows as one relation in one block. That is the one part, or every part merged
    into one, made once and kept. It is offered for a relation with no view, no dead rows and some columns.
    `resident` binds it as a `DevRel::in_block`: the block's own device buffer (`Held::kept` keeps the block),
    the descriptor's offset, and every column at its own place. Nothing is uploaded.
  - A stage carries the offset (`Stage::r_at`; 0 for a relation of its own, column-major). Every read of the
    probed relation goes through `rget`, which calls the generated `clif_rel_get`. That covers `colval`,
    `matches`, the aggregate, the binds, the witness's probes of its outer relation, and `rel_hash`. The column
    statistics read a relation's column in its block (`StatCol::Device`'s `col`). A copy (`copy_rows`) reads and
    writes only relations of their own, and asserts it.
  - FACTS blocks lease only spare room (`lease_block_spare`): idle buffers released, nothing evicted. A lease
    within the budget (`lease_block_within`) retired the tape residency's slabs to make room. The run then
    admitted its tapes again: 20–35 s runs, 534 B instructions, about one run in four on hogql.
  - On posthog/hogql (load about 40): Metal retires a median 80.7 B instructions over five runs, against 87.1 B
    for step 1 over three. The CPU binding is a median 90.7 B against 93.2 B. Findings 1,629. Flat relations
    upload 0 MB (519 read in place), against 160 MB at step 1. The views (the source views over a batch, 88 MB)
    still upload: they are not blocks.
  - Still copied: a relation with dead rows (its bits are setfix's, not the block's), the views, and the rows
    back (step 4).
- **Step 3, sources** (2026-11-17): the vtab's `Relation` (line, col, docof, node text and the producers' compact
  relations) keeps its rows in blocks too, and the device reads a whole view of it in place.
  - Of the 84 MB of views still read on the host for an upload on hogql, 51 MB were these relations and 36 MB
    chunk views with no device form (`View::Line`).
  - The rows' storage moved from setfix to `facts::rows` (`Facts`, `Part`, `Rows`, `Row`, `InBlock`); setfix
    re-exports it and keeps `RowData`. `Relation`'s cells are a `Rows`.
  - A `Relation` stages its pushed rows and writes them into their block whole, exactly sized, at the first
    read. Built a row at a time in a block, its tail's growth and moves held about 70 MB more at the peak.
  - The rows have a `Life` that picks their blocks: a batch's sources (`Life::Source`, 16 MB blocks) do not
    share blocks with a run's relations (`Life::Run`, 64 MB). Shared, they kept every block leased alive.
  - `Source::in_block` gives a source's rows as one relation in a block. `resident` binds a whole view of such a
    source in place, every source column at its own place (the owner's numbering, `owner_col`).
  - The hot reads keep their cost across the crate boundary: `Rows::get`, `row` and `part_of` are
    `#[inline(always)]`, as the generated reads are, and a part resolves its columns once (`ColRead`). Not
    inlined, `Rows::get` was a call taking 5% of the CPU binding's samples.
  - `Relation`'s column index is built from (value, row) pairs, each value read once. Its sort read two values
    through the rows at every comparison.
  - FACTS blocks lease pool room only while half the Metal budget stays free for the device's own buffers
    (`lease_block_spare`'s `keep`). Taking the last room left the device's joins 6–48 MB, and `resident` dropped
    every relation it held 28 times a run (once now). An entry in place trims the dropped relations' entries,
    since it holds its block.
  - On posthog/hogql (load 40–50, three runs each, against step 3 in the same window): the CPU binding retires a
    median 91.9 B instructions against 90.3 B, and Metal 83.1 B against 80.2 B (step 3's runs spread 79–87 B).
    Findings 1,629. Read in place 610–737 relations a run (519 before). Views read on the host for an upload
    33–41 MB (84 MB before).
  - The peak resident size is higher: 590 MB against 565 MB on the CPU binding, and 718 MB against 668 MB on
    Metal. That is the sources' blocks' room, whose granularity the 16 MB blocks bound.
  - Left in step 3: the line views (`View::Line`, no device form), relations with dead rows, and the views the
    device makes from the chunk (111–170 MB a run), which are copies made on the device.
- **Step 3, the chunk's views** (2026-11-17): every view of a syntax batch is made on the device from its chunk.
  None is read on the host for an upload.
  - A batch with a guest (decision 0055) had its `Loc` views (`line`, `end_line`, `col`) read on the host: 28 MB
    on hogql. `fill_locs` refused the whole batch, because a guest's places are its host's lines through its
    placement. Now `fill_locs` puts a guest's places in the chunk (`ChunkDoc::placed`: first line, last line and
    start column per local, by `Locs::place`, as the chunk holds start, end and kind). The kernel reads them for a
    guest, and finds the others' places in their lines as before.
  - T's `line` view (`View::Line`) gets a device form (`VIEW_LINE`): `fill_lines` puts the facts' line starts in
    the chunk (`ChunkDoc::flines`), and the kernel counts those at or before a node's start (`start_line`).
  - On hogql, views read on the host for an upload are 5.4 MB in 6.5 ms (84 MB in 115 ms at step 3). What is left
    is the native reach closure (`Closed`), its own index structure, for step 7. Instructions and peak resident
    size are level with the sources' commit (Metal a median 87.4 B against 85.9 B in a 81–87 B spread; CPU
    binding 92.4 B against 93.2 B).
  - Tests: `views_made_on_the_device_are_the_hosts_rows` covers `line` and a guest placed in its host's second
    line, whose places are that line.
- **Step 3, dead rows** (2026-11-17): a relation with dead rows (a maintained relation's kills of the kept rows it
  shares) is read in place too, its dead rows skipped by their bits.
  - The bits are a blocks delta's: `Relation::try_kills` makes a delta of the relation's one relation in its
    block, with no rows of its own, whose dead span holds setfix's bits (`Rows::in_block_dead`). It is made once
    and kept as `merged` is. That is 0266's "a maintained run is a delta of the kept block", for its kills.
  - `InBlock::dead`, `DevRel::dead` and `Stage::r_dead` (the old `r_pad`, so the size is unchanged) carry the
    delta's offset. `row_live` reads it through the generated `clif_rel_dead`. `matches` skips a dead row, and
    so does mode 3 (every row, untested). Every loop over the probed relation's rows goes through one of them,
    and a probe's index holds the dead rows, which a probe skips.
  - The column statistics read only relations whose rows are all live in place; one with dead rows is gathered.
  - Test: `stages_skip_the_dead_rows_of_relations_in_their_blocks` covers a scan, a keyed join through the
    device's index of the relation, and an anti-join whose dead rows hold no binding back, against the nested
    loops over the live rows.
  - On hogql nothing has dead rows (one run from scratch): its numbers are unchanged, findings 1,629.
  - What step 3 leaves copied: the native reach closure's relation (`Closed`, 5.6 MB), and the chunk's views,
    which the device makes itself from the chunk block (164 MB on the device, no upload). `DevRel::upload` stays
    for those, for a view filtered by constants (`ids`), and for a block that cannot hold a dead rows' delta.
- **Step 4, a stratum's seed heads** (2026-11-17): the device writes a head's rows in place, into a relation of a
  block, and the store takes that relation as it lies. No row is read back.
  - Between the passes (`device::place`, after the stratum's joins finish, so each rule's distinct count is known):
    one submission for the stratum's heads. Each head gets the union of its rules' distinct rows, in its columns
    with its constants (`copy_rows`). The union is made distinct when more than one rule makes it. Then come the
    rows the head holds: read where they lie (`Rel::in_block`, dead rows marked), and an anti stage removes them.
    The `rows_place` kernel writes what is left into a relation whose chunks the host reserved for the union
    (`Part::for_writes`), through the generated `clif_rows_at`. Its count is read from the last stage's slot.
  - The host then sets the part's length (`Part::written`) and hands it to the wave (`Fresh::place`). `push_fresh`
    appends it to the head's rows as a segment (`Rows::append_part`): no copy, no sort, no held check. The rows
    are distinct and none is held, as the CPU's `settle` would make them. The head's index runs sort the new ids
    reading values where they lie (`set_delta`); step 5 makes those indexes the relation's own.
  - The device's rows sit beside rows derived on the CPU in the same wave, which may repeat them, or go to a
    lattice, which folds each row. Either way they are read into the wave's batch and settled as before.
  - Left to the rows read back: heads a rule makes texts into (arena values are symbols only once the host names
    them), lattices, heads whose held rows have more columns than a probe's keys (8), and the recursive fixpoint's
    rows (`fixpoint`'s last read).
  - On hogql: 4.50 M of 4.95 M distinct head rows written in place. "Rows into the store" fell from 116 ms to
    6 ms of host CPU, and `Fresh::settle` from 71 ms to 3 ms. Metal 68.2-72.2 B instructions against
    71.0-75.5 B (interleaved runs, one load). Peak 747-820 MB against 783-848 MB. Findings 1,629.
  - Tests: `a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts` runs again with the head holding rows
    before the run, some of them derived again; both runs' rows written in place (`placed == distinct`) and equal
    the CPU's. `a_part_written_elsewhere_is_appended_as_a_segment` (facts::rows).
- **Step 4, the recursive fixpoint's set heads** (2026-11-17): at the fixpoint, each set head's total (every row
  new, since its relation is empty before the run) is written in place by `rows_place` from the device relation
  itself (`place_rows` takes a table or a relation), one submission for the component. Each row carries its
  round where the head keeps levels (`Fresh::place(part, rounds)`), and `push_fresh` gives it level
  `lbase + round + 1`, as `push_leveled` did. Lattice heads, the `shortlex_min` text heads among them, are still
  read back (their fold and their texts' symbols are the host's).
  - Tests: `a_recursive_stratum_on_the_device_is_the_cpus_fixpoint` asserts `placed == distinct`;
    `a_leveled_recursion_on_the_device_has_the_cpus_levels` asserts the set head placed and the lattice read, with
    rows and levels equal to the CPU's.
  - On hogql, few rows are recursive: 4.507 M rows written in place against 4.498 M. Findings 1,629.
- **Step 5, every device relation in a block** (2026-11-17): `DevRel` is a relation in a block. The device's own
  relations (`DevRel::empty`, `upload`) are `Part::for_device` relations, written through `clif_rows_at` by one
  copy kernel (`rows_copy`: a binding table or a relation on either side, an optional count slot; step 4's
  `rows_place` is that kernel), by `view_expand` and by the lattice fold, which updates its total in place
  through `rel_get` and `rows_at`. `rget` and `rel_hash` read only through `clif_rel_get`. A text's names and a
  test's verdict bits are words (`DevRel::words`), not relations.
  - A relation's chunks are not zeroed (`Block::try_alloc_raw`): a value is read only below `len`, and a dirty
    pooled block would otherwise pay a memset for every view. Since the merge of django-bugs (36f3475c0) this is
    0342's `Block::try_alloc_chunk_any`, which also takes a chunk from the block's free list.
  - On hogql (load 38–41, interleaved): Metal 67.5–72.0 B instructions against 68.2–68.7 B, peak 724–785 MB
    against 739–783 MB, findings 1,629. Budget refusals 0–4 a run against 2–5.
- **Step 5, the device's probes read the relation's own index** (2026-11-17): `Batch::index` finds the relation's
  index on the key (`Relation::find_index`, over the same rows), else reserves one in the relation's block
  (`try_reserve_index`, under the block's `Facts::ix` lock) and builds it there. `ix_bucket` hashes each row's key,
  the radix sort orders the rows by bucket stably, and `ix_place` writes the buckets' starts and the rows. The batch
  marks the index built when its work is done (`index_built`), so the CPU's builder and the device's make the same
  index, read by `rel_group`. The stage and witness kernels find a key's rows through `clif_rel_group` and
  `clif_rel_group_row`. The per-binding sort buffers (`perm`, `starts`, `rkeys`, `nruns`) and `rel_hash` are gone.
  - One key hash on the device: `key_mix` is the generated `clif_rel_key_mix`, for an index, its probes, dedup's
    set and a lattice's key table. The old mix and `hash_bits` are gone.
  - Every device relation's block has room for one index of its rows: about 8–12 bytes a row, next to 8 a column.
    An index on a second key copies the relation once into a block that has room (`DevIndex::moved`).
  - An index binds its block's buffer as it is after the index was added (`DevIndex::buffer`). A block with no slab
    of its own (a run's block when the device's spare budget refuses one) is bound by a wrap of its used extent,
    taken when it was first bound. An index added after that lay past the wrap: the device's writes went outside
    the buffer, and probes read empty buckets. On hogql that lost 13 files' findings in one run of four, and
    another panicked on a corrupt tape. `the_devices_index_of_a_relation_is_the_hosts` covers that case (a relation
    in anonymous memory, `facts::rows::anon_in_block`), and fails with the old binding.
  - On hogql (load 68–80): indexes found the relation's own 913–1,048 a run against 741, built 916–925 against
    937. Instructions 67.7–76.8 B against 68.3–74.1 B, peak 730–797 MB against 699–787 MB. Findings 1,629 in 5
    runs, cold included.
- **Step 6, placement prices no copies** (2026-11-17): the device's price (`place`) is a run, its stages, its
  indexes, the views it makes, and its joins' device work. `DEVICE_ROW_BACK`, `DEVICE_UPLOAD_PER_VALUE`,
  `DEVICE_SUBMISSION` and `DEVICE_RULE` are gone.
  - Fitted on hogql's `placement.tsv`, which now carries each stratum's GPU host time by part and its priced terms:
    a run 0.18 ms (its share of its level's wave, one command buffer for 77% of runs), a stage 30 µs, an index 65
    µs, a view made 0.4 ms. A relation the program derives is read in place and costs nothing to read.
  - A view the device made stays resident for the run's later strata (`Run::device_views`), which are then priced
    as finding it. Neither views nor their values predict a stratum's share of view making (R² under 0.3): `ann_ctx`
    pays 2.1 ms a run when it makes its view of 1.2 M values, 0.5 ms when `ann_root` made it first.
  - `GPU_BIAS` refitted: 2 is still the best of 1.25, 1.5, 2 and 3 for the new prices (strata CPU 2.18–2.23 s
    against 2.33–2.58 s at 1.25 and 1.5, and 2.95–3.05 s at 3). `RERAIL_GPU_BIAS` sets it for such a refit.
  - **A measured loss against the old prices** (three interleaved pairs, load 33–38): strata CPU 1.81–1.97 s
    against 1.52–1.88 s, instructions 70.1–76.5 B against 65.7–71.0 B, peak 849–913 MB against 761–847 MB. Findings
    1,629 in all six runs. Instructions vary by 5.3 B between runs of the same binary, so these are three pairs, not
    a settled figure. By the profiles (own CPU by span): the device path's own host CPU is the same (0.37 s against
    0.38 s in the pair with the same 19 chunks). The difference is the CPU's join walk of strata refused at run time
    and redone on the CPU: `pre_118` +0.11 s, `pre_126` +0.16 s. It follows the refusals: 5 and 6 refusals gave
    +0.51 s and +0.36 s, 1 refusal gave +0.02 s; the old runs had 0–2. Strata run apart from the wave because its
    budget was spent: 103–181 a run against 36–55. Refused at run time: indexes of 0.5–1.2 M rows, views of 0.5–1.2 M
    rows (`ann_ctx`'s among them), so the new placement spends the device's budget sooner. Not separated yet: how
    much each refusal's dropped residency cache (ef6c6c386) adds (Q86).
  - Testing this at a 384 MB Metal budget found refusals at run time losing findings: a refused batch left the
    views it encoded cached as made, so later strata read relations nothing wrote. Fixed in ef6c6c386 (`unrun`,
    `a_view_made_in_a_refused_batch_is_made_again`), with two buffers that went back to the pool while an open
    command buffer still wrote them.

## A chunk's blocks, and a CPU run's (2026-11-17, Q58)

- **Found**: on PostHog cold (CPU binding) the shared FACTS blocks raised the median footprint from about 1,160 MB
  to 1,306–1,382 MB. Every chunk evaluating at once wrote into the one open block, so a few rows of each kept every
  block whole; and the blocks were the device pool's pinned buffers even with no device bound. The memory line
  (`RUN_MB`) then cut 3,409 chunks against 2,563, and user CPU rose 14–20 s at the same instructions.
- **Decided**:
  - A chunk's relations go in blocks of its own (`setfix::in_chunk`, around the chunk's `evaluate`; decision
    0289, a chunk is a block). The scope is the run's context (`rows::CHUNK`, carried), so the jobs a chunk starts
    write in its blocks too. Its blocks go back with the last of its rows. Outside a chunk the run's open block is
    used as before. The profile counts the blocks still held when a chunk's evaluation ends (`a chunk's blocks
    outlived it`): on PostHog those are the rows each hands on until it is filed, almost all under 1 MB.
  - A block is the pool's only in a run bound to the device (`placement::device()`); a CPU run's is anonymous,
    resident only where written. The device can still read an anonymous block in place (a no-copy buffer).
- **Pinned by**: `a_chunks_relations_go_in_its_own_blocks_and_go_with_it`,
  `a_job_of_the_chunk_puts_its_relations_in_the_chunks_block`, `a_cpu_run_takes_anonymous_blocks` (setfix::rows;
  `facts::rows` since the merge into Final-shape-quality, where the rows' storage lives, step 3 above);
  `a_dropped_relations_chunks_are_taken_by_the_next`, `a_reader_in_place_keeps_the_relation` (`facts::rows`). There a
  chunk's open blocks are one a `Life`.
- **Measured** (M3, PostHog cold, profiled, two runs, load 26–35): user CPU 205.5 and 217.8 s (before 0325:
  204.1–210.8; with it: 224.6 twice); chunks 2,797 and 2,982 (2,563; 3,409); median footprint 1,189 and 1,194 MB
  (1,155–1,161; 1,306–1,382); Metal driver and leased MB 0 (256–320 and 192–256). Findings 29,747, the same hash.
- **Left**: about 35 MB of median footprint above the run before 0325, and with it a few hundred more chunks: a
  block is bump-allocated, so a relation a chunk drops (or a tail moved to a bigger place) keeps its written pages
  until the chunk ends, where a vector's memory went back at once.

## Exports held where they lie (2026-11-17, Q156)

A chunk's exports used to be copied into heap columns (`Exports::add`), with a hash set over them. The cross-file
part then read that copy. At 300 files the copy took 140 MB at the peak ("hold the export").

- **Held in place.** An export now holds the chunk's own relation: its parts are appended as segments
  (`Rows::append_part`), not copied. A row another chunk gave first, or one dead in its relation, is marked dead in
  the export. The set that keeps rows once is a table of row ids; the values are read where they lie. The
  cross-file part reads the export as a held relation (`InRel::held`), and the engine binds those rows as its input
  (no view, no reads through the source).
- **Kept rows get blocks of their own.** Holding a chunk's relation first kept the whole run block it lay in:
  681–795 MB of run blocks, against 141–295 MB, and the peak rose from 2.39 to 2.73 GB. The relations a run keeps
  past it are now named in its spec (`keptrows`), and setfix makes them in `Life::Kept` blocks. Holding them keeps
  only export rows: 12–17 blocks, 163–243 MB.
- **Read part by part.** A value-at-a-time read of a relation made of many parts looks for its part each time. The
  input digest, the inputs' frames and `exported/public_name` slowed 3–15× that way. `Rows::each_run` walks the
  parts in order, and `each_live_batch`, `each_live_column`, `retain::each_row` and the frames' column reads use it.
- **Measured** (cold, 300 files, profiling build, load 54–68):
  - before: peak 2.39 GB, 94 s CPU;
  - after: peak 2.38 GB, 89 s CPU, heap at the peak 1.67 GB against 1.83 GB;
  - findings: the reference's (30,449 rows);
  - strict 10-file outputs: byte-identical.

## Held rows probed by their own indexes (2026-11-17, Q165)

A relation held in memory for the runs that read it (`columns_source`: a base relation's rows, the cross part's `r__old`)
was a set of heap columns. A column was sorted into a host index on its first probe (`Stored`, `Sorted`), and a key of
several columns into a lexicographic order. At 300 files those sorted ids took 120–140 MB at the peak, the largest
host index left.

- **One relation in a block.** The rows are now one relation alone in a `Life::Kept` block (`setfix/held.rs`,
  `Rows::with_index_room`), with room past its rows for its indexes. Columns no rule reads are not held, and read as 0.
- **Its own indexes.** A probe of a column, or of a key of several, reads the relation's own hashed index there
  (`InBlock::own_index_for`). The same index serves the device. Its buckets are sized to the key's distinct values: 4
  bytes a row and about 4 a value. The sorted index was 4 a row and 12 a value. A whole view of such a source is bound
  to that index (`Source::own_index`, ahead of the source's lookups). Other sources keep their lookups, and get a block
  index only where nothing else answers (Q162).
- **Statistics** are measured a column at a time, by a sort that is dropped once it is measured.
- **Past the directory.** A source with more probed columns than the directory's 8 entries has its other columns
  probed by a host sorted index. The profile counts this as "held rows: a column's index on the host".
- **The carry is gone.** Decision 0101 §5 carried last run's sorted columns into this run's copy. It is removed:
  nothing is sorted any more, and an index is hashed on its first probe.
- `Stored` is left for SQL tables, whose columns load as a statement asks for them.
- **Measured** (cold, 300 files, profiling build, load 47–72):
  - sorted columns at the peak: 122.9 MB before, 0 after;
  - the cross part's live heap at the peak: 380 MB before, 248 MB after;
  - CPU of every head: 34.5 s before, 25.0 s after;
  - the peak footprint stays 2.29–2.35 GB. The file-local chunks running beside the cross part set it now: 592 MB at
    the peak before, 702 MB after, because the cross part finishes sooner and overlaps more of them;
  - findings: the reference's;
  - strict 10-file outputs: byte-identical.

## Merged with django-bugs's frames and free lists (2026-11-17)

Both lines changed how a dead relation's memory goes back and how frames are written. The merge keeps one form of
each, with both sides' properties:

- **A dead relation's pages.** django-bugs gave its chunks to the block's free list (0342), to be reused; this line
  gave their pages to the OS (Q76), but never reused the room. Now a chunk freed in an anonymous
  block goes on the free list and its whole pages past the list's link go back to the OS (`Block::free_chunk`); the
  list marks them in use again when it hands the chunk out (`reuse_pages`). A pooled slab's or a file's pages stay.
  Pinned by `a_dead_relations_pages_go_back_and_its_neighbours_rows_stay` (`rerail_blocks::rows`): the bytes given
  back, the neighbour's rows, and the next relation taking the freed chunks.
- **Frames.** django-bugs encoded each frame straight into its write's segments (`FramePlacer`, Q89); this line read
  the rows part by part (`each_live_batch`), wrote the inputs' frames before the run, and handed derived relations
  off after their last reader (Q74). Kept: this line's structure, with every frame placed. An input's job places
  its granules in a placer of its own and queues it with them; the cross part opens its write's placer before the
  run, so a handed-off relation is encoded into it then (no bytes on the heap until `write_frames`), and one whose
  rows are the stored ones keeps its frame (its reads retagged). The heap form is left only where no segment takes
  a frame, encoded through the same `encode_windows`; the column-at-a-time encoder is gone.

At the hub's merge of django-bugs (2026-11-18) the hub had made the same merge its own way. It kept both row readers
(`each_live_window` beside `each_live_batch`), both digests (`rows_digest_windows` beside `held_digest`) and the
column-at-a-time encoder, and its handed-off relations were encoded on the heap. One of each is kept now:
`each_live_batch`, `held_digest`, and the hub's `encode_windows`, which encodes outside the placer's lock. A handed-off
relation is placed in the cross part's write, and one whose rows are stored keeps its frame. The dead relation's pages
take the form above. The hub's `Owned` hold is folded into the relation's last holder (`Held`'s drop).
