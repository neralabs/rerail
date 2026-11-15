# The engine's data structures

A living catalogue: every structure the engine keeps, its form today, who reads it, and the block it becomes.
Keep it current when a structure is added, changed, or moved onto a block.

## Intro

Decision 0260 says every structure becomes a block. A block is one page-aligned slab that uses relative offsets
and no pointers, so the same bytes serve the CPU, a GPU and a file. Decision 0266 says how a block's layout is
written down once, and how CPU code and GPU kernels then reach into it the same way.

This page lists what has to move. For each structure it gives:

- **Today:** its current form, and where it lives;
- **Readers:** who reads it: host threads, device kernels, the store on disk;
- **Block:** its part kind and regions (0266 §4 names the region forms), or why it stays a Rust structure. A
  part is one structure inside a block. Small parts with one lifetime share a block (0266 §2): a chunk's tapes,
  a wave's writes, a run's outputs. Integer columns written below as `records<i64>` are `packed` per column
  (0266 §6).

A structure stays an ordinary Rust structure only when all three of these hold:

- no device reads it;
- no file holds it;
- no other run reads it.

That covers plans while they are compiled, schedulers, sessions' bookkeeping, and tallies of a few numbers. It
does not cover anything a kernel, a wave or the store reads.

Region forms used below (0266 §4): `items<T>` (a fixed-width array), `records<R>` (rows or columns, chosen by the
writer), `strings` (bytes, ends and an optional hash), `csr` (offsets and ids), `hash<K,V>` (open-addressed
buckets), `sorted<K>` (a sorted key array), `bits` (a bitset), `packed(w)` (integers at a width chosen at write),
`links` (other blocks, by digest).

## 1. Inputs: sources and syntax

| Structure | Today | Readers | Block |
|---|---|---|---|
| Source text | bytes read from the file, held per document; on Metal, copied into a residency slab | host lexers, Metal lexing passes | `Source`: `items<u8>`, the digest in the header. It is read in place, never copied into a second form. |
| Tape | `crates/tape_format`: a header of offsets, aligned columns; held as `Vec<u8>`, or as zstd frames on disk | host matchers, native facts, Metal tape kernels (`metal_tape.metal`) | `Tape`: already in block form. Its header becomes the shared one: `code: items<u16>`, `size`/`off`/`tail: packed(w)`, `parent: items<u32>`, `strings`. Read in place, with no decoding (zstd on disk only). |
| Tape residency view | `crates/metal/src/slabs.rs` `DocView` (96 bytes, `#[repr(C)]`), copied by hand as `DocView` in `metal_tape.metal`; `DocTable` per batch | Metal tape kernels | Gone as a hand-kept mirror: it is the tape block's header read through its `PartRef` (0266 §3). The batch's `DocTable` becomes `items<PartRef>`. |
| Residency slabs | `crates/metal` `MappedBuffer::slab`: 64 MiB anonymous mappings, one no-copy `MTLBuffer` each, bump-allocated at 16 bytes | the Metal path | The pool's slabs (0260 §2). `Slabs { s0..s3 }` (four bindings) becomes the block table (0266 §3). |
| Long texts | `syntax::LongTexts`: bound per run, an index by document key | the `text` functions in `setfix` | `strings` of the run's batch, reached by document key through `sorted<i64>` → index. |
| Python batch, documents | `syntax::Batch`, `value/python_ast::Document`, `matcher/python_facts::Batch` (`BTreeMap<NodeRange, Vec<NodeFacts>>`) | host matchers, vtab `Source::python_batch` | `PyFacts`: `records<NodeFacts>` sorted by range, `strings`. The `BTreeMap` goes, replaced by `sorted<(u32,u32)>` over ranges. |
| File digests | `[u8; 32]` per path, computed on the pool | the store, memo keys | `items<[u8;32]>` beside a `strings` of paths, in the run's manifest block. |

## 2. Strings and atoms

| Structure | Today | Readers | Block |
|---|---|---|---|
| Atoms | `content_store/atoms.rs` `Interner`: stored segments read from SQLite (0111 §2), pending atoms back to back above `base`; `hash_str` the one hash | every run, the store's writer | A chain (0266 §6) of `Atoms` blocks: `strings` with the hash table. Each stored segment is a frozen block, and the pending tail is the open one. An id is global to the chain, so a delta never renumbers. |
| Image strings | each image's own strings region (program, module, output) | host readers | The one `strings` form, shared by every kind (0260 §1). |
| Output texts | `output::Rows.texts: Vec<Box<str>>` and `ids: HashMap<Box<str>, u32>` | the writer, findings | `strings` with its hash: the `HashMap` and the boxed strings go. |

