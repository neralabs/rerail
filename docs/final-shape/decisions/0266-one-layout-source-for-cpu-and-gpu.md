# 0266: One layout source for every block: generated Rust views, MSL accessors, offset checks and kernel signatures

Status: draft (2026-11-15), the target shape. Builds on decision 0260 (every structure is a block) and is its
step 1's design. The catalogue of structures it covers is `docs/final-shape/data-structures.md`.

Revised the same day, after a second pass on ergonomics and efficiency. The first draft had four costs:

- every element read went through the block table and the header;
- layout and width choices were branched on at run time;
- small structures each took a page;
- values were `i64` throughout, which the GPU handles poorly.

"What the second pass changed" at the end lists each change.

## Intro

Decision 0260 makes every structure a block: page-aligned, offsets instead of pointers, the same bytes on CPU, GPU
and disk. That settles what the bytes look like. It does not yet say how code reaches them. Today each side
writes its own access:

- `DocView` is a 96-byte `#[repr(C)]` struct in `crates/metal/src/slabs.rs`, and the same struct again by hand in
  `metal_tape.metal`. One `size_of` assertion guards it, and no check covers a field's offset.
- `SortJob` (`accel/src/gpu/join.rs`), `ProgramRow` and `BlockStep` (`block_program.metal`) are kept the same way.
- `block_batch_*` takes 19 positional buffers. The Rust side binds them by number, and nothing checks the two
  lists against each other.
- Accessors such as `tape_size` are written twice, once in MSL and once in Rust (`tape/resident.rs`).
- Three code generators print MSL struct text from Rust strings.

The user asked (2026-11-15) for all the data structures to be designed and documented, with good ergonomics for
CPU and GPU kernels reaching into them. "Good" here means these things:

1. A layout is written once, and both sides are generated from that one source.
2. A mismatch is a build or startup error, never a wrong read on a device.
3. A kernel names its inputs by kind and access, not by binding number.
4. A CPU kernel and a GPU kernel read a block with the same names, and get the same answer.
5. The ergonomics cost nothing in the inner loop. Reading an element is one load from a pointer the kernel
   already holds. Layout choices are compiled in, not branched on.

Prior art that this follows:

- MLDB's `ComputeKernel`: `addParameter(name, "r"|"w"|"rw", "u32[n]")`, `setGridExpression`, `addTuneable`.
  Shapes and access modes are declared, so a scheduler can see what a kernel reads and writes.
- `accel`'s fused sort (0100 §9): device pointers held in a table that the host writes (`gpu_address`), made
  resident with one `use_resources` call.
- Metal function constants, already used in `crates/metal/src/runtime.rs`: a pipeline is specialised by values
  fixed when it is made.
- The compiler's `RecordLayout` (0231): rows or columns, chosen per writer.
- `setfix::rows`: frozen segments plus a tail.
- The Metal join's overflow rerun: a stage whose rows overflow is run again at the exact size.

## Decision

### 1. One source: the `rerail-blocks` crate and its macros

The crate below the store that 0260 §1 calls for is `rerail-blocks`. Its macros are the only place a layout is
written. The examples are illustrative: a ported structure keeps its fields and names.

```rust
blocks::item! {
    /// A document's place in a batch (today's DocView, less what the tape's header already says).
    struct DocRef {
        tape: PartRef,    // the document's tape (§2)
        node_base: u32,   // its first node in the batch's numbering
        pad: u32,
    }
}

blocks::layout! {
    /// One document's syntax tree, in preorder.
    part Tape (kind = 0x0101, format = 4) {
        header { nodes: u32 }
        code:   items<u16>[nodes],   // each node's kind
        size:   packed[nodes],       // subtree sizes, at the width the writer chose (§5)
        parent: packed[nodes],       // u32::MAX at the root, narrowed when it fits
        text:   strings,
    }
}
```

From that one source, the macros generate:

- **Rust.**
  - For an `item!`: the `#[repr(C)]` struct, a `Pod` implementation, and `const` assertions of its size and of
    each field's offset (`core::mem::offset_of!`).
  - For a `layout!`: a view `Tape<'a>` with `code() -> &'a [u16]` and so on, a writer `TapeWriter`, and the
    region numbers as constants.
- **MSL and C.** The text is made at run time from the Rust constants. The engine compiles its MSL at run time
  from concatenated sources (`include_str!` lists), and the generated text is prepended to them. Two choices
  rule out a layout mismatch:
  - Every struct is emitted with its padding as named fields, so the MSL compiler has nothing to lay out
    differently.
  - Each field's offset is asserted (`static_assert(offsetof(..) == N)`) against the Rust value.

  No generated file is checked in. A declarative macro is enough, because the offsets come from `offset_of!` and
  not from the macro.
- **A description,** for `rerail-blocks dump FILE`, a `Debug` impl and tests: kind, format, and each region's
  form, width, layout and count.

The hand-written copies all go:

- the `metal_tape.metal` copy of `DocView`;
- the MSL text of `SortJob`, `ProgramRow` and `BlockStep`;
- the struct text the three generators print.

A generator asks `rerail-blocks` for a struct's text. A `#[repr(C)]` struct that crosses to a device or a file
becomes an `item!`. A host-only FFI struct stays as it is.

### 2. A block holds many parts

