# 0260: Everything lives in blocks: linear, relative offsets, the same bytes on CPU, GPU and disk

Status: draft (2026-11-15), the target shape. §1–§6 are revised by decision 0266: a block is one of the
engine's few big things (a program, a batch, a phase's facts, a run's outputs), with every structure nested
inside it; a delta refers to its full version only; no checksums.
Takes over decision 0259's stage 2: the journal is the first block
written to a file. Generalises spec §20 (tape residency slabs) from tapes to every structure. Answers 0246's open
question (one layer of regions and strings for program, module and output images): yes, and this is that layer.

## Intro

Today the engine holds its data in two kinds of form. Some of it is already flat:

- tapes: aligned columns;
- program, module and output images: header, fixed-width regions, ids in place of pointers;
- `cross_rows` frames: varint deltas between rows;
- Metal residency slabs.

Most of the rest is Rust heap structures: trees of `Vec`s, `HashMap`s, `Arc`s, closures queued for the writer. A
heap structure is cheap to reach into. But it costs allocator work, chases pointers, and cannot go to a GPU or a
file without being copied into another form first.

The user set the direction (2026-11-15):

> My longer-term goal is to use only CPU-and-GPU-compatible data structures. These are linear, single-slab,
> memory-mappable data structures that write no pointers but use relative offsets instead. [...] They are allocated
> from a pool which makes them page-sized and page-aligned so that they can be put on a GPU. [...] They can be
> mutable, but at some point need to be frozen. They can refer to other blocks, whose addresses need to be looked up
> on each device separately. [...] We will have multiple versions of some. They need to be represented by base +
> delta and periodically rolled up so that the cost of mutating them is O(delta size) not O(overall size). [...]
> eventually, every stage of our evaluation will be running kernels over CPU and GPU over pre-allocated linear
> memory chunks as inputs and outputs, with lightweight scheduling and scoreboarding and memory
> management/transfer between them.

and, for the journal:

> allocating a memory-mapped, file-backed region, pointing our kernels at it and having them write out a linear
> delta data-structure, truncating it to its actual size, and then letting the OS take care of the write-back to
> disk. If it's invalid or partially written back, the transaction will abort.

The bet: compact blocks have good locality and almost no allocator overhead, but cost more to access. Across
CPU plus GPU, and with compute outrunning memory bandwidth, they win on large workloads. Small workloads are fast
anyway.

## What is there already

| Piece | Where | Has | Lacks |
|---|---|---|---|
| Tape | `crates/tape_format` | aligned columns, a header of offsets, read in place | a pool; it is held in a slab only on the Metal path |
| Program image, versions | `logic/program_image.rs`, `program_image/version.rs` (0173, 0241, 0244) | regions; ids global to a stack; base, view and overlay; folded at half its parent | page alignment, mmap; stored as SQLite blobs and copied into a `Vec<u128>` to read |
| Module image, linked library | `logic/rr/module_image.rs` (0246, Benchmarks-and-parity) | the program image format | as above |
| Output image | `logic/plan/output.rs` (0239) | regions, strings, a stack of overlays folded at depth 8, mapped from `outputs.rro` | a pool; written whole by the run's thread |
| Frames | `content_store/frames.rs`, `cross_rows` | rows as varint deltas | a block form: `Arc<Vec<u8>>` on the heap, metadata in Rust structs |
| Residency slab | `crates/metal/src/runtime.rs` `MappedBuffer::slab`, spec §20 | page-rounded anonymous mapping, one no-copy shared `MTLBuffer` kept alive (pinning), objects bump-allocated at 16 bytes, at most two generations | file backing; anything but tapes |
| GPU inputs | `accel/src/gpu` | no copy when page-aligned, copied otherwise | a pool that makes every input page-aligned |
| Memory regions | `oscar/forest/src/mldb/block/memory_region.rs` (an MLDB port, for the cost model) | `MutableMemoryRegion` → `FrozenMemoryRegion`, states New/Allocated/Frozen/Zombie/Deleted, `MappedSerializer`, Pod views | being in the engine; a pool; file backing |

MLDB's bit-compressed and frozen tables (its tabular plugin) are not in this checkout. Only `block/` (memory
regions and compute kernels) and `builtin/metal` were ported. Port them in place (copy first, then edit) when the
first bit-compressed table is needed.

## Decision

### 1. A block

A block is one contiguous, page-aligned region that holds one of the engine's big things whole (0266 §1). It
holds:

- a header: magic, format, kind, length, a state, and the base a delta refers to;
- a description of its layout, and a root structure, below which every structure it holds is nested (0266 §2);
- no pointers: a reference is relative to its own address, and only descriptors hold one; bulk data holds
  indices (0266 §3).

The images (program, module, output) become blocks, and tapes become structures nested in a batch's block. They
share one layer (`rerail-blocks`, 0266 §4). That layer lives in the lowest crate both the store and logic use (`rerail-store`'s dependency,
or a new `rerail-blocks`), so the store can write blocks too.

### 2. A pool

