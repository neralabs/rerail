# 0266: One layout source for every block: generated Rust views, MSL accessors, offset checks and kernel signatures

Status: draft (2026-11-15), the target shape. Builds on decision 0260 (every structure is a block) and is its
step 1's design. The catalogue of structures it covers is `docs/final-shape/data-structures.md`.

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
CPU and GPU kernels reaching into them. "Good" here means four things:

1. A layout is written once, and both sides are generated from that one source.
2. A mismatch is a build or startup error, never a wrong read on a device.
3. A kernel names its inputs by kind and access, not by binding number.
4. A CPU kernel and a GPU kernel read a block with the same names, and get the same answer.

Prior art that this follows:

- MLDB's `ComputeKernel`: `addParameter(name, "r"|"w"|"rw", "u32[n]")`, `setGridExpression`, `addTuneable`.
  The shapes and access modes are declared, so a scheduler can see what a kernel reads and writes.
- MLDB's UKL (`forest/src/mldb/block/ukl/`): one kernel source for CPU, Metal and OpenCL.
- `accel`'s fused sort (0100 §9): device pointers held in a table that the host writes (`gpu_address`), and made
  resident with one `use_resources` call. This is the block table in small.
- The compiler's `RecordLayout` (0231): rows or columns, chosen per writer.

## Decision

### 1. One source: the `rerail-blocks` crate and its macros

The crate below the store that 0260 §1 calls for is `rerail-blocks`. Its macros are the only place a layout is
written:

```rust
blocks::item! {
    /// A document's place in a batch (today's DocView, less what the tape's header already says).
    struct DocRef {
        tape: BlockRef,   // the document's tape block
        node_base: u32,   // its first node in the batch's numbering
        pad: u32,
    }
}

blocks::layout! {
    /// One document's syntax tree, in preorder.
    block Tape (kind = 0x0101, format = 4) {
        header { nodes: u32, widths: u32 }
        code:   items<u16>[nodes],        // each node's kind
        size:   packed(widths.size)[nodes], // subtree sizes, at the width the writer chose
        parent: items<u32>[nodes],        // u32::MAX at the root
        text:   strings,
    }
}
```

The examples are illustrative: a ported structure keeps its fields and names. From that one source, the macros
generate:

- **Rust.** For an `item!`:
  - the `#[repr(C)]` struct and a `Pod` implementation;
  - `const` assertions of its size and of each field's offset.

  For a `layout!`:
  - `Tape<'a>`, a view checked once at open, with `fn code(&self) -> &'a [u16]`, `fn size(&self, i) -> u32`, …;
  - `TapeWriter`, which sizes its regions, claims rows, and freezes;
  - the region numbers, as constants.
- **MSL.** A header text, `blocks::msl::<Tape>()`, containing:
  - the header struct;
  - accessors over a `BlockRef` (`tape_code(t, r, i)`, `tape_size(t, r, i)`, …);
  - `static_assert(sizeof(..))` and `static_assert(offsetof(..))` for every field.

  The engine compiles its MSL at runtime from concatenated sources (`include_str!` lists), so the generated text
  is prepended to the source the kernel is compiled from. No generated file is checked in.
- **C.** The same header for OpenCL and for MLDB's C++, with the address space as a macro (UKL's form).
- **A description**, used by `rerail-blocks dump FILE` and by tests: kind, format, and each region's form, width
  and count.

The `metal_tape.metal` copy of `DocView`, the MSL text of `SortJob`, `ProgramRow` and `BlockStep`, and the struct
text the three generators print are all replaced by generated text. The code generators ask `rerail-blocks` for a
struct's text; they never spell it out themselves.

A `#[repr(C)]` struct that crosses to a device or a file becomes an `item!`. A host-only FFI struct stays.

### 2. Region forms

A region is one of a few forms. Each form has one accessor set, generated for both sides:

| Form | Bytes | Accessors |
|---|---|---|
| `items<T>[n]` | `n` items of `T` (a scalar or an `item!`) | `x(i)` |
| `records<R>[n]` | an `item!`'s fields as rows or as columns (§4) | `x_f(i)` for each field `f` |
| `packed(w)[n]` | integers at `w` bytes (1, 2, 4 or 8), chosen at write and kept in the header | `x(i)` |
| `bits[n]` | a bitset in 64-bit words | `x_has(i)`, `x_words()` |
| `strings` | bytes and `u32` ends, plus an optional table of 64-byte hash buckets | `x_get(i) -> (ptr, len)`, `x_find(hash, bytes)` |
| `csr` | `u32` offsets `[n+1]`, then ids | `x_row(i) -> (first, end)` |
| `hash<K,V>` | open-addressed 64-byte buckets with a bounded probe count | `x_find(k) -> V or none` |
| `sorted<K>[n]` | keys in order | `x_lower(k)`, a binary search of bounded depth |
| `links[n]` | other blocks, by digest | `x(i) -> BlockRef` (§3) |

None of these accessors loops without a bound. A probe or search has a bound fixed by the header (a probe count,
or `log2 n`), so the accessors are fit for device code under the work-queue rule (bounded loops, no unbounded
waits).

### 3. References and the block table

A reference is `BlockRef { id: u32, off: u32 }`: a block's number in the run's block table, and an offset into
that block. It is never an address.

- **On the host,** the block table is a `&[BlockBase]` of the run's mapped blocks.
- **On Metal,** it is one buffer of `gpu_address`es, bound at `[[buffer(0)]]` in every kernel. One `use_resources`
  call per slab makes the blocks resident, as `accel`'s fused sort does already. A slab is pinned once, never one
  block at a time (0260 §2).
- **On a device without shared memory,** the table holds the block's place in the device's copy. It is filled
  when the scheduler moves the block there.

`Slabs { s0..s3 }`, `DocTable::slabs`, and every `device const uint4* sN` binding become the block table.

A block on disk refers to another block through its `links` region, by digest. When the block is mapped, each
link is resolved to a table id, the way a dynamic linker resolves a symbol. A block's bytes therefore never change
from one run to the next.

The accessors take the table and the reference:

```c
// generated: the tape's code of node i, in the block r names
inline ushort tape_code(device const BlockTable& t, BlockRef r, uint i) {
    device const uchar* b = t.base[r.id] + r.off;
    return ((device const ushort*)(b + tape_region_off(b, TAPE_CODE)))[i];
}
```

In Rust, `Tape::at(&table, r)` returns the checked view, and `tape.code()` is a slice.

### 4. Rows or columns: chosen by the writer, recorded in the header

A `records<R>` region keeps the layout its writer chose (0231: `RecordLayout::choose`) in the region's entry of
the header. The accessors handle both layouts:

