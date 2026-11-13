# Tempo tape and source residency

The short version: Tempo has real page-allocated, uncompressed caches and real pinning, but its production Metal path still copies every tape on every evaluation. v9np can avoid that copy, but four lines in our spec currently forbid or ignore the residency design (Part D lists them).

Abbreviations used below:
- **T:** = `/Users/jeremy/orca/workspaces/nera-landing-2/ner-1562-scale-test-the-re-railer/tempo/`. Branch `jbarnes/ner-1562-scale-test-the-re-railer`, HEAD `d737fad16`. The working tree is clean (`git status` shows nothing), so everything below is committed.
- **M:** = `/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/native/src/`.
- **S:** = `/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/docs/rule-engine-direction/`.

---

# Part A. What Tempo actually does

## A.0 Summary

What the user called "cached, pinned, page-allocated uncompressed tapes and source", split into what is built and what is only designed:

| Claim | Status in Tempo | Where |
|---|---|---|
| Uncompressed tapes and source cached | **Real.** One 64 MiB anonymous mapping holds decompressed tapes and sources, with a 32 MiB logical limit and a 30 s sliding lifetime. The cache was too small to get hits | `T:native/src/content_store.rs:968-1287` |
| Page-allocated | **Real.** Allocations are page-rounded anonymous `mmap`s (`sysconf(_SC_PAGESIZE)`, which is 16 KiB on Apple Silicon). This covers no-copy Metal buffers, CPU JIT workspaces and the cache arena | `metal_backend.rs:170-237`, `native_cpu.rs:30-100`, `content_store.rs:987-1077` |
| Zero-copy Metal (`newBufferWithBytesNoCopy`) | **Real but partial.** Programs and small inputs use shared no-copy buffers. The big per-evaluation input (nodes, source, line starts) is copied through blits into a private buffer | `metal_backend.rs:344-382`; `metal_register_execution.rs:32-163` |
| Pinned | **Real for retained mutable buffers** (measured 30–60× faster on reuse). A design exists for a tape cache keyed by tape identity. **It is not implemented**: the resident input is rebuilt every evaluation | `docs/shared-sparse-bump-arena-design.md:176-221, 275`; `metal_program_backend.rs:78-94` |
| Pinned (in SQL) | **Real.** Lease rows in `object_pins` protect objects from garbage collection | `content_store.rs:149-156` |
| Huge pages, `mlock`, `MTLHeap`, `MTLResidencySet`, `useResource` | **Absent.** `mlock` was explicitly rejected; `MTLResidencySet` was deferred because the `metal` crate lacks it | `shared-sparse-bump-arena-design.md:193-221`; grep finds none |

## A.1 At rest: the content-addressed store (SQLite)

- **Schema** (`T:native/src/content_store.rs:54-160`, `SCHEMA_VERSION=22` at `:31`):
  - `source_objects(source_id, canonical_hash_id UNIQUE, raw_length, codec_id, frame BLOB, created_generation)` at `:77-84`.
  - `tape_objects(..., source_id, canonical_hash_id, grammar_id, grammar_version, tape_format_version, codec_id, raw_length, frame BLOB, …, UNIQUE(source_id, grammar_id, grammar_version, tape_format_version))` at `:90-102`.
  - `snapshot_files` and `workspace_overlay` reference `(source_id, tape_id)` at `:122-145`.
  - `object_pins(object_kind_id, object_id, owner_string_id, lease_until)` holds garbage-collection lease pins (`:149-156`).
- **Compression:** each source and each tape is its own zstd level-3 frame (`CODEC_ZSTD=1`, `DEFAULT_COMPRESSION_LEVEL=3`, `:35,:46`; encode at `:2080, :2389, :3269-3273`). The raw length and canonical SHA-256 stay outside the frame.
  - Design rationale is in `T:docs/content-addressed-workspace-store.md:75-100`. Over 5,725 files, 95.7 MB of source and 160.38 MB of raw v4 tapes compress to 56.08 MB, i.e. 21.9% of raw.
  - Measured zstd levels: L1 58.26 MB in 466 ms; L3 56.08 MB in 590 ms; L5 53.58 MB in 1,269 ms; L7 51.59 MB in 2,113 ms.
  - Independent frames give a per-file access p50 of 59–77 µs and p95 of 374–497 µs.
  - At the 44-workspace scale, frames were 20.3% (sources) and 21.5% (tapes) of their logical length (`T:docs/workspace-store-runtime-journal.md:600-614`).
