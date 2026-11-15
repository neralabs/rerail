# 0266: A block is one of the engine's few big things; everything it holds is nested inside it

Status: draft (2026-11-15), the target shape. This is a rewrite: two earlier drafts of this decision treated each
data structure as a block of its own, and the user corrected that. It builds on decision 0260 and revises its
§1–§6 (0260 now points here). The catalogue of what goes where is `docs/final-shape/data-structures.md`.

## Intro

Decision 0260 says the engine's data will live in blocks: linear memory, relative offsets, the same bytes on CPU,
GPU and disk. This decision says what a block is, what it holds, and how CPU and GPU code read it.

The user's direction (2026-11-15):

> blocks will represent things big enough that there are only a handful of them
> on each block, there will be a structure which gives its layout, and all of the subparts will be in the block
> for example, we may have, in the same block, a list of facts, a hash index of them, and all the metadata
> associated with them. This will be linear; at the very most we might have a block with a delta contain a
> reference to its full version
> think in terms of much bigger blocks that represent meaningful things to this engine, not as a data-structure is
> a block
> and things will be nested.

So a block is not a tape, an index or a string table. It is a thing the engine reasons about as a whole: a
program, a batch of documents, the facts of a phase, a run's outputs. A block holds every structure that belongs
to that thing, nested as deeply as the thing is: relations hold columns and indexes, indexes hold hash tables, a
program holds its modules.

Prior art:

- **Boost.Interprocess.** `managed_mapped_file` is one mapped segment. Its objects are named at the segment's
  root, and its containers (vectors of strings, maps of vectors) nest inside the segment through `offset_ptr`, a
  pointer stored relative to its own address. A segment can be mapped at any address, and nothing in it needs
  fixing up.
- **MLDB's mapped tables** (`plugins/tabular`, the user's code; a copy is at `oscar/tmp/ext/mldb-tab`):
  - A mapped structure is a plain struct in the arena, whose fields are `MappedPtr<T>`, `MappedVector<T>` and
    other mapped structures, nested. For example, `MappedStringTable` holds a `MappedIntTable` of offsets and a
    `MappedPtr<char>`. `MappedIntTableBase` holds a `MappedVector<Range>` whose ranges each hold
    `RawMappedIntTable`s.
  - `freeze(MappingContext&, Mapped& out, const Input&)` writes a structure's children into the context's arena,
    and then fills its descriptor in place.
  - A small table's descriptor is inline. `RawMappedIntTable` is 16 bytes: a 3-bit encoding tag (bit-compressed,
    run-length, factored) and that encoding's fields.
  - Every struct has a value description (`DECLARE_STRUCTURE_DESCRIPTION`), so a mapped file says what it holds.
- **MLDB's `StructuredSerializer` and `StructuredReconstituter`** (`block/memory_region.h`): a directory of named
  entries, each a region or a nested structure (`newStructure(name)`, `getStructure(name)`, `getDirectory()`).

## Decision

### 1. A block is one of the engine's big things

There are a few kinds, and a run holds a handful of blocks at a time:

| Kind | What it is | What is nested inside it | How many at once |
|---|---|---|---|
| `Program` | a version of the program a run evaluates | rules, declarations, facts, strata, the modules and library it was linked from, their strings | one, and its delta (§5) |
| `Batch` | a chunk of documents in flight | each document's source text, tape, digest and long texts; the document table; the python facts | one per chunk being evaluated (at most 8, 0254) |
| `Facts` | a fact space: one phase's relations (0083) | each relation's rows (columns), its indexes, and its metadata: types, statistics, sorted runs, levels, dead rows, a lattice's keys | one per live phase, and its delta |
| `Outputs` | a run's outputs | its relations' rows, its findings, their strings | one per run, and its delta |
| `Atoms` | a store's atoms | the strings and their hash table | one per store, and its delta (the pending atoms) |
| `Journal` | a wave's writes (0260 §8) | relation deltas (frames), memo entries, tapes, sources | one or two (the wave being written, the one being applied) |

The pool, the scheduler and the scoreboard deal in these blocks, so they keep track of tens of things, not
millions. Each block is one mapping of its own: reserved large, and sparse until written (§5). It is pinned once
on a GPU with shared memory, so one residency set holds every block of a run. A device with its own memory gets
the parts of a block that its kernels read, and §2 makes those parts cheap to move.

The structures the engine has today become nested structures inside these blocks. The catalogue maps each one to
its block and its place there.

### 2. Inside a block: a header, a layout description, a root, and nested structures

```
block  ┌ header       magic, kind, format, state, length, base (the full version a delta refers to)
       ├ description  the layout of every structure type in the block: fields, kinds, offsets
       ├ root         one struct of the block's kind
       └ the nested structures below the root, children before parents
```

- **The description** is the "structure which gives its layout". It is generated from the same source as the
  code (§4), and written into the block. Generic code needs nothing else to walk the whole tree: `dump`, copying
  a subtree, moving the parts a kernel reads to a device, and reading an older format. This is MLDB's value
  descriptions and Boost's named root, as data in the block.
