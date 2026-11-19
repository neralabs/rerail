# The engine's data structures

A living catalogue: every structure the engine keeps, its form today, and where it goes. Keep it current when a
structure is added, changed, or moved into its block.

## Intro

Decision 0266 puts the engine's data in a few big blocks, each one of the engine's meaningful things:

- a program;
- a chunk of documents and their file-local facts;
- a phase's cross-file facts;
- a run's outputs;
- a store's atoms;
- a wave's journal.

Everything that belongs to a thing is nested inside its block, under one root, with a description of the layout
in the block. For example, a relation's rows, its hash indexes and its statistics sit side by side in its phase's
`Facts` block.

This page lists, block by block, today's structures and where each one sits in its block. Each table has three
columns:

- **Today:** the structure, and where it lives;
- **Readers:** who reads it: host threads, device kernels, the store;
- **Place in the block:** its path from the root, in 0266 §2's notation.

Then come the structures that stay Rust, and the gaps the survey found.

Notation:

- `Vec<T>`, `Ptr<T>` and `Opt<T>` are references relative to their own address (0266 §3).
- `IntTable` is MLDB's 16-byte inline integer table: bit-compressed, run-length or factored.
- `StringTable` is MLDB's: an `IntTable` of offsets, and the text.
- `HashTable<K,V>` is open-addressed buckets.
- `Bits` is a bitset.

## `Program`: a version of the program a run evaluates

| Today | Readers | Place in the block |
|---|---|---|
| Program image (`logic/program_image.rs`, 0173): 40 regions under a 16 + 24·R byte header; SQLite blobs copied into a `Vec<u128>` on open | planner, lowering, setfix | the root: `rules`, `declarations`, `facts`, `strata`: each a `Vec` of its structure, with `names: StringTable`. Read in place; the copy goes. |
| Program versions (`program_image/version.rs`, 0241, 0244): base, view, overlay; folded at half | the same | a `Program` delta block naming its full block. An overlay's parent is always a base. |
| Module image, linked library (`logic/rr/module_image.rs`, 0246) | lowering, the linker | `modules: Vec<Module>`: each module nested whole, with its `.about` relations. |
| Packed transducer program: `ProgramRow`, `BlockStep` (MSL in `block_program.metal`), its encoders, struct text printed from Rust strings | interpreter, Cranelift, Metal | `transducers: Vec<Transducer>`, each with `programs: Vec<ProgramRow>`, `steps: Vec<BlockStep>` and `outs: IntTable`. The structs come from `structure!`. |
| Compiled sets, prefilters, anchor postings (`compile/sets.rs`, `text/canon.rs`, `waves/relational/syntax/packed.rs`) | host and device matchers | `matchers: Vec<Matcher>`: DFA tables, set `Bits`, postings as an `IntTable` grouped by an `IntTable` of starts. |
| Plan for a kernel: stratum specs, steps, slots | `setfix::open`, Metal strata | `strata[s].plan`: its steps and slots. `Placed` and `Fields` stay Rust while being planned. |

## `Chunk`: a chunk of documents and everything made of them

| Today | Readers | Place in the block |
|---|---|---|
| Source text, per document; copied into a residency slab on Metal | lexers, Metal lexing | `docs[d].source: Vec<u8>` |
| Tape (`crates/tape_format`): a header of offsets, aligned columns | matchers, native facts, Metal tape kernels | `docs[d].tape: Tape`: `code: IntTable`, `size`, `off`, `tail`, `parent` (each an `IntTable`), `text: StringTable`. A tape is a structure, not a block. |
| `DocView` (`crates/metal/src/slabs.rs`, 96 bytes) and its MSL copy; `DocTable`; `Slabs { s0..s3 }` | Metal tape kernels | Gone. The chunk root's `docs: Vec<Doc>` is the table; a document's place is its index, and its tape is a nested structure. |
| Long texts (`syntax::LongTexts`, an index by document key) | the `text` functions | `long: StringTable` and `long_by_key: HashTable<i64, u32>` |
| Python facts (`matcher/python_facts::Batch`: `BTreeMap<NodeRange, Vec<NodeFacts>>`) | matchers, vtab | `docs[d].python: Vec<NodeFacts>`, sorted by range |
| File digests | the store, memo keys | `docs[d].digest: [u8; 32]`, `paths: StringTable` |
| The chunk's file-local relations (made by its job, 0254) | the chunk's job; their exports go to the cross part | `facts: Table<Relation>`, written after the documents, each relation's rows grouped by document. A document's rows are its range, which is what the memo keeps. |

## `Facts`: a phase's cross-file relations, as a run derives them