## 3. Programs

| Structure | Today | Readers | Block |
|---|---|---|---|
| Program image, versions | `logic/program_image.rs`, `program_image/version.rs` (0173, 0241, 0244): 40 regions, a 16 + 24·R byte header, a stack of base, view and overlay; SQLite blobs copied into a `Vec<u128>` on open | the planner, lowering, setfix | `Program`: the same regions under the shared header, mapped in place (the copy goes). Versions are a chain (base + overlays, folded at half). |
| Module image, linked library | `logic/rr/module_image.rs` (0246): the program image format | lowering, the linker | `Program` blocks; the library is `links` to its modules. |
| Plan structures | `Placed`, `Fields`, stratum specs: Rust trees | the planner, then `setfix::open` | They stay Rust while planned. What a kernel runs from them is a `Plan` block: `records<Step>`, `items<u32>` of slots, `links` to the programs. |
| Packed transducer program | `crates/metal/src/block_program.metal` `ProgramRow`, `BlockStep` (MSL structs), the Rust encoders in `transducer/packed`; some structs printed from Rust strings (`metal_program_backend.rs`, `metal_register_codegen.rs`, `metal_label_patterns.rs`) | the interpreter, Cranelift, Metal | `Transducer`: `records<ProgramRow>`, `records<BlockStep>`, `items<u32>` outs. The structs are `item!`s (0266 §2), so the MSL is generated, not hand-copied or hand-printed. |
| Compiled sets, prefilters, anchor postings | `transducer/packed/compile/sets.rs`, `text/canon.rs`, `waves/relational/syntax/packed.rs` | host and device matchers | `Matcher`: DFA tables `items<u16>`, set bitsets `bits`, postings `csr`. |
| Generated tiers | `rerail-rr-aot`, Cranelift and MSL code | — | Code, not data. Its constants (tables) are `Matcher` blocks. |
| Cost model | mldb bagged boosted trees on Metal | the planner (off its path) | `Forest`: trees in preorder with subtree sizes, `records<Node>`. Already flat in MLDB's form; it takes the shared header. |

## 4. Facts inside a run

| Structure | Today | Readers | Block |
|---|---|---|---|
| A relation being derived | `setfix::Rel`: `Rows` (frozen segments `Arc<Vec<i64>>` plus a tail), `idx: Vec<Index>`, `pending: HashTable<u32>`, `sketches`, `dead: Vec<u64>`, levels `Vec<u32>`, `Lat`, `KeptStats` | setfix's joins, the Metal strata (0095) | `Relation`: a chain. Each frozen segment is a block of `records<i64>` (columns or rows, 0266 §4); the tail is the open block. Beside them, `dead: bits` and `level: items<u32>` (when `leveled`), and `pending: hash<u32>`. `Sketch` and `KeptStats` stay Rust (a few numbers per column). |
| An input relation | `setfix::InRel`: `rows`, `idx: Mutex<HashMap<u64, Arc<Csr>>>`, `chains: Vec<Index>`, `view` | setfix's probes, Metal joins | The same `Relation` block. Each index is a block of its own: `Index` (`csr` of ids by key, `hash<(u32,u32,u32)>`), linked from the relation by `links`. The `Mutex<HashMap>` becomes a block table entry, made once per key set. |
| `Csr` index | `setfix::Csr`: `cols`, `table: HashTable<(u32,u32,u32)>`, `ids: Vec<u32>`, `order: OnceLock<Index>` | probes | `Index` (above). |
| Native extension relations | `native/extension/setfix.rs` `InRel` (rows `Vec<i64>`) | the extension path | `Relation`. |
| Vtab sources | `facts/vtab.rs` `Source`, `RowCells`, `RowStream`: an access interface over many producers | SQLite's virtual tables | It stays an interface. Its producers read blocks: `Source::row_at` over a `Relation` block is a slice read. |
| Column maps, plan tables | `plan/table.rs` `ColMap<T>`, `Stats` | the planner | They stay Rust. |
| Metal join state | `metal_join::Batch`, `Resident` (relations uploaded once per chunk); `accel/src/gpu/join.rs` `SortJob` (a struct of device pointers, mirrored by hand in MSL) | Metal | The relations are `Relation` blocks, resident by the pool. `SortJob` is an `item!` whose pointers are `PartRef`s (0266 §3). |
| accel table and closure inputs | `table.metal` (`ushort` codes, `uint` offsets, `uint2` pairs), `closure.metal` (CSR `off`/`dst`, `R` bitsets) passed as positional buffers | accel kernels | `Codes`: `csr` over `items<u16>`; `Closure`: `csr` plus `bits`. Their kernels are declared with `kernel!` (0266 §5). |
| SQL executor arenas | `sql_executor/component_loop/arena.rs` `Rows`/`Batch`, `temporal/cold_loop` `Rows<A>`/`Batch`, `work_graph/worker.rs` `Rows`, `reductions` `Batch` | the SQL executor | Not yet surveyed field by field. They take `Relation` when their path is ported. |

