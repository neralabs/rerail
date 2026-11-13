# T2 notes: PIAT research core (tape, residency, facts, transducer, Metal, regex)

Root: `Pulling-it-all-together/docs/rule-engine-direction/`. Sections are appended as each source is read in full. Marks: M = measured, D = derived, A = assumed, as the source states.

---

## research/tape-regularity.md (291 lines, read in full)

### History (§0-§1)
- The record encoding has been unchanged from v3 to v7: the spike's rec6 `[kf:2][par:2][size:2]` with `0xFFFF` → sorted overflow tables, varint spans `(zigzag Δstart, len)`, and a 6 B checkpoint every 16 nodes (:12-13). `tape/index.rs` says so itself.
- v4–v6 were irregular because they held **multiple grammars or trees in one tape** (:14-18):
  - v4: Blade plus offset-shifted PHP;
  - v5: grafted programs;
  - v6: nested guest images.
- v7 regularised the *document* (one source, one grammar, one tree), but not the *record* (:19-25): varint spans, sentinel escapes, flag side lists, ERROR = code 0 plus a list, and aliases as a per-file string table.
- Magic collision (:26): Tempo's `TMPTAPE4` ≠ Rerail's `TMPTAPE4`; Tempo's Metal is still on Tempo v4.
- Consequences of the irregularity (:27-29):
  - DLRE rebuilds row-per-node `decoded_*` caches, which is the spike's rejected S089 sidecar (211 B/node on SQLite, 88% of the store);
  - **N1562 expands each node to a 44 B `MetalNode`**;
  - Tempo's full-corpus run spends about 2 GB on expansion (:85).
- Restored in-memory `TapeDoc.Rec` is ~40 B/node and regular; every consumer pays an O(n) restore (:62).
- Three representations at the time (:64-73): the content-store `TMPTAPE7` (zstd whole blob, not randomly accessible); DLRE's anchor-corpus copy (spike-style columns, a regular 1-byte `flags`, `kmask` 128 B, demand-filled `decoded_*`); executor ABIs (`Rec`, `MetalNode` 44 B AoS + child array + `MetalFile{node_start, source_start, line_start}`).

### Multiple trees per file (§3)
- v7 model (:104-112):
  - primary plus derived documents, each with its own text and single-grammar tape;
  - a `SegmentMap` to disjoint parent ranges; `host_errors`;
  - content-addressed source + tape rows linked by `file_documents(role, via, parent, source map, host errors)`;
  - half-open zero-width ownership; embedders run on the primary only.
- Spike S070/S071 (:114-130):
  - segments are standalone tapes, with sparse `t_seg`;
  - splicing was rejected (it forces a global kind space and cross-language par/size arithmetic);
  - +11% storage (M);
  - Option B, global codes `lang·1024+local` in the anchor index only: FTS instances 320,767 → 180,005, latency −5 to −22% (M).
- **Gap on main (:135-137): `search_kind_fts` tokens are `k%04x` of the local code**, so postings and `fts5vocab` statistics mix languages. DLRE uses `LANG_STRIDE = 1024`. (Check in T7 whether main has changed since.)
- Proposed regular multi-document form (:141-149):
  - a per-file fixed-width document table `doc, parent_doc, role, via, lang, node_count, text_len, map_off, map_count, width_flags`;
  - map triples `(doc_start, parent_start, len)`, binary search, off the hot path because locations are normalised once per finding in consolidation;
  - host_errors as pairs.
  - **":147 For execution batches: concatenate the columns of all documents in the batch with a batch document table (node_base, text_base, line_base, lang)".**
  - **CONTRADICTION:** this is superseded by spec §20 (no copy or concatenate; a batch document table points into slabs). tape-regularity predates §20. The final shape follows §20.
  - Cross-document relations use the document table and the map, never par/size arithmetic across documents (:149).

### Spike experiments E1–E35 (§4), the ones that bear on the final shape
- E7 (:163): pure-SQL decode needs an uncompressed tape (2.46× source; 43× slower than a TVF) → **compression at rest only; the SQL-addressable form is uncompressed** (M).
- E12 (:168): fixed 7 B blob 30× faster than TEXT substr; **global sibling ordinals in SQL window functions are 75× worse**, so compute them in the executor (M).
- E13 (:169): paying fixed bytes for random access paid 5–6× on tail queries (M).
- E14 (:170): rec6 overflow happens only when node_count > 65,536 (par-ovf 0.31%), so a per-document width removes the escapes (D).
- E15 (:171): store anonymous nodes (named-only loses 210 shapes) (M).
- E16 (:172): **no storage reason to prefer SoA or interleaved; compression vs access dominates**; compressed random slices 9.5–17.3 s vs EXTERNAL 202 ms (M).
- E17 (:173): decompress once per file per batch; keep per-column access so a batch reads only the columns it needs (M/D).
- E18 (:174): checkpoint stride sweep, K=128 6.34 s → K=16 2.80 s → K=8 2.58 s; the trend is toward fixed-width spans (M).
- E20 (:176): line checkpoints every 4 KB of source, 0.004 B/node; a dense per-line vector was rejected (D15) (M).
- E21/E23 (:177,179): intern at grammar registration; `code_ctx` spare codes carry aliases regularly.
- E22 (:178): Option B is the anchor-index global code.
- E26 (:182): `extra` is a per-kind property (uniform over 18.5M nodes) (M).
- **E28 (:184): sidecars `t_nidx/t_fidx/t_vpos` were 88–93% of the store and bought only latency; deleted (D14), enforced by a test (M).** The disk-budget lesson.
- E29 (:185): chunking into a side table was rejected; one row or object per document.
- E30 (:186): per-touch PostgreSQL cost 110 ns = 51 row machinery + 21 slicing + 38 kind decode. In SQL encoding isn't the main lever, but **in JIT/GPU executors decode becomes the main cost** (M/D).
- E31 (:187): native TVF boundary: pass an address or handle, not the value (153 ns/call, ~120 ns of it dispatch); `sqlite3_blob_reopen` is nearly free. Fixed stride lets a vtab return runs of rows per call (M).
- E33 (:189): **postings may stay delta/varint; regularity is needed for per-node columns, not postings** (M: absolute kidx 2.6 → 4.2 B/node for a 10 ms gain, reverted D10).
- E34 (:190): spike logical 8.66 B/node (6 rec + ~2.3 spans + ~0.4), missing the 6 B budget (M).
- E35 (:191): an edge index buys nothing; aggregate `efreq` at 0.039 B/node (D21) (M).

### The v8 candidate (§5), later rejected by the measurements below
- Columns `kf, par, size, start, len, flags`, with per-document u16/u32 widths, 11 B/node normal and 19 large (D). Optional `lines`, `ordinal`, `next_named` (:211-229).
- It proposed aliases and ERROR as registration-time codes (:236-238), a distinct magic, and re-encoding from stored source (:272).
- Open measurements (:285-291) were answered by tape-v8-measurements.

---

## research/tape-v8-measurements.md (215 lines) + experiments/tape-v8/results/tables.md (67), read in full

Corpora (:3): CPython 3.13 `Lib/`, nera-data-layer `impl/`, rerail TS, Rust and PHP fixtures, **24.2M nodes**. Grammars are from crates.io, not Rerail's patched ones (caveat :196).

### Recommendation (:5-20)
1. **Do not adopt v8.** It is 11.1–13.5 B/node logical, **1.24–1.59× v7**, and **1.47–1.90× v7 after zstd** (M).
   - The cause is large files: 124 of 1,702 CPython files ≥ 64 KiB hold 47% of the nodes and force u32 absolute start/len.
   - Absolute positions compress badly; delta varints compress well.
2. **Adopt v9:**
   - `start(i) = base[i/16] + off[i]` (u32 base per 16 nodes, u16 off);
   - `end(i) = start(i+size(i)-1) + tail[i]`;
   - `kf` u16 with **flags folded into the kind code space** (named/extra/missing/error/aliases as distinct codes);
   - `size` u16/u32 per document; parent optional.
   - No document out of 24M nodes needed u32 off/tail; only size/par switch width (24 of 6,251 documents).
3. **v9np (`kf, size, base, off, tail`) = 8.25–8.55 B/node logical, 0.93–1.01× v7; zstd 0.94–1.06× v7 (M).** With parent: 10.25–10.84 (1.15–1.28×).
4. **Recommended at rest: v9np SoA, with the parent derived when needed** at 4.1–5.1 ns/node (stack), about the cost of v7's restore (3.7–7.8 ns/node), paid only where parents are used. Keep a stored parent only if SQL climbs prove hot: in SQL a climb is 1.6–3× faster than interval containment (M).
5. The benefits are random access and spans, not scans (M):
   - random (kind, parent, size, span, named): v7 113–490 ns, v9 23–199, v8 10–96;
   - random span: v7 33–364, v9 13–57, v8 3.5–13;
   - sequential: v7 3.8–4.3 ns/node, v8 2.9–3.8, **v9 8.8–24** when each end is recomputed. A stack-carried end should match v8 (not measured, :200);
   - JIT/Metal: columns upload as-is, with no 44 B `MetalNode` restore.