A run holds a handful of blocks, so each is a mapping of its own, anonymous or file-backed, reserved large and
sparse until written (0266 §5). On a device with shared memory, a block is pinned by one no-copy buffer kept for
its life (spec §20.4). On a device with its own memory, the subtrees a kernel reads are transferred ahead of it,
or demand-paged. The scheduler decides which, from the kernels' declared inputs.

### 3. Mutable, then frozen

A block is mutable while one writer fills it, then frozen: read-only and shareable by any number of
readers and devices (MLDB's region states). A frozen block's length may shrink to what was written (truncation)
before it freezes. Only a frozen block may be handed to another device or written to disk as a durable form.

### 4. References between blocks

A block refers to no other block, except that a delta names its full version (0266 §5). A kernel is given each
block's base on its device, and the addresses of the structures it starts from, resolved at launch (0266 §6). A
block never stores an address.

### 5. Structures

All of them are regions of fixed-width items:

- integers, arrays;
- strings as offsets into a string table;
- offset tables (CSR);
- structures as rows of a region;
- maps as sorted tables, and hash maps as open-addressed tables of buckets;
- trees in preorder with subtree sizes (as tapes and program nodes are);
- tries, heaps;
- bit-compressed integer tables, each column packed to its width (MLDB's prior art).

### 6. Versions

A block with versions is a full block plus at most one delta block, which names it. A delta holds what was added,
and what was removed by row. Ids are numbered across the pair, so a delta never renumbers its base.

Mutation costs O(delta). A delta is rolled up into a new full block when it reaches a fraction of the base (the
program image: half). The rolled-up base is a new block, and the old one is
freed when its last reader goes.

### 7. Evaluation as kernels over blocks

A stage of evaluation is a kernel that reads blocks and writes blocks that were allocated before it ran. A
scoreboard tracks which blocks are ready on which device. A lightweight scheduler runs a kernel once its inputs are
ready where it runs, and moves or pins blocks between devices. The tapes, the matchers' chunked passes and the
Metal strata are the first kernels of this kind.

### 8. The journal is a block in a file

A wave's writes are a delta block (§6) of the store's tables, one region per kind of write. The writes are typed
rows, and their blobs are kept in the encoding they already have: a frame's varint rows, a memo entry's image, a
tape's compressed frame.

1. At a wave's start, the run allocates a file-backed mapped region (`MAP_SHARED`) in the store's journal
   directory, sized from the wave's estimate (sparse: unwritten pages cost nothing).
2. The wave's writers, on CPU or GPU, write their rows into it at offsets they claim from the block's bump counter.
3. At the wave's end, the block is truncated to its length, its header (state and length) is written last, and it is
   frozen. The OS writes the pages back to disk. There is no `fsync`: durability is not required, because the work
   can be redone (0024 §5).
4. The store's writer maps the frozen block and applies it to SQLite in one transaction. A block whose header,
   or length does not check (partly written, or written back only in part before a crash) aborts that
   transaction. The wave is not applied and a later run makes its work again. A block left by a process that ended
   is applied or removed; it is never trusted unchecked.
5. Memory ahead of the store reads the frozen block in place. The frames' snapshot and a queued write's `pending`
   value are views of it, not separate heap copies, so the wave's bytes are held once, in pages the OS may evict.
6. Back-pressure is the disk and the writer's pace (0259 §5), not a memory cap.

0259 stage 1 (waves, boundaries, one transaction per wave) stands. This replaces its stage 2 (closures converted
to rows one kind at a time into a spool file of its own design).

## Path

Each step leaves the engine working and measured:

1. **One block layer.** `rerail-blocks` (0266: header, description, nested structures, freeze), in a crate below the store.
   The output image (0239) and the program image (0173) are ported onto it, with no format change beyond the
   shared header. The module image follows when 0246 is merged. Reading maps or views the bytes in place: the
   program image's copy into a `Vec<u128>` goes.
2. **The pool.** A mapping per block (anonymous and file-backed), and MLDB's region states (the forest's port,
   placed in the engine). Tapes' residency slabs become `Batch` blocks (0266 §1).
3. **The journal block** (§8): first the frames (the largest writes; the snapshot reads the block), then memo
   entries, then tapes and sources, each removing its closure. Then the guard: no `write_back` queues a closure.
4. **Facts as blocks.** The relations a wave hands to the next (today `Vec`-based `Rel`s and `Rows`) become blocks
   with base + delta, the input of the next wave's kernels.
5. **The scheduler and scoreboard** over the kernels that exist: tape passes, matchers, Metal strata, native joins.

## Open

- File-backed pages under a GPU no-copy buffer (spec Q-R5): safe on Apple's shared memory? Until measured, a
  journal block that a GPU kernel writes is anonymous and copied to the file at freeze.
- Bump allocation from several writers at once: one atomic counter per block, or per-writer sub-blocks.
- The block table's form on a device with its own memory (an argument buffer of addresses: spec Q-R3).
- Bit-compressed tables: which MLDB form to port first, and for which relation.
