# 0266: A block is one of the engine's few big things; everything it holds is nested inside it

Status: draft (2026-11-15), the target shape. A rewrite: two earlier drafts treated each data structure as a block
of its own, and the user corrected that. Builds on decision 0260 and revises its §1–§6 (0260 points here). The
catalogue of what goes where is `docs/final-shape/data-structures.md`.

## Intro

Decision 0260 says the engine's data will live in blocks: linear memory, relative offsets, the same bytes on CPU,
GPU and disk. This decision says what a block is, what it holds, how CPU and GPU code read and write it, and how a
block changes by base and delta.

The user's direction (2026-11-15):

> blocks will represent things big enough that there are only a handful of them
> on each block, there will be a structure which gives its layout, and all of the subparts will be in the block
> for example, we may have, in the same block, a list of facts, a hash index of them, and all the metadata
> associated with them. This will be linear; at the very most we might have a block with a delta contain a
> reference to its full version
> think in terms of much bigger blocks that represent meaningful things to this engine, not as a data-structure is
> a block
> and things will be nested.

and:

> GPUs will need to be able write these structures, and many of them will need a base + delta version
> I want the APIs to be very ergonomic as a lot of code will be rewritten around them

So a block is not a tape, an index or a string table. It is a thing the engine reasons about as a whole:

- a program;
- a chunk of documents and their facts;
- a phase's cross-file facts;
- a run's outputs.

A block holds every structure that belongs to that thing, nested as deeply as the thing is. The CPU and the GPU
both read and write it. Most kinds have a base and a delta, and code reads the pair as one.

Prior art:

- **Boost.Interprocess.** `managed_mapped_file` is one mapped segment. Its objects are named at its root, and its
  containers nest inside the segment through `offset_ptr`, a pointer stored relative to its own address. The
  segment maps at any address with nothing to fix up.
- **MLDB's mapped tables** (`plugins/tabular`, the user's code; a copy is at `oscar/tmp/ext/mldb-tab`):
  - A mapped structure is a plain struct in the arena, holding `MappedPtr<T>`, `MappedVector<T>` and other
    mapped structures, nested. For example, `MappedStringTable` holds a `MappedIntTable` of offsets and a
    `MappedPtr<char>`.
  - `freeze(MappingContext&, Mapped& out, const Input&)` writes a structure's children, then fills its
    descriptor.
  - Small tables are inline. `RawMappedIntTable` is 16 bytes: a 3-bit encoding tag (bit-compressed, run-length,
    factored) and that encoding's fields.
  - Each struct has a value description (`DECLARE_STRUCTURE_DESCRIPTION`), so a file says what it holds.
- **MLDB's `StructuredSerializer` and `StructuredReconstituter`** (`block/memory_region.h`): named entries, each a
  region or a nested structure.

## What was measured

bandit-b101 over 50 corpus0 files, from a cold store, then again after one file was edited (2026-11-15, profiles
from `RERAIL_PROFILE`). The run has three program parts. Each part splits its rules in two:

| Part | Rules per chunk (file-local) | Rules once over all files (cross-file) | Strata in the cross part |
|---|---|---|---|
| 1 | 16 | 80 | 10 |
| 2 | 4 | 21 | 18 |
| 3 | 9 | 11 | 8 |

- **Cold:** three chunks were evaluated at once, as 0254 runs them. Each chunk's file-local relations live as
  long as its job.
- **After one edit:**
  - one chunk was evaluated, holding the one changed file;
  - the cross parts ran 4 of 10 and 4 of 18 strata, all maintained from deltas, with none derived again;
  - 10 of 13 strata were not run at all, and 8 of their relations were held as they were (50 rows).

So a delta is about one file's worth of a base that covers every file. On the 9,010-file repository, that is about
1/9,000 of the base.

## Decision

### 1. A block is one of the engine's big things