### Size tables (tables.md; M)
- Logical B/node, v7 / v9np:
  - CPython 8.49 / 8.38;
  - data-layer 8.47 / 8.55;
  - TS 8.47 / 8.32;
  - Rust 8.47 / 8.25;
  - PHP 8.93 / 8.31.
- zstd-3 B/node, v7 / v9np / v9 / v8:
  - CPython 1.74 / 1.77 / 2.15 / 3.29;
  - data-layer 1.85 / 1.97 / 2.36 / 3.48;
  - TS 2.18 / 2.05;
  - Rust 2.07 / 2.01;
  - PHP 2.69 / 2.53.
- Source: 3.26–5.03 B/node raw, **0.96–1.23 B/node zstd**. At rest, source and tape are of comparable size: about 1 + 1.8–2 B/node.
- Interleaved (v8i, v9i) compresses **6–21% worse** than columns, so columns are the at-rest layout (:144).
- Folded codes: 194–277 per grammar of 1,024; 0 aliases in the crates.io grammars; extra is non-uniform for only 1–2 nodes per corpus (:145, tables).
- `tail` is needed: end ≠ end(last descendant) for 0.09–0.63% of internal nodes; start ≠ start(first child) up to 0.73%, so start is not derived (:146).
- Encode 6–77 ns/node vs parse 904–1,671 ns/node: **encode is 12–260× cheaper than parse** (:184).

### Content audit (Part 1, :22-43)
- Structure (a) vs associated (b):
  - aliases are a grammar property duplicated per file;
  - the header digest is the store's key and needn't be in the tape;
  - derived-document maps are already in `file_documents` (`schema.rs:92`);
  - the Blade compiled view's omitted nodes are "b leaking into a".
- The minimal structure-only tape (:39-43) is exactly the parser's tree: kf, size, base/off/tail, optional parent, width byte, node count. Everything else goes to sidecars keyed by the tape's content digest, or is recomputed.

### Blade (Part 1b, :45-57)
- Today:
  - the primary is Forte's tree plus **synthetic zero-width ERROR leaves**;
  - the compiled view is a tree-sitter-php tape **with nodes over synthetic placeholders removed** ("a directive call has no `function` child").
- Proposal:
  - the compiled-view tape is the plain tree-sitter-php tape, with omission applied by the matcher through the segment-map flags (bit 2);
  - view text, map and host errors are a derivation sidecar keyed by (template digest, via);
  - Forte lexer errors go to a sidecar, not ERROR leaves.
- Audited from code, not measured.

### SQL (Part 4, :149-177): 387 CPython files, pure-SQL `instr(substr())`, load avg 50–65, min of 3 runs (M)
- Spans: v7 6.2–9.5 s vs v8 1.1–1.2 s vs v9 2.2–2.5 s vs v9i 1.7 s.
- Structure queries are dominated by **which blob SQLite reloads**:
  - interleaved is ~2× faster when the anchor row loads once (`has`);
  - SoA is 3–4× faster for recursive CTEs (`nthChild`) because only the 2 B/node blobs reload.
- A parent climb is 1.6–3× faster than interval containment; dropping the parent costs SQL `inside` about 2×.
- v7 = v9 for structure in SQL; spans are 6–9× costlier in v7.
- Store `kf` and `size` as their own blob columns, or interleave them for single-pass anchor scans; measure on the real workload (:191).
- Global FTS anchor codes `lang*1024 + folded code` follow from the folded code space (:192).

### Answer to "why the expansion in data layout" (Q2)
- The final format is v9np: **≈ v7 on disk** (0.94–1.06× zstd) and in memory (0.93–1.01× logical). There is **no expansion**.
- v8, the expansion candidate, was measured and rejected at 1.24–1.90×.
- The parent column (+2 B/node) is **derived on load**, only where used. It is not stored.
- Executors and SQL read the uncompressed SoA columns directly. Tempo's 44 B/node `MetalNode` / `TapeDoc` ~40 B/node expansion is exactly what v9np is designed to eliminate.
- **My withdrawn 01-design's 16 B/node batch arena re-introduced the expansion v9np exists to kill.**

---

## research/tempo-residency.md (316 lines, read in full)

Normative §20 (Part E, :215-306) is already in t1. These notes cover the evidence and the arithmetic. T: = Tempo `d737fad16`.

### What Tempo really has (Part A)
- **At rest** (:27-43):
  - `source_objects` / `tape_objects`, one zstd-3 frame each, with raw_length and SHA-256 outside the frame;
  - tape identity is UNIQUE(source, grammar_id, grammar_version, tape_format_version);
  - `object_pins` lease rows for GC;
  - `SCHEMA_VERSION=22`.
- Compression evidence (M):
  - 5,725 files, 95.7 MB source + 160.38 MB raw v4 tapes → 56.08 MB (21.9%);
  - zstd L1 58.26 MB in 466 ms; **L3 56.08 MB in 590 ms**; L5 53.58 in 1,269 ms; L7 51.59 in 2,113 ms;
  - independent frames: per-file p50 59–77 µs, p95 374–497 µs (:35-37).
- Why on disk, not RAM (:39): for 44 workspaces, compressed is 0.85–0.95 GiB; restored is 3.8–4.2 GiB content-addressed, or **13–15 GiB path-addressed** (M). "Content addressing reduces RAM only when processes share mappings or one process owns the object cache."
- Bulk load: one `json_each` query fetches all needed tapes before the workers start; a missing row means "parse again" (:41).
- Validation on **every** read: codec, length, SHA-256 (:42). §20 moves this to once per admission.
- GC: relationships plus leases, bounded batches, tapes before sources (:43).
- **Decompressed cache** (:45-65):
  - key = row id (main uses the content hash);
  - one 64 MiB anon mmap arena, 8 B alignment, 32 MiB limit, 30 s TTL, LRU, two-generation compaction;
  - `get_source` copies on hit.
  - Measured (M):
    - full scan, **0 tape-cache hits**, 5,663 decompressions;
    - warm run: 8,478 decompressions, **37.60 worker-seconds of storage+decompression vs 0.003 s native matching**;
    - 4.15 GB RSS for 46 workspaces, 3.3 GiB of dirty malloc high-water from heap decompression.
  - Lesson: a TTL cache smaller than the working set is useless; heap decompression inflates high-water.
- Per-object mappings were rejected for page-table/VM-zone metadata (:59).
- CPU residency (Phase B, :67-71): source + compact tape resident, tree-sitter tree transient. Initial RSS **−29.98%**, CPU +17.45%, edit latency unchanged (M). Logical retained: 33.8 / 241.0 / 437.0 MB (subset, landing, data-layer).
- Restore on every use (:77-80): `Arc<Vec<Rec>>` AoS; SHA-256 of the source on every restore; `LineIndex` rescans (a Phase 5 request cache cut 1,310,720 → 2,560 scanned bytes).
- **Packing copy** (:82-90): `PackedBatch::build_from_tapes` does, per file per evaluation, restore + append source + rescan lines + a **44 B MetalNode** per node + a global children array, all on heap `Vec`s. The CPU JIT reads raw pointers; scratch lives in `DemandZeroRegion` (mmap, DONTNEED on drop).
- **Metal** (:92-129):
  - a 2 GB atomic ledger with reserve-before-allocate;
  - `MappedRegion` (page-rounded mmap, **copies bytes in**);
  - the no-copy wrap uses a deallocator that owns `Arc<MappedRegion>` plus `objc_setAssociatedObject` (the fix for a SIGBUS under parallel first touch, `1b139bd36`);
  - private buffers through an 8 MiB staging blit;
  - `METAL_EXECUTION_LOCK` (process-serial).
  - **Production `ResidentInput`** is one Private buffer with a 40×u32 header plus ~25 arrays at 16 B alignment. It is **built and uploaded per call, nothing retained** (:109-112).