- **Why it is on disk rather than in RAM** (`content-addressed-workspace-store.md:166-181`): compressed frames for 44 workspaces are 0.85–0.95 GiB. Restored source and tapes are 3.8–4.2 GiB content-addressed, or 13–15 GiB path-addressed. The doc also notes that "content addressing reduces RAM only when processes share mappings or one process owns the object cache."
- **SQLite settings:** WAL, `cache_size=-16384` (16 MiB), `journal_size_limit` 16 MiB, `wal_autocheckpoint` 4096 (`content_store.rs:1325-1337`).
- **Bulk loading:** the evaluator fetches every needed tape in one `json_each` query before workers start (`content_store.rs:2410-2477`, called at `T:native/src/policy_dsl/workspace.rs:1201-1212`). A missing row means "parse again"; there are no point lookups.
- **Validation:** the codec, raw length and SHA-256 of the decompressed bytes are checked on every store read (`:2461-2470`). A failure drops the entry and the object is rebuilt from source (`content-addressed-workspace-store.md:743-753`).
- **Garbage collection:** liveness comes from database relationships plus leases. Deletion runs in bounded batches in a fixed order: tapes before sources (`content-addressed-workspace-store.md:610-663`).

## A.2 The decompressed-object cache (uncompressed and page-allocated)

Code is at `T:native/src/content_store.rs:968-1287`.

- **Key:** `enum CacheKey { Source(i64), Tape(i64) }`, which is a SQLite row id, not a content hash (`:968-972`).
- **Value:** `Arc<CachedBytes>`. The backing is either `Arena{Arc<MmapArena>, offset, length}` or `Heap(Box<[u8]>)` (`:974-985`).
- **Arena:** one `mmap(PROT_READ|WRITE, MAP_PRIVATE|MAP_ANON)` of `2 × byte_limit`, which is 64 MiB (`:987-1032`). Entries are bump-allocated at **8-byte-aligned** offsets: `(cursor+7)&!7` at `:1259`. On drop the arena runs `MADV_DONTNEED` then `munmap` (`:1065-1077`).
- **Policy** (`:1111-1287`):
  - LRU order is a `VecDeque` plus a `HashMap`, with a 30 s sliding TTL refreshed on every `get` (`:1191-1200`).
  - Limits are `DEFAULT_READ_CACHE_BYTES=32 MiB` and `DEFAULT_DECOMPRESSED_CACHE_TTL=30s` (`:48-49`).
  - Eviction pops the LRU head until the new entry fits (`:1236-1243`).
  - An entry larger than the limit gets heap bytes and is not cached (`:1234-1235`).
- **Compaction** (`:1201-1229`): live entries are copied into a **new** mapping, and the old one is kept as `Weak` until its readers finish. Compaction never creates a third mapping; if a retired mapping is still alive, the entry falls back to heap.
- **Sharing:** one `ContentStore` per database path per process, so all workspaces share one cache (commit `a553debac`; `content-addressed-workspace-store.md:716-731`).
- **Rejected alternative:** one mapping per large tape, rejected because every mapping adds page-table and VM-zone metadata (`workspace-store-runtime-journal.md:517-520`).
- **Copy behaviour:** `get_source` returns `source.to_vec()`, a heap copy on every hit (`:2337, :2372`). Tapes return an `Arc` slice into the arena.
- **Measurements:**
  - Entry 22 (`journal:536-551`): a full scan took 4.83 s, with **0 tape-cache hits** and 5,663 decompressions. Each tape is read once per scan.
  - Entry 23 (`journal:616-636`): a warm run over an already-stored worktree still **decompressed 8,478 tapes**, with 1 cache hit. Storage reads plus decompression cost **37.60 worker-seconds**, against 0.003 s of native matching.
  - Entry 24 (`journal:638-681`): 4.15 GB RSS with 46 workspaces. `vmmap` showed **3.3 GiB of dirty malloc high-water** from repeated decompression and source ingestion. The cache arena itself had only 32 KiB resident after expiry.
  - **Lesson:** a TTL cache much smaller than the working set does nothing, and decompressing into heap `Vec`s inflates allocator high-water.

## A.3 Workspace residency of source and tape (CPU)

- `FileState` keeps `input.source: Arc<String>` (the whole UTF-8 source) and `tape: Option<(revision, Arc<TapeImage>)>` resident (`T:native/src/policy_dsl/workspace.rs:187-210`). Phase B made source and compact tape resident and the tree-sitter tree transient (`T:docs/phase-b-tape-residency.md:11`).
  - Measured: initial RSS **−29.98%**, initial CPU **+17.45%**, and no change in edit latency (`phase-b…:23`).
  - Logical retained bytes (subset, landing, data-layer): 33.8 MB, 241.0 MB and 437.0 MB (`phase-b…:30`).
