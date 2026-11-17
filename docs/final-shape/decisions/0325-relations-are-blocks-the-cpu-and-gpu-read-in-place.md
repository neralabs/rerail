# 0325: Relations are blocks that the CPU and the GPU read in place

Status: decided, building (2026-11-16). 0266's path step 4 ("Facts"; 0295 step 6), built in Native-and-GPU-strata.
Follows 0266 (blocks), 0289 (a chunk is a block), 0295 (the pool's tiers), 0310 (reads written once in CLIF),
0321 (placement leans to the GPU).

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
     become relations in blocks of their own life (`Life::Device`: they die with the device's run or its resident
     entries, and do not keep a run's blocks alive), written through `clif_rows_at` (a group's room is at most a
     quarter of it). The column-major form (`at == 0`) goes, so every read and write of a relation on the device
     is one form. A fixpoint's total is then its head's part, with no copy at the end.
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