- **Pinning evidence** (:114-129, M):
  - Metal System Trace: **213.3 ms GPU queue delay vs 6.1 ms GPU execution**, with 7.77 GB of mutable buffers mapped;
  - reused buffers cut host wait 230.5 → 3.8 ms, and later 110.6 → 2.4 ms (**30–60×**);
  - rejected: tail-page DONTNEED (84.94 s, "Metal treated later page use as new resource mapping"); periodic workspace eviction (74.83 s, doubled budget failures); 64-program batches (224 s);
  - the retained workspace reached 3.19 GB RSS from sparse task ranges faulting in, unreported.
  - Design positions: never DONTNEED retained; MTLResidencySet optional after measurement; `mlock` does not keep a GPU mapping; daemon warm-up pre-pin; the tape cache by tape identity is "remaining step 6", **not implemented**.
  - **No measurement of upload cost or of private vs shared was found** (:129).
- Invalidation (:131-138):
  - a changed source means a new SHA-256, a new source row and a new tape;
  - a changed source or language **fully invalidates** the file state, with no range reuse of old trees;
  - results are reused by canonical source hash + catalog identity (journal :683-700).
- SQL (:140-144):
  - tapes are opaque to SQL;
  - contentless FTS5 over the kind stream (position = preorder ordinal, detail=full), values and source words;
  - `tempo_matches` vtab with a batch token in xFilter, inputs from in-memory TapeDocs.

### Main at the time (Part B, :148-159; delegated survey, to verify in T7)
- v7 store with the same schema shape, `content-v1.sqlite`, `cache_size=-1024`.
- `DecompressedReadCache` keyed by content hash, 32 MiB / 30 s.
- `DocumentCache` 64 MiB LRU keyed by `FileVersion{path, source, context, grammar, grammar_version}`.
- Load chain with about three copies: zstd → Vec → arena → `from_bytes` `.to_vec()` per column → `restore_tree` AoS.
- Metal rebuilds kinds, spans and source from each TapeDoc per batch.
- The captured-SQL lab stores uncompressed spike columns and reads them through per-node scalar functions (forbidden in production by §9).

### Adopt / reject (Part C, :163-183)
- Adopt:
  - zstd-3 frames;
  - content identity with hash keys;
  - one mapping per slab;
  - DONTNEED only at final release;
  - the no-copy lifetime pattern;
  - reserve-before-allocate;
  - retain MTLBuffers (30–60×);
  - reject mlock;
  - pre-pin warm-up;
  - bulk fetch;
  - lease GC;
  - one store/cache per process.
- Reject:
  - MetalNode repack, source concatenation, line rescans;
  - per-evaluation upload;
  - re-hash on restore;
  - the 32 MiB/30 s cache;
  - heap decompression;
  - copy on hit;
  - fixed 2 GB;
  - unaccounted retained workspaces.

### Footprint (Part C, :185-194; D from v9np B/node)
| Corpus | v9np tape | Source | Resident | +par u32 |
|---|---|---|---|---|
| CPython 7.90M nodes | 66 MB | 33 MB | ≈ 99 MB | +32 MB |
| data-layer 11.62M | 99 MB | 58 MB | ≈ 158 MB | +46 MB |

- **The whole gate corpus stays resident.** Tempo's 32 MiB held about a fifth of it.
- Cold load from the store ≈ 59–77 µs × 5,725 files ≈ 0.34–0.44 s single-threaded, too much for 1–2 s, hence the pinned snapshot class and Q-R8.
- Pinning scaled from 213 ms/7.77 GB to 158 MB ≈ 4 ms (D, unmeasured extrapolation).

### Spec conflicts it raised (Part D, :198-211), all resolved into §20 + consequential edits
1. §4 "batches concatenate columns" → batch document tables.
2. "upload" → none on UMA.
3. per-document width vs concatenation → width-homogeneous batches.
4. `par`/`cidx`/lines derived per batch → resident sidecars by tape digest.
5. line tables once per resident source.
6. `tape.columns` per-batch pool → `tape.resident`/`source.resident` + `batch.doctable`.
7. pure-SQL fallback can't read zstd → `tape_column` no-copy TVF; never store uncompressed columns in SQLite (that would duplicate them in the page cache).
8. per-edit → edit slab.
9. caps double-count → count shared once.
10. **Tempo's GPU ABI can't be reused under v9np**: MetalNode, ChildIndex, named order and the relations array are deleted; kernels read kf/size/base/off/tail + sidecars.

### Answer to "tempo work on tape caching and pinned-memory uncompressed tapes" (Q3)
- Tempo has:
  - built and measured: page-allocated anon-mmap arenas, no-copy shared MTLBuffers with a deallocator + associated-object lifetime, a reserve-before-allocate ledger, lease pins;
  - measured: that **retaining** buffers (pinning = retention) wins 30–60× and first-use mapping dominates (213 ms vs 6 ms).
- Tempo did **not** implement the tape cache by identity: production rebuilds and uploads a 44 B/node private copy per evaluation.
- The final shape (spec §20):
  - takes the mechanisms and the evidence;
  - decompresses v9np once, directly into pinned 64 MiB slabs (16 B-aligned columns), whole gate corpus ≈ 100–160 MB;
  - executes by reference through a batch document table;
  - encodes edits straight into a pinned edit slab.

---

## research/derived-facts.md (379 lines, read in full)

Normative output: spec §21 + decision 0005 (§21 text is in t1). These notes cover the evidence and inventory.

### Prior designs (a)
- **PR 2539** (nera-data-layer `281f500d7c`):
  - four passes (0 paths, 1 AST primitives, **2 generic derived facts**, 3 YAML policy), with Pass 2 "still belongs to fact materialization" (`LINTER_GENERIC_FRAMEWORK_FACTS_DESIGN.md:93-147`);
  - SQL-only boundary: no linter-shaped collectors, adapters or precomputed candidate tables (:21-29); operators from/join/left_join/recursive/group_by/aggregate/project/union/materialized (:67-71); a relation named after a linter or rule is suspect (:105-106); **projection invalidation by declared dependency edges and the dependency cone** (:136-143);
  - **"Process Failure"** (:169-186): "Python prepares convenient rows, YAML queries them".
  - Dependencies: `DECLARATIVE_RELATION_EXTRACTORS`, `..._PHYSICAL_RELATION_DEPENDENCIES` (hand-declared); demand from the YAML graph folded into `cache_version`; `SqliteProjectFactCache` keyed by contributing fact content; the memo plan says "Filesystem metadata alone is not a validity proof".
  - Base facts are per file, owned by `bundle_key`, keyed by content hash + cache_version, and deleted and rewritten in one transaction. "Persist query results only when measurement shows a query is expensive."
  - Skinny EAV → typed per-family tables: 500 files **47.1 s → 18.0 s** (M).
  - **Tip shortfalls:** catalog relations inlined as CTEs everywhere (never shared); `materialized` only a CTE hint; dependency-cone tables created but never read; `SqliteProjectFactCache` has no callers; `DerivedFactSpec` rejected on the declarative path; `lint_function_transitive_summary` rebuilt in full on every save.
- **Moti facts-engine** (`685db3f7`):
  - derived relations are SQL views in SCHEMA.md (`py_owner`, `py_function`, `py_call`, `cg_def`, …), read with include_str;
  - **memo per clause**, keyed (content hash, plan digest, schema version[, path]), content-pure clauses path-free;
  - plan digest = blake3(engine version, schema, rewrites, rule, inputs, physical plans);
  - invalidation purely by address with ON DELETE CASCADE;
  - warm restart 486 ms, 1.05M clause runs served from the memo (M).
  - DECLARED-FACTS proposal: `facts` blocks (`over: content|membership`, key, fd, define sql|match); identity = digest of the lowered algebra, not the text; motivation: "34.9 s of byte-identical stage text" → 12 declared facts.
- **DLRE**:
  - shared syntax facts c4bd76bf cold CPU **12.62 → 3.19 s (−74.7%)** (M);
  - "adding names to already shared rows alone does not establish a performance improvement";
  - the producer catalog caches nothing across evaluations ("No digest can establish sharing").
- **N1562** `semantic_facts.rs`: per-anchor rows, no cache.
- Conclusion (:76): all agree. The unbuilt pieces everywhere are a persistent per-fact cache by definition digest and dependency-cone invalidation.

### Design specifics beyond §21
- Producer split (B1): fact program (transducer, EXPORT labels into a schema), native producer (resolver/evaluator), derived fact (algebra → SQL). "**Shared does not mean SQL**": tree-walking shared analyses (annotation atoms ~18 rules, receiver chains ~12) are fact programs.
- Why an algebra and not reviewed SQL (B2):
  - the compiler must choose the physical form: wording swings of **22× and 34×** (reachability 0.80 vs 17.8 s; scoped resolution 0.73 vs 24.8 s) (M);
  - prove the partition;
  - derive lineage for invalidation;
  - give the second binding.