| Today | Readers | Place in the block |
|---|---|---|
| `setfix::Rel`: `Rows` (segments `Arc<Vec<i64>>` plus a tail), `idx: Vec<Index>`, `pending: HashTable<u32>`, `sketches`, `dead: Vec<u64>`, `level: Vec<u32>`, `sorted`, `Lat`, `KeptStats` | setfix's joins, Metal strata | `relations[r]`: `columns: Vec<Column>` (`Rows`: a `Grow` of doubling column-major chunks, 0266 §5), `indexes`, `pending: HashTable`, `dead: Opt<Bits>`, `levels: Opt<IntTable>`, `sorted: Vec<Run>`, `stats: Vec<ColumnStats>` (sketches and kept stats included), `lattice: Opt<Lattice>` |
| `setfix::InRel`: rows, `idx: Mutex<HashMap<u64, Arc<Csr>>>`, `chains`, `view` | probes, Metal joins | a relation of the full `Facts` block (a kept or lower-stratum relation). Each index is in its `indexes`, built once per key. The `Mutex<HashMap>` goes. |
| `setfix::Csr`: `table: HashTable<(u32,u32,u32)>`, `ids: Vec<u32>`, `order` | probes | `relations[r].indexes[i]`: `key: Vec<u16>`, `table: HashTable<u64, u32>`, `rows: IntTable` |
| The extension's `InRel` (`native/extension/setfix.rs`) | the extension path | a relation, as above |
| Metal join state: `metal_join::Batch`, `Resident`; `SortJob` (`accel/src/gpu/join.rs`, device pointers mirrored by hand) | Metal | The relations are read in place from the pinned block. `SortJob` becomes a `structure!`, and its pointers are paths resolved at launch (0266 §6). |
| `accel`: `table.metal` codes and offsets, `closure.metal` CSR and bitsets, as positional buffers | `accel` kernels | relation columns and indexes, read in place through `kernel!` arguments |
| Maintained runs: kept relations plus this run's changes (0077, 0083) | setfix, the session | a cumulative `Facts` delta naming the kept full block: added rows, dead rows, replaced statistics; rolled up by the cost rule (0266 §6) |
| Kept statistics, sketches | the planner | `relations[r].stats` |

## `Outputs`: a run's outputs

| Today | Readers | Place in the block |
|---|---|---|
| `plan/output.rs` `Rows`: `types`, `vals`, `texts`, `ids: HashMap`, `bare` | the outcome, sessions, the writer | `relations[r]`: the same `Relation` structure as `Facts`, plus `texts: StringTable` |
| The output image stack (`outputs.rro`, overlays folded at depth 8) | sessions, findings | an `Outputs` full block and one cumulative delta |
| Findings: rows of a `.violation`'s relation | the report | `relations[r]` of the violation relations |

## `Atoms`: a store's atoms

| Today | Readers | Place in the block |
|---|---|---|
| `content_store/atoms.rs` `Interner`: stored segments read from SQLite, pending atoms above `base` | every run, the writer | full block: `text: StringTable`, `hash: HashTable<u64, u32>`. The pending atoms are its delta block. A roll-up appends them. |

## `Journal`: a wave's writes (0260 §8)

| Today | Readers | Place in the block |
|---|---|---|
| `write_back` closures per wave (0259 stage 1) | the store's writer | the block itself, in a file |
| Frames (`content_store/frames.rs`: `Rows`, `Rel`, `Stored`, `Retag`; varint deltas in `Arc<Vec<u8>>`) | the store, maintained runs | `frames: Vec<Frame>`: each a relation subtree copied whole from `Facts`, plus its reads and digest |
| Memo entries (`corpus/memo.rs` `Entry`: `BTreeMap`s of rows and outputs; a `MemoBlob` image, decoded on a hit) | the corpus run | `memo: Vec<MemoEntry>`: per document, its range of the chunk. A stored entry is that range's bytes, so a hit is one copy into a new `Chunk`. |
| Tapes, sources to store | the store | `tapes`: ranges copied from the `Chunk` |

## What stays Rust

These stay ordinary Rust structures, because nothing but the host's control flow reads them: no kernel, no
other run, and no file.

- **Planning:** plans while being planned (`Placed`, `Fields`, `ColMap`, `Stats`), and the scheduler's and the
  scoreboard's own state.
- **Bookkeeping:** sessions' bookkeeping, tallies (`QuietTally`, `Report`), and options.
- **Interfaces:** the vtab traits (`Source`, `RowCells`, `RowStream`). They are interfaces, and the sources
  behind them read the blocks.
- **SQLite:** its tables stay SQLite. A blob that holds a block's subtree holds its bytes as they are.

Not yet surveyed field by field: the SQL executor's arenas (`sql_executor/component_loop/arena.rs`,
`temporal/cold_loop`, `work_graph`, `reductions`).

## Device memory

| Today | In the block model |
|---|---|
| `accel/src/gpu/shared.rs`: power-of-two classes from 16 KiB; inputs copied when not page-aligned | Each block is a page-aligned mapping, pinned once, so no input is copied. Kernel scratch is a `Scratch` block per device, reused. |
| `crates/metal` `MappedBuffer`, `slabs.rs` (64 MiB slabs) | The same: residency slabs become `Chunk` blocks. |
| MLDB's `memory_region.rs` (the forest's port): region states | The blocks' states: open, frozen, retired. |

## Gaps the survey found

- **Hand-kept mirrors.** `DocView` is kept by hand on both sides (`slabs.rs` and `metal_tape.metal`), as are
  `SortJob`, `ProgramRow` and `BlockStep`. Only 9 `static_assert(sizeof)` checks exist, and none checks a field's
  offset.
- **Positional buffers.** Kernels take positional `[[buffer(n)]]` arguments, up to 19 (`block_batch_*`). Their
  Rust callers bind by index, and nothing checks the two sides against each other.
- **Duplicated accessors.** Each accessor is written twice, in MSL (`tape_size`, …) and in Rust
  (`tape/resident.rs`).
- **Struct text printed from Rust strings,** by three code generators.
- **Four relation-rows types.** `frames::Rows`, `output::Rows`, `setfix::rows::Rows` and the memo entry's rows
  all hold rows of typed `i64` columns. They become one `Relation` structure.
- **Two device pools,** in `accel` and in `crates/metal`.
- **144 `#[repr(C)]` structs.** Those that cross to a device or a file become `structure!`s.