- **The root** is a typed struct. For example:

```rust
blocks::structure! {
    /// A phase's facts.
    struct Facts {
        phase: u32,
        relations: Vec<Relation>,       // nested: each relation below
        names: StringTable,             // the relations' names, by the same index
    }

    /// One relation: its rows, its indexes, and what is known about them, together.
    struct Relation {
        arity: u16, flags: u16, rows: u32,
        columns: Vec<Column>,           // one per column
        indexes: Vec<Index>,            // the hash or sorted indexes probes use
        dead: Opt<Bits>,                // rows removed but kept in place
        levels: Opt<IntTable>,          // decision 0090, only when the relation is leveled
        stats: Vec<ColumnStats>,        // the planner's statistics, kept with the rows they describe
        sorted: Vec<Run>,               // row ranges known to be in order (0180)
    }

    struct Column { ty: u8, values: IntTable }   // IntTable: MLDB's 16-byte inline descriptor (§3)

    struct Index {
        key: Vec<u16>,                  // the columns it is keyed on
        table: HashTable<u64, u32>,     // key hash -> the first row of its run in `rows`
        rows: IntTable,                 // row numbers, grouped by key
    }
}
```

The relation's rows, its hash index and its metadata are in one block, next to each other. That is the user's
example.

### 3. References are relative, and only descriptors hold them

- **A reference is relative to its own address,** as `offset_ptr` and `MappedPtr` are:
  - `Ptr<T>` is one `T`;
  - `Vec<T>` is an offset and a count;
  - `Opt<T>` is a `Ptr<T>` where 0 means none.

  Reading one is one add: `(char*)&field + field.off`.
- **Why relative to itself, not to the block's start.** A frozen structure and everything below it occupy one
  contiguous range, because children are written before their parent (§5). With self-relative references, that
  range can be moved by `memcpy`, with nothing to fix up:
  - a roll-up copies an unchanged relation from the old full block into the new one;
  - a journal takes a relation's delta out of a `Facts` delta;
  - a memo entry stored in SQLite is such a range, so a hit is one copy into the run's `Facts` block, not a
    decode;
  - a device without shared memory gets the subtrees its kernels read, one copy each.

  Offsets relative to the block's start would need rewriting at every one of these moves.
- **Bulk data holds indices, never references.** A hash bucket holds a row number, an index holds row numbers,
  and a tape holds node numbers. References appear only in descriptors (a relation's `columns`, an index's
  `table`), which an inner loop reads once before it starts. The inner loop then reads dense arrays.
- **Small structures are inline.** A column's `IntTable` is a 16-byte descriptor holding its encoding tag and its
  fields: a base and a width, or a run-length table, or a dictionary. A short column's values fit in the
  descriptor itself. This is MLDB's `RawMappedIntTable`.
- **The one reference outside a block** is a delta's `base`, in its header (§5). Nothing else in a block refers to
  another block, so there is no per-device block table to keep. A kernel is given the base address of each block
  it reads, which is a handful of addresses.

### 4. One source for the layout: generated Rust, MSL and C

`rerail-blocks`, the crate below the store that 0260 §1 calls for, has one macro: `structure!` (above). From each
structure, it generates:

- **Rust**
  - The `#[repr(C)]` struct, and `const` assertions of its size and of each field's offset (`offset_of!`).
  - A view, `Ref<'b, T>`: a reference into a mapped block whose lifetime is the block's. Following `Vec` and
    `Ptr` returns more `Ref`s or slices: `facts.relations()[r].columns()[c].values()`.
  - A writer, `freeze(&mut Context, &Input) -> T`: MLDB's `freeze`. It writes the children first, and returns
    the filled descriptor for its parent to place.
- **MSL and C**
  - The same structs, with their padding as named fields, and `static_assert(offsetof(..))` for every field
    against the Rust value.
  - The accessors, as one-line functions: `relation_columns(device const Relation* r)` returns a
    `device const Column*`. The text is generated at run time and prepended to the kernel's source; the engine
    already builds its MSL from concatenated sources. No generated file is checked in.
- **The description** written into each block (§2), and a `Debug` and `dump` built from it.

The hand-written copies go:

- the MSL copy of `DocView`, and `DocView` itself: the tape's place in its `Batch` is a `Ptr<Tape>`;
- the MSL text of `SortJob`, `ProgramRow` and `BlockStep`;
- the struct text that three code generators print from Rust strings.

A generator asks `rerail-blocks` for a struct's text. A `#[repr(C)]` struct that never leaves the host stays as
it is.

### 5. Linear writing, freezing, and one level of delta

- **Linear writing.** A block is written front to back by a bump allocator, `Context`.
  - A block being built from known inputs (a batch's tapes, a roll-up, a journal at the end of its wave) is
    written children-first, then the root, and is frozen.
  - A block that grows while a run derives into it (a phase's facts) has several structures growing at once in
    one linear space. Each growing array is a list of chunks inside the block while it is open, as
    `setfix::rows` keeps frozen segments plus a tail today. Its descriptor names the chunks. Freezing writes
    each array contiguous, so every frozen subtree is one range.
- **Size.** A block's mapping is reserved large and sparse, so unwritten pages cost nothing. At freeze it is
  truncated to its length.
- **Validity.** The header's state and length are written last. A block whose state is frozen and whose length
  matches is valid. A journal block that is not valid aborts its transaction (0260 §8). Corruption of memory or
  storage is not this layer's concern: ECC and the storage handle it, and there are no checksums.
- **Deltas.** A frozen block changes by a delta block: per structure, what was added and what was removed by
  row. Its header names its base, which must be a full block. A delta never names another delta. A reader reads
  the base's rows, then the delta's, skipping the removed ones. Ids are numbered across the pair, so a delta
  never renumbers its base. When the delta reaches a fraction of its base, a roll-up writes a new full block,
  copying the unchanged subtrees whole (§3).

  This changes two of today's forms:
  - The output image's stack of up to 8 overlays becomes one delta.
  - A program image overlay's parent must be a base, never another overlay: a second edit folds first.

### 6. Kernels read nested structures in place

- **What a kernel is given.** Each argument is a block kind plus a path to the structure the kernel starts
  from:

```rust
blocks::kernel! {
    /// Probe one relation's index with another's rows.
    probe (grid = left.rows) {
        r left:  Facts / relations[$l],          // the structure, resolved at launch
        r right: Facts / relations[$r] / indexes[$i],
        w out:   Facts / relations[$o],          // an open relation of the delta being written
    }
}
```

  The launcher resolves each path to an address once, on the host, and passes the base of each block and these
  addresses (a few words in `constant` memory). The kernel follows the nested references itself with the
  generated accessors (one add per hop), then loops over dense arrays.
- **The declaration generates** the MSL prologue (no positional buffers), a typed launcher (a `Batch` cannot be
  passed for a `Facts`), and access metadata. The scoreboard uses the metadata to know which blocks a kernel
  reads and writes. A device without shared memory uses it to know which subtrees to copy.
- **The bodies.** A kernel's body that does real work comes from the existing compiler's IR, which has
  interpreter, Cranelift and Metal backends, so there is no third code generator. A small hand-written kernel
  keeps an MSL body and a Rust body behind the same arguments, and has one parity test.
- **Encodings are specialised once, not per element.** A column's encoding tag is read once at bind. On Metal it
  picks a specialised pipeline through function constants, as `crates/metal/src/runtime.rs` already does. On the
  host it picks a function compiled for that encoding.
- **Narrow values.** Values are `IntTable`s, not `i64`. A node id column within one document is its index (base
  `key << 32`), and an atom or kind column is its width. Apple GPUs emulate 64-bit integer arithmetic, and the
  bandwidth saved is most of the bytes. `i64` stays where values meet SQLite and the vtab.
- **Writers on a device** claim space for a whole threadgroup with one atomic. A claim past the end sets the
  structure's overflow count, and the launcher reruns the kernel at the exact size, as the Metal join does now.
  This is one protocol for every writer.

### 7. Checks

- **At build and at kernel compile,** the size and offset assertions on both sides, from one source.
- **At open,** the magic, kind, format, state and length, and the block's description against the reader's
  layout for that format. These are header reads only. No check runs on each access, and no check reads the
  data.
- **In debug builds,** device accessors check bounds under `#if BLOCKS_CHECK`. Host slices check their own.
- **A guard test** refuses a `.metal` struct named like a `structure!`, and a `[[buffer(n)]]` outside a generated
  prologue. Its allowlist of kernels not yet ported may only shrink.

## Path

Each step leaves the engine working, and is measured in retired instructions against the access it replaces:

1. **`rerail-blocks`:** `structure!`, `Ptr`/`Vec`/`Opt`, `IntTable` (MLDB's raw table, ported in place), strings,
   `Context` and `freeze`, the description, and the generated MSL. Proved on the `Outputs` block: the output
   image (0239) becomes its first form, with one delta in place of the stack.
2. **`Batch`:** sources, tapes and the document table nested in one block. `DocView`, `Slabs { s0..s3 }` and the
   tape accessors duplicated in Rust and MSL go. The Metal tape tests are the check.
3. **`Program`:** the program image, its modules and library nested, read in place (its copy into `Vec<u128>`
   goes). An overlay's parent is always a base.
4. **`Facts`:** a phase's relations with their indexes and metadata. `setfix`'s `Rel` and `InRel` read and grow
   it. Then the Metal join and `accel` kernels read relations in place (`kernel!`).
5. **`Journal`:** a wave's writes as a delta block in a file (0260 §8). A memo entry is a relation subtree copied
   whole.

## Open

- A `Facts` block per phase, or per stratum group within a phase. This is decided by measurement of how often
  the relations are rolled up together.
- The fraction at which a delta rolls up, per kind: the program image uses half today.
- The form of a growing array's chunks in an open block: their size, and whether they double.