- Admission guard (B2 a–d) against Process Failure: concept names, ≥ 2 rules, rows/doc ≤ nodes (L4), no rule seeds (`serverplan_step_class` → `py.subclass_of` + atom policy data).
- Hard rules (B4): one physical owner per (doc/snapshot, digest) per evaluation (lesson 16: **710 CTEs in one statement**, one definition repeated across 61 and 90 statements); narrow typed tables, u32 atoms, no EAV/union; demand-driven closure.
- Vtab form for base facts: real tables measured at **16.4 s for 7.9M rows vs an estimated ~1 s vtab** (M/A; M3 decides).
- Per-edit invalidation steps (B6):
  1. recompute d's base facts (< 2.5 ms typical);
  2. delete+insert d's per-document derived facts;
  3. repository facts: semi-naive Δ on changed join keys; recursive components DRed, or a full recompute past a threshold (full reachability 342 ms, M);
  4. re-run rules whose read set intersects.
  - Target M8: one file through facts + resolution + reachability ≤ 1 s p95.
- Three consumption bindings for transducer programs (B9):
  - **SIDE columns**: a fact keyed (doc, node) with FD node → value, scanned in doc order into a resident u32 column, default NONE;
  - **FACT sets**: posting lists read by IN_SET/COUNT_SET;
  - demand seeds or SQL joins over t_match/t_export.
  - The pipeline per document is stratified: base facts → SQL derived → rule programs → composition. Cross-boundary cycles are rejected.
- Boundary (c):
  - stays in the transducer: kind/field/pattern/text, boolean, all tree relations and stopBy, nthChild, bounded COUNT, non-recursive utilities, **E14 monotone FIX**, TEXT_EQ/SIDE_EQ inside EXISTS when navigable, E16 ORDER, SIDE/FACT reads;
  - moves out:
    - **E15 KEYS/IN_KEYS → L5** (two exporting programs, SQL semi-join on (doc, key)): M7's quadratic case 0.451 s → 3.6 ms with keyed sets is solved equally by an indexed join;
    - KEYS inside FIX → `py.call_edge_local` (85,560 rows) + `reaches`: recursive SQL taint 0.49 s vs native worklist 0.94 s (M); call-graph closure 5 ms;
    - **E17 FOLD_UP → native `py.flow_exit`** SIDE column.
  - Of 48 per-document heavy clauses, ~38 stay purely IR and ~10 read derived facts.
- Inventory (d), gate corpus **9,010 files, 28.23M nodes** (M rows):
  - base facts: py.def 194,281; py.call 993,650 (+482,606 kw args); py.param ~400–500k (A); py.annotation_atom 0.8–1.2M (A); py.receiver_token ~1.9M (A); py.binding_site 411,275; py.import 226,642; py.class_base 22,061; py.decorator 38,934; py.export_name 16,740; py.return 126,609; py.flow_edge 773,510 on one scope; file.module 9,010; py.scope/binding/value/flow_exit as SIDE columns (py.value replaced 325,944 exported rows costing **20.0 s**);
  - derived:
    - py.def_family (doc): 2,421 pairs;
    - py.call_edge_local (doc): 85,560, closure 5 ms;
    - py.module_const (doc): 27,481;
    - py.local_flow (doc): 51,626, 0.49 s;
    - py.import_res (repo): 120,229, 0.43–0.68 s;
    - py.call_edge (repo): 352,111, 0.73–1.08 s (24.8 s in the bad form);
    - py.subclass_closure (repo): 16,561 closure rows, 0.28 s;
    - py.export_closure (repo).
  - Whole-repo reachability is not a fact. `reaches` over call_edge + containment (100,618) with rule seeds gives 174,914 reachable / 9,371 dead, 0.80 s, memoised per plan.
  - Removed:
    - `descendants` (~4.4M–16.7M rows) → interval tests on the tape;
    - nodes/edges/fields rows → vtab only;
    - document gates → L1;
    - first/last-def-wins → L2.
- Load estimate (A): all derived facts < 0.7M rows; per-document derived < 0.1 s cold; repository 1.5–2.5 s cold in the good form; ms per edit through deltas; fact-program rows ~4–5M (the vtab-vs-copy question, M3).
- New measurements (e6):
  - M10: L5 vs keyed sets within 1.5×;
  - M11: cold materialisation of all derived facts ≤ 2.5 s on one core;
  - M8 gains the delta path.

### Bearing on Q4 (fact storage) and Q6 (invalidation)
- Fact storage = narrow typed SQLite tables in the content store for derived facts, plus vtab/native columnar for base facts (decision pending, M3), with u32 atoms. SIDE columns are resident u32 per node (sidecars under §20.6).
- Cache keys use tape identity (document facts) or input generation vectors (repository facts). Invalidation is by address for document facts and by Δ/DRed for repository facts.
- **Open:** where the u32 atom intern table lives and how it is shared across the transducer TEXT_ID and the facts. Check spec-transducer TEXT_ID (t1) and Typing-support (T9).
- 2026-10-25 1–2 s audit target vs the 1.5–2.5 s repository-fact estimate: a tension to record. Typing-support's native engine (1.48–1.67 s program mode vs 14 s SQL) bears on it (T9).

---

## research/transducer-ir.md (308 lines, read in full)

Sources: Tempo `e95613625` docs and code; N1562 `c674f4b6`. Predates spec-transducer (which normalises its recommendations).

### The IR as built in Tempo (§1)
- A verified fixed-width structural automaton: **one 32-bit word** `opcode:u8|mode:u8|operand:u16`, so ≤ 65,536 instructions and table entries; local tables for strings, predicates, labels and u64 constants (:58-59).
- It is a **path-enumerating tree walker** from supplied roots. A path = (pc, node, ordered capture list, repeat counters, cursor slots). Navigation with several results forks; FORK defers; only EMIT outputs. Logical order is depth-first, which parallel backends reproduce by sorting on (task ordinal, local ordinal). Cycles must progress (:41-55).
- Structural ops (:63-82): STOP, KIND, FIELD, FLAGS, TEXT/SIZE, SUPERTYPE, CHILD, NEXT, PARENT, DESCEND, FOLLOWING, SAVE/RESTORE_CURSOR, CAPTURE, REPEAT_*, FORK policy, GOTO, EMIT.
- Side programs (:84-91):
  - `ValueProgram`: byte ops, CALL/RETURN inside value programs only, DFA via `regex-automata`, Lua captures/backrefs/balanced;
  - `CaptureProgram`: #eq?/#match?/#has-ancestor? etc.;
  - `OutputProgram`: #set!/#offset!/#strip!…
- Output: typed arenas `ResultRecord`, `FinalCaptureRecord`, `PropertyRecord`, a byte arena, `TaskStatus` (`metal_batch_abi.rs:256-299`). The "six typed output relations" of the handover are not listed exactly anywhere (:104).
- Batching (:106-109):
  - `execute_batch(many programs, many tapes)`;
  - root lists per (file, program); empty = no candidates; an incomplete index must fall back to unanchored execution;
  - **on Metal one invocation covers every (file, program, root) task, packed as blocks of ≤ 256 tuples**;
  - grouped by execution signature;
  - count/scan/scatter output, no atomics.
  - **Tempo's execution unit is already cross-file batched.**
- Determinism: errors per task; a failed task publishes nothing; an incomplete result is never a negative (:112).
- Tempo's IR is canonical and newer; N1562's is an older copy plus `packed/**` ported from Landing v3/v4 (:114-117).

### N1562's lowering (§0.2) — the anti-pattern
- Tree relations (inside/has/precedes/follows/stopBy) were lowered to **SQL over navigation-edge rows**:
  - a `Navigation` producer emits a row per parent→child edge;
  - the 35-column `rerail_matches` union vtab (`sql_backend.rs:1567-1612`);
  - a recursive closure in SQL (`query/relational.rs:2783` `lower_navigation`);
  - a `Universe` terminal with one row per node.
- ast-grep patterns ran as a separate CPU-only Rust `PackedPattern` interpreter inside xFilter.
- This is the spike's dead end (row per node, closure in SQL).

### Expressivity gaps (§2)
- Most of the gap came from **exact-parity requirements the loose contract dropped** (:18-22): first-match binding, ordered alternatives, three-valued work accounting, exact capture order and multiplicity.
- Real gaps (:150-157):
  - EXISTS/NOT_EXISTS barrier;
  - PREV/PRECEDING axes and child position;
  - bounded COUNT;
  - pattern opcodes (Smart terminals, ellipsis);
  - bounded CALL;
  - SIDE reads.