- **Tape v4 (`TMPTAPE4`)** (`T:native/src/policy_dsl/tape.rs:16-28`; `TapeImage` at `:348-370`):
  - a 6-byte `rec` (kind | field<<10, parent delta, size, with `0xFFFF` sentinels backed by overflow pairs);
  - zigzag-varint spans plus 6-byte checkpoints every 16 nodes;
  - sorted extra, error and missing lists, alias tables, supertypes, and the source digest.
  - This is the same family as main's v7.
- **Restore on every use:** `TapeDoc::restore_with_lines` decodes into `Arc<Vec<Rec>>`. `Rec` is array-of-structs: u16 kind and field, seven u32 fields, three bools (`tape.rs:1032-1046`, `:1111-1119`).
  - Every restore **re-hashes the whole source with SHA-256** to check revision binding (`:1153-1159`).
  - `LineIndex` rescans the source for `\n` (`:1066-1105`). Phase 5 added a request-local cache so a source is scanned once per request instead of once per witness: 1,310,720 → 2,560 scanned bytes (`T:docs/metal-execution-cutover-phase5.md:302-306`).
- `TapeSource{source, grammar, image, lines}` is the executor input. It is restored lazily ("expanded navigation exists only while this file is packed", `tape.rs:1121-1142`, built at `workspace.rs:3932-3938`).

## A.4 Packing for executors: the copy that v9np would delete

`PackedBatch::build_from_tapes` (`T:native/src/policy_dsl/transducer/packed_batch.rs:199-290`) does the following for every file in every evaluation:
- restores the `TapeDoc` (`:180-195`);
- appends the whole source to `batch.source` (`:215-217`);
- rescans that source for `\n` to rebuild `line_starts` (`:218-226`);
- expands every node into a **44-byte `MetalNode`**: parent, next_any, next_named, child_start, child_count, descendant_end, source_start, source_end, kind, field, flags. That is 11×u32 (`metal_batch_abi.rs:34-46`, built at `packed_batch.rs:236-257`), plus a global `children` array.

`PackedBatch` (`:59-83`) is a set of `Vec`s on the ordinary heap. The CPU JIT reads it through raw pointers in `NativeTaskContext` (`native_cpu.rs:102-180`: nodes, children, source and line_starts are `*const`). CPU scratch lives in `DemandZeroRegion` (page-rounded anonymous `mmap` with `MADV_DONTNEED` on drop, `native_cpu.rs:30-100`; commit `ec1de49a6` "demand-map CPU packed workspaces").

## A.5 The Metal side

**Allocation primitives** (`T:native/src/policy_dsl/transducer/metal_backend.rs`):

- **Budget ledger before any allocation.** `METAL_BUFFER_LIMIT = 2_000_000_000` is a process-wide atomic ledger, with a separate private-bytes ledger (`:20-38`). A `BufferReservation` must be acquired before `mmap`/`new_buffer`, using page-rounded sizes, and is released on drop (`:74-126`). `METAL_ALLOCATION_LOCK` also checks `device.current_allocated_size()` against the limit (`:349-356, :397-404`).
- **`MappedRegion`** (`:170-237`), documented as "a page-backed host allocation retained for the complete lifetime of a no-copy Metal buffer":
  - the length is rounded up to `sysconf(_SC_PAGESIZE)`;
  - the memory comes from `mmap(MAP_PRIVATE|MAP_ANON)`;
  - `new()` **copies** the bytes in (`:216-222`);
  - drop runs `MADV_DONTNEED` then `munmap`.
- **Zero-copy wrap** (`from_mapping`, `:344-382`): `new_buffer_with_bytes_no_copy(ptr, len, StorageModeShared, deallocator)`. The deallocator block captures `Arc<MappedRegion>`. A second owner is attached with `objc_setAssociatedObject` (`retain_buffer_owner`, `:58-73, :380`), because validation wrappers can release the callback before the buffer dies. This guards against the SIGBUS seen under parallel first-touch (commit `1b139bd36`).
- **Private and staged inputs:**
  - `private_from_slice` and `private_capacity` create `StorageModePrivate` buffers rounded to a page (`:425-505`).
  - `PrivateInputUpload` uses **one 8 MiB shared staging buffer** (`METAL_UPLOAD_CHUNK_BYTES`, `:27`) and batches blit copies, waiting for completion on each flush (`:244-336`).
  - `zeroed` uses a Metal-allocated shared buffer plus `write_bytes` (`:508-537`).
