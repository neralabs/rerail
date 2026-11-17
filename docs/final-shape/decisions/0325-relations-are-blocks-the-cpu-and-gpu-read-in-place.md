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