- These were left out because SCM is a positive pattern language (:159).
- The relational boundary (:175-190):
  - the IR hosts semi/anti-joins and closure along tree axes, bounded counts and within-match equality;
  - within-document value joins between independent matches are expensive on Metal (they became L5 in derived-facts);
  - cross-file, repository, name graphs, allowlists, priorities and consolidation go to SQL.

### Execution numbers (§3, M from Tempo docs)
- CPU, root-terminal patterns (8 Python identifier patterns, 5,725 files): transducer **359 ms** vs ast-grep on tapes 818 ms vs ast-grep on tree-sitter trees 12,609 ms. Parse 8.65 s; tape encode/restore 4.57 s.
- 128-comment quantifier stress: interpreter 37.6 ms vs Cranelift JIT 34.6 ms. That JIT was declared "not a JIT for transducer semantics" (a dispatcher calling Rust), replaced by a shared `native_ir.rs` emitting Cranelift + MSL.
- **Metal, CPython 335 programs** (2,079 files, 82.9M tasks, 37.4M outputs, 85.1M normalized rows):
  - 219–275 s end to end;
  - **100.4 s Metal across 68 commands**;
  - **100.7 s SQLite publication**.
  - The deleted scheduler made 1,978,757 launches and 25,000 command buffers.
- **Shape spike: full production volumes in 34 invocations and 40 ms**; integrated coverage kernel 106.9 ms; tuples 64% direct, 29% bounded, 7% dynamic frontier (`metal-optimal-execution-shape.md`).
- Pinned 335-pattern differential: 109,114 matches = CPU; 395–451 dispatches; 75–200 s; 142 MB.
- Full corpus on `95b59a9d0`: 584 dispatches, 1.91 GB, then failed the terminal-containment guard after 732.6 s (the reduced repro needed **21.7M comparison pairs**). The `e95613625` fix does the reduced case in 72 dispatches; the full corpus is unproved.
- The guards (600 dispatches, 2 GB, 4 GiB RSS) are self-imposed; the machine's recommended working set is **115 GB**.
- **Inherent vs implementation** (:222-230): the costs come from capture-containment antichains, ordered output, dynamic quantifier frontiers, per-task arenas and **SQLite publication of 85M rows**, i.e. from exact SCM semantics. The inherent issue is path enumeration with no existence or aggregation barrier, so `has` and counts multiply work. EXISTS/COUNT fix it.

### Tape relationship (§4)
- Executors don't read the persisted tape: per batch they decode into the 44 B/node regular table + child array + line scan + supertypes. That feeds the 2 GB guard. Derived documents are not modelled.

### Recommendations (§5), all carried into spec-transducer
- Push into the IR: patterns (full PatternNode lowering, `P_*` opcodes replacing PackedPattern), boolean ops, all relations and stopBy, nthChild, non-recursive utilities, capture equality, counts, transforms, line/suppression reads, SIDE reads.
- SQL above: candidates from statistics-backed indexes, demand/batches, cross-file, counts, allowlists, priority, consolidation, invalidation.
- Interface: `demand(program_id, doc_id, roots)` with one xFilter per batch (`sqlite3_vtab_in`); narrow outputs `t_match`, `t_capture` (only when needed), `t_status`. **Never a 35-column union, never navigation or universe rows.**
- Extensions in order: set mode → EXISTS → axes + CHILD_INDEX/COUNT → COUNT → pattern ops → CALL → SIDE → documents → status.
- Deletions: N1562 Navigation/Universe/recursive-SQL relations/union table (keep only as tests or the pure-SQL fallback); PackedPattern; DLRE's tree-as-SQL (category C); SSC's grammar→relations compiler as an executor (kept as the reference pure-SQL lowering).
- Risks: Metal divergence for EXISTS/COUNT/ellipsis in the dynamic-frontier class; **per-edit launch count dominates for one file → fused or persistent kernels**; rehydration consistency.

### Bearing on Q1
- "Transducer" in the final shape = Tempo's IR word format + verifier + side programs + shared `native_ir` lowering, with the path model changed to **set mode with barriers** (spec-transducer). The Metal failures Tempo hit are artefacts of exact SCM semantics plus per-batch repacking, not of the IR or of batching.

---

## research/sql-vs-transducer.md (222 lines, read in full)

Sources: SSC `b87eba3b` + its uncommitted fragment pivot, DLRE merged, N1562 `8b6576f0`, Moti's sql-dl60, facts-engine, facts-plan, spike indexes. Dated 2026-10-25.

### Verdict (§0)
- **About 85–90% of SQL-lowering code computes tape-local, per-document things** (or only reproduces exact operational semantics): T, T+ or D. About 10% is legitimately SQL: file-level candidate scope, cross-file composition, cross-file closures, stats-driven planning, consolidation (:37).
- Evidence (M):
  - DLRE first-50: **anchor selection 537 s + index prep 137 s vs 58 s of AST matching**; total 958.6 s vs 125.8 s pre-SQL native;
  - SSC pivot: native fragments behind one batched vtab **287.5 ms vs 288.2 ms native**; the SQL state-machine emitters ran at ~3× native;
  - spike: SQL row machinery 110 ns/node; recursive climbs 765 ns/touch; native child TVF ~390× on the hot query; a query-as-data interpreter in SQL 12× slower (D73); temp tables 57–115× slower (D50).
- The spike lowered structure to SQL only because constraint C1 (no native code on PG/MariaDB) forced it (:39).

### Inventory highlights (§2)
- **SSC `sql/**`**: 105,140 lines, ~89k excluding tests.
  - relational compiler core → T, kept only as the **pure-SQL fallback emitter**;
  - `outcomes/` 43k → T/T+/D;
  - memo/continuation/peak machinery → D (exact work accounting only);
  - frames/emit/fused/exec ~18k → D (D73);
  - `nodes.rs` node vtab → D as an execution path (the old callback engine took 1,141 s vs 10 s native);
  - `terminals/` batched native terminals → T, merged into the vtab boundary;
  - crossfile/select_when → S;
  - **fragment pivot `fragments.rs`/`fragment_job.rs` → the right boundary pattern**, with the transducer behind it (:79).
- **DLRE**:
  - in-file anchor SQL lowering (~5.5k) is the 537 s + 137 s, with run-time `count(*)` pricing (trap T18) and a constant 32-row estimate → T (start instruction or a seeded start set);
  - file-level scope (corpus/store/postings/plans, ~2k) → **S** (fts5vocab point lookup 0.27 ms);
  - `relational/syntax.rs` CST table → D;
  - authored-SQL stage machinery (~4.5k; 733 of 760 CPU samples in batch six) → D;
  - reachability/forest → S, as a native graph producer vtab;
  - `producers/` → S (relation catalog).
- **N1562 `query/`**:
  - `relational_ir` + `relational.rs` (8,746 lines): ~80% in-file → T/T+, cross-file → S;
  - `compact_output_sql.rs` → D;
  - `query/plan/` (identity, scope, grouping, utility dedup) → **S, kept as `engine/plan/`**;
  - `sql_backend.rs` → reshaped boundary.
- Moti: sql-dl60 frame engine → D; facts-engine `facts_memo` → S (storage of outputs per (doc digest, program digest)); facts-plan Y1/Y2 → S, with per-document extraction feeding Y1 → T.
- The spike's linter analysis: **77 of 99 rules are single-file**; the caller composes anti-join/count/closure around structural fragments (:119).

### IR extensions named (§3), mostly absorbed into spec-transducer
1. seeded start set;
2. capture-bearing demand;
3. boolean sub-program `Call`/`Not` (→ EXISTS);
4. enclosing/ancestor-stack facts;
5. counted sub-search (→ COUNT);
6. utility calls with memo (on Metal "may stay CPU-only");
7. document switch (→ DOC_GUEST/HOST);
8. per-document fact families (→ fact programs, §21);
9. **backend capability flags, "a program using 3 or 6 can be routed to CPU"**;
10. completeness only.

- **CONTRADICTION (resolved later):** extension 9 routes non-regular programs to CPU. spec-tooling §2.3.2 obligation 7 (2026-10-25 user position) says **every program must be Metal-capable; there is no Metal → CPU routing**, and a non-Metal program is a compiler bug. The final shape follows spec-tooling. Data-dependent constructs are handled by set-stage promotion and queue pools, not by CPU routing.

### Boundary (§4)
- Transducer → SQL, narrow typed set-semantic relations, each row with (batch, program, doc):
  - `t_match(program, doc, root)`;
  - `t_export_<schema>` (only declared join keys and tags);
  - `t_outcome`;
  - `t_count`;
  - `fact_<family>` (rule-independent, cached per document digest).