## 5. Handed between runs and waves

| Structure | Today | Readers | Block |
|---|---|---|---|
| A run's output rows | `plan/output.rs` `Rows`: `types`, `vals`, `texts`, `ids: HashMap`, `bare` (0239), shared by `Arc` | the outcome, a session's kept outputs, the writer | `Rows`: `items<u8>` types, `records<i64>` values, `strings`. It is the same kind as a frame's rows (below): one relation-rows block, not four types. |
| Output image stack | `outputs.rro`: regions, strings, overlays folded at depth 8 | sessions, findings | A chain of `Output` blocks (already regions; takes the shared header). |
| Memo entry | `corpus/memo.rs` `Entry`: `BTreeMap<String, (usize, Vec<i64>)>` and outputs `BTreeMap<String, (Vec<usize>, output::Rows)>`; stored as a `MemoBlob` image, decoded on a hit | the corpus run | `Memo`: `links` to `Rows` blocks by relation, named in a `strings`. A hit maps the blocks and decodes nothing. |
| Frames | `content_store/frames.rs` `Rows { types, rows: Vec<i64>, n }`, `Rel`, `Stored`, `Retag`; stored as varint deltas (`Arc<Vec<u8>>`) | the store, maintained runs | `Rows` in memory. On disk, `packed(w)` columns (bit-compressed per column, MLDB's form) replace the varints, so a frame is read in place. |
| Cross rows | `cross_rows` | the cross part | `Rows`. |
| Quiet tally, report counts | `QuietTally`, `Report` | the report | They stay Rust. |
| Wave writes | `write_back` closures per wave (0259 stage 1) | the store's writer | The journal block (0260 §8): a file-backed delta block, one region per kind of write. |

## 6. The store

| Structure | Today | Readers | Block |
|---|---|---|---|
| SQLite tables | the source of truth (writes may lag; memory may be ahead) | every run | They stay SQLite. Blobs in them are frozen blocks, byte for byte, so reading a blob maps or views it. |
| Journal | 0260 §8 (planned) | the writer | `Journal`: a delta block in a file. |

## 7. Device memory managers

| Today | Block |
|---|---|
| `accel/src/gpu/shared.rs`: a pool of power-of-two classes from 16 KiB; inputs copied when not page-aligned | One pool (0260 §2). Every block is page-aligned, so no input is copied. |
| `crates/metal` `MappedBuffer`, `slabs.rs` (64 MiB slabs) | The same pool. |
| MLDB's `memory_region.rs` (the forest's port): New/Allocated/Frozen/Zombie/Deleted, versioned handles | The pool's states (0266 §7). |

## Gaps this catalogue found

- **Hand-kept mirrors.** `DocView` is kept twice by hand (`slabs.rs` and `metal_tape.metal`), as are `SortJob`,
  `ProgramRow` and `BlockStep`. Only 9 `static_assert(sizeof)` checks exist, and none checks a field's offset.
- **Positional buffers.** Most kernels take positional `[[buffer(n)]]` arguments, up to 19 (`block_program.metal`
  `block_batch_*`), and their Rust callers bind by index. Nothing checks the two sides against each other.
- **Duplicated accessors.** Each accessor is written twice, in MSL (`tape_size`, …) and in Rust
  (`tape/resident.rs`).
- **Struct text printed from Rust strings.** `metal_program_backend.rs`, `metal_register_codegen.rs` and
  `metal_label_patterns.rs` print MSL struct text from Rust strings.
- **Four relation-rows types.** `frames::Rows`, `output::Rows`, `setfix::rows::Rows` and the memo entry's
  `(usize, Vec<i64>)` all hold rows of typed `i64` columns. The `sql_executor` adds several more.
- **Two device pools.** `accel` and `crates/metal` each manage device memory, with different classes.
- **144 `#[repr(C)]` structs.** Those that cross to a device or a file become `item!`s. Host-only FFI ones stay.