- **Serial execution:** `METAL_EXECUTION_LOCK` makes Metal execution process-serial (`:562-578`). Pipelines and libraries are cached per thread by SHA-256 of the source (`:580-581`).

**Production resident input** (`metal_register_execution::upload`, `T:…/metal_register_execution.rs:32-163`):
- One `StorageModePrivate` buffer holds a 40×u32 header of offsets followed by about 25 arrays. Each array starts on a **16-byte boundary** (`:129-137`). The arrays are nodes, files, roots, selectors, spans, blocks, program words and longs, derived relations (first and last named child), fields, supertypes, children, source, strings, `line_starts`, assertions, the child indexes and named order.
- Upload goes through the 8 MiB staging blit, with odd tails zero-padded to 4 bytes (`:146-162`).
- `ResidentInput` is "one immutable input for the complete evaluation, independent of publication batches" (`metal_grid_plan.rs:174-195`). It is still **built and uploaded per call**: `execute_shared_corpus_pages` → `build_streamed_with_upload` → `executor()` (`metal_program_backend.rs:78-94`). Nothing is retained across evaluations.

**Pinning evidence** (all from Tempo docs):
- A Metal System Trace showed a 213.3 ms GPU queue delay against **6.1 ms of GPU execution**, with about 7.77 GB of mutable buffers mapped (`T:docs/packed-result-throughput-design.md`, commit `dfa1897a1`; `shared-sparse-bump-arena-design.md:14-35`).
- A reused-buffer dispatch cut the host wait from 230.5 to 3.8 ms. Later, with the retained workspace, it went from 110.6 to 2.4 ms (`:29-35, :260`). Conclusion: first-use mapping/pinning dominates.
- Rejected approaches:
  - tail-page `MADV_DONTNEED`: no RSS gain, wall time 84.94 s ("Metal treated later page use as new resource mapping");
  - periodic workspace eviction (8-dispatch generation): 74.83 s and doubled budget failures;
  - 64-Program batches: 224 s (`T:docs/packed-result-bottleneck-profile.md:93-104`).
- The retained workspace reached **3.19 GB RSS**, because sparse task ranges fault in new pages, and the workspace-state report missed this Metal residency entirely (`:82-90`).
- Design positions (`shared-sparse-bump-arena-design.md:176-221`):
  - never `MADV_DONTNEED` a retained workspace;
  - `MTLResidencySet` is optional, only after a direct measurement (the `metal` crate lacks it);
  - `mlock` is **not** the method, because it locks CPU pages but does not keep a GPU mapping;
  - a daemon warm-up command pre-pins common classes outside production batches;
  - "the JIT cache retains immutable Program buffers… A tape cache retains node, child, supertype, line, and source buffers by tape identity" (`:215-219`). This is listed as **remaining** step 6 (`:275`).
- Later designs (`T:docs/metal-production-grid-cutover-design.md:451-476`; `metal-optimal-execution-shape.md:273-290`) say: upload the corpus once, do not rebuild per publication batch, use private storage for large immutable buffers with bounded staging, report private bytes separately from RSS, and let incremental runs reuse resident Program and corpus buffers.
- **No measurement of resident-input upload cost or of private vs shared storage was found.**

## A.6 Invalidation on edit

Invalidation follows from content addressing:
- A changed source gets a new SHA-256, which makes it a new `source_objects` row and a new tape.
- Tape identity is `(source, grammar_id, grammar_version, tape_format_version)` (`content_store.rs:90-102`). Restore rejects a grammar, length or digest mismatch (`tape.rs:1153-1159`).
- Changed source or language **fully invalidates** the file state. Old trees are never used for range reuse (`phase-b…:11`).
- Results are reused by canonical source hash plus catalog identity (`journal:683-700`).
- Cache entries are keyed by row id, so a re-inserted object gets a new key.

## A.7 SQL access in Tempo

- Tapes are opaque zstd blobs, never read by SQL.
- Query-independent contentless FTS5 indexes exist: the kind stream (position = preorder ordinal, `detail=full`), values, and source words (`content_store.rs:359-386`). The tape-delete trigger cleans them up.
- `tempo_matches` is a vtab where `xFilter` takes a batch token and runs all (file, program) pairs. Inputs are in-memory `TapeDoc`s, not SQL columns (`T:…/transducer/sql_backend.rs:1-5`).

---

# Part B. Main today (`M:`)