- One xFilter per (program group, batch); groups by identical candidate scope (N1562 plan/grouping); `estimatedRows` from statistics.
- SQL → transducer: `demand(program_group, doc [, root] [, bound exports])` via `sqlite3_vtab_in`. Usually doc only: the file set comes from FTS file postings and fts5vocab, **intersected with the incremental change set for per-edit runs** (:159).
- SQL above owns:
  - cross-file composition;
  - repository counts/budget;
  - cross-file closures (recursive CTE or native graph vtab: DLRE reachability, facts-plan Y1);
  - allowlists;
  - stats-driven planning (seeded vs scan from fts5vocab and edge stats, cached at plan time);
  - incremental invalidation of cached fact_*/t_*;
  - outer query + consolidation;
  - the pure-SQL fallback (specialised per-program SQL over a tape TVF; a generic interpreter in SQL is forbidden, D73).

### Revised module map (§5), the precursor of spec §16
`engine/transducer/{ir,lower,exec/{cpu,jit,metal},facts}`, `engine/vtab/{transducer,graph}.rs`, `engine/compose`, `engine/plan`, `engine/stats`, `engine/relations`, `engine/fallback`, `engine/findings`. The rest is deleted (§5 table).

### Risks (§7)
- Metal regularity (superseded by the no-routing position);
- **the start-kind/seed choice is a plan-time parameter compiled per stats bucket, never a runtime count(*) (T18)**;
- fact families become a versioned schema;
- the fallback's coverage lags.

---

## research/metal-recursion.md (152 lines, read in full)

- Header (:3): **superseded 2026-10-25** by metal-enqueue.md and the no-fallback rule. Monotone recursion runs on Metal as GPU-scheduled waves; non-stratifiable constructs are rejected by the verifier; hard limits give `Unknown(resource)`. Every "CPU/JIT rerun" below is void.

### Core result
- **Don't run tree recursion as recursion on the GPU.** A preorder tape + `size` turns every relation into range arithmetic over sorted node sets (:9-16):
  - has/DESCEND = the interval `[r+1, r+size[r])`;
  - stopBy = skip a subtree (`j += size[j]`), or a nearest-pruning-ancestor column `anc_P` (h is reachable from r iff anc_P[h] is NONE or ≤ r);
  - inside = a parent walk bounded by max depth;
  - siblings = skip pointers.
- **Nested sub-rules become strata.** The inner rule is materialised first as a sorted node set (postings or bitmap); the outer relation is a range query or set operation. **Dispatches are O(rule nesting depth) per batch**, not O(tree depth) and not O(instructions × generations) (:18-22).
- Only monotone single-direction recursive utilities need O(depth) waves, GPU-driven in one command buffer (:24).

### Why Tempo and N1562 hit their guards (:30-40)
1. Path enumeration: DESCEND forks one frontier state per descendant (`metal_step_helpers.metal:36`).
2. **One instruction per generation under exact ordering/containment: 44 launches per generation**; a two-node fixture hit the 200-dispatch cap at generation 5.
3. Quadratic capture-containment antichains: 584 dispatches, 1.91 GB, 21.7M comparison pairs.
- N1562 memory scales with live paths × state (per-path arenas).
- Set semantics removes 2–3; the range formulation removes 1. Tempo's shape spike already showed broad grid kernels with count/scan/scatter (34 invocations, 40 ms).

### Technique verdicts (:42-56, M unless noted)
- Adopt: interval arithmetic; postings + range search (**20–2000× faster than any traversal; the same postings as the kind index**); `anc_P`/parent columns (0.14–0.63 ms to build over 5.4M nodes); SIMD-group cooperative scan for huge intervals (10.6 → 0.86 ms); indirect GPU-driven dispatch (70 dispatches in one command buffer, no CPU round trips).
- Recursive utilities only: BFS frontier waves (correct, but 10–100× slower).
- CPU/JIT only: explicit per-thread stacks (overflowed at cap 64 on 25–467 roots/query).
- **Reject: persistent-threads queue.** Metal gives no forward-progress guarantee within a SIMD group; with a 2^24 spin cap it didn't finish in 10 minutes.

### Design: stratified range execution (:58-117)
- "Resident data per batch": kind u16; size and parent fixed-width per document; optional field/flags; the document table; **kind postings from the index**; optional text-hash column. No MetalNode, no CSR children.
- Operators:
  - Leaf = kind postings;
  - Filter = node-parallel + count/scan/scatter;
  - has stopBy end = two binary searches per root or a merge-join;
  - has stopBy P = range + anc_P (or SIMD scan);
  - stopBy neighbor/child/field = `parent[s] ∈ R`;
  - inside = bounded parent walk;
  - follows/precedes = sibling range + parent equality;
  - nthChild = sibling walk or an ordinal column;
  - all/any/not = sorted merges or bitmaps;
  - count = range length or prefix sums;
  - capture equality = join on (root, text_hash);
  - utilities = shared DAG nodes deduplicated by digest, once per batch.
- Recursive utilities: only monotone one-direction (has-only or inside-only, no negation or count through the recursive edge). has-recursion runs as depth-level waves deepest → shallowest (one evaluate + one scan dispatch per level); inside runs top-down.
- Dispatch shape:
  - dispatches = strata × a small constant + 2 × max depth for recursive utilities;
  - all of a batch's programs grouped per stratum, threadgroups uniform by signature;
  - count → scan → scatter with indirect arguments written by GPU prep kernels;
  - **one command buffer per plan, with the dispatch count known before execution.**
- Memory: every set is bounded by the batch's node count; leaf bounds come from kind statistics and propagate; the arena is sized at plan time from simultaneously live sets; postings 4 B/elem or a bitmap n/8 (675 KB for 5.4M nodes); no per-path arenas.
- Verifier rules 1–5 became spec-tooling §2.3.2.
- **Relationship to SQL (:117):** node sets are the SQL layer's own objects (kind postings, interval containment), so the same plan lowers to pure SQL via fts5vocab offsets and interval predicates, the spike's approach. **This is the basis of "one plan, multiple bindings".**

### Measurements (:119-143)
- M5 Max, 40 GPU cores; CPython 3.15 stdlib 2,049 files parsed with Python `ast` (not a CST: 1.5–2× fewer nodes) = **5,396,512 nodes, max depth 34**; GPU medians of 7, resident, upload excluded.
- GPU ms, interval scan / posting-owner / SIMD [+anc] / frontier waves / persistent queue:
  - Q1 fn has Yield stopBy scope (72k roots): 0.88 / **0.021** / 0.15 [+0.21] / 2.58 / 5.1 (stack 3.15 with 135 ovf);
  - Q2 fn has Return: 0.85 / 0.020 / 0.11 / 1.61 / 11.0;
  - Q3 count Call in fn: 2.05 / 0.050 / 0.45 / 2.33 / 5.2;
  - Q4 Module has Lambda (2,049 huge roots): 10.56 / 0.005 / 0.86 / 2.55 / 5.0;
  - Q5 each Call has Lambda (421k): 0.11 / 0.026 / 0.20 / 0.90 / 5.7;
  - Q6 each Call inside Try (421k): parent walk 0.12.
- Single-threaded CPU interval scan: 11–190 ms on the same queries.
- 0 mismatches except stack overflow.