| Kind | What it is | What is nested inside it | How many at once | Base + delta |
|---|---|---|---|---|
| `Program` | a version of the program a run evaluates | rules, declarations, facts, strata and their plans, the modules and library it was linked from, matchers, transducer programs | one per phase's program | yes: an edit of the rules |
| `Chunk` | a chunk of documents and everything made of them | each document's source, tape, digest, long texts and python facts; the document table; the chunk's file-local facts | one per chunk in flight (at most 8, 0254) | no: a chunk is made once. Its documents' subtrees are what the memo keeps (§3) |
| `Facts` | a phase's cross-file facts (0083) | the cross part's relations, each with its rows, its indexes and its metadata (statistics, sorted runs, levels, dead rows, a lattice's keys), in stratum order | one per phase | yes: a maintained run is a delta of the kept relations |
| `Outputs` | a run's outputs | its relations' rows, its findings, their strings | one per run | yes: a session's next run |
| `Atoms` | a store's atoms | the strings and their hash table | one per store | yes: the pending atoms are the delta |
| `Journal` | a wave's writes (0260 §8) | frames (relation subtrees), memo entries (document subtrees), tapes, sources | one or two (the wave being written, and the one being applied) | no: it is itself the store's delta |

A run holds about 15 blocks: a program and a `Facts` per phase (three here), up to 8 chunks, and one each of
`Outputs`, `Atoms` and `Journal`. The pool, the scheduler and the scoreboard keep track of these, never of single
structures. Each block is one mapping, reserved large and sparse until written (§5). It is pinned once on a GPU
with shared memory, so one residency set holds a run's blocks.

**Why `Facts` is per phase, and the file-local facts belong to the chunk.** The measurement shows two lifetimes:

- **A chunk's file-local relations** are made by that chunk's job, at the same time as the other chunks' jobs.
  Their exports go to the cross part, their documents' rows go to the memo, and then the chunk's data is done.
  This is the lifetime of the chunk's documents, so they share the chunk's block, written after the documents.
  Sharing a block with other chunks would mix lifetimes: the block could not be freed until the last chunk ended,
  and concurrent jobs would grow into one space.
- **A phase's cross-file relations** live for the run, and are kept for the next one. Each stratum reads the
  strata below it. Splitting a phase's cross part into several blocks, one per stratum group, would make strata
  read across blocks, and the only reference allowed between blocks is a delta's base. Strata are written in
  order, so each stratum's structures are one range of the block (§5).

### 2. Inside a block: a header, a layout description, a root, nested structures

```
block  ┌ header       magic, kind, format, state, length, base (the full block a delta refers to)
       ├ description  the layout of every structure type in it: fields, kinds, offsets
       ├ root         one struct of the block's kind
       └ the structures below the root, written as they are made
```

- **The description** is the "structure which gives its layout". It is generated from the same source as the code
  (§4) and written into the block. Generic code needs nothing else to walk it:
  - `dump`;
  - copying a subtree;
  - moving the parts a kernel reads to a device without shared memory;
  - reading an older format.
- **The root** is a typed struct, and everything else nests below it:

```rust
blocks::structure! {
    /// A phase's cross-file facts.
    #[versioned]                          // has a delta form (§6)
    struct Facts {
        phase: u32,
        relations: Table<Relation>,       // by relation number; names in `names`
        names: StringTable,
        strata: Vec<StratumRange>,        // each stratum's range of the block (§5)
    }

    /// One relation: its rows, its indexes, and what is known about them, together.
    #[versioned]
    struct Relation {
        arity: u16, flags: u16,
        rows: Rows,                       // columns of IntTables, growing in chunks (§5)
        indexes: Vec<Index>,              // the hash indexes probes use
        dead: Bits,                       // rows removed, kept in place
        levels: Opt<IntTable>,            // decision 0090, only when leveled
        #[replace] stats: Vec<ColumnStats>,  // small metadata: a delta holds a new copy
        #[replace] sorted: Vec<Run>,      // row ranges known to be in order (0180)
    }

    #[versioned]
    struct Index {
        key: Vec<u16>,                    // the columns it is keyed on
        table: HashTable<u64, u32>,       // key hash -> first row of its group in `rows`
        rows: Grow<u32>,                  // row numbers, grouped by key
    }
}
```

A relation's rows, its hash index and its metadata are in one block, next to each other.

### 3. References are relative, and only descriptors hold them

- **A reference is relative to its own address,** like `offset_ptr` and `MappedPtr`.
  - `Ptr<T>` points to one `T`.
  - `Vec<T>` is an offset and a count.
  - `Opt<T>` is a `Ptr<T>` where 0 means none.
  - `Table<T>` is a `Vec<T>` whose entries are found by number.

  Reading a reference is one add.
- **The ranges they cover can be moved.** A stratum's structures are one range, and so are a document's
  structures in a chunk. Self-relative references inside a range stay valid when the range is moved with
  `memcpy`, so moves need no fix-up:
  - a roll-up copies an unchanged stratum into the new full block;
  - the journal takes a document's subtree from a chunk, and a memo hit copies it back into a new chunk;
  - a device without shared memory gets the ranges its kernels read.
- **Bulk data holds indices, never references.** A hash bucket holds a row number, an index holds row numbers, and
  a tape holds node numbers. References are only in descriptors, which an inner loop reads once before it starts.
- **Small structures are inline.** An `IntTable` is MLDB's 16-byte descriptor: an encoding tag and its fields (a
  base and a width, a run-length table, or a dictionary). The values of a short column fit inside the descriptor.
- **The one reference out of a block** is a delta's `base`, in its header. A kernel is given the base address of
  each block it reads.

### 4. One source: generated Rust, MSL and C

`rerail-blocks` (the crate below the store, 0260 §1) has `structure!`. From each structure it generates:

- **Rust:** the `#[repr(C)]` struct, with `const` assertions of its size and of each field's offset. It also
  generates the view, the writer, the delta form and the merged view (§6–§8).
- **MSL and C:**
  - the same structs, with their padding as named fields;
  - `static_assert(offsetof(..))` against the Rust values;
  - one-line accessors and writers (§7).

  The text is made at run time and prepended to the kernel's source; the engine already builds its MSL by
  concatenation. No generated file is checked in.
- **The description** written into each block, and a `Debug` and `dump` built from it.

The hand-written copies go:

- `DocView` and its MSL copy: a tape's place is its document's entry in the chunk;
- the MSL text of `SortJob`, `ProgramRow` and `BlockStep`;
- the struct text that three code generators print.

A generator asks `rerail-blocks` for a struct's text. A `#[repr(C)]` struct that stays on the host stays as it is.

### 5. Growing in place: chunks that double, allocated between passes

A block grows as a run derives into it, with many structures growing at once (every relation of a stratum, and
each relation's index). Each growing array is a `Grow<T>`:

- **Chunks that double.** A `Grow<T>`'s descriptor holds a fixed directory of 32 chunk offsets. Chunk `k` holds
  `min << k` items (`min` is 4 KiB of items), so an array of `n` items uses at most `log2(n / min) + 1` chunks.
  32 chunks cover any block.
- **Finding an item is arithmetic.** Item `i` is in chunk `k = 63 - clz((i / min) + 1)`, at
  `i - min·(2^k − 1)`. There is no search and no loop, so it is fit for device code.
- **The columns of a row share chunks.** A `Rows` region is a `Grow` of column-major chunks. A chunk holds every
  column of its rows, so one claim covers a whole row, and each column within the chunk is a dense array.
- **Waste.** The block is a sparse mapping. The unwritten end of a chunk is pages never touched, so a chunk wastes
  at most one page of memory, not half its size.
- **No compaction.** A frozen block keeps its chunks, since reading by chunk costs one `clz`. A stratum's chunks
  are all written while that stratum runs, so each stratum is one range (`strata` in the root), and a range moves
  whole (§3). Only a roll-up (§6) writes arrays contiguous, because it writes a new block anyway.

**Writing from a GPU.** A kernel cannot allocate, and a device must never wait without a bound. So chunks are
allocated between passes, never during one:

1. **Count.** A count pass computes each threadgroup's output, or the writer gives an estimate.
2. **Reserve.** A one-thread step in the same command buffer (or the host, between passes) extends each `Grow` by
   the counted items. It allocates the chunks they need from the block's bump counter and writes their directory
   entries.
3. **Write.** Each threadgroup claims its rows with one atomic on the array's length, then writes into chunks that
   already exist.
4. **Overflow.** With an estimate in place of a count, a claim past the reserved length is not written. It adds to
   the array's `needed`, and the launcher reruns that pass at the exact size, as the Metal join does now.

The host's writers do the same in morsels (0263): each morsel claims once, and a morsel that crosses into a chunk
not yet made makes it under the block's allocation lock. Host claims are short, and their lock is the allocator's,
not a wait inside a kernel.

**Validity.** The header's state and length are written last. A block whose state is frozen and whose length
matches is valid. A journal block that is not valid aborts its transaction (0260 §8). Corruption of memory or
storage is not this layer's concern: ECC and the storage handle it.

### 6. Base and delta, read as one

A `#[versioned]` structure has a delta form, generated from the same source. A delta block's header names its full
block, which is never another delta. For each field of a versioned struct, the delta holds:

| Field | In the delta |
|---|---|
| `Rows`, `Grow<T>`, `StringTable` | the items added; their ids continue after the base's count, so the base is never renumbered |
| `dead: Bits` | the base's rows removed by this delta (and the delta's own) |
| `Table<T>` of versioned structures | a sparse table from the base's entry number to that entry's delta, plus entries that are new. An entry the delta does not name is unchanged |
| `Index` | an index over the delta's rows only; a probe asks both |
| `#[replace]` fields (statistics, sorted runs, small metadata) | a new copy, which wins |

**A delta is cumulative.** A run's delta holds every change since its full block, not only this run's. The next
run starts from the same full block and writes a new delta: the old delta's ranges copied whole, then the new
changes. So a delta never names a delta, and a reader never reads more than two blocks.

**When a delta rolls up: when the delta has cost as much as a roll-up would.** A fixed fraction does not fit the
measurement. One edit's delta is about 1/9,000 of its base, so a delta at half the base would mean thousands of
runs that each copy a large delta first. The rule is the ski-rental one: roll up once the work the delta has
caused equals what a roll-up costs. That is never more than twice the cost of the best schedule.

- **The delta's cost** is counted in its header, per run: the bytes the run copied to carry the delta forward,
  plus the extra rows and probes readers spent on it (reading both sides, probing both indexes, skipping dead
  base rows).
- **A roll-up's cost** is the bytes of the new full block. Unchanged strata are copied whole (§3).
- **Arithmetic.** Each edit adds `e` to the delta, and carrying it forward costs its size each run. So after `k`
  runs the delta has cost about `e·k²/2`, and it rolls up when that reaches the base `B`: `k ≈ √(2B/e)`. With
  9,010 files and one file per edit, that is about 134 edits.
- **Off the critical path.** A roll-up is done by the store's writer or at idle. The base and delta stay correct
  until it is done, and the next run takes the new full block once it is frozen.

The same rule serves every versioned kind (`Program`, `Facts`, `Outputs`). `Atoms` only ever adds, so its roll-up
appends the delta's strings to the base: the roll-up costs the delta, not the base. This replaces today's fixed
rules: the program image's fold at half its parent, and the output image's fold at depth 8.

### 7. GPU access: views and writers in MSL

The generated MSL gives each structure a view (a pointer to its descriptor) and, for a versioned one, a merged
view of a base and a delta:

```c
// generated. A relation as base + delta: ids 0..base.n are the base's, the rest the delta's.
struct RelationV { device const Relation* base; device const RelationD* delta; };

inline uint  rel_len(RelationV r);                      // base rows + delta rows
inline bool  rel_live(RelationV r, uint row);           // not in either `dead`
inline long  rel_get(RelationV r, uint row, uint col);  // decodes the column's IntTable at `row`
inline Range rel_probe(RelationV r, uint idx, ulong key, thread Cursor& c);  // the base's index, then the delta's

// generated writers, used after the reserve step (§5)
inline uint  rel_claim(device Relation* r, uint rows);  // one atomic per threadgroup: the first row
inline void  rel_put(device Relation* r, uint row, uint col, long v);  // into its chunk and column
inline void  rel_kill(device RelationD* d, uint row);  // a base or delta row, into the delta's `dead`
```

- **Scans** cover the base's rows, then the delta's. On a device, the grid is the merged length, and each
  thread's first compare picks the side.
- **Encodings.** A column's `IntTable` tag is read once at bind. On Metal it selects a pipeline specialised by
  function constants, as `crates/metal/src/runtime.rs` already does. So `rel_get` has no branch on the encoding
  in the loop.
- **Values are narrow.** A node id column within one document is its index (base `key << 32`), and an atom or
  kind column is its own width. Apple GPUs emulate 64-bit integer arithmetic, and the bytes saved are most of the
  bytes. `rel_get` widens a value at the edge; kernels that only compare compare narrow values.
- **Kernels are declared** with their block arguments and paths:

```rust
blocks::kernel! {
    /// Probe one relation's index with another's rows, appending the matches to a third.
    probe (grid = left.len) {
        r left:  Facts / relations[$l],           // a RelationV: base and delta, resolved at launch
        r right: Facts / relations[$r] / indexes[$i],
        w out:   Facts / relations[$o],           // in the delta being written
    }
}
```

The declaration generates:

- **the MSL prologue,** with no positional buffers;
- **a typed launcher,** so a `Chunk` cannot be passed for a `Facts`;
- **access metadata** for the scoreboard, and for copying ranges to a device without shared memory.

Bodies that do real work come from the existing compiler's IR (interpreter, Cranelift and Metal backends). A small
hand-written kernel keeps an MSL body and a Rust body behind the same arguments, and has one parity test.

### 8. Host access: one API whether a block is full, a delta, open or frozen

Code is written against views and writers that do not change with a block's form. The bulk of the rewrite is in
setfix, the corpus run, memo and frames. All of it calls these:

```rust
// Open a phase's facts for a run: a delta of the kept full block, or a new full block.
let mut facts = blocks::Facts::open(&pool, Base::Kept(&kept))?;        // Open<Facts>

// Writing. A relation is found or made by number; writers claim rows and fill them.
let mut rel = facts.relation_mut(r)?;                                  // RelationW<'_>
rel.push(&[doc, node, kind]);                                           // one row
let mut w = rel.writer();                                               // Send: one per morsel
w.extend(rows_iter);                                                    // claims once, fills chunks
rel.kill(row);                                                          // a base or delta row
rel.index(&[0])?;                                                       // (re)builds the delta's index on column 0

// Reading: the same calls on a full block, a delta and its base, open or frozen.
let rel = facts.relation(r)?;                                           // RelationV<'_>
for row in rel.live_rows() { /* row.get(0), row.get(1) */ }
for row in rel.probe(&[0], &[key]) { /* rows whose column 0 is key */ }
let doc_ids: &[u32] = rel.column(0).as_u32()?;                          // dense, when one chunk and u32-coded

// Freezing, and what it hands on.
let frozen: Frozen<Facts> = facts.freeze()?;                            // shareable, Send + Sync
let range = frozen.stratum(s);                                          // a range to copy whole (journal, roll-up)
```

The rules that keep this one API:

- **One reading type for both forms.** `RelationV` is the merged view. A full block is a base with no delta, so
  readers have a single type and never ask which form they hold.
- **The lifetime follows the block.** A view borrows its block, so it cannot outlive the mapping. A
  `Frozen<Facts>` is `Arc`-shared and `Send + Sync`.
- **Many writers, one owner.** `Open<T>` is owned by one run. Its row writers are `Send`, one per morsel or
  thread, and each claims space with an atomic. Structural changes (a new relation, a new index) take `&mut`.
- **Typed columns.** `get` returns `i64`, and narrow typed access (`as_u32`, `as_u16`) is there for loops that
  want dense slices. The writer picks each column's encoding at freeze, from the values written.
- **The same verbs on the GPU.** The MSL accessors (§7) are this API's names with a prefix (`rel_len`,
  `rel_get`, `rel_probe`, `rel_claim`, `rel_put`, `rel_kill`). A body moved between host and device keeps its
  calls.
- **Existing types become views over blocks:**
  - `setfix::Rel` and `InRel` become a `RelationW` and a `RelationV`;
  - `output::Rows` and `frames::Rows` become `RelationV` over `Outputs` and `Journal`;
  - a memo entry is a document subtree copied into a `Chunk`.

### 9. Checks

- **At build and at kernel compile,** the size and offset assertions, on both sides, from one source.
- **At open,** the magic, kind, format, state and length, and the block's description against the reader's for
  that format. These read the header only. No check runs on each access.
- **In debug builds,** device accessors check bounds under `#if BLOCKS_CHECK`. Host slices check their own.
- **The guard test** refuses a `.metal` struct named like a `structure!`, and a `[[buffer(n)]]` outside a
  generated prologue. Its allowlist of kernels not yet ported may only shrink.

## Consequence to settle while building

GPU writers claim rows in no fixed order, so a relation's row order varies from run to run. Anything keyed on a
block's bytes depends on that order: frame digests, and a memo entry's key if it covers rows. That includes the
determinism 0263 kept by merging morsels in order. So a range that leaves the run (a journal's frames, a memo
entry) is either digested as a set or sorted when it is frozen. Which one is decided at step 4, with the
measurement of both.

## Path

Each step leaves the engine working and is measured in retired instructions against the access it replaces:

1. **`rerail-blocks`:**
   - `structure!`, `Ptr`/`Vec`/`Opt`/`Table`, and `Grow` (doubling chunks);
   - `IntTable` (MLDB's raw table, ported in place) and `StringTable`;
   - `#[versioned]` (the delta forms and merged views);
   - the description, and the generated MSL.

   It is proved on `Outputs`: the output image (0239) becomes its first form, with base + delta in place of the
   overlay stack. A session's runs are the check.
2. **`Chunk`:** documents, sources and tapes nested in one block. `DocView`, `Slabs { s0..s3 }` and the tape
   accessors duplicated in Rust and MSL go. The Metal tape tests are the check.
3. **`Program`:** the program image, with its modules and library nested, read in place, and edits as deltas.
4. **`Facts`:**
   - setfix's `Rel` and `InRel` become `RelationW` and `RelationV`;
   - a maintained run is a delta of the kept block;
   - the chunk's file-local facts move into its `Chunk`;
   - the roll-up runs by the cost rule.

   Then the Metal join and `accel` kernels read relations in place, and write them through the reserve step
   (`kernel!`).
5. **`Journal`:** a wave's writes as a block in a file (0260 §8). Frames are stratum ranges, and memo entries are
   document ranges, each copied whole.
