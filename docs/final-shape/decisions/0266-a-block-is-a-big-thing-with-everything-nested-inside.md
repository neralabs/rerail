# 0266: A block is one of the engine's few big things; everything it holds is nested inside it

Status: partly built (audited 2026-11-23). Left: step 4: setfix relations as RelationW/RelationV blocks, maintained
runs as deltas, kernels reading relations in place. Draft (2026-11-15), the target shape. A rewrite: two earlier
drafts treated each data structure as a block of its own, and the user corrected that. Builds on decision 0260 and
revises its §1–§6 (0260 points here). The catalogue of what goes where is `docs/final-shape/data-structures.md`.

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

The host's writers do the same in morsels (0278): each morsel claims once, and a morsel that crosses into a chunk
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

### 10. Freeze: what makes reads come in a fixed order

Writers fill a relation in any order: many threads at once, or a GPU's claims. Freeze, once per relation at the
end of its stratum, turns that into a relation every reader sees the same way, whoever wrote it:

- **Set semantics.** It drops duplicate rows. In a delta it also drops rows the base still holds live (a binary
  search of the base's order).
- **The canonical order.** The live rows, sorted by the relation's order key (its declared key columns first, then
  the rest). It is a permutation of row ids, so no row moves and no reference into the rows breaks.
- **Hash indexes** on the declared keys, grouped stably, so a probe's answer comes in canonical order too.
- **Dead bits** (the kills, plus the duplicates it dropped) and **statistics** (each column's minimum and maximum).
- **The flag `FROZEN`,** written last. From then on the relation changes only by a new delta.

A base and its delta read as one through the merged view: liveness from both, the two canonical orders merged,
and a probe answered from both indexes, merged. So the earlier question of what anything keyed on a block's
bytes does about GPU claim order is answered: nothing keys on bytes. Digests, frames and memo keys read the
canonical order, which is the same for the same set of rows.

```rust
p1.begin(S_EDGE);
join_gpu(&p1, ...);                              // claims in any order, on the device or on threads
let st = p1.freeze_rel(P_EDGE);                  // set, order, indexes, dead bits, stats; FreezeStats
p1.end(S_EDGE);
let v = frozen.view(P_EDGE);                     // base + delta as one
for id in v.ordered() { /* canonical order */ }
for id in v.probe(0, &[callee]) { /* canonical order within the key */ }
```

Fields that change while a block is open (row counts, chunk offsets, the spans freeze sets, a stratum's range,
the delta's cost) are atomics in the layout: relaxed loads, which are plain loads on ARM, and plain types in MSL.
Writing them through pointers made from `&` let the optimiser keep a stale chunk offset. The profiling build
faulted where the debug build passed. Atomics are the only way to write through a shared reference that Rust
allows.

### 11. The spike: every operation, every phase

`native/crates/blocks` (with `examples/paces.rs`) builds this decision's blocks and runs a small rule workload
through them, across every phase:

- documents into `Chunk`s, with file-local facts, on 4 threads;
- P1 cross-file facts in three strata: exports, a call-edge join (on the CPU and on the GPU), and a reachability
  fixpoint;
- P2, then the outputs;
- a `Journal` in a file, holding memo entries, reopened and used for a memo-warm run;
- one-file edits maintained as cumulative deltas, with roll-ups;
- `Program` blocks with module deltas and roll-ups.

After every run, every relation is checked against a from-scratch oracle, as a digest in canonical order. Checks
also cover:

- the GPU join against the CPU join, over base + delta;
- the GPU overflow path, which reruns once;
- memo-warm runs against cold ones;
- roll-ups, which read the same before and after;
- the journal refusing short files, never-frozen files and crashed files.

Measured in the profiling build, with retired instructions:

| Run | Checks | Cold | Freeze, share of cold | One edit, maintained | Peak footprint |
|---|---|---|---|---|---|
| 300 docs, 0.8M nodes, 8 edits | 128 pass | 1.76 G insn, 31 ms | about 6% | 108 M insn, 6 ms | 152 MB |
| 50 docs, 40 edits | 480 pass | | | | 134 MB |
| 20 docs, 40 edits | 485 pass, 1 roll-up | | | | 126 MB |
| 3,000 docs, 8.1M nodes, 3 edits | 73 pass | 17.6 G insn, 274 ms | about 5% | 977 M insn, 40 ms | 255 MB |

- **Freeze costs about 5% of a cold run.** Its delta form, which dedups against the base, costs 0.5 M
  instructions per edit at 3,000 docs.
- **Most of an edit's cost is the spike's workload,** not blocks. At 3,000 docs the reachability stratum is
  recomputed and diffed (680 M of the 977 M). The edited chunk costs 6 M, carrying the delta forward 2.6 M, and
  the maintained edge join 1.3 M.
- **A delta stays small.** It is about 1/200 of its base at 3,000 docs after 3 edits. Roll-up fires by the cost
  rule: at 20 docs, at the 30th edit, the cost was 165,604 bytes against a base of 164,424. The new full block
  reads the same.
- **The GPU's submission cost follows the length of the wrapped buffer,** not the bytes a kernel touches:
  wrapping a 2 GiB reservation cost 120 ms per join, and wrapping 256 MiB about 17 ms. So a block is wrapped for
  its used extent, rounded to 1 MiB, and a join costs about 3 ms. The first dispatch costs about 112 MB of
  driver footprint once.
- **Memo entries are gathered by document index.** A chunk's hits are a table of entry references, each entry
  copied whole from the journal. They are not one contiguous range.

The spike's ergonomics gaps, for step 1:

- `View::ordered` and `probe` return `Vec`s. They become iterators over the merged order.
- The carry step re-pushes a delta's rows. It becomes a range copy, like `copy_stratum`.

### 12. Against the engine's relations, and GPU residency

`setfix::blocks_compare_tests` puts the same 1M rows of 3 columns (about 10% repeats; 909,164 distinct) through
setfix's `Rel`, closed into an `InRel` and started again by `Rel::based`, and through a block, with `freeze` and a
delta. These are profiling-build instructions retired, with every malloc counted by libmalloc's logger hook:

| Step | setfix | blocks |
|---|---|---|
| Build | 420 M insn, 39 mallocs (dedup at push) | 117 M insn, 1 malloc (one claim) |
| Close / freeze | 746 M insn, 49 mallocs | 1,322 M insn, 25 mallocs (scratch, freed) |
| Build + close together | 1,166 M insn | 1,439 M insn (+23%) |
| Held after it | values in 8-byte `i64`s, 24 MB; about 34 MB live heap with its order | 28 MB in one block (4-byte columns), no heap |
| 1M probes on column 1 | 752 M insn | 903 M insn (+20%), no allocation |
| Scan of every live row | 32 M insn (id order) | 59 M insn (canonical order), 64 M (id order) |
| One edit (kill 100, add 100) | 7.3 M insn, 52 mallocs, +12.8 MB heap | 0.2 M insn, 12 mallocs, a 144 KB delta block |

- **Cold work costs blocks about 20% more.** The spike's freeze sorts whole rows and groups by hash with two
  sorts, where setfix dedups at push and keeps sorted runs. A probe walks a hash group and checks liveness through
  the merged view. Both are open to the usual tuning. Neither cost is in the shape.
- **Memory and an edit's cost favour blocks by one to two orders of magnitude.** A block holds one allocation
  where a relation holds Vecs, its columns are 4 bytes wide where `i64`s are 8, and it is the bytes a journal or a
  GPU reads, with no encoding. A maintained edit writes a delta of its own size: 1/36 of setfix's instructions
  and 1/88 of its memory.
- **A freeze's scratch is transient heap:** the sort keys, ids and keyed pairs, about 45 MB at 1M rows.
  Building them in a scratch region of the block's pool, not the heap, keeps a freeze inside the memory cap.

`examples/pinning.rs` measures what the GPU pays to read a block:

| Wrapped | Wrapping (no-copy) | A dispatch with a fresh wrap | The first dispatch with a kept buffer | Later dispatches with a kept buffer |
|---|---|---|---|---|
| 16 MiB | 0.02 ms | 0.6 ms | 0.6–1.0 ms | 0.2 ms |
| 256 MiB | 0.04 ms | 4.9 ms | 7.9–11.4 ms | 0.2–0.5 ms |
| 1 GiB | 0.1 ms | 20–23 ms | 24–42 ms | 0.2–0.4 ms |
| 2 GiB | 0.2 ms | 39 ms | 49–87 ms | 0.3–0.7 ms |

Wrapping is cheap. The cost is mapping a buffer into the GPU's address space the first time a command uses it:
about 20 ms a GiB, and more for pages never touched. A buffer kept and used again pays nothing for its length.
So blocks for the GPU come from a **pool of pinned slabs**:

- slabs are made once, mapped once and kept;
- a run takes a slab, resets its bump pointer and fills it;
- a slab is given back when the last command buffer that reads it has completed, as `accel::gpu::shared` leases
  are now.

Four 256 MiB slabs recycled over 40 runs cost 2.2 ms a run, an 8 MiB write included. The pool is
`accel::gpu::shared`'s, grown to hold blocks: its size classes become slab sizes, and a block whose base is a slab
never needs a fresh wrap. A block outlives its slab only by being frozen to a file (the journal) or copied into a
new full block (a roll-up), both `memcpy`.

### 13. What a frozen relation costs, and what was cut

`paces` now packs every block of a cold run and breaks the in-place form's bytes down by part. At first, at 300
documents, an in-place relation held 56.7 bytes a row of phase facts and 92 of chunk relations, against about
3.3 and 13.3 packed. Three changes to the in-place form, each checked by every run against the oracle, the
GPU join and the packed form:

| Part (B/row, phase facts / chunk relations) | Before | After |
|---|---|---|
| Hash buckets: `{hash u64, start, len}` at up to 4 slots a key | 29.2 / 49.3 | 3.2 / 6.2: bucket offsets, 4 bytes a bucket, about one key a bucket; no hash kept (a reader checks the key) |
| Order array | 4.0 / 4.0 | 0: freeze sorts the rows in place, so an own row's id is its place in the order |
| Dead bits | 0.1 / 0.1 | 0 in a full relation: only a delta's kills of its base remain |
| Index rows | 7.5 / 4.1 | 3.5 / 4.0: no index for a key that leads the order (a binary search finds its range) |
| Total, in place | 56.7 / 92.0 | 22.6 / 44.5 |

At 3,000 documents (8.1M nodes): phase facts are 19.2 B/row in place and 3.5 packed, chunk relations 43.7 and
13.2, and document arrays 10.0 bytes a node plain and 2.7 packed. Sorting in place also made freeze cheaper (P1's
first stratum: 4.58 G instructions to 2.66 G), and the cold run fell from 17.6 G to 14.9 G.

Against setfix (§12's test, rerun): the block for 1M rows is 17.3 MB, where it was 28.4 MB and setfix's heap is
about 34 MB. Its freeze is 1.66 G instructions, where it was 1.32 G and setfix's close is 0.74 G: sorting in
place costs a pass over the columns, which the order array it replaces did not. An edit is 0.30 M instructions
against setfix's 7.3 M.

Sorting in place found a bug: the dead bits a delta keeps for its base end in a word whose high bits are its own
rows' ids, and those ids moved. That word's own bits are now cleared.

Left to cut, by size:

- **Chunk room** (4.8 / 10.2 B/row): the unwritten part of the last doubling chunk. On the CPU it is address
  space, resident only to the page. In a pinned slab it is held. Four chunks per doubling would bound it at an
  eighth. Packing removes it.
- **8-byte columns** (most of chunk relations' 23.4 B/row of values): names held as 64-bit values. As atoms they
  are 4 bytes.
- **A freeze's scratch** (about 45 MB of heap per 1M rows): from a scratch slab of the pool instead.
- **`RUNS` columns read by position** pay a binary search a row. Scans use `for_each`; probes need a run cursor.
- **Probe instructions.** A bucket now holds every key whose hash falls in it, so a probe checks more rows:
  setfix's 1M probes cost 0.75 G instructions, a block's 0.90 G with the old buckets and 1.23 G with bucket
  offsets. Twice the buckets cut that 12% for 1.1 B/row on phase facts, but 6 B/row on chunk relations, whose
  keys are nearly unique. So it is one bucket a key, and the doubling is a setting per index, for one probed hot.
  A probe of a leading key by binary search (1.77 G per 1M) costs a third more than a bucket's: the search reads
  `Rows::get` per step. Searching each chunk's slice would cut that.

### 14. Frozen in place, packed, or both

**Packed** (`packed.rs`) uses MLDB's `RawMappedIntTable` choices, made once at freeze and by exact size. Each
column is one of:

- `CONST`: no bytes;
- `PACK`: `v - min` in the bits it needs;
- `RUNS`: run values and run starts, each packed;
- `LINE`: residuals from a fitted line (MLDB's `Predictor`).

The rows are in canonical order. A key that leads the order is a range found by binary search. Any other key has
a permutation index: its sorted distinct values, where each value's positions start, and the positions, all
packed. The block is made exactly its size. MLDB's factored packing (up to a third smaller at some widths) is not
used here.

On 1M rows (`examples/encodings.rs`, profiling build):

| | In place (sorted) | Packed |
|---|---|---|
| Bytes | 19.0 B/row | 8.0 B/row (2.6 MB of it the index on column 1) |
| Making it | freeze: 1.67 G insn | straight from open rows: 1.38 G; from the in-place form: 0.64 G more |
| 1M probes, column 1 | 1.30 G insn | 1.51 G (+16%) |
| 1M probes, column 0 (leading) | 1.76 G | 2.01 G (+14%) |
| Scan of a column | 61 M | 33 M (`for_each`) |

On the GPU (64M values under 2^18, buffers kept, warm):

| | Plain (4 bytes) | Packed (18 bits) |
|---|---|---|
| Scan, a value a load | 0.74 ms | 1.00 ms |
| Scan, bits compiled in, 32 values a thread from `bits` aligned words | | 0.77 ms |
| Gather of 16M scattered values | 2.52 ms | 2.11 ms |

- **Decoding is free on the GPU only when it is written for the GPU.** 64-bit shifts are emulated, so decoders
  use 32-bit words. The bit width is a function constant, the shifts unroll, and each word is loaded once. A
  column the GPU reads is laid out by groups at freeze, a target choice made by the compiler, as record layouts
  are. Then a packed scan runs at the plain one's speed in 56% of the memory. A gather, bound by latency and cache,
  is faster packed.
- **On the CPU, packed costs about 15% more instructions to probe and less to scan.**

**The two coexist, at no cost in ids.** A full relation frozen in place is sorted, so its row ids are the packed
form's positions. `paces` checks this for every relation. A delta's kills name the same rows whichever form its
base is in. So:

- **In place, inside a run.** A stratum's relations freeze where they were written: in the run's slab, where the
  GPU writes fixed-width slots. They are readable at once, with no copy and plain loads. A delta stays in place;
  it is small.
- **Packed, when a block leaves the run:** kept, journaled, rolled up, or put in a slab for many later reads. The
  packing costs 0.64 G instructions per 1M rows from the in-place form. At 3,000 documents, packing every block
  costs about a third of the cold run (4.6 G of 14.9 G), so it runs once the run's answer is out. A block known to
  leave (a chunk's file-local facts, kept for the next run) packs straight from its open rows, skipping the
  in-place freeze: 1.38 G against 1.67 G.
- **Reading across forms** is next: a view over a packed base and an in-place delta. Liveness and kills work
  unchanged by the ids above; get and probe dispatch on the base's form.

### 15. The pool, generalized

`accel::gpu::shared`'s pool now holds blocks:

- **Size classes** are powers of two to 1 MiB, then four a doubling: at most a quarter wasted where it was half.
- **Idle buffers** were first kept under two allowances: 32 MB for buffers under 16 MiB, and slabs (16 MiB and
  up) to a quarter of the Metal budget. Before that, a returned buffer past 32 MB was released, so a large slab
  was never kept. §16 replaces both with the one budget.
- **`Pooled::after(cmd)`** returns a lease when its command buffer completes. **`Pooled::grow`** moves a lease's
  contents into a larger one.
- **`lease_block(kind, bytes)`** makes a block in a pooled slab (`Block::over`). It binds by the slab's own
  buffer, which a command has used before once it is recycled, so no dispatch maps it again. A recycled slab
  holds the last block's bytes, so the block zeroes each allocation as it makes it. Freezing does not protect a
  slab's pages, since they are the pool's.
- **`grow_block`** moves an open block into a larger slab by one copy; its references are self-relative.

Any other block keeps the device buffer it was first bound with (`Block::gpu_buffer`). It remakes it, twice as
long, only when the block outgrows it, so no block is wrapped afresh per dispatch. Tests: the classes; a slab
recycled with its buffer and read fresh; a lease returned at its command's completion; and a block grown into a
larger slab that reads the same.

### 16. One budget and one eviction order for all GPU memory

There were four keepers of GPU memory:

- this pool;
- `rerail_metal::pool`, the device solves' blocks. Its idle blocks were mapped buffers under its own share, the
  whole Metal budget;
- `rerail_metal::slabs`, the tapes' residency: anonymous 64 MiB mappings, retired least recently used under half
  the budget;
- `rerail-metal`'s reservation ledger, which every mapped buffer reserved in. Only the device blocks' idle buffers
  gave way to it.

Each refused the others' requests without asking them for room. Now:

- **The residency's slabs are pool leases.** A slab is still append-only and keyed by content, and it is still
  retired whole, least recently used, under the residency's half of the budget. A retired slab's buffer goes back
  to the pool when its last batch lets go, and the next lease of its class takes it, whoever leases. A recycled
  slab holds its last lease's bytes, so each room in it is zeroed as it is handed out (`Pooled::fresh` tells a
  new buffer, which reads zero, from a recycled one).
- **The device blocks are pool leases.** `rerail_metal::pool` keeps its API (`lease`, `upload`, `zeroed`,
  `made`) and a block made for its lease is still written once. Its own idle lists, its share and `give_way` are
  gone.
- **One budget.** The Metal budget holds the ledger's reservations, every live lease and every idle buffer. A
  bounded lease (`lease_within`) fits the ledger, the live leases and itself. A ledger reservation fits itself,
  the ledger and the live leases. Idle buffers fill what both leave, and no more: each new buffer and each
  reservation trims them first. The two idle allowances are gone; `set_slab_idle_limit` caps idle slabs further
  when set.
- **One eviction order** (`make_room`): idle buffers, least recently returned first, then the evictor's holdings
  one at a time. The evictor is the residency (`set_evictor(slabs::evict)`): it retires its least recently used
  slab, which comes back as an idle buffer once its readers end. It takes the residency's lock only when the lock
  is free (`try_lock`), so a request made while the residency places a document is not deadlocked. The
  residency's own loop retires in that case.

A test build leaves the live leases out of the ledger's check: there, the ledger is each test thread's own
(`test_pool`), and the pool is the process's.

Checked by the metal crate's tests, which include a new one: an evicted slab's buffer serves the next lease of its
class, and a room in it reads zero past the bytes written. Also by the accel pool's tests, the engine's resident
sources test, and `tapes_are_tape`, in which tier 1 on Metal reads its sources from the slabs and its tapes equal
the CPU's.

### 17. Costs cut: freeze, probes, chunk room, atoms, scratch

The list from §12–§14, each fixed and measured in instructions retired. The 1M-row comparison against setfix
(`blocks_compare_tests`, the same 1M rows of a call-edge shape):

| Step, 1M rows | Before | Now | setfix |
|---|---|---|---|
| build | 0.14 G | 0.14 G | 0.42 G |
| freeze (setfix: close) | 1.66 G | 0.43 G | 0.74 G |
| 1M probes on col 1, with a read per hit | 1.23 G | 0.79 G | 0.75 G |
| 1M leading-key probes (`encodings`) | 1.77 G | 0.98 G | - |

- **Freeze sorts packed keys by radix** (`sort::radix_pairs`, `relation::sort_canonical`):
  - each column is read once, a chunk at a time (`Rows::read_col`), and its range taken;
  - the order key is packed into a `u64`: each column less its minimum, in the bits its range needs, the first
    highest. Packed keys sort as the rows do, and equal keys are equal rows, so dedup compares one word;
  - the sort is a least-significant-digit radix of 11 bits a pass, with every pass's counts taken in one read,
    and a pass whose digit is the same for every key skipped;
  - a key past 64 bits falls back to comparing rows;
  - the columns are written back in order, a chunk at a time (`Rows::write_col`);
  - an index's buckets are filled by one counting pass over the rows in canonical order, which keeps them in
    canonical order within a bucket. Before, the buckets took a stable sort of `(hash, id)` pairs. The bucket
    count comes from the number of distinct keys: a bitmap of the key's packed range when it is at most 2^24,
    else the hashes sorted.
- **Freeze's temporaries come from scratch** (`scratch::Scratch`), not the heap. A thread's region is mapped
  once, a take is a bump, and the region is emptied per use; past 64 MB, what a use touched is given back with
  `MADV_FREE`. `rerail_accel::gpu::shared::lease_scratch` makes a scratch region in a pooled slab, inside the one
  budget (§16). A freeze of 1M rows now makes 5 small mallocs and holds no heap after (it held 45 MB).
- **Probes:**
  - a frozen relation's own rows are always live (its dead bits name only its base's rows), so a probe of a full
    relation checks the key alone and merges nothing;
  - a leading key's range is exact, so its rows are not checked;
  - the probe resolves its first key column once (`ColRead`: directory, width and `min_log`); a wider key's other
    columns are checked off the hot path;
  - the reference and row accessors are `#[inline]`. In the profiling build's 256 codegen units, a call to one
    was a call;
  - the hash probe was within a few percent of a hand-written bucket loop once the iterator reloaded nothing
    per row. Its yield reloaded eight resolved columns; it now holds two.
- **Leading-key search:**
  - `Rows::lower_bound` binary-searches the chunks' first values, then one chunk's slice (`partition_point`), so
    there is no chunk arithmetic per step;
  - the upper bound gallops forward from the lower one (`upper_from`): a key's rows are few.
- **Chunk room unwritten:** chunks grow by quarter steps. A doubling group of rows is four chunks of a quarter
  each (`chunk_of`, the same in MSL), so a relation's unwritten room is at most an eighth of its rows; whole
  doubling chunks left up to half. The directory is 96 slots a column (24 groups), and `min_log` is at least 4.
  Chunk room per row:

  | | Before | Now |
  |---|---|---|
  | Phase facts | 7.5 B | 1.9 B |
  | Chunk relations | 10.9 B | 1.8 B |

- **4-byte atoms:** `paces`'s node ids were `doc << 32 | i` in 8-byte columns. They are now `doc << 12 | i` in 4
  bytes: a document has under 4,096 nodes, and a run under 2^20 documents. The chunk relations' values went from
  23.4 to 15.7 B/row.

At 3,000 documents (`paces 3000 3`; 3,485 checks pass):

| | Before | Now |
|---|---|---|
| Chunk relations in place | 43.7 B/row | 27.2 B/row |
| Phase facts in place | 19.2 B/row | 15.5 B/row |
| The cold run | 14.9 G | 8.6 G |
| Packing every block | 4.6 G | 2.8 G |

At 300 documents, 498 checks pass. Packed, the chunk relations went from 13.2 to 5.8 B/row at 3,000 documents:
their node ids now take 12 bits of a document's place, not 32. In place, the phase facts are now 4.8x their
packed size at 3,000 documents (5.4x at 300).

Tests: the chunk geometry covers every row once; both searches agree with a scan; the radix sort agrees with a
stable sort; and freeze agrees with a `BTreeSet` for a full relation and a delta with kills and repeats, with
the packed key and with a key too wide to pack, with probes on both kinds of index.

### 18. Packing fast, on the CPU and on the GPU

- **Choose, then pack in place.** A column's encoding is chosen first (`col::choose`), without packing anything.
  The block is then made exactly the size of all of them, and each column packed straight into it
  (`IntCol::place_chosen`): there are no per-column word vectors and no copy.
- **A packer per width** (`pack_into` → `pack_fixed::<B>`): 64 values of `B` bits are `B` whole words, so a group
  of 64 is packed by a loop whose shifts and word boundaries are constants once unrolled. There is one
  instantiation per width (64), and the input is a slice of `u64`, so callers do not multiply them.
- **Choosing in fewer passes:**
  - the range and the run count are one branch-free pass;
  - the LINE residual pass runs only when a sample (every 64th row) leaves the line a chance. The sample's
    residual range is inside the full one, so a sample that already needs PACK's bits proves LINE cannot win;
  - the pass runs in `i64` when `slope * n` cannot overflow it, else in `i128`.

  On 64M values, one thread: choose 106 → 41 ms, 44 → 25 instructions a value with the write.
- **Packing from open rows** uses freeze's packed-key radix sort (`sort_canonical`). The permutation index takes a
  radix sort of the column's values. Both come from scratch.
- **Overflow fixed:** a column spanning more than 2^63 (`[i64::MIN, i64::MAX, 0]`) overflowed the encoder. PACK
  and RUNS now subtract and add the base wrapping, which is exact modulo 2^64. LINE's residuals are computed in
  `i128` when they could overflow, and a column whose residuals need more than 64 bits never takes LINE.

Measured, in instructions retired:

| | Before | Now |
|---|---|---|
| Pack from the frozen view, 1M rows | 0.68 G | 0.19 G |
| Pack straight from open rows | 1.41 G | 0.47 G |

A full frozen relation is already in canonical order, so its columns are read a chunk at a time with no merge.

**On the GPU** (`encodings`; 64M values of 18 bits, best of 6, warm):

- A min/max reduction (grid-stride, `simd_min`/`simd_max`, one atomic per simdgroup) takes 0.64 ms.
- Each thread then packs one group of 32 values into exactly `B` 32-bit words (`B` compiled in as a function
  constant). No word is shared between threads, so there are no atomics.
- In sequential order, the words are the CPU's bit stream: a stream of 64-bit words is the same bits as 32-bit
  words on a little-endian machine. The example checks that every word equals the CPU's.

| Layout | ms | Read + written |
|---|---|---|
| CPU layout (`g * B`) | 1.10 | 383 GB/s |
| Interleaved (word `k` of group `g` at `k * groups + g`: the scan's layout, coalesced stores) | 1.02 | 412 GB/s |
| Loads through threadgroup memory (64 groups, rows padded to 33 words) | 1.09 | 385 GB/s |

That is about 1.7 ms with the range against about 125 ms on one CPU thread, and at the device's bandwidth: the
plain scan reads 256 MB in 0.74 ms. Coalescing the loads through threadgroup memory gains nothing, because a
thread's 32 values are one 128-byte line, which L1 serves. So a block bound for the GPU (a slab of later reads)
can be packed there in its interleaved layout, which its scans read at 0.52–0.78 ms against 0.74 for plain. The
CPU packs what leaves for disk. RUNS and LINE on the GPU are in §19.

### 19. Packed probes, packing on every thread, RUNS and LINE on the GPU

The three items §17–§18 left open.

**Packed probes.** They were about twice the cost of in-place probes, because in-place probes got faster and
packed probes did not. A packed probe made about four full binary searches, and every read went through the
column's descriptor: its spans are atomics, which the compiler reloads, and each read checked bounds and
dispatched on the encoding. Now:

- **A column is resolved once a probe** (`IntCol::dec` → `Dec`): its fields, mask and word slices in registers.
  A read is two loads and two shifts with no branch (`rd`; the spare word at each column's end makes the second
  load safe).
- **Searches guess by interpolation, then gallop** (`col::search`). A packed block's sorted columns are near
  linear: an index's keys, the run values, the starts, a leading column. So one guess between the ends lands
  within a few positions, a gallop brackets the answer, and a binary search of the bracket finishes it. That is
  about six reads, where a binary search reads `log2 n`. A skewed column costs at most about twice the binary
  search.
- **A leading key's range is one search** (`Dec::equal_range`). RUNS takes the run's end; the others gallop over
  the key's few rows. It replaces two lower bounds, each with two run lookups.
- **`IntCol::get` reads PACK without a branch** (at least one bit); the other encodings go through `Dec`.

On 1M rows (`encodings`), in instructions retired:

| | In place | Packed, before | Packed, now |
|---|---|---|---|
| 1M probes on col 1, with a read per hit | 0.79 G | 1.51 G | 1.00 G |
| 1M probes on col 0 (leading), with a read per hit | 0.98 G | 2.01 G | 1.09 G |

Packed probes now cost 11–27% more instructions than in place. In wall time they took less in every run (in
one, 133 against 194 ms on col 1 and 127 against 285 ms on col 0), likely because a packed block is 2.4 times
smaller and misses cache less. The test checks both searches against a scan, for each encoding
and over sub-ranges.

**Packing on every thread.** A relation's packing ran on one thread. Now each of its steps runs on every thread
once a relation holds 64K values (rows times columns); under that, handing work to other threads costs more than
the work:

- **Reading** the columns, and gathering them into canonical order, a column a task.
- **Choosing** each column in parallel, and each column in pieces of 64K values (`over_pieces`): the range, the
  run count and LINE's sample (the residuals at every 64th row) in one read a piece, folded, and LINE's full
  residual pass the same way when the sample leaves it a chance. The slope needs only the first and last values,
  so the sample needs no read of its own. It had one, strided and on one thread, which took a third of the
  choice's time on one thread and most of it on 18.
- **Writing** after laying out: the block, its descriptors and every column's words are allocated first
  (`IntCol::lay_out`), then the columns are written in parallel, each into its own words. Within a column, a
  piece of 64K values is `64K * bits / 64` whole words, so pieces are packed in parallel and share no word.
  PACK and LINE now subtract while packing (`pack_resid`, a packer per width and per whether there is a line),
  with no pass to a temporary. RUNS counts each piece's runs, so each piece knows its first run index, and
  writes its runs from there. A piece's first and last words may be shared with its neighbours, so a run's bits
  are ORed in atomically, one atomic per full word.
- **Sorting** (`sort::split_par`) at 256K keys or more. The top bits split the keys into buckets of about 32K:
  each piece counts its top digits, and the offsets run bucket by bucket and, within a bucket, piece by piece,
  so the split is stable. Each bucket is then sorted by the rest of its bits on its own, in cache. When the low
  bits are a whole number of digits, that is as many passes as one thread's sort, otherwise one more. Recounting
  every pass of a parallel LSD sort was tried first: it cost 182 M instructions to the split's 138 M, against
  124 M on one thread, for 1M 49-bit keys.

Two runs, at loads of 38 and 41 (nice 19), in instructions and wall time:

| 1M rows | One thread, before | One thread, now | 18 threads, now |
|---|---|---|---|
| Pack from the frozen view | 0.19 G, 14 ms | 0.21 G, 14–19 ms | 0.21 G, 3.5–4.9 ms |
| Pack from open rows | 0.47 G, 38 ms | 0.46 G, 33–43 ms | 0.52 G, 12–22 ms |
| 64M values, choose + write | 1.68 G, 43 + 83 ms | 1.60 G, 19–29 + 46–75 ms | 1.56 G, 2.8–31 + 3.5–14 ms |

On one thread, the split sort and the parallel plumbing cost about 10% more instructions packing 1M rows from
the frozen view, and none from open rows. On every thread, packing 1M rows took 2 to 4 times less wall time.
Wall times on a loaded machine vary by run: a fork-join waits for its slowest piece, and at nice 19 some worker
is often descheduled. In earlier runs at loads of 17 to 27, a pool of 18 packed slower than one thread, and a
pool of 4 took 9.0 ms against 23. On 64M values, the write took 5 to 13 times less wall time on
18 threads, and the choice up to 7 times less. In `paces` (300 documents), packing 123 mostly small blocks took
25.6 ms. With no size threshold it took 421 ms, because a small relation's handoff to other threads cost more
than its packing.

**RUNS and LINE on the GPU** (`encodings`, 32M values, best of 6, warm). Each is checked word for word against
the CPU's encoder.

- **The choice in one read** (`stats`): the range, the runs that start (a value unlike the one before it) and
  the line's residual range, as threadgroup partials (simdgroup reductions, then the threadgroup's 8) that the
  host folds. The line through the first and last values needs only those two, so its residuals come in the
  same read. The host chooses by exact size, as `col::choose` does.
- **32-bit arithmetic.** Every residual is within the column's range of 0, because the line runs between the
  first and last values. So below a range of 2^31, the line and the residuals are exact modulo 2^32. The slope
  goes in as its high 32 bits (signed) and its low 32 bits, and `j * slope / 2^16` is formed from 32-bit
  products and `mulhi`. A first version in 64-bit arithmetic, with a threadgroup tree to reduce the residuals,
  took 1.0–1.4 ms for the choice; emulated 64-bit operations made it bound by arithmetic, not memory. A column
  with a wider range does not try LINE on the GPU. That is a valid encoding, and LINE cannot win on such columns
  in practice.
- **LINE write** (`pack_line`): a thread packs a group of 32 residuals into `B` words, as PACK does.
- **RUNS write**, three steps. Each threadgroup (8,192 values) counts the runs that start in it. One threadgroup
  of 1,024 threads turns the counts into an exclusive prefix sum. Then each thread finds its first run index
  (its threadgroup's offset, plus a simdgroup prefix sum and the earlier simdgroups' totals) and writes each of
  its runs: the value and the start, ORed in atomically, since a run's bits may share a word with another
  thread's.

| 32M values | GPU choose | GPU write | CPU `encode`, 1 thread | CPU, 18 threads |
|---|---|---|---|---|
| RUNS (1.03M runs; 29 + 26 bits) | 0.64–0.72 ms | 0.99–1.93 ms (two fills, three kernels) | 42–67 ms | 5.6–21 ms |
| LINE (3 bits) | 0.68–0.71 ms | 0.75–0.82 ms | 48–81 ms | 5.8–22 ms |

So the GPU now packs every encoding: 3 to 15 times faster than the CPU on every thread, and 25 to 55 times faster
than one. The RUNS write's
second read could be folded into the count (the count's threadgroups would then take the write's layout); it
has not been, as 1.9 ms is already small next to the CPU's figures.

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