- **A generic reader** branches once for each region it opens, never once for each item.
- **Generated code** that knows the layout when it is compiled (the transducer's tiers, Cranelift, MSL) asks for
  that layout's accessor and checks the header's choice once, at bind.

So a layout is never fixed per backend (the record-layout rule), and it is never guessed by a reader.

### 5. Kernels are declared

A kernel's signature is declared once, MLDB-style:

```rust
blocks::kernel! {
    /// Count each probe row's matches (accel's ct_count).
    ct_count (grid = left.groups, tile: u32 = 256) {
        r left:   Codes,           // a block of kind Codes
        r right:  Codes,
        w counts: items<u32>[left.groups],
    }
}
```

The declaration generates three things:

- **The MSL prologue.** It is the kernel's signature:
  - the block table at `[[buffer(0)]]`;
  - `constant CtCountArgs& a [[buffer(1)]]`, a struct of `BlockRef`s and scalars;
  - the thread position arguments.

  The kernel's body reads `a.left` through the generated accessors. No kernel declares a positional buffer of its
  own, so a binding cannot be off by one.
- **A typed launcher.** `ct_count::launch(&device, Args { left: &frozen_codes, right, counts: &mut out })`. The
  argument types are block kinds, so a `Tape` cannot be passed where `Codes` is wanted. A `w` argument must be a
  block that is open for writing.
- **Access metadata.** `ct_count::ACCESS` lists each argument's mode and kind. The scoreboard (0260 §7) reads it to
  decide which blocks must be ready where the kernel runs, and which become ready after it. `tile` is a tunable
  that the cost model may set.

A CPU kernel implements the same `Args` (`fn ct_count_host(a: Args<Host>)`) over the same views. Both
implementations take the same inputs, so a parity test is one generic function, run once per device.

When a kernel's body itself should be written once (a map, a scan, a histogram), it uses UKL's form: one body
over the generated accessors, compiled as MSL and as Rust. A body that needs threadgroup barriers on the CPU runs
them as UKL does, as coroutines. The transducer's compiler already emits each backend from one IR. It uses this
layer for its records and buffers, not a second one.

### 6. Versions: a chain is a kernel argument too

A versioned structure is a base block plus at most a few delta blocks (0260 §6; rolled up at half the base, or at
depth 8). Its view, `Chain<K>`, holds:

- the `BlockRef` of each member;
- each member's first id;
- the union of the members' `dead` bits.

On the host, reading a chain is an iterator over the members' slices: today's `setfix::rows` segments and tail,
in block form. A kernel takes a `chain<K>` argument, which is a `BlockRef` array in its `Args`, and finds an id's
member with a linear scan of at most 9 first ids. Ids are global to a chain, so a delta never renumbers its base.

### 7. Lifecycle in the types

A block's state is in its handle's type, after MLDB's region states:

- `Open<K>`: one writer. It claims rows from the header's bump counter, an atomic on a device (`x_claim(n)`).
- `Frozen<K>`: read-only and checksummed. It may be shared, and it may move to another device or to disk.
- Retired: freed when its last lease goes.

`Open::freeze` truncates the block to what was written, writes the checksum last, and returns a `Frozen<K>`. A
kernel's `r` arguments take `&Frozen<K>`, and its `w` arguments take `&mut Open<K>`. So reading a block while it
is still written is a type error on the host. On a device, the scoreboard keeps a kernel from reading a block
until the block's writer has finished.

### 8. Checks

- **At build:** Rust's `const` assertions of size, alignment and each field's offset.
- **At kernel compile:** MSL's and C's `static_assert`s, from the same source.
- **At open:** the magic, kind and format; every region's bounds and alignment; and the checksum, when the block
  came from disk or another process. This happens once per block, never once per access.
- **In debug builds:** device accessors check their bounds under `#if BLOCKS_CHECK`, and the host's slices check
  theirs anyway.
- **Guard:** a test fails when a `.metal` file declares a struct with the name of an `item!`, or when a kernel
  declares a `[[buffer(n)]]` argument outside a `kernel!` prologue. The guard starts with an allowlist of the
  kernels not yet ported, and the list may only shrink.

## Path

0260's step 1 becomes these steps. Each leaves the engine working:

1. **`rerail-blocks` with `item!` and the MSL generator, proved on `DocView`.** `DocView` is the one struct kept by
   hand on both sides. `metal_tape.metal` then includes the generated text, and the offset assertions cover it.
   The Metal tape tests are the check.
2. **`layout!` and the shared header,** with the output image (0239) and the program image (0173) ported onto it
   (0260 step 1). The program image's copy into a `Vec<u128>` goes.
3. **`kernel!` on accel's `table.metal` and `closure.metal`.** These are small kernels with positional buffers,
   and the parity tests already exist. The block table replaces their bindings.
4. **The block table for the packed transducer.** `block_batch_*`'s 19 bindings become an `Args`, and `s0..s3`
   become table entries. `ProgramRow` and `BlockStep` become `item!`s.
5. **One relation-rows kind,** `Rows`, for `frames::Rows`, `output::Rows`, the memo entry's rows and
   `setfix::rows` (0260 step 4).

Steps 1 to 3 are each a few hundred lines and are measured as they land: no read becomes slower than today's
hand-written access, measured in retired instructions.

## Open

- Whether `layout!` is a declarative macro or a proc macro. A proc macro gives better errors and can compute
  offsets. A declarative macro adds no build dependency. Start declarative, and move when the offsets need it.
- `BlockRef.off` is 32 bits, so a block is at most 4 GiB. A bigger structure is a chain of blocks.
- OpenCL: whether SVM is needed for the block table, or a slab index plus an offset will do (0260, spec Q-R3).
- The SQL executor's arenas (catalogue §4) are not yet surveyed field by field.