### Bearing on Q1/Q3
- The Metal execution model of the final shape is this stratified set formulation (spec-transducer's set stage) over the resident v9np columns + kind postings. Posting-based operators run at **0.005–0.05 ms GPU for whole-corpus queries**. That is why batching across the corpus is the point and per-file execution is wrong.
- **Contradiction with v9np:** this design lists `parent` as resident per batch, and v9np does not store it. Resolution per §20.6: `par` is a resident sidecar keyed by tape digest (+32–46 MB for the gate corpora, per tempo-residency). Building `anc_P`-like columns costs 0.14–0.63 ms GPU.

---

## research/regex.md (251 lines, read in full). M on the 9,010-file gate corpus, M5 Max, 18 cores

### Answers (§0)
- The 14 whole-file regex rules are ports of nera-data-layer `scripts/lint_*.py` line linters (9 of 14 never parse Python). The exact-parity port rebuilt a tokenizer by hand: the line state machine for triple-quoted strings, `^\s*#`, `excludeUnder: TYPE_CHECKING`. The loose contract removes the reason for all of it.
- **Census of 113 clauses: 107 AST now, 2 need line-position facts, 4 path-only, 0 textual (M).**
- **All 113 produce 0 findings on the gate corpus** (M). Today's regex stage costs 9–17 s for nothing.
- Identifier regexes are evaluated **once per distinct name**: 396/444 are exact literal sets; the rest takes 0.11 s; per occurrence it would be 33–69 s.
- Trigram index 0.127 B per source byte, built in 0.5 s, keeps 6.2% of pairs with 0 of 119 missed. It has no consumer after lowering, so **don't build it now**.
- Expected regex stage: **0.1–0.3 s cold, ~0.01 s warm (D)**, vs 9–17 s today.

### Today's cost (§1.1, M)
- Per-clause pipeline over 116,306 (clause, file) pairs, 2.42 GB: 6.45 s single-threaded (1.0 s on 18 threads).
- **Line splitting alone is 3.5 s (55%)**; `is_match` of all 109 content patterns on whole files 0.52 s (4.7 GB/s); content gates prune almost nothing.
- **One combined `RegexSet` pass per file: 92.6 s** (it falls off the lazy DFA). Spec §19.3 item 5, "single multi-pattern pass", should be dropped.
- Findings context of 5,125 raw whole-corpus matches: code 77%, strings 18%, docstrings 2.8%, comments 1.4%. String matches are false positives today; under AST lowering they disappear, which is a correction, not a parity break.

### Node regex (§2)
- 17,345 atoms over 588 queries; 709 distinct node/value patterns (444 on identifier hosts, 195 metavariable hosts without kind, 42 on `module`).
- No look-around or backrefs. Dense DFA (`regex-automata` 0.4.18): 725/725 compile with ASCII `\b`, 710/725 with strict Unicode `\b`. DFA median 3.4 KB, p95 35 KB, max 759 KB; 708/725 ≤ 64 KB.
- Identifiers: 6,018,578 occurrences; **231,862 distinct names**; Σ per-document distinct = 5.1× global; **global interning 0.22 s on one thread**.
- Methods (M):
  - per occurrence 32.6 s;
  - per distinct name one at a time 2.02 s;
  - RegexSet 7.1 s;
  - one multi-pattern DFA: build 7.9 s, 14.6 MB, run 0.18 s (639 patterns: 676 s build, 201 MB);
  - **hybrid hash on exact sets + one DFA for the other 48: build 0.44 s, DFA 3.0 MB, run 0.11 s**.
  - Result table 231,862 × 444 bits = 13 MB.
- IR encoding (§2.3), which became spec-transducer §5:
  - **TEXT_ID b**: the tape producer writes a **`tid` sidecar (u32 per token node) into a global append-only `text_vocab(id, bytes)` in the content store**; plan-time evaluation to a result table `R[tid]`; executor = one load; no DFA on the GPU; vocabulary growth evaluates only new ids; TEXT_ID_EQ for a single literal.
  - **TEXT_DFA t**: byte-class map, u16/u32 transitions, accept bits; deduplicated by digest; constant memory if ≤ 64 KB; bounded by node length; `\b` must be defined as ASCII (or strict Unicode) to satisfy no-fallback.
  - **TEXT_SET**: ≤ 64 patterns per group; determinisation blows up past ~450.
  - Rewrites: `^async\b` → KIND of the keyword child; `module`-hosted literal alternations → `search_term_fts` document prefilters.
  - Line words: if ever needed, use `memchr` (0.49 s vs 3.5 s).

### Trigram (§3, M)
| Index | Size | Build | Query |
|---|---|---|---|
| custom u32 trigram → delta-varint file ids | 0.127 B/src byte (17.5 MB ≈ 0.62 B/node) | 0.15 s + 0.36 s merge | 0.14 s |
| FTS5 trigram detail=none | 0.159 | 36 s | 3.55 s |
| FTS5 trigram detail=full | 2.01 (277 MB) | 68 s | — |

- If ever needed: the custom index, not FTS5. Decision DI 4b "no trigram index" holds.

### Bearing on the final shape
- **TEXT_ID's interned `tid` and `text_vocab` is a global atom table**, the natural u32 atom space shared with derived facts' "atoms interned as u32" (§21.6). **To check in t1/spec-indexes whether `text_vocab` = the facts atom table.** (Open.)
- `tid` is a resident sidecar (per §20.6 class) built at ingest; interning belongs at ingest (0.22 s cold).

---

## research/metal-enqueue.md (178 lines, read in full). Extends and supersedes metal-recursion

Machine: M5 Max 40 GPU cores, macOS 26.4.1, runtime-compiled shaders, checked against a CPU BFS reference.

### Conclusion (§1)
- Metal has no dynamic parallelism. Three facilities suffice:
  1. GPU-written indirect dispatch arguments (size 0 = no-op);
  2. GPU-encoded ICB commands (executed in a *later* `executeCommandsInBuffer`);
  3. a single-threadgroup persistent loop (progress guaranteed by `threadgroup_barrier` within one threadgroup only).
- **Model: GPU-scheduled hybrid waves.** Device-wide waves are sized by a one-thread `prep` kernel; at ≤ 1,024 frontier nodes, hand off to one 1,024-thread threadgroup that runs the remaining waves in one dispatch. **The CPU encodes slots and submits; it never makes per-wave decisions or executes rule logic.**
- Per-wave cost (M):

  | Driver | Per wave |
  |---|---|
  | CPU command buffer per wave | 386–890 µs |
  | pre-encoded indirect | 5.9–7.9 µs |
  | GPU ICB | 18–30 µs |
  | threadgroup loop | 1.13–1.18 µs |
  | hybrid | 1.2–1.6 µs |

- 100,000-wave fixpoint: 113 ms GPU in one command buffer. **Wide CPython fixpoint (5.4M nodes, 27 waves): 1.57 ms GPU.**

### Facilities verified (§2, M)
- 200,000 indirect dispatches in one command buffer; an empty slot ≈ 3.5 µs.
- ICB GPU encoding works.
- **A GPU write to an ICB slot during the running execute call is not visible**: each slot needs its own execute call plus a barrier.
- Encoding ≈ 2 µs/wave on the CPU; a pre-encoded ICB is reusable across submissions.
- No grid-wide barrier.
- Watchdog risk for multi-second command buffers on a display GPU; loops bounded by `limit`.

### Design (§3)
- **Admissibility:** the set stage is **stratified Datalog over finite node sets**; the EDB is the tape columns (kind, size, derived par).
  - Positive recursion within a stratum (≤ |nodes| waves, each node enters once); negation and counting between strata; **any axes, either direction, any depth**. Only unstratifiable programs are rejected.
  - This is **wider than** metal-recursion's monotone-one-direction rule.
- Per stratum:
  1. a seed kernel writes F0 and its count;
  2. wave slots in a `.serial` encoder: `expand` (indirect, visits axis neighbours via par/children `c += size[c]`/siblings, marks atomically, appends) + one-thread `prep` (next args, done flag, handoff at ≤ H);
  3. a loop slot `tg_loop`;
  4. strata run in order in the same command buffer.
- prep_icb is used only when the kernel choice is data-dependent (fused-group variant or axis kernel): 18–20 µs vs 6–7 µs per wave.
- Waves per submission:
  - narrow/deep: one loop dispatch;
  - wide: W pre-encoded slots per stratum, **W from statistics** (max depth/diameter of scoped documents; CPython 27);
  - a frontier live at the end of a buffer gets a new chunk of slots (~6.4 µs/wave, correct across 1,564 sequential command buffers).
- Memory:
  - marks 1 bit/node (`atomic_fetch_or`);
  - frontiers 2 × 4 B × nodes worst case (can't overflow);
  - outputs exact by count/scan/scatter;
  - live sets postings or bitmap (converted on GPU above n/32);
  - arena exact at plan time;
  - batches cut to fit the budget (e.g. 50% of recommendedMaxWorkingSetSize); **splitting a batch by documents is always correct**;
  - a 1M-node document ≈ 9 MB;
  - subtree-local programs can be chunked along preorder ranges; recursion across a chunk boundary needs the whole document resident (the only hard limit).
- Termination: ≤ n waves per stratum; progress or end; bounded loops; no spinning.

### Measurements (§4, M, single runs on a busy machine; ast-based tape)
- Chain depth 10,000: CPU per wave 4,968 ms wall; pre-encoded 71–78; threadgroup loop 11.8; hybrid 14.1–15.5; GPU ICB 204.
- Chain depth 100,000: CPU 38,589 ms; pre-encoded 712–728; loop 113; hybrid 119–123.
- CPython 5.4M nodes, 27 wide waves: CPU 24.0 ms wall / 1.71 GPU; pre-encoded 29 slots 1.89/1.61; **hybrid H=1,024 2.65/1.57**; loop 8.61 (a wide frontier on one threadgroup is slow).
- Readings: the CPU round trip costs 50–400× per wave; H ≈ 1,024 works for both shapes; size W from statistics (empty slots cost); ICB only when data-dependent.

### Resource outcomes (§5)
- **No CPU/JIT route at run time.** The CPU interpreter/JIT exist as test oracles **and as the executor on machines with no Metal at all**.
- Unknown(resource) reasons: `doc_exceeds_device`, `batch_split_exhausted`, `device_lost`/`command_buffer_error` (with dump), `plan_bound_violation`.
- `metal_coverage = 1 − Unknown(resource)/evaluated`. **CI gate: 1.0 for every admitted rule on data-layer, CPython+335, and rerail**; a rule that can't pass is not admitted.
- The verifier rejects:
  - recursion through negation or aggregation;
  - recursion through anything except set-stage axes and local tests (string transforms, **facts from other documents: cross-document closure belongs to the SQL graph producers**);
  - live sets that cannot fit;
  - unlowered constructs, which are counted compiler gaps, never executed elsewhere.

### Bearing on the final shape
- This is the answer to Tempo's handover blocker, "dynamic frontier continuation": frontiers stay resident and GPU-scheduled waves continue them; the CPU only appends slot chunks on "not done".
- The final executor model = **stratified Datalog over node sets per batch, as GPU-scheduled waves in one command buffer**, with per-document batch splitting for budget. It is the same algebra as Typing-support's Datalog (T9), which runs the cross-file/name-keyed part on CPU natively. Relationship to check against T9: are Typing's node-local lattice waves the same machinery?

---

## research/fusion.md (117 lines, read in full). Superseded header: no CPU/JIT rerun (as metal-recursion)

CPU only. A minimal copy of Tempo's interpreter + ANCESTOR, EXISTS/NOT_EXISTS/ACCEPT, DESCEND_KIND, set mode with label bitsets and per-pc reachable-label masks, lazy enumeration and peephole. Tape: a regular table from tree-sitter-python 0.25 over the Python 3.14 stdlib, **1,847 files, 8.54M nodes**. Min of 3–5 runs; differences < 1.3× are noise.

### Results (M)
- **Shared relation-mask passes:**
  - one reverse-preorder pass gives each node a bitset of target kinds in its subtree;
  - one forward pass does the same for ancestors;
  - each has/inside is then a bit test;
  - 210 programs: 348–442 → 6.4–17.3 ns/node, **25–54×**.
- **Multi-rule tries by root kind:** K=512 2.3–2.5× (tasks 62.5M → 4.37M); K=128 1.3–1.5×; K ≤ 32 break-even. The gain comes from shared navigation (one child enumeration serves 25 programs); code shrank only 1.25×.
- **Fused dispatch without an index is as fast as anchored execution with prebuilt postings** (postings cost 12.4 ns/node to build). **"Right for per-edit checks."**
- **Lazy descendant cursor + DESCEND_KIND peephole:** pushed paths **5.26M → 96k (55×)**, ~2–2.5× faster. It moves `has` out of Metal's dynamic-frontier class.
- **Fused AND/OR/NOT:** 1.3–2.0× vs separate programs + hash join; conjunct order up to 1.8×, ordered by `cost/(1−s)`. Not a win when the first conjunct barely filters (0.84–1.07×).

### Forms
- OR = FORK + EXISTS + EMIT (first emit stops the task); AND = EXISTS sequence; NOT = NOT_EXISTS (also `stopBy: rule`).
- Relations:
  1. an existence barrier with a lazy range cursor;
  2. DESCEND_KIND, a contiguous kind-column scan over `[node+1, end)`;
  3. batch mask passes (`sub[parent] |= sub[child] | bit`; `anc[i] = anc[parent] | bit`; d passes for nesting d; use masks when Σ_roots subtree scan > nodes·c_pass/targets_per_pass).
- Capture equality between independent matches stays in SQL (→ L5).
- Counts via child-mask popcounts or per-target prefix counts.
- Utilities: inline by digest (974k → 316k expansions) or CALL with a depth bound.

### Cost model and planner (became spec-fusion)
- Inputs in SQL, cached with the plan: N_k from fts5vocab, fan-out/subtree size per kind, selectivity from co-occurrence refined by `plan_feedback(program_digest, s, cost, at)`.
- Fuse when ≥ 16 programs share a root kind or ≥ 2 share a relation target.
- Grouping only within the same candidate-scope digest and grammar (1.45× vs 0.86× for a union).
- Cache key = (sorted member digests, fusion version, IR version, grammar, stat buckets). Recompile only when a decision would flip.
- Demux: `t_match(program_group, label, doc, root)` ⋈ label table.
- **Per edit: fused dispatch without an index, plus masks, for the one file.**
  - **Note:** this is CPU-measured advice that predates "never per-file". In the final shape the per-edit request is the batch that arrives, run by the same plan class.
- Legality from the findings contract: first emit per label, EXISTS barriers, conjunct reordering, short-circuit OR, trie merging, label pruning, lazy enumeration and mask passes are all legal **only because findings are a set**.
- §"Relationship": **mask passes ≡ stratified range execution**: the inner rule is materialised as a per-node set (bitset column or postings); fusion gives the multi-rule form (one pass carries many rules' target bits); metal-recursion gives the bounds and dispatch shape.

---

## research/m2-m7.md (140 lines, read in full)

Thread CPU, min of 5 runs (3 for M7), single core, parse excluded; load avg 38–70 on 18 cores.

### M2: fused executor speed: PASS
- Native finding sets on 9,010 `.py` files (M):
  - TP default 0 keys, 70 s CPU;
  - ASD 0 keys, 113 s;
  - SAS 1 failure (`generated_facades.py` budget), 91 s;
  - TP all 58 clauses 1,244 keys, 89 s;
  - peak RSS 584–637 MB.
- The fusion-model harness is a superset (4,402 vs 0). This executor implements the fact keys exactly and uses native's **patched grammar** (the stock grammar flags 46 files with errors and loses 67 TP keys).
- Executor features:
  - hash-consed sub-rule DAG (ASD 111,549 expansions → 1,629 ops);
  - recursive prefix trie on capture-free conjuncts;
  - cost/(1−s) reordering calibrated on every 10th file (~0.02 s);
  - epoch-stamped memo columns; keyed set stage; adaptive masks;
  - **regex results interned per distinct leaf text**;
  - **dispatch through a kind-postings index**.
- **Parity:** identical finding sets wherever native completes (1,244 = 1,244; fixtures 97/97, 53 keys, in all modes).
- Speed (M):
  - fused TP-all + ASD (71 programs, 835.8M program-nodes): no fusion plain 2.467 s (2.95 ns/pn) → **trie + reorder + index 0.214 s (0.255 ns/pn)**, 15.9M evaluations at 13.4 ns/eval;
  - TP default + ASD: 0.171 s (0.40 ns/pn);
  - per rule: ASD 0.86 ns/pn (borderline, deep recursive utilities), TP 0.11 ns/pn.
- **Per-evaluation cost 13–17 ns vs the spec's 3–5 ns. The pass comes entirely from 20–40× fewer evaluations**, not cheap evaluations. Profile: interpretive evaluator ~74%, regex ~6% (module-text gates), trie 5%, variables 4%, malloc 3%.
- Memo columns didn't pay once regex was interned (10–15% slower). Masks were almost never promoted.
- Bugs found:
  - the transform cache was keyed without the program;
  - the bind-memo ignored variables under not/count/nth.
  - **A single-level trie fails when rules combine (326M evaluations); it must be a recursive prefix trie.**

### M7: budget cases: PASS
- SAS `generated_facades.py` (206,043 nodes, 906 functions): 0.141 s without keyed → **0.020 s with keyed**; native: budget exceeded after 1.9 s.
- s1 (1,000 sync + 1,000 async pairs): 0.451 s (20.7M evaluations) → **0.0036 s** (201k); native exceeded at 651 ms, 220 MB.
- No budget or depth cut hit. Without keyed lookup the cost is quadratic (p100 5 ms, p200 16, p400 71).
- The keyed stage was later withdrawn as E15 in favour of L5 SQL joins (derived-facts); M10 must show L5 within 1.5×.

### N1562 packed IR
None of TP/ASD/SAS lowers to it: missing `syntax`/`filePath` keys, all-or-nothing lowering, recursive utilities under negation rejected. No benchmark exists.

### Bearing on the final shape
- The batch, cross-rule, fused shape is measured on CPU at 0.26–0.40 ns per program-node. The lever is **shared evaluation** (tries, DAG, ordering, postings dispatch), which only exists when many rules × many files run together.
- The remaining gap is the per-evaluation cost of an interpretive evaluator (13–17 ns vs 3–5 ns). That gap is what the JIT/Metal backends and the set stage address.