Same lineage as Tempo, one generation older on the Metal side. This is from a delegated survey; the citations are the survey's.
- **Tape format:** v7 (`TMPTAPE7`, `code_rule_dsl/tape.rs:26-27`).
- **Storage:** zstd-3 `frame BLOB` in `source_objects` and `tape_objects`, same schema shape (`content_store/schema.rs:60-86`), in `content-v1.sqlite`. `object_pins` lease table (`:167-175`). SQLite `cache_size=-1024`, about 1 MiB (`connection.rs:320`).
- **Read cache:** `DecompressedReadCache` keyed by **content hash** (`CacheKey::{Source(HashKey),Tape(HashKey)}`, `read_cache.rs:13-17`), 32 MiB, 30 s TTL, one 64 MiB mmap arena with 8-byte slots, compaction and `MADV_DONTNEED` (`read_cache.rs:41-117, 312-360`).
- **Document cache:** `DocumentCache` of decoded documents, 64 MiB, LRU, keyed by `FileVersion{path, source, context, grammar, grammar_version}` (`workspace/documents.rs:21-27, 61-157`).
- **Load chain, about three copies:** zstd → `Vec` → arena → `TapeImage::from_bytes` (`.to_vec()` per column, `tape.rs:1039-1152`) → `restore_tree` into `Arc<Vec<Rec>>` array-of-structs.
- **Metal:** `MappedRegion` page-aligned mmap plus `new_buffer_with_bytes_no_copy` in shared storage (`code_rule_dsl/transducer/metal_backend.rs:74-149`). Kinds, spans and source are **rebuilt from each `TapeDoc` per batch** (`:340-357, :538-545`). No persistent GPU copy and no `mlock`.
- **SQL over tapes:** the production store never reads frames.
  - The "captured-SQL laboratory" (`code_rule_dsl/matcher/anchors/sql/schema.sql:1-40`) stores the tape **uncompressed** as `file(src, rec, spans, checkpoints, parb, sizb, flags BLOB)`. It reads them through per-node scalar functions (`nera_decode`, `nera_span`, `nera_parent`, … at `functions.rs:22-183`) using incremental blob handles. Spec §9 forbids per-node callbacks in production.
  - The relational vtab and `source_text` read the in-memory snapshot.

---

# Part C. What to adopt, and what not to