A **block** (0260) is the unit of memory, lifetime and residency: page-aligned, pinned once, frozen once, freed
once. A **part** is one structure (a tape, a relation's rows, an index) at a 64-byte-aligned offset inside a
block, with its own small header and regions.

One structure per block would cost a page for each small one. Apple Silicon's pages are 16 KiB. A memo entry's
rows, one relation's rows for one document, or a 40-node tape is a few hundred bytes. Across the 9,010-file
repository with several relations per file, page-sized blocks would spend hundreds of MB on padding, and the pool
would track and pin each one. So the parts with one lifetime share a block:

- a chunk's tapes;
- a wave's writes (the journal, 0260 §8);
- a run's outputs;
- a stored program's image.

A large part (a whole relation's segment, a program) fills a block of its own.

A reference is `PartRef { block: u32, off: u32 }`. The scoreboard and the pool work by block. Readers work by
part.

### 3. Access: resolved when bound, one load per element

A part's header is frozen, so the place of each of its regions is fixed for its life. The cost is paid when a
kernel is bound, never on each element read:

- **Arguments resolve at launch.** For each argument part, the launcher writes into the kernel's `Args` the
  device pointer of each region the kernel reads: `a.left_code` is a `device const ushort*` already. The kernel
  reads `a.left_code[i]`, which is one load, as hand-written code does today.
- **References held in data use the block table.** A batch's `items<DocRef>`, a library's `links` and a chain's
  members are references found while the kernel runs. They are resolved through the block table at
  `[[buffer(0)]]`. Resolution is two loads (the block's base, then the part header's region offset), done once
  per reference and then held in a register. It is never done once per element read. The generated
  `tape_open(t, ref)` returns a struct of region pointers, `TapeP`, and the accessors take that struct.
- **On the host,** a view is a struct of slices made once when it is opened. `Tape::at(&table, r)` does the same
  resolution once.

```c
// generated. Resolve once per reference; then each read is one load.
struct TapeP { device const ushort* code; device const uchar* size; device const uchar* parent; uint nodes; };
inline TapeP tape_open(device const BlockTable& t, PartRef r);
inline ushort tape_code(TapeP p, uint i) { return p.code[i]; }
```

The block table is one buffer of `gpu_address`es on Metal. Residency is one `MTLResidencySet` per pool, where the
OS has it (macOS 15), and is otherwise one `use_resources` call per slab per encoder, as `accel` does now. A slab
is pinned once, never one block at a time.

On a device without shared memory, the table holds each block's place in the device's copy. A part on disk refers
to another part through `links`, by digest. Links are resolved to table ids when the part is mapped, so its bytes
never change between runs.

### 4. Region forms

| Form | Bytes | Accessors |
|---|---|---|
| `items<T>[n]` | `n` of `T` (a scalar or an `item!`) | `x(i)` |
| `records<R>[n]` | an `item!`'s fields, as rows or columns (§5) | `x_f(i)` for each field `f` |
| `packed[n]` | integers at a width fixed at freeze: 1, 2, 4 or 8 bytes, with an optional base (frame of reference) | `x(i)` |
| `bits[n]` | 64-bit words | `x_has(i)`, `x_words()` |
| `strings` | bytes, `u32` ends, an optional table of 64-byte hash buckets | `x_get(i)`, `x_find(hash, bytes)` |
| `csr` | `u32` offsets `[n+1]`, then ids | `x_row(i) -> (first, end)` |
| `hash<K,V>` | open-addressed 64-byte buckets, with the probe bound in the header | `x_find(k)` |
| `sorted<K>[n]` | keys in order | `x_lower(k)`: `ceil(log2 n)` steps, a fixed bound |
| `links[n]` | other parts, by digest, resolved when mapped | `x(i) -> PartRef` |

The rules for every region:

- A region starts at a 64-byte boundary, so a GPU's loads coalesce and a CPU's SIMD loads never split a line.
- Every accessor's loops have a fixed bound, as the work-queue rule requires on the device.
- `x_find` in `strings` compares a word at a time, never a byte at a time.
- Kernels compare atom ids, not text, wherever the atom is known.

### 5. Choices are compiled in, never branched on per element

A part records the choices its writer made:

- each `records` region's layout (rows or columns);
- each `packed` region's width and base.

A reader never branches on them per element:

- **Metal.** The choices are function constants. The pipeline for a kernel is specialised by them, and the
  pipelines are cached by the kernel and its choices. The launcher checks each part's header against the
  pipeline it binds, once.
- **Rust.** A view's `packed` accessor is generic over its width (`Packed<W>`). A host kernel is monomorphised
  for the widths it meets. It is dispatched by one `match` when the part is bound, and then runs the specialised
  loop.
- **Generated code** (the transducer's tiers, Cranelift, MSL) asks for the choice's accessor when it is compiled.

The writer should choose for the part's readers, not for itself. 0231 picks a record layout per writer. The
scoreboard knows which kernels read a part, so the choice takes them in too. Columns suit a GPU scan, and rows
suit host probes. When the readers disagree, a transposition is a kernel with a cost like any other. Readers
never meet a part laid out for nobody in particular.

### 6. Narrow values

Relations hold `i64` values today: `frames::Rows`, `output::Rows`, `setfix` segments, and the `long*` columns of
`SortJob`. This has two costs:

- **On the GPU,** Apple GPUs have no native 64-bit integer ALU, so 64-bit compares and adds are emulated with
  32-bit operations.
- **In bandwidth,** most columns fit far fewer bytes. A node id is `key << 32 | index`. A kind is a small code.
  An atom id fits 32 bits.

So a relation's values are `packed` columns. Each column's width and base are chosen at freeze from the values it
holds. A node-id column whose rows share one document is its index alone (base `key << 32`, width 4 or less).
`i64` stays the interchange at the edges, SQLite and the vtab's `Value`, and never the form a kernel scans. This
is also where 0260 §5's bit-compressed tables land: `packed` is their first form.

### 7. Kernels are declared; bodies come from the compiler

A kernel's signature is declared once, MLDB-style:

```rust
blocks::kernel! {
    /// Count each probe row's matches (accel's ct_count).
    ct_count (grid = left.groups, tile: u32 = 256) {
        r left:   Codes,             // a part of kind Codes; the kernel reads its code and off regions
        r right:  Codes,
        w counts: items<u32>[left.groups],
    }
}
```

The declaration generates:

- **The MSL prologue.** It contains:
  - the block table at `[[buffer(0)]]`;
  - `constant CtCountArgs& a [[buffer(1)]]`, which holds each argument's resolved region pointers (§3) and its
    scalars;
  - the function constants of the arguments' choices (§5);
  - the thread position arguments.

  No kernel declares a positional buffer of its own.
- **A typed launcher,** `ct_count::launch(&device, Args { left: &frozen, right, counts: &mut out })`. A `Tape`
  cannot be passed where `Codes` is wanted. A `w` argument is a part that is still open for writing.
- **Access metadata,** `ct_count::ACCESS`: each argument's mode and kind, for the scoreboard, and the regions the
  kernel reads, so a part's other regions need not move to a device without shared memory. `tile` is a tunable
  that the cost model may set.
- **Batched launches.** Many launches of one kernel write their `Args` back to back in one buffer. A grid
  computed on the device runs as an indirect dispatch. The host's cost per launch is small, and it is not paid
  once per document.

The bodies have one source of truth:

- A body that does real work comes from the existing compiler's IR, which already has interpreter, Cranelift and
  Metal backends. That compiler asks this layer for its records and arguments. A third code generator, such as
  UKL's C-macro bodies, is not added.
- A small hand-written kernel, such as `accel`'s, keeps an MSL body and a Rust body. Both implement the same
  `Args`, so their parity test is one generic function, run once per device.

### 8. Writing: claims per group; overflow is a protocol

- **Claims.** A writer claims space from the part's bump counter.
  - On a device, a threadgroup claims once for all its threads: one atomic, then a prefix sum inside the group
    (`x_claim_group(n)`). A per-thread atomic would serialise on the counter.
  - On the host, a morsel claims once (0263).
- **Space.** An open part is given room from an estimate. On the host this is a sparse reservation, where
  unwritten pages cost nothing. On a device it is a size fixed at launch.
- **Overflow** is one protocol, not one per kernel. A claim past the end sets the part's overflow flag and adds
  to its `needed` count, and no rows are written past the end. The launcher reruns the kernel (or the stage that
  overflowed) with the exact size, as the Metal join does today. Every writer gets this for free.
- **Freezing.** `Open::freeze` truncates the part to what was written, writes the checksum last, and returns
  `Frozen<K>`.

### 9. Versions: kernels walk members; they do not search them

A versioned structure is a base plus a few deltas (0260 §6, rolled up at half the base or at depth 8). Its view,
`Chain<K>`, holds each member's `PartRef`, each member's first id, and each member's own `dead` bits.

- **A scan** walks the members as an outer loop. On a device, the grid runs over (member, row), so no element
  read searches for its member. This is the host's iterator over `setfix::rows` segments, in block form.
- **A read by id** (a probe that lands on an id) finds the member in a table of at most 9 first ids. The table
  is in threadgroup memory on a device and in registers on the host. Ids are global to the chain, so a delta never
  renumbers its base.
- **Before upload.** A chain deeper than a kernel's declared limit is rolled up before the kernel runs. The
  scoreboard prices the roll-up like any other kernel.

### 10. Lifecycle in the types

A handle's type is its state, following MLDB's region states:

- `Open<K>`: one writer, or one kernel's writers through claims.
- `Frozen<K>`: read-only. It may be shared, moved to another device, or written to disk.
- Retired: freed when its last lease goes.

A kernel's `r` arguments take `&Frozen<K>`, and its `w` arguments take `&mut Open<K>`. So reading a part while it
is written is a type error on the host. On a device, the scoreboard keeps a kernel from reading a part until its
writer has finished.

### 11. Checks that cost nothing per access

- **At build:** the Rust `const` assertions of size, alignment and each field's offset.
- **At kernel compile:** the MSL and C `static_assert`s, generated from the same values.
- **At bind:** a part's choices against the pipeline (§5).
- **At open:** the magic, kind and format, and each region's bounds and alignment. This reads the header only.
- **Checksums,** for parts that come from a file or another process only:
  - There is one checksum per 64 KiB chunk, not one per block. Checking a whole block when it is opened would
    read every page of a mapped block, and so undo demand paging. Each chunk is checked the first time it is read.
  - The journal's apply reads every chunk anyway, and checks them all before its transaction (0260 §8).
  - A part frozen in this process is not checked.
- **In debug builds:** device accessors check their bounds under `#if BLOCKS_CHECK`. Host slices check theirs
  anyway.
- **The guard test** fails when a `.metal` file declares a struct with an `item!`'s name, or when a kernel
  declares a `[[buffer(n)]]` outside a `kernel!` prologue. It starts with an allowlist of the kernels not yet
  ported, and the list may only shrink.
- **Labels.** Every block's buffer is labelled with its kind and its parts' kinds, so a Metal trace (xctrace) and
  `dump` name what they show.

## Path

0260's step 1 becomes these steps. Each leaves the engine working, and each is measured in retired instructions
against the hand-written access it replaces:

1. **`rerail-blocks` with `item!` and the MSL generator, proved on `DocView`.** `DocView` is the struct kept by
   hand on both sides. `metal_tape.metal` includes the generated text, and the offset assertions cover it. The
   Metal tape tests are the check.
2. **`layout!`, parts and the shared header,** with the output image (0239) and the program image (0173) ported
   onto it (0260 step 1). The program image's copy into a `Vec<u128>` goes. A run's outputs are one block of
   parts.
3. **`kernel!` on accel's `table.metal` and `closure.metal`.** Their arguments are resolved at launch and their
   choices are function constants. Their parity tests exist already.
4. **The packed transducer's arguments.** `block_batch_*`'s 19 bindings become an `Args`, `s0..s3` become the
   block table, and `ProgramRow` and `BlockStep` become `item!`s.
5. **One relation-rows kind with `packed` columns,** for `frames::Rows`, `output::Rows`, the memo entry's rows
   and `setfix::rows` (0260 step 4). Its measure is bytes per row as well as instructions.

## Open

- `PartRef.off` is 32 bits, so a block is at most 4 GiB. A bigger structure is a chain.
- The OpenCL block table: SVM, or a slab index plus an offset (0260, spec Q-R3).
- How a part's choices are made from its readers (§5): which planner owns that choice, the cost model or the
  scoreboard.
- The SQL executor's arenas (catalogue §4) are not yet surveyed field by field.

## What the second pass changed

| First draft | Now | Why |
|---|---|---|
| Each accessor went through the block table and the header (`t.base[r.id]`, then a region offset) | Regions resolved at launch into `Args`; the table only for references found in data, resolved once per reference (§3) | Two dependent loads per element read in an inner loop |
| Width and layout read from the header at each access | Function constants on Metal; monomorphised by width on the host (§5) | A branch per element, and no specialisation |
| One structure per block | Blocks hold parts; a reference is `PartRef` (§2) | 16 KiB pages; per-document structures are a few hundred bytes |
| `records` layout chosen by its writer | Chosen for the part's readers (§5) | A GPU scan wants columns and a host probe wants rows; the writer is neither |
| `i64` values | `packed` columns by width and base (§6) | Apple GPUs emulate 64-bit integers; most columns fit 4 bytes or fewer |
| UKL-style shared bodies | Bodies from the compiler's IR; a hand kernel keeps an MSL body and a Rust body behind one `Args` (§7) | One code generator, not three |
| One atomic bump counter per block | One claim per threadgroup or morsel; overflow as a shared protocol (§8) | Contention on the counter, and an overflow path written per kernel |
| A chain's member found by a scan of first ids on every read | Scans walk members; a search only for reads by id (§9) | Up to 9 compares per element read |
| A whole-block checksum at open | Per 64 KiB chunk, checked when first read, only for blocks from outside this process (§11) | Opening a mapped block must not read all its pages |
| `use_resources` per encoder | `MTLResidencySet` where the OS has it (§3) | Residency set once, not per encoder |
| An open question: proc macro or not | Declarative: offsets come from `offset_of!` and the MSL is made at run time (§1) | The question is closed |