**Adopt:**
1. zstd-3 independent frames at rest, with raw length and canonical hash outside the frame. Tempo's L1/L3/L5/L7 curve supports L3.
2. Content-addressed identity `(source, grammar, grammar_version, tape_format_version)`. Use main's **hash-keyed** cache keys, not Tempo's row ids.
3. **One large anonymous mapping per slab** instead of one mapping per object (Tempo's VM-metadata finding), with two-generation compaction and refcount retirement.
4. `MADV_DONTNEED` + `munmap` **only at final release**, never on memory Metal still maps.
5. The no-copy `MTLBuffer` lifetime pattern: deallocator owns the mapping, plus associated-object retention.
6. A reserve-before-allocate ledger with page rounding.
7. **Keep `MTLBuffer` objects alive across dispatches to avoid repinning** (measured 30–60×). Reject `mlock`. Treat `MTLResidencySet` as a measured option. Do pre-pin warm-up outside production commands.
8. Bulk object fetch before workers start. Garbage-collection liveness from relationships plus lease pins. One store and one cache per process, shared across workspaces.

**Do not adopt:**
- the 44-byte `MetalNode` repack and per-batch source concatenation and line rescans;
- the per-evaluation private-buffer upload;
- SHA-256 of the source on every restore;
- a 32 MiB, 30 s cache (0 hits);
- heap-`Vec` decompression (3.3 GiB high-water);
- `get_source` copying on hit;
- a fixed 2 GB limit (already removed by spec-tooling §2.1);
- retained mutable workspaces that grow RSS without accounting.

**Footprint arithmetic** (my calculation from `S:research/tape-v8-measurements.md` B/node figures):

| Corpus | Tape (v9np) | Source | Total resident | Derived `par` (u32) if cached |
|---|---|---|---|---|
| CPython (7.90M nodes) | 66 MB | 33 MB | ≈ 99 MB | +32 MB |
| data-layer `impl/` (11.62M nodes) | 99 MB | 58 MB | ≈ 158 MB | +46 MB |

- **The whole gate corpus can stay resident.** Tempo's 32 MiB cache held about a fifth of it, which explains its zero hits.
- **Cold loading from the store costs too much** for the 1–2 s audit target: about 59–77 µs per file (Tempo p50) × 5,725 files ≈ 0.34–0.44 s single-threaded.
- **Pinning tapes is cheap if done once.** Scaling Tempo's 213 ms first-use delay over 7.77 GB down to 158 MB gives roughly 4 ms. This is an unmeasured extrapolation. The cost matters only if it is paid every evaluation, which Tempo's production path currently does.

---

# Part D. Conflicts with the current spec (v9np and related)

1. **§4 "Execution batches concatenate documents' columns"** contradicts zero-copy residency, because concatenating is a copy. Replace it with batch document tables that reference resident slices (§20.5 below).
2. **§3 "JIT/Metal: columns upload as they are."** On UMA with shared no-copy there is no upload at all. Tempo's production path (private plus blit) is an upload; §20 makes the choice a measurement (Q-R1).
3. **§3 per-document width switch** (`size` is u16 or u32) conflicts with one concatenated column per batch. Without concatenation it becomes a per-document descriptor. Batches should be width-homogeneous (§20.5).
4. **§3 "`par` derived" and spec-transducer §8 "`par`, `cidx` and line columns derived per batch."** For resident documents, per-batch derivation repeats work (4.1–5.1 ns/node). These should become cached resident sidecars keyed by tape digest.
5. **§3 "line tables recomputed"** stays, but once per resident source, not per batch. Tempo paid this per batch and per witness.
6. **spec-tooling §2.1 `tape.columns` pool** ("per batch, host and device; upload refuses the batch") assumes per-batch tape memory. Replace it with process-scope `tape.resident` and `source.resident` pools plus a small per-batch `batch.doctable`.
7. **§13 pure-SQL fallback "TVF that reads the tape"** cannot read zstd frames. It needs the zero-copy column TVF (§20.9). Storing uncompressed columns in SQLite would duplicate them in SQLite's page cache.
8. **§14 per-edit** says nothing about residency. "While typing" needs the edited document's tape written straight into pinned memory (§20.8).
9. **§18 caps** (`device_bytes` as 50% of `recommendedMaxWorkingSetSize`, `host_bytes`) would double-count shared no-copy memory unless the spec defines how it is counted (§20.10).
10. **Tempo's GPU ABI cannot be reused under v9np.** `MetalNode`, `ChildIndex`, named order and the relations array are deleted. Kernels read `kf`, `size`, `base`, `off`, `tail` plus the derived sidecars.

`S:spec-transducer.md:551` already points to "`spec.md` §20 (tape residency)", which does not exist yet. The draft below fills it.

---

# Part E. Proposed normative text: `spec.md` §20 Tape residency and caching

> ## 20. Tape residency and caching
>
> Tempo evidence: first-use Metal mapping cost 213 ms against 6.1 ms of GPU time, and reused buffers cut the wait 30–60× (`shared-sparse-bump-arena-design.md:14-35,260`). A 32 MiB / 30 s decompressed cache got 0 hits on full scans and spent 37.6 worker-seconds decompressing in a warm run. Heap decompression left 3.3 GiB of malloc high-water (`workspace-store-runtime-journal.md:536-681`). Per-object mappings were rejected for VM metadata cost (`:517-520`).
>
> **20.1 Forms.** A document's tape and source exist in exactly these forms:
>
> | Form | Where | Encoding | Owner |
> |---|---|---|---|
> | at rest | `tape_objects.frame` / `source_objects.frame` | one zstd frame each (level 3), `raw_length` and canonical SHA-256 outside the frame | content store |
> | resident | a residency slab (20.2) | the decompressed v9np blob, byte-identical to the frame's content; UTF-8 source bytes | residency manager |
> | device view | the slab's `MTLBuffer` | the same pages (20.4) | residency manager |
> | sidecars | a residency slab | `line_starts` u32, derived `par`, `cidx`, text-hash and side-fact columns | residency manager, keyed by tape or source digest |
>
> No other copy of tape or source bytes may exist during evaluation. Executors, the native matcher used for rehydration, `source_text`, and the vtab read the resident form in place. `TapeDoc::restore` (array-of-structs) is permitted only for the native matcher and parity oracles, and never feeds an executor.
>
> **20.2 Slabs.**
> 1. Resident bytes live in **slabs**: anonymous `mmap(PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANON)` regions whose size is a multiple of the OS page size (`sysconf(_SC_PAGESIZE)`, which is 16 KiB on Apple Silicon). The default slab is 64 MiB, a configuration value. An object larger than a slab gets its own page-rounded mapping. Per-object mappings are forbidden otherwise.
> 2. Objects are bump-allocated in a slab at 16-byte alignment.
> 3. The at-rest v9np blob is laid out so that every column starts at a 16-byte offset from the blob start. One zstd decompression **directly into the slab** (known `raw_length`) therefore produces aligned, ready columns. Decompressing into an intermediate heap buffer is forbidden.
> 4. A slab is append-only until it is retired. Compaction copies live objects into a **new** slab, publishes new handles, and retires the old slab when its lease count reaches zero. At most two generations of a slab exist at once. If a third would be needed, the load waits.
> 5. A slab is released with `MADV_DONTNEED` then `munmap`, only after its `MTLBuffer` is gone. Partial `MADV_DONTNEED` on a live slab is forbidden, because Metal treats re-touched pages as new mappings (Tempo: 84.94 s, `packed-result-bottleneck-profile.md:102`).
> 6. Transparent huge pages, superpages and `mlock`/wired memory are not used. `mlock` does not keep a GPU mapping.
>
> **20.3 Identity, admission and validation.**
> 1. A resident object is keyed by its content identity: source by `(hash algorithm, canonical digest)`; tape by `(source digest, grammar_id, grammar_version, tape_format_version)`. Row ids are never cache keys.
> 2. **Admission** decompresses the object into a slab, checks codec, `raw_length` and the SHA-256 of the decompressed bytes, and checks that the tape's node count and width descriptor match `search_index_sets`. This validation runs **once per admission**. Restores, batches and evaluations trust the handle and must not re-hash source.
> 3. A failed validation removes the store row from use, increments `residency_corrupt`, and the document is re-parsed. It never becomes a clean negative.
> 4. Loads are bulk: the planner asks for the demanded document set, the residency manager issues one batched store read for the misses, and workers decompress outside any SQLite transaction.
>
> **20.4 Pinning and device views.**
> 1. On Apple Silicon each slab has exactly one `MTLBuffer` created with `newBufferWithBytesNoCopy(…, StorageModeShared, deallocator)`. The deallocator and an associated object both own the slab, so the mapping outlives every command buffer that references it.
> 2. A **pinned** slab keeps its `MTLBuffer` alive across dispatches and evaluations. Pinning means retention. Buffers over resident objects are never created per batch or per evaluation.
> 3. At daemon start, and after compaction, the residency manager may submit one warm-up command that touches each pinned slab, outside any production command. Its time is reported as `residency.pin`.
> 4. `MTLResidencySet` (macOS 15+) may be added only after the differential measurement Q-R2. CPU-to-GPU blits of resident objects are forbidden unless Q-R1 shows private storage wins. In that case the private copy is made once per admission, never per evaluation.
> 5. Executors bind slabs, not documents. A batch references at most the configured number of slabs. If it needs more, it uses an argument buffer of slab GPU addresses (subject to Q-R3).
>
> **20.5 Batches reference, never copy.**
> 1. A batch is a **batch document table** of fixed records `(slab, tape_offset, node_count, width, source_offset, source_len, line_offset, derived_offsets…)`, plus demand. Executors compute document-local indices: `start(i) = base[i/16] + off[i]` uses the document-local `i`. No tape column, source byte or line start is concatenated or copied into a batch.
> 2. Batches are **width-homogeneous**. The planner groups documents whose `size` width is u32 (24 of 6,251 in the spike) into their own batches, and kernels are specialised by width with function constants. Mixed-width batches are forbidden.
> 3. CPU executors read the same slabs through the same table.
>
> **20.6 Source text and sidecars.**
> 1. A document's source is resident whenever its tape is resident and any admitted program, rehydration or SQL function can read its text. On UMA this costs no extra device memory, because the pages are shared.
> 2. `line_starts` is computed once per resident source digest, in the same pass as admission or lazily on first demand. It is never recomputed per batch or per finding.
> 3. Derived columns (`par`, `cidx`, text hashes) are sidecars keyed by tape digest, built once on first demand (on CPU or GPU), and evictable before their tape. They are charged to `tape.derived`.
> 4. Derivation sidecars (the Blade view text, segment map and host errors) follow the same rules, keyed by (template digest, via).
>
> **20.7 Cache policy.** Residency classes:
>
> | Class | Members | Eviction |
> |---|---|---|
> | `snapshot` | documents of active workspace snapshots in scope of any admitted rule | not evicted while Σ ≤ `tape.resident` limit; these slabs are pinned |
> | `edit` | the newest overlay version per open path | pinned; superseded versions are freed when their leases reach zero |
> | `transient` | everything else (other snapshots, historical versions, oversized one-offs) | LRU by last lease release; no fixed TTL |
>
> 1. Sharing is by content identity across all workspaces in the process. Cross-process sharing goes through the store (and Q-R5).
> 2. If the `snapshot` class exceeds its limit, the planner runs the audit in **residency waves**. Each wave loads, pins, executes and releases. Waves are reported as `residency_waves`, and misses as `residency_miss_bytes`. Being non-resident is never an `Unknown`. A single document larger than the device budget is `Unknown(resource: doc_exceeds_device)` (§8).
> 3. Residency state may be exposed to the planner as a cost input (load bytes). It is never a correctness input.
>
> **20.8 Per-edit residency.**
> 1. The edited buffer is parsed, and its v9np tape is encoded **directly into an `edit` slab**. The node count is taken from the parse before columns are written, so there is no intermediate tape buffer. Its source bytes and `line_starts` go into the same slab.
> 2. The edit slab is already pinned, so per-edit execution has zero upload and zero first-use mapping.
> 3. Compressing and writing the store row happen off the evaluation path. §14's synchronous index publication is unchanged.
> 4. The previous version keeps its handle until its last lease is released. Edit-slab compaction never runs on the edit path.
>
> **20.9 SQL access to tapes.**
> 1. The transducer vtab (§9) resolves demanded documents to residency handles before `xFilter` dispatch. It never `SELECT`s frames.
> 2. The pure-SQL binding (§13) and the ast-index-spike techniques read columns through a TVF `tape_column(doc, column)` that returns a blob **pointing into the slab** (`sqlite3_result_blob64` with a no-copy destructor). A statement-scoped residency lease, held by the cursor or function auxiliary data, keeps the bytes valid for the statement's lifetime. The same applies to `source_text`.
> 3. Uncompressed tape columns are never stored in content-store rows. The captured-SQL laboratory schema is test-only.
> 4. The FTS5 kind, term and edge indexes stay in SQLite as specified in §5. They are built from the resident columns at publication.
>
> **20.10 Accounting and outcomes.**
> 1. Pools: `tape.resident`, `source.resident`, `tape.derived` (process scope) and `batch.doctable` (per batch). They replace `tape.columns` in `spec-tooling.md` §2.1.
> 2. Limits are sized from Σ `node_count` × v9np widths and `raw_length`, taken from the store and `search_index_sets`.
> 3. Shared no-copy slabs are counted **once** against both `host_bytes` and `device_bytes`. Private copies (if Q-R1 adopts them) are counted on the device only.
> 4. Every slab reserves its page-rounded size in the ledger before `mmap`.
> 5. The profile records slab count, resident, pinned and derived bytes, admissions, decompressed bytes, hits and misses per class, compactions, `residency.pin` time and `residency_waves`.
> 6. The always-on Metal counters add `bytes_bound` per command and `slabs_bound`.
>
> **20.11 Forbidden.** Per-batch repacking into node records, such as Tempo's 44-byte `MetalNode`. Per-batch source concatenation or line rescans. Heap-`Vec` decompression. Copying source on cache hit. Re-hashing on restore. Per-object `mmap`. TTL-only caches smaller than the working set. Partial `MADV_DONTNEED` on mapped slabs. `mlock` as pinning. Fixed byte caps in code.
>
> **20.12 Open measurements before locking.**
> - **Q-R1:** shared no-copy vs a private copy made once per admission, measured on GPU time of the stratified kernels over the gate corpora.
> - **Q-R2:** `MTLResidencySet` vs retained buffers alone, measured on first-dispatch latency after idle and under memory pressure.
> - **Q-R3:** slab count per batch vs argument buffers (`gpuAddress`); the 31-buffer binding limit.
> - **Q-R4:** slab size (16/64/256 MiB) vs VM metadata and compaction cost.
> - **Q-R5:** a file-backed, read-only shared mapping of a snapshot image for multi-daemon sharing, and whether GPU no-copy over file-backed pages is safe.
> - **Q-R6:** whether no-copy buffers count in `currentAllocatedSize`.
> - **Q-R7:** pin cost per GB of tape and source slabs, to check the about-27 µs/MB extrapolation from Tempo's scratch measurement.
> - **Q-R8:** cold admission throughput (zstd into slab) at 4/8/16 workers against the 1–2 s audit target.

**Consequential edits** (each is a short change):
- **§3 benefits bullet:** "columns are bound in place (§20); the 44-byte `MetalNode` restore is deleted."
- **§3 layout at rest:** add "every column starts at a 16-byte offset."
- **§4 last bullet:** "Execution batches reference resident documents through a batch document table (§20.5)."
- **§8:** add "Executors bind residency slabs (§20.4)."
- **§14:** add "the edited document is encoded into a pinned edit slab (§20.8)."
- **spec-tooling §2.1:** swap the pools per §20.10.
- **spec-transducer §8:** change "derived per batch" to "resident sidecars (§20.6)."

The cache policy and the slab/pin mechanics are the parts with evidence behind them. Q-R1 (whether to keep Tempo's private storage) is the one Tempo chose without measuring, so it should be measured before the spec takes a position.