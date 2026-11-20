# T1 notes: the PIAT normative spec

Source root: `/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/docs/rule-engine-direction/` at `85098ee3`. Citations are `file:line`.

## spec.md (597 lines), read in full

### §0–§2 Targets, principles, data flow (1–51)
- The three targets (per-edit, which eventually needs the GPU; full repository in 1–2 s; outer SQL with a pure-SQL fallback) are at :9-17.
- Out of scope for the engine: presentation, ordering, ids, history and payload rendering (:17).
- Principle 2 (:22), "the transducer is the concrete execution IR for local structure":
  - It stays small and backend-uniform.
  - Shared intermediates, joins between independent sets and fixpoints over names are named derived facts (§21).
  - Base facts that need a resolver come from native producers.
  - A new transducer op must be local, cheap on every backend and used by several rules (`spec-transducer.md` §0).
- Data flow (:34-50):
  1. rule documents compile, cached by rule digest + compiler version + stats decisions;
  2. the plan is per-document programs (transducer IR, fused) plus cross-file composition (SQL over relation contracts);
  3. outer SQL statements;
  4. SQLite sends `demand(program_group, doc[, root][, exports])` to the transducer vtab (CPU interp, JIT or Metal);
  5. the vtab returns `t_match`, `t_export_*`, `t_count`, `t_outcome`, `fact_*`;
  6. SQL does the joins, anti-joins, repository counts, cross-file closures and allowlists;
  7. finding keys plus outcome go to consolidation.

### §3 Tape v9np (52–95), decision 0003
Measured over 24.2M nodes: CPython 3.13 Lib, data-layer `impl/`, and rerail's TS, Rust and PHP. All variants decode to identical checksums (:54).

**The tape is structure only** (:56-66). Columns:

| Column | Width | Notes |
|---|---|---|
| `kf` | u16 | 10-bit folded kind code + 6-bit field. Named, extra, missing, error and contextual aliases are distinct codes, assigned per grammar at registration: 194–277 codes of 1,024 |
| `size` | u16 or u32 per document | u32 only when the document has ≥ 65,535 nodes (24 of 6,251 files) |
| `base` | u32 per 16-node block | |
| `off` | u16 | `start(i) = base[i/16] + off[i]` |
| `tail` | u16 | `end(i) = start(i+size(i)-1) + tail[i]` |
| `par` | not stored | rebuilt with a stack at 4.1–5.1 ns/node, once per resident tape, as a sidecar. Stored (+2 B/node) only if SQL parent climbs prove hot; a climb is 1.6–3× faster than interval containment |

- Layout at rest is SoA plus a one-byte width descriptor and the node count (:67).
- **Not in the tape** (:68-72): digest and source length (store row); alias string tables and flags (folded into codes); line tables (recomputed per resident source); derived-document data (`file_documents`).
- **Size** (:73): 8.25–8.55 B/node logical, 0.93–1.01× v7, and 0.94–1.06× v7 after zstd.
  - The first v8 proposal (absolute u32 start and length) measured 1.24–1.59×, and 1.47–1.90× after zstd.
  - Files of 64 KiB or more hold 47% of CPython's nodes, and absolute positions compress poorly. **That is why a layout expansion was rejected.**
- **Benefits** (:75-82):
  - Rust random access to (kind, parent, size, span, named): 113–490 → 23–199 ns; random span alone 33–364 → 13–57 ns.
  - SQL span extraction 2.7–3.8× faster (columns) and 5.6× (interleaved).
  - SQL structure queries are no faster, since v7's rec6 is already regular.
  - JIT/Metal bind columns in place from residency slabs with no upload on Apple UMA. The 44-byte `MetalNode` restore (3.7–7.8 ns/node) is deleted.
  - No gain for sequential scans: ends are recomputed at 8.8–24 ns/node against v7's 3.8–4.3. A stack-carried walker is not yet measured.
- Deleted readers (:84): the checkpoint walk, overflow and flag binary searches, alias tables, DLRE's `decoded_*` caches. `TapeDoc::restore` becomes optional.
- **Blade** (:86-89):
  - It stores exactly the tree-sitter-php tape of the view text.
  - ERROR nodes carry the ERROR code and their real span; MISSING nodes are zero-width with the MISSING code (as ast-index-spike does).
  - No grammar-specific tables.
  - The view text, segment map and host errors are a derivation sidecar keyed by (template digest, via).
- **Before locking** (:91-95): code space and aliases on the patched grammars; the stack-carried walker; kind+size layout for SQL; whether to keep `par`.

### §4 Documents and sidecars (97–106)
- One tape per document. Derived documents (embedded, generated views) each get their own tape in their own grammar (spike S070); v5's splice into one tape was rejected (:99).
- `file_documents` exists today (`schema.rs:92`): `(ordinal, role, via, parent_ordinal, tape_id, source_id, source map, host errors)` (:100).
- Source maps are `(doc_start, parent_start, len, flags)` triples, used only to normalise locations, apply host suppressions and skip synthetic segments (flag bit 2) (:101).
- Sidecars (line starts, error ranges, the Blade view) are keyed by tape or template digest and recomputable (:102-105).
- **Execution batches reference resident documents through a batch document table. Columns are never concatenated or copied into a batch.** Every column starts at a 16-byte offset in the at-rest blob, so one decompression into a slab yields aligned columns (:106).

### §5 Indexes and statistics (108–128)
- **5.1 Global kind codes** `lang·1024 + local` (spike Option B: −5 to −22% latency, removes foreign hits). Main emits language-local `k{kind:04x}` (`native/src/code_rule_dsl/transducer.rs:525`) (:112).
- **5.2 Kind index:** contentless FTS5, positions = preorder ordinals. Point lookup 0.27 ms, workspace subset 6.6 ms, full vocabulary 348 ms, so the last is forbidden at plan time (:113).
- **5.3 Index table** (:116-122). `spec-indexes.md` Part 2 is normative.

  | Index | Budget |
  |---|---|
  | `search_kind_fts` v3, tokens `k%06x`, one row per document | ~2.6 B/node; Q2: `detail=none` + `kind_freq` if positions are unneeded |
  | `edge_freq` aggregate (parent, child, field) → edges, with a per-document `edge_counts` blob for exact deletion | ≤ 0.15 |
  | `search_term_fts`, contentless, `detail=none`, language-prefixed words of named leaves | ≤ 1.3 |
  | `workspace_docs` view | 0 |
  | `compiled_plans` (rule digest, compiler version, grammar set, index version), with decisions and valid intervals | negligible |

- **All secondary indexes ≤ 5 B/node** (spike C4). No trigram index (:124).
- `PRAGMA optimize` on base tables only; never ANALYZE temp tables. `xBestIndex` estimates come from stored statistics, never constants (:125-126).
- **"DLRE's private anchor store is retired"** (:127). To reconcile with decision 0006 ("anchor index always built"): 0006 concerns the anchor index's existence; §5 places it in the shared content-store FTS indexes. Check `spec-indexes.md`.

### §6 Transducer IR (130–172)
Normative text is `spec-transducer.md`. Base is Tempo's IR: 32-bit words, path-forking execution over one document from root nodes, typed output arenas (:134).
- Tests: KIND, FIELD, FLAGS, TEXT/TEXT_EVAL, SIZE, SUPERTYPE.
- Navigation: CHILD, NEXT, PARENT, DESCEND, FOLLOWING.
- SAVE/RESTORE_CURSOR, CAPTURE, REPEAT_*, FORK, GOTO, EMIT.
- Side programs: value (regex as byte NFA with DFA forms), capture-predicate, output.
- Text (:140): token-text regexes are evaluated once per distinct text and read by `TEXT_ID`; other text uses `TEXT_DFA`; whole-file text clauses are rewritten as AST rules.

Extensions E1–E16 (:142-160):

| # | Extension |
|---|---|
| E1 | set mode |
| E2 | EXISTS/NOT_EXISTS |
| E3 | PREV/PRECEDING/ANCESTOR, child index |
| E4 | bounded COUNT |
| E5 | ast-grep pattern opcodes |
| E6 | bounded CALL/RETURN |
| E7 | seeded starts and capture-bearing demand |
| E8 | SIDE fact columns |
| E9 | document switch |
| E10 | per-op capability declarations, no backend fallback |
| E11 | completeness bit and reason per (program, doc) |
| E12 | lazy range enumeration plus `*_KIND` superinstructions: pushed paths fell 55×, and DESCEND leaves Metal's dynamic-frontier class |
| E13 | program-group header: root-kind → entry-pc, labels, reachable-label masks |
| E14 | **set stage:** batch passes materialise per-node sets (bitsets or sorted postings), read by MASK/IN_SET; subtree, ancestor and stopBy-gated passes, one per nesting stratum; 25–54× CPU, 0.005–2 ms Metal |
| E15′ | FACT set from a fact keyed (doc, node), replacing keyed sets (E15) and bottom-up folds (E17, now the native `py.flow_exit`) |
| E16 | ORDER against a capture |

- Excluded (:162): first-match binding, three-valued logic as semantics, exact work charges, memo-replay accounting.
- Verifier (:164-171):
  - stratifiable; positive recursion within a stratum over set-stage axes and local tests, any direction; negation and counting only between strata;
  - bounded CALL; progress on every cycle;
  - every op implemented on every backend including Metal;
  - strata, live sets and arena bounded at plan time;
  - set semantics only; every loop bounded by a batch constant; table sizes within encoding limits.

### §7 Lowering and fusion (174–210)
- **Two-level execution model** (:178-181):
  - instruction programs for node-local tests and bounded paths (Tempo classes 1–3);
  - the set stage (E14) for relations and nesting: the inner rule is materialised as a per-node set by a pass over the regular tape, and the outer rule tests membership;
  - Metal stratified-range and CPU mask passes are this model.
- **Frontends** (:183-187):
  - AstMatcher is one frontend; SCM is a test frontend.
  - A feature that can't lower becomes a derived fact or a native producer.
  - **Program boundaries follow shared state:** capture rollback, recursive utility calls and ordered searches that share state stay in one program (SSC `sql-matcher-fragment-transition.md:54-56`).
  - Lowering to the IR: all/any/not, every relation and stopBy, field, nthChild, non-recursive utilities, monotone single-axis recursion, capture equality within a match, axis counts, transforms.
  - Joins between independent sets and recursion through names go to derived facts (L5).
- **Fusion forms** (:191-200), measured on CPU:
  - OR 1.4–2.4×; AND 1.5–2.0× (wrong order up to 1.8×); NOT 1.3–1.9× (0.84–1.07× when A barely filters);
  - has/inside: lazy cursor → DESCEND_KIND → mask, chosen by cost (25–54× for 210 programs);
  - tries K=512 2.3–2.5×;
  - utilities: inline by digest when small, bounded CALL when large, a mask target as a relation target, set-stage fixpoints with device enqueue when recursive;
  - capture equality across matches: in-document EXISTS+TEXT_EQ, otherwise L5 (candidate exports, SQL semi-join on (doc, key), optionally a FACT set); across documents, a SQL hash join over `t_export`.
- **Priorities measured on the 60 data-layer rules** (`fusion-model.md`, 28.2M nodes) (:202-207):
  1. **Shared sub-rule evaluation is primary.** 535 queries become one hash-consed graph (279,204 expansions → 6,821 ops), evaluated once per document into lazy per-file memo columns: 1.4–3.1×, almost all from regexes no longer re-scanning large nodes (2.59 TB → 3.1 GB of text scanned).
  2. **Masks only for range-scanning relations** above the threshold: break-even to 1.3×. Eager masks are 0.74–1.5×; neighbour relations never qualify.
  3. **Tries:** 0.9–1.2× on data-layer, but on the shipped Python catalog, zero-match fragments on common kinds are 39% of native time in the top 10 (SSC). Re-evaluated per ruleset.
  4. **Rewriting relational inputs** (0002) is the largest item: 191 inputs take 146.6 of the 171.5 s `sep` total (~85%). The six heaviest groups are estimated at 0.40–1.29 s instead of 139.6 s, and exported rows fall from 6.7M to ~0.
- **Per edit** (:209): fused dispatch without an index, plus masks, for one file. Measured: 1 file × 535 queries, parse included, p50 2.9 ms, p99 189 ms, worst file 1.3 s, with a cached plan. A cold plan compile takes 5–6 s, so **the plan cache is mandatory**.
- Legality rests on the findings contract (:210).

### §8 Executors (212–236)
- Rust interpreter (reference), CPU JIT (arm64), Metal interpreter, Metal JIT.
- Every admitted program runs on every backend, with no Metal→CPU fallback. A hard limit yields `Unknown(resource)`, which counts against `metal_coverage` (:214).
- **Executors read tape columns directly; no per-batch repacking** (:215).
- **Stratified range execution on Metal** (:216-235):
  - subtree = `[r+1, r+size[r])`; stopBy uses a subtree skip or a nearest-pruning-ancestor column; inside is a parent walk bounded by max depth;
  - nested rules are strata, the inner materialised as a sorted node set (posting or bitmap);
  - **dispatches are O(nesting depth) per batch** in one command buffer with GPU-written indirect arguments;
  - recursion uses device-side enqueue; the set stage is stratified Datalog;
  - hybrid GPU-scheduled waves: 1.2–1.6 µs/wave; 100k waves in 113 ms; a 27-wave fixpoint over 5.4M nodes in 1.57 ms; CPU round trips 386–890 µs/wave;
  - ICB only when the kernel depends on data (18–30 µs/wave); chained command buffers ~6.4 µs/wave;
  - postings range search 0.005–0.05 ms per query; interval scan 0.1–10.6 ms (0.86 ms with a SIMD-group scan); stacks overflowed at cap 64; persistent-threads queue rejected.
- **Memory sized at plan time; no per-path arenas** (:228). The 600-dispatch and 2 GB guards are replaced by plan-time bounds. Batches are cut to fit; subtree-local programs chunk along preorder ranges (:230).
- Resource outcomes (:231-235): `doc_exceeds_device`, `batch_split_exhausted`, `device_lost`/`command_buffer_error`, `plan_bound_violation`; `metal_coverage` gate at 1.0.
- Inputs (:236): executors bind residency slabs, never per-batch copies. **Batches are width-homogeneous** (u16 or u32 size), with kernels specialised by function constants.

### §9 Vtab boundary (238–256)
- `demand(program_group, doc[, root][, bound exports…])`, one xFilter per (program group, batch); `estimatedRows` from statistics.
- Scalar demand uses `sqlite3_vtab_in`. **Capture-bearing demand uses a demand handle:** an aggregate `demand_<group>(…)` builds an immutable typed request relation and returns a single-use handle, passed through a hidden `demand` column. It expires at batch end; an empty aggregate is an empty batch; transport is bounded (1M requests / 128 MiB) (SSC `sql/fragments/demand.rs`) (:240-242).
- Output relations (:245-252): `t_match(program, doc, root)`, `t_export_<schema>`, `t_count`, `t_outcome(program, doc, complete, reason)`, `fact_<family>` (e.g. `fact_ref`, `fact_def`, `fact_import`, `fact_call`, `fact_scope`; per document digest, cached), `d_<name>` (derived, by `fact_digest`).
- Completion is per domain; absent rows mean false; a resource failure discards that (group, document)'s rows only (:254).
- Forbidden: rows per node or edge, wide unions, per-node callbacks (:255).
- **Measured** (:256): typed completed output costs 4.06 s of 83.56 s (4.9%), ~55 ns per candidate root and 1.2 ms per xFilter, over 73.2M roots and 3,289 xFilters on 5,725 files, with exact parity. "The boundary is cheap; the executor decides speed."

### §10 SQL composition and rule language (258–268)
- SQL owns: derived facts, cross-file joins and anti-joins (`require_each`, `select_when`), repository counts and budgets, closures over cross-file graphs (recursive CTE or a native graph vtab: DLRE reachability/forest, facts-plan Y1), allowlists, **incremental invalidation**, and the outer query.
- Cross-file composition uses a closed algebra (join, union, anti-join, group/count) with the `Predicate` AST. Generic analyses are `fact_*` contracts. `relational` is frozen, then removed.

### §11 Planning and plan cache (270–283)
- Source: N1562 `query/plan/`.
- An explicit execution budget of statement executions; statement count follows SQLite compound limits (128 arms per UNION ALL), never file or root count; a stats refresh re-prepares only when a decision changes (:273).
- **Batches are cut by resident bytes, not file count; prepared plans are reused across batches** (:274).
- Shape choices are made at plan time from statistics, never by `count(*)` at run time (DLRE spent 537 s; trap T18) (:275).
- Fusion grouping: same candidate-scope digest and grammar; fuse when ≥16 programs share a root kind or ≥2 share a relation target; masks above threshold (:276-278).
- Cost inputs: fts5vocab root counts, edge and size aggregates, `plan_feedback` (:279).
- Fused-group cache key: (sorted member digests, fusion-plan version, IR version, grammar, stat buckets); FACT/SIDE inputs enter through `fact_digest` (:280).
- Fact strata are scheduled before the rule programs (:281). Demux through the label table; an Unknown group is split and re-run (:282).
- `compiled_plans`: each decision stores the stat keys it read and its validity interval, re-checked by one batched lookup (:283).

### §12 Findings and consolidation (285–291)
- Keys `(ruleId, code, path, line[, column|tag])` plus Complete/Unknown per (rule, file). Consolidation normalises, collapses, applies suppressions, rehydrates with the native matcher, derives ids and order, and diffs.
- Outcome classes: matched; **logical unknown** (from three-valued frontends, never counts against coverage, published as no match at the boundary, kept only inside in-document composition); resource `Unknown(reason)`. To be recorded as a decision (:291).

### §13 Pure-SQL fallback (293–299)
- Each transducer program lowers to specialised SQL over a TVF reading the tape. A generic SQL interpreter is forbidden (D73).
- Sequential contracts (recursive utilities, capture rollback, ordered search, parsed regions) have **no pure-SQL binding**; they are reported as uncovered.
- In the main engine, structural predicates never lower to SQL over a node relation.

### §14 Per-edit and incremental (301–306)
- **The batch is one edited file × all rules. Repository facts come from the snapshot** (:303).
- The edited document is parsed and its v9np tape, source and line starts go directly into a pinned edit slab: zero upload, zero first-use mapping (:304).
- Base and derived facts are maintained by dependency cone (§21.8); Y1 0.7/2.4 ms p50/p95 (:305).
- The edited document is published synchronously; its scope reads Unknown until its row exists; single-document stat changes never invalidate plans (:306).

Note for the design: per-edit is "one file × all rules" as a batch. The objection to per-file execution is about the audit, where per-file execution forgoes cross-file batching. The edit batch is what arrives, and it uses the same programs and fused groups.

### §15 Measurement gate (308–312)
The SSC checker pattern: ceilings (cold CPU ratio, edit ratio, RSS, execution count), hashes, one process per engine, add/add/restore edit control. Gate corpora: full data-layer repository, CPython with 335 patterns, per-edit. DLRE `full_repository.py`.

### §16 Module map (314–333)
Already recorded in `00-review.md`. Notable: `engine/vtab/graph.rs` comes from DLRE reachability/forest and facts-plan Y1; `engine/facts/{catalog,derived,programs}`.

### §18 Tooling (339–370)
Summary of `spec-tooling.md` (read separately):
- **Memory pools sized at plan time** (tape columns, set stages, outputs, device queues, facts, SQL temp, caches); exceeding one yields `Unknown(resource)`; CI gates peak RSS, device bytes and Unknown rate.
- Tempo launch accounting is kept and checked against the plan's dispatch count.
- **Resident-batch pool:** CPU batches cut by resident bytes. SSC measured 760 MB of navigation data retained; a 4 MiB-of-source partition target cut peak RSS 2,933 → 1,233 MB at unchanged CPU.
- Debugging: text dumps and assembler, `explain`, interpreter trace, Metal debug mode, repro bundles, inspectors, the `rerail engine …` CLI.
- Testing: differential on finding keys across every backend and against the matcher and oracle; proptest and fuzz; v7↔v9np round trip; Blade vs php check.
- Profiling: fixed stage names, always-on executor counters, per-rule attribution in fused groups, SQLite stmt stats and `dbstat`, JSON-lines profiles.

### §19 Performance model (372–441)
Recorded in `00-review.md` and the earlier read. Key points:
- AST row 1.9/6.3/19 s against 56–120 s (fusion harness).
- M2 measured 0.26–0.40 ns/pn at 13–17 ns per evaluation.
- Warm 16-core gate ≈ 5–17 s with current executors.
- The 1–2 s target needs M3 facts by vtab, stable good-form cross-file plans, M5 native data flow, an executor at ≤ 1 ns/pn, the regex rewrites, the heavy-input rewrites, and 16-way parallelism.
- Cold is not the target case; parse and fact extraction are content-addressed and cached.

### §20 Tape residency and caching (443–549)
Tempo evidence (:447):
- first-use Metal mapping 213 ms against 6.1 ms of GPU time; reused buffers cut the wait 30–60× (`shared-sparse-bump-arena-design.md:14-35,260`);
- a 32 MiB / 30 s decompressed cache got **0 hits** on full scans and spent 37.6 worker-seconds decompressing in a warm run;
- heap decompression left 3.3 GiB of malloc high-water (`workspace-store-runtime-journal.md:536-681`);
- per-object mappings were rejected for VM metadata cost (:517-520).

**20.1 Forms** (:451-460):

| Form | Encoding |
|---|---|
| at rest | `tape_objects.frame` / `source_objects.frame`: one zstd frame (level 3), `raw_length` and SHA-256 outside |
| resident | the decompressed v9np blob in a slab, byte-identical; UTF-8 source |
| device view | the slab's `MTLBuffer`, the same pages |
| sidecars | in a slab: `line_starts` u32, derived `par`, `cidx`, text-hash and side-fact columns |

- No other copy exists during evaluation. Executors, the rehydration matcher, `source_text` and the vtab all read the resident form in place. `TapeDoc::restore` is only for the native matcher and oracles.

**20.2 Slabs** (:464-469):
- anonymous mmap regions, page-multiple (16 KiB on Apple Silicon), 64 MiB by default; larger objects get their own mapping; per-object mappings are otherwise forbidden;
- bump allocation at 16-byte alignment;
- **one zstd decompression directly into the slab**; intermediate heap buffers are forbidden;
- append-only; compaction copies live objects to a new slab, with at most two generations;
- release is `MADV_DONTNEED` + `munmap` after the `MTLBuffer` is gone; partial `MADV_DONTNEED` on a live slab is forbidden (Metal treats re-touched pages as new mappings: Tempo 84.94 s, `packed-result-bottleneck-profile.md:102`);
- no huge pages, no `mlock`; `mlock` does not keep a GPU mapping.

**20.3 Identity and admission** (:473-476):
- Source is keyed by (algorithm, digest); tape by (source digest, grammar_id, grammar_version, tape_format_version). Row ids are never keys.
- Admission decompresses, checks codec, `raw_length`, SHA-256 and the node count and width against `search_index_sets`, **once per admission**; there is no re-hash after that.
- A failed validation removes the row from use and increments `residency_corrupt`; the document is re-parsed.
- Loads are bulk: one batched store read for the misses; workers decompress outside any SQLite transaction.

**20.4 Pinning and device views** (:480-484):
- One `MTLBuffer` per slab via `newBufferWithBytesNoCopy(StorageModeShared)`; the deallocator and an associated object own the slab.
- **Pinned = the MTLBuffer is retained across dispatches and evaluations.** Buffers over resident objects are never created per batch.
- An optional warm-up command touches pinned slabs (`residency.pin`). `MTLResidencySet` only after Q-R2. Blits only if Q-R1 shows private storage wins, then once per admission.
- Executors bind slabs, not documents. Above the binding limit, an argument buffer of slab GPU addresses (Q-R3).

**20.5 Batches reference, never copy** (:488-490):
- The batch document table holds fixed records `(slab, tape_offset, node_count, width, source_offset, source_len, line_offset, derived_offsets…)` plus demand.
- Executors compute document-local indices. Nothing is concatenated or copied.
- Batches are width-homogeneous; u32-size documents are batched separately; kernels are specialised by width.
- CPU executors read the same slabs.

**20.6 Source and sidecars** (:494-497):
- Source is resident with its tape (free on UMA).
- `line_starts` once per source digest.
- Derived columns (`par`, `cidx`, text hashes) are sidecars keyed by tape digest, built on first demand on CPU or GPU, evictable before their tape, charged to `tape.derived`.

**20.7 Cache policy** (:501-511):

| Class | Eviction |
|---|---|
| `snapshot` (in scope of an admitted rule) | pinned; not evicted while Σ ≤ the `tape.resident` limit |
| `edit` (newest overlay version per open path) | pinned; superseded versions freed when leases reach zero |
| `transient` | LRU by last lease release, no TTL |

- Sharing is by content identity across workspaces in the process; cross-process sharing goes through the store (Q-R5).
- If `snapshot` exceeds its limit, the audit runs in **residency waves** (load, pin, execute, release), reported as `residency_waves`. Non-resident is never Unknown. A document larger than the device is `Unknown(doc_exceeds_device)`.
- Residency is a planner cost input, never a correctness input.

**20.8 Per-edit residency** (:515-518):
- The edited buffer is parsed and its v9np tape encoded **directly into an edit slab**, with the node count taken from the parse first. Source and line starts go into the same slab.
- The slab is pinned, so there is zero upload. Compression and the store write happen off the evaluation path.
- The previous version keeps its handle until its leases release; compaction never runs on the edit path.

**20.9 SQL access** (:522-525):
- The vtab resolves demanded documents to handles before xFilter and never SELECTs frames.
- The pure-SQL binding reads columns through the TVF `tape_column(doc, column)`, returning a blob pointing into the slab (no-copy destructor) under a statement-scoped lease.
- **Uncompressed tape columns are never stored in content-store rows.**
- The FTS5 kind, term and edge indexes stay in SQLite and are built from resident columns at publication.

**20.10 Accounting** (:529-534):
- Pools `tape.resident`, `source.resident`, `tape.derived`, `batch.doctable`, sized from Σ node_count × widths.
- Shared no-copy slabs are counted once for host and device; the ledger reserves before mmap.
- Profile fields: slabs, bytes, admissions, hits and misses, compactions, `residency.pin`, waves; Metal `bytes_bound`, `slabs_bound`.

**20.11 Forbidden** (:538): per-batch repacking (`MetalNode`), per-batch source concatenation, heap decompression, copying on hit, re-hashing, per-object mmap, TTL caches smaller than the working set, partial `MADV_DONTNEED`, `mlock`, fixed byte caps in code.

**20.12 Open measurements** (:542-549): Q-R1 shared vs private; Q-R2 ResidencySet; Q-R3 binding limit (31 buffers); Q-R4 slab size; Q-R5 file-backed shared snapshot image; Q-R6 `currentAllocatedSize`; Q-R7 pin cost per GB (~27 µs/MB extrapolated); Q-R8 cold admission throughput at 4/8/16 workers.

### §21 Named derived facts (551–597), decision 0005
- **21.1 Producers.** Exactly one producer per fact:
  - fact program: a rule-independent transducer program for base facts from tree structure;
  - native producer: reviewed code for facts needing a resolver or evaluator;
  - derived fact: declarative algebra compiled to SQL and run in SQLite.

  "Shared does not imply SQL": shared tree-walking analyses are fact programs; relational analyses (name-keyed joins, fixpoints over names, aggregates, membership-dependent) are derived facts.
- **21.2 Definitions.** `rerail.facts/v1` documents in `engine/facts/catalog/` and `packs/<pack>/facts/`. Closed algebra: from, join, left_join, anti_join, union, group_by/aggregate, project, stratified recursive, literal policy relations, with the Predicate AST. No SQL, no UDFs. The generated SQL is dumped with the plan.
- **21.3 Admission:**
  - concept-named, never rule-named;
  - ≥ 2 rules or a catalog family;
  - rows per document ≤ node count;
  - no rule-specific seeds.
- **21.4 Identity.** `fact_digest = H(lowered def ‖ schema with key and FDs ‖ input digests ‖ compiler version)`; native producers contribute id and revision; a name clash is a compile error.
- **21.5 Partition** is inferred: `document` or `repository`.
- **21.6 Materialisation:**
  - **table** by default; required with ≥ 2 consumers, recursion, or repository partition;
  - **inline** for one consumer, non-recursive, document partition;
  - **vtab** over native columnar data for base facts (M3).

  One physical owner per (doc or snapshot, digest). Narrow typed tables with atoms interned u32; no EAV or unions. Indexes come from consumer access paths. Demand-driven.
- **21.7 Cache.** Per document: `(fact_digest, tape identity[, path if read])`. Repository: `(fact_digest, input generation vector)`. Content-store tables `derived_facts` and `derived_fact_parts(digest, doc_key, complete, reason, rows)`, cascading with their source. Completeness per (fact, document).
- **21.8 Invalidation:**
  - the dependency DAG comes from the definitions;
  - an edit to `d` recomputes d's base facts and the demanded per-document derived facts;
  - repository facts update by **semi-naive delta over changed join keys**; recursive components use **delete-and-rederive**, or a full recompute above a threshold;
  - **only rules whose recorded read set intersects a changed (fact, key) re-run**.
- **21.9 Statistics.** `fact_stats(digest, column, rows, ndv, max_rows_per_doc, null_frac)`, written to `sqlite_stat1` (never ANALYZE), with validity intervals in `compiled_plans`.
- **21.10 Consumption.** Rules use `from: {fact:…}`, joins, `reaches … via`. AST rules use semantic keys, `fact:` semi-joins and capture projections. Transducers read SIDE columns, FACT sets and demand seeds. Strata run base facts → derived facts → rule programs → composition; boundary cycles are rejected.
- **21.11 Catalog** (`derived-facts.md` §d):
  - base fact programs: `py.def`, `py.call`, `py.param`, `py.annotation_atom`, `py.receiver_token`, `py.binding_site`, `py.import`, `py.class_base`, `py.decorator`, `py.export_name`, `py.return`, `py.flow_edge`, `file.module`;
  - native: `py.scope`, `py.binding`, `py.value`, `py.flow_exit`;
  - derived, document partition: `py.def_family`, `py.call_edge_local`, `py.module_const`, `py.local_flow`;
  - derived, repository partition: `py.import_res`, `py.call_edge`, `py.subclass_closure`, `py.export_closure`.

  Under ~0.7M derived rows on the gate corpus.

### Corrections to my withdrawn draft (`01-design.md`)
- It copied columns into a 16 B/node batch arena. §4:106, §20.5 and §20.11 forbid concatenating or copying. Batches are document tables over resident slabs, and the tape stays 8.25–8.55 B/node at rest, 0.93–1.01× v7.
- It treated the transducer as residual. §7:178-181 already defines the two-level model (instruction programs for bounded local tests and paths, the set stage for relations and nesting), and §8:216-219 already gives the Metal form. The draft's "set-form" claim restated PIAT's model and lost the parts PIAT had worked out: program boundaries following shared state, E1–E16, the verifier, capture-bearing demand, and the facts boundary.
- It ignored §21 (fact producers, admission, identity, cache tables, semi-naive delta / DRed invalidation, read-set re-run) and §20 (residency) entirely.

## spec-transducer.md (722 lines), read in full

### §0 Lowering principle (14–34)
Every condition goes to one of three places (:18-22):
- **transducer program:** local structure and text, all/any/not, relations with every stopBy, nthChild, bounded counts, non-recursive utilities, monotone single-axis recursion, capture equality within a match, fact-column reads; "bounded state per path, Metal-friendly, fusable";
- **named derived fact (SQL):** shared intermediates and relational analyses (scopes/qualnames, bindings/refs, intra-file call graphs, class hierarchies, data flow and fixpoints over names, joins between independent sets, aggregates);
- **native fact producer:** resolver or evaluator facts (scope binding, literal values, format strings, parsed literals).

Rules of thumb (:26-32):
1. local/navigational → transducer;
2. shared/relational → derived fact (a rule-local join uses L5);
3. **a new op must earn its place** (local, cheap on every backend, multi-rule, not better as a fact); the default answer is a derived fact;
4. facts flow both ways (programs produce base facts, consume derived facts through SIDE or seeds);
5. code conditions are AST conditions;
6. no SQL in rules;
7. shared tree-walking analyses are named fact programs.

Boundary test (:34): "a transducer program should read like a structural pattern; anything that reads like a query belongs in a derived fact."

### §1 Machine model in set mode (38–111)
- **Document columns** (:44-54): `kind`/`field` from `kf`; `flags` nibble (named, extra, error, missing) from the grammar code table; `size`; `end_pre = i + size`; `start`/`end` from base/off/tail; `par` (sidecar, stack pass); `cidx`, `cidx_named` (sidecar on first demand by NTH); `side_c` (fact columns).
- **Nodes** are (document, preorder) pairs; nothing is concatenated; no op crosses documents except DOC (:56).
- **Units** (:60-62):
  - program group G: the unit of fusion and dispatch; ≤ 65,536 words, a header, tables;
  - label ℓ: (rule, fragment), kind KEY/EXPORT/COUNT;
  - task (G, D, r): r from demand or the dispatch table.
- **Path** π = (pc, x, C captures, K cursor slots, Q repeat counters, σ sub-program stack). Navigation maps a path to a finite set of paths; FORK to two (:66-74).
- **Output** is a set over terminal events (:76-93), independent of path order, task order, run multiplicity and which run is found first.
- **Executor freedoms** (:95-100): stop a KEY label after its first emit; drop paths whose reachable labels ⊆ fired; any order and lazy enumeration. EXPORT ALL labels must enumerate every distinct tuple.
- **Completeness** (:102-107): Complete or Unknown(resource | verifier | fault). A KEY emitted before Unknown is a confirmed finding; EXPORT ALL in an Unknown task is Unknown.
- **Determinism** (:109-111): Out is a function; backends and fused/unfused produce the same set, which is the differential invariant.

### §2 Operations (115–343)
- **Encoding** (:119-130): `opcode | mode<<8 | operand<<16` (Tempo `transducer.rs:1111`). Existing opcodes 0x00–0x40 are listed at :122-125. New ranges:
  - tests 0x0b–0x0f (NTH, TEXT_ID, TEXT_DFA, TEXT_SET, TEXT_ID_EQ);
  - navigation 0x17–0x1f; sub-programs/calls 0x32–0x3f; set stage 0x50–0x57; documents/side facts 0x58–0x5f; pattern 0x60–0x6f; line documents 0x70–0x77.
- REPEAT_* is admitted in set mode only without capture-containment reduction (:132).
- **Base ops** (:138-160): STOP, KIND, FIELD, FLAGS, TEXT (value program), SIZE (byte length compare), ASSERT (capture program), FIRST/LAST, ABSENT_FIELD, SUPERTYPE, CHILD (fork), NEXT, FOLLOWING (fork), PARENT, DESCEND (fork), SAVE/RESTORE_CURSOR, CAPTURE, REPEAT_*, FORK (greedy has no effect in set mode), GOTO, EMIT.
- **§2.1 Axes** (:164-177): ANCESTOR, PREV, PRECEDING, DESCEND_KIND, CHILD_KIND, FOLLOWING_KIND, ANCESTOR_KIND, PRECEDING_KIND, CHILD_FIELD. `*_KIND` superinstructions are a contiguous kind scan over the axis range, filtering before forking.
- **§2.2 NTH** (:181-187): CSS An+B from an `nth` table {a, b}, reading `cidx`/`cidx_named` sidecars and an `nkids` column. `nthChild.ofRule` lowers to COUNT.
- **§2.3 EXISTS/NOT_EXISTS/ACCEPT** (:189-215):
  - **barrier scoping:** the sub-run sees a snapshot of the outer captures and cursors; nothing flows out; no EMIT inside; contiguous pc regions;
  - short circuit; NOT_EXISTS of an Unknown sub-run makes the outer path Unknown;
  - nesting ≤ 8 (`MAX_SUB_DEPTH`);
  - **Metal:** shallow (nesting ≤ 2, no DESCEND/ANCESTOR/FOLLOWING/PRECEDING inside) runs inline as nested loops with one cursor register per enumeration level; **deep ones, or any with an unbounded axis, are promoted to the set stage as IN_SET, so there is never an unbounded per-thread stack** (:212-215).
- **§2.4 CALL/RETURN** (:217-242):
  - utility table `{entry, end, params, bindings, group, axis, depth_bound}`;
  - recursion is allowed only with **monotone single-direction progress**: every recursive CALL is dominated by navigation along the declared axis, and none is under NOT_EXISTS/COUNT/negated CALL;
  - **recursive groups never use per-thread recursion; they are lowered to set-stage `FIX`** (depth-level waves on Metal), and call sites become IN_SET;
  - non-recursive utilities are inlined or kept as CALL regions run like EXISTS.
- **§2.5 COUNT** (:244-255): cmp × axis (child, named child, descendant, following, preceding, ancestor), with a count table {sub, c, stop}; early stop; c is also a work bound; descendant counts with a kind sub use COUNT_PREFIX `pc[end_pre(x)] − pc[x+1]`.
- **§2.6 Set stage** (:257-303): set definitions in the group header, evaluated before any program of a task that reads them; per document in the batch they produce S ⊆ [0, n). Read by IN_SET and COUNT_SET (counts from prefix counts or popcounts).

  | Definition | Meaning |
  |---|---|
  | `LEAF` | sub-program p with no unbounded axis, candidates from kind postings or a kind scan |
  | `HAS` | inner S, NEIGHBOR or END, stop P, field f, named |
  | `INSIDE` | the same, upward |
  | `PRECEDES` / `FOLLOWS` | IMMEDIATE or ANY, stop P |
  | `AND` / `OR` / `ANDNOT` | set algebra |
  | `COUNT_PREFIX` | prefix-count column |
  | `FIX` | monotone recursive equations |
  | `FACT` | posting list from the fact cache |

  Evaluation formulas (:280-291):
  - HAS END: posting form is two binary searches per candidate; bitset form is a reverse-preorder pass `sub[par(i)] |= sub[i] | [i∈S]`;
  - HAS with stop: an `anc_P` pass, then a reverse pass clearing at P;
  - INSIDE: a forward pass `anc[i] = [par(i)∈S] | (par(i)∉P ? anc[par(i)] : 0)`;
  - PRECEDES/FOLLOWS: a sibling-chain pass;
  - **FIX:** has-direction groups run deepest→shallowest, inside-direction shallowest→deepest, one wave per depth level (node-parallel evaluate plus scan), wave count = the batch's max depth, enqueued device-side.

  Representation (:293): a bitset (n bits per document, packed u32) or sorted postings (u32), chosen from the bound; both on every backend.

  **Multi-target packing** (:295): up to 32 or 64 sets of the same pass type, depth and stop in one pass as bit-planes (fusion.md: 42 subtree + 30 ancestor targets at 25–54×).

  Strata (:297): the longest path to a LEAF; one stratum's passes are packed together. Verifier (:299-303): acyclic except monotone FIX (no ANDNOT or negated IN_SET through recursion); LEAF has no unbounded axis; ≤ 16 strata.
- **§2.7 Lazy enumeration** (:305-309): normative on every backend. One resumable cursor (axis, origin, next, end) per enumeration; never one pending path per result. The pending set grows with program nesting, not subtree size. Measured 5.26M → 96k pushed paths.
- **§2.8 Side facts** (:311-320): SIDE_TEST (column cmp const), SIDE_EQ (column equal to a capture's column), TEXT_EQ (a 64-bit text-hash column first, then bytes). Fact columns (`fact_scope.owner`, `fact_def.qualname_id`, `fact_ref.target_id`, text hash) are per document digest, cached by digest, made resident before dispatch.
- **§2.9 DOC_GUEST/DOC_HOST** (:322-330): within a file's documents only, through the source map; grammar tracked per pc.
- **§2.10 EMIT** (:332-334): exports convert captures to typed values per schema.
- **§2.11 ORDER** (:336-343): position of x against a captured node (E16). **Withdrawn by 0005:** KEYS/IN_KEYS (E15) and FOLD_UP (E17), because each added an execution kind to every backend and the verifier. Joins use L5, name recursion uses derived facts (`py.call_edge_local`, `py.local_flow`, with `reaches`), "always raises" uses the native `py.flow_exit` through SIDE_TEST, and M7's quadratic case is served by L5's indexed join (M10).

### §3 Group header and tables (347–395)
- **Dispatch** `u16[CODES]` root-kind → entry pc, plus `any_kind[]`. Demand without roots starts one task per node whose kind is dispatched (**fused dispatch without an index**); with roots, only at R (:351-356).
- **LabelEntry** `{rule u32, fragment u16, kind u8, flags u8 (ALL), schema u16, budget u32}`; ≤ 4,096 labels; executor label bitsets of 64/128/256, and the planner splits larger groups (:358-366).
- **Reach masks** per pc, deduplicated; a path is dropped when R(pc) ⊆ Fired ∪ AllLabels. Without pruning, tries were 2–3× slower at small K (:368-377).
- **Other tables** (:379-391), u16-indexed ≤ 65,536: strings, predicates, capture predicates, u64, nth, count, count_set, sub-programs, utilities, bindings, side predicates, side-column list, set definitions `{type, params, repr, bound}`, export schemas.
- **Canonical digest:** SHA-256 over the normalised serialisation. It keys the plan cache and the dump tools (:393-395).

### §4 Side programs (399–430)
- **Value programs** (Tempo `value.rs:157`, :403-420): Equal, Prefix, Regex (byte NFA: Bytes, Look, Split, Capture, Fail, Match; Metal NFA frontier `metal_regex_frontier.metal`), Lua patterns. Set-mode additions:
  - TEXT_ID (once per distinct token text, a bit in a result table indexed by interned tid);
  - TEXT_ID_EQ;
  - TEXT_DFA (dense byte DFA where admissible, required on Metal; NFA is the reference);
  - TEXT_SET (multi-pattern combined automaton writing a bitset).

  Every value program declares a work bound; over it is Unknown(resource).
- **Capture programs** (`value.rs:17`): Equal, Text, AnyOf, Kind, Contains, Relation, SameLine, combined with CAPTURE_EACH/ZIP, AGGREGATE, SELECT_POLARITY (tree-sitter predicates). TEXT_EQ covers the common case (:422-426).
- **Output programs** (#set!, #offset!, #strip!, …) are used only where they change the finding key; otherwise they are dropped and recomputed in rehydration (:428-430).

### §5 Text and regex (434–507)
- **§5.1** (:438-452): of 113 whole-file text clauses, 107 are expressible as AST rules, 2 need small extensions, 4 are path-only, and **0 are genuinely textual**. The compiler rejects Content regex on a grammar language unless `textual: true`. The line state machine is removed; today's filters miss the 18% of raw matches inside strings. `# noqa` goes to integration suppression.
- **§5.2 TEXT_ID** (:454-465):
  - 6.0M identifier occurrences, 231,862 distinct; per occurrence 33–69 s, per distinct 0.11 s;
  - the tape producer writes a `tid` sidecar (u32 per token node) into a global append-only `text_vocab(id, bytes)` in the content store; interning 0.22 s per corpus at ingest;
  - result table `R[tid]` (W × u32), built from literal sets (396 of 444 by hash), Aho-Corasick, and a combined DFA (hybrid build 0.44 s, 3.0 MB, run 0.11 s); cached by (pattern-set digest, vocab length), extended incrementally;
  - the executor does one load; **no automaton runs on the GPU**.
- **§5.3 TEXT_DFA/TEXT_SET** (:467-479):
  - **`\b` is ASCII in the rule language**: then all 725 node and 51 value patterns compile to dense DFAs; with Unicode \b, 15 fail;
  - DFA tables of ≤ 64 KB go in constant memory (708 of 725);
  - TEXT_SET ≤ 64 patterns; determinisation blows up past ~450 (639 patterns: 676 s, 201 MB); RegexSet on whole files 92.6 s against 0.52 s separately;
  - rewrites: `^async\b` → keyword child KIND; f-string → TEXT_ID on `string_start`; token-in-large-node → has END + TEXT_ID; module literal alternations → `search_term_fts` prefilter.
- **§5.4 Line documents** (:481-492): for genuinely textual clauses, one node per line built from `line_starts` with memchr (0.49 s against 3.5 s); LINE_NEXT/PREV/HOST/OF. No data-layer rule needs them.
- **§5.5 Trigram** (:494-498): custom postings at 0.127 B/source byte (~0.62 B/node), 0.14 s for all 113 queries, keeping 6.2% of pairs; FTS5 trigram 25× slower and 16× larger. **Not built** for the current packs.
- **§5.6 Cost** (:500-507): cold ≈ 0.1–0.3 s, warm ≈ 0.01 s; today 9–17 s.

### §6 Lowering (511–604)
- **§6.1** (:515-540): the full table, inline vs set-stage lowering per construct. Notable:
  - `any` = FORK + branches;
  - stopBy rule = axis loop with NOT_EXISTS stop;
  - field = CHILD_FIELD;
  - matches with parameters = CALL with bindings (no set-stage form);
  - constraints = RESTORE + EXISTS;
  - cross-match capture equality in one document = EXISTS + TEXT_EQ, or L5 + FACT;
  - before/after = ORDER;
  - all-branches = SIDE_TEST on `py.flow_exit`;
  - compare = TEXT_EQ / ASSERT / SIDE_EQ.
- **§6.2 Semantic families** (:542-561): each is a **per-document fact producer writing node-keyed columns**, read by SIDE_TEST/SIDE_EQ, rule-independent, cached by document digest:
  - `resolves` (`scope_id`, `binding_id`, `resolved_target`; scope transducer program + native binder);
  - `binds` (`binding_site`, `value`, `annotation` node ids; SIDE read, jump, sub-program);
  - `value` (`value_kind`, `value_const`);
  - `arguments` (`arg_slot`);
  - `parse` (derived document per parsed literal, DOC_GUEST via=parse);
  - `host`/`embedded`;
  - `formatArgs`;
  - `layout` (line columns);
  - `filePath` (demand-time SQL scope);
  - `flow_exit` (never/always/maybe);
  - `calls` (in-file `callee_target` column; the cross-file closure is an SQL relation over exported (file, node, callee)).
- **§6.3 Pattern words** 0x60–0x67 (:563-582): P_KIND, P_TERMINAL, P_BIND (TEXT_EQ if already bound), P_SKIP, P_ENTER, P_NEXT, P_LEAVE (strictness levels smart/cst/ast/relaxed/signature), P_ELLIPSIS (one path per split; adjacent ellipses need ≥ 1 between).
  - Set mode removes the transactional capture environment.
  - Of 4.4k pattern atoms in the data-layer rules only 20 are structural; the rest are single metavariables (KIND + CAPTURE).
  - Metal: a per-thread register child-cursor stack of static pattern depth.
- **§6.4 Query forms** (:584-594):
  - Ast → programs;
  - Regex Content → AST rule or line document;
  - Regex Path → SQL scope;
  - SelectWhen → EXISTS per file, or a cross-file SQL semi-join;
  - RequireEach → NOT_EXISTS, or a cross-file anti-join;
  - Budget → COUNT_SET plus SQL totals over t_count;
  - **Relational:** of 52 clauses, 44 lower fully to the IR, 1 reads a fact column, 3 are IR plus a small SQL tail, 4 are genuinely cross-file.
- **§6.5 Lowering rules** (:596-604):
  - **L1** document-invariant hoisting (removes the 2.59 TB module-text rescan);
  - **L2** selection drop (first/latest-definition-wins dropped or restated as NOT_EXISTS + TEXT_EQ + ORDER);
  - **L3** literal allowlists as constants;
  - **L5** relational split: candidate-exporting programs (`(doc, key_atom, node)` for candidates only) plus a derived fact or SQL semi-join on (doc, key), returning findings or a FACT set; shared joins must use a catalog fact. Measured: legacy_execution_surfaces 1,290+ → 34 CPU s on the full corpus;
  - **L4** no tree copies: exports whose estimated rows exceed node count are rejected.

### §7 Op summary (608–645)
CPU and Metal implementations per op. Notable Metal forms:
- ANCESTOR is a `par` walk bounded by max depth;
- PREV/PRECEDING use `par` + `cidx`;
- `*_KIND` is a contiguous kind scan;
- EXISTS runs inline ≤ 2 levels, otherwise in the set stage;
- set definitions are segmented passes plus GPU-driven waves;
- DOC_* use the document table plus a binary search over segments;
- group headers live in constant buffers;
- the lazy cursor is a register;
- per-label budgets are atomic per-label counters.

### §8 Inputs and outputs (649–667)
- **Inputs per batch:** tape columns in place from slabs; `par`/`cidx`/line sidecars; source for text-reading programs; declared fact columns; the tid column and result tables; **kind postings from the index for the set stage**; demand rows.
- **Outputs:** t_match, t_export, t_count, t_outcome. Metal output placement is count → scan → scatter, with no atomics on order.

### §9 Verifier (671–684)
Ten rules: encoding; control flow; progress (every CFG cycle advances along an axis, from Tempo `cycle_progress.rs`, or is a bounded REPEAT); stratified recursion, ≤ 16 strata, ≤ 8 sub-depth; grammar typing; set mode; bounds (every loop bounded by a batch constant, value work bounds, arena bounded from stats); every backend implements every op; patterns; text rules.

### §10 Backends (688–696)
- Rust interpreter (reference);
- CPU JIT (arm64, from Tempo `native_ir.rs`);
- Metal (interpreter kernels, then MSL codegen from `native_ir.rs`): production on Apple GPUs and the per-edit target.

**The same abstract work units are charged per op on every backend**, so budgets give the same Complete/Unknown everywhere.

### §11–12 Constants and open measurements (700–722)
- Constants: ≤ 65,536 words, ≤ 4,096 labels, sub-depth 8, strata 16, CPU steps per path 2^20, work per task 2^26 (these become per-label budgets).
- Open: compiled evaluation (13–17 ns/eval at 74% interpretive); N1562 packed lowers none of TP/ASD/SAS (E2, E6, E14 FIX close the gaps); the 113-clause rewrite; pattern words vs `PackedPattern`; Metal cursor stacks and DFA placement.

### What this means against Tempo's Metal state (from the T6 survey earlier)
The spec already defines the Metal execution that avoids per-path continuation growth:
- lazy cursors (§2.7);
- EXISTS inline only when shallow and bounded, otherwise promoted to the set stage (§2.3);
- recursive utilities as FIX waves (§2.4);
- set-stage passes with multi-target packing (§2.6);
- dispatches O(nesting depth) per batch (spec.md §8).

Tempo's current executor (generations of successor paths with cursor and repeat frames, "dynamic frontier continuation") implements the pre-spec model. Confirm against T6 notes.

## spec-indexes.md (390 lines), read in full

Framing (:5): "the index layer has one job under 0004: file- and document-level candidate scope plus statistics for plan choice. Finding candidate nodes inside a document is the transducer's work: it scans the kind column. Per-node positional postings in SQL serve only the pure-SQL fallback."

### Part 1: inventory
**Main** (:11-57). `content_store/schema.rs`:
- `tape_objects(tape_id PK, source_id, canonical_hash_id, grammar_id, grammar_version, tape_format_version, codec_id, raw_length, frame BLOB, created_generation, UNIQUE(source_id, grammar_id, grammar_version, tape_format_version))` :73-86;
- `file_documents((tape_id, ordinal) PK WITHOUT ROWID, parent_ordinal, role_id, via_id, document_tape_id, source_map, host_errors)` :92-103;
- `snapshots` / `snapshot_files(snapshot_id, path_id, source_id, tape_id, PK) WITHOUT ROWID` :123-141;
- `workspace_overlay` :156-166;
- `search_index_sets(tape_id PK → tape_objects ON DELETE CASCADE, index_version, node_count, complete, has_errors)` :245-251;
- `search_kind_fts` fts5 contentless, `contentless_delete=1`, `detail=full`, `tokenize='ascii'` :252-254;
- trigger `delete_tape_search_index` :255-257;
- result caches: `candidate_checks`, `local_check_results`, `context_check_results`, `completed_*` (:182-238).

Main's token is `k{kind:04x}`, one per preorder node, so the FTS instance offset is the preorder ordinal (`transducer.rs:525-548`). Local codes are 10 bits (`tape.rs:31`), `kf = code | field<<10` (`tape.rs:825`), ERROR decodes as `u16::MAX`. `POSITIONAL_INDEX_VERSION = 2`.

**Main's writer and invalidation** (:32-37):
- a background backlog `TAPES_WITHOUT_SEARCH_INDEX_SQL` (`search_index.rs:18-28`) writes ≤ 64 tapes per transaction;
- an undecodable tape gets an incomplete set;
- **the invalidation key is the tape id, content-addressed by source × grammar version × tape format**; a delete trigger follows GC;
- derived documents are ordinary tape rows.

**Main's reader** (:39-51): file-level presence only (`search_index.rs:202-260`, `workspace/persisted.rs:330-380`). A file is skipped only if every AST clause answers Absent; errors, an incomplete set or a version mismatch answer Unknown; cross-document clauses are never narrowed.

**Main's problems** (:53-57): positions stored but unread; language-local tokens (spike: 43.9% foreign instances, 13% tax); no statistics, ANALYZE or optimize; no value or term index.

**DLRE private store** (:59-83). A temp SQLite per corpus that copies source and tape again:
- `ftsk`, `ftsk_inst` (bypassed);
- `file` copy;
- `fk` kind bitmap;
- `file_kind_counts(fid, code, n, postings BLOB delta-varint)`, the anchor source;
- `kfreq`, read by no planner;
- `parovf`/`sizeovf`;
- `vidx`/`vlex`/`vcov` (value postings, `TERM_CAP = 96`);
- `vocabulary_*`;
- `decoded_*` (one row per decoded node, the spike's deleted design).

Tokens are global base36. **Shape chosen at run time by `count(*)`** (trap T18): 537 s anchor selection + 137 s index prep against 58 s matching. `ANALYZE temp.*` after stages (D50).

**N1562** (:85-88): constant estimates (100,000, `i64::MAX`); statistics designed but not built.

**SSC** (:90-93): in-memory `PlannerStats` (kinds, edges) rebuilt per batch in Rust: the right statistics in the wrong place.

**Moti Y1** (:95-98): a fact relation, not a statistic.

**Spike** (:100-125):
- two-stage execution: postings generate candidates, the tape confirms; never under-produce;
- bare-kind FTS 2.60 B/node, positions = preorder, phrase exact, NEAR not recall-safe; file pruning saturated at 1.03× *in its SQL executor*;
- **kidx** positional posting per (kind[, batch]) with columnar fid + preorder deltas, 1.61 B/node PG / 2.40 MariaDB; took an identifier anchor from 1.8 s to 64 ms; absolute preorder rejected (+2.6–4.2 B/node);
- Option B global codes in the index only; the tape stays local (widening `kf` would cost +16.7%); removes the 13% tax (565 → 533.5 ms);
- statistics: `t_kfreq`, `efreq(lang, parent, child, field)` at 0.039 B/node (reverse anchor 1,061 → 844 ms); planning statistics are per-file or running aggregates surviving restart, never a corpus scan (C7); planning is one statement with a ROW_NUMBER argmin (S241);
- anchor rule: rarest mandatory concrete variable (112 → 33 s, 1.05 rows per answer), value before kind;
- vidx: case-exact text, linear hashing 0.91 B/node on PG; SQLite RUNS postings 10.57 → 5.67 B/node; exact-token 15.8× faster; vocabulary per language; vidx was 87–89% of indexing time, fixed by size-tiered runs;
- regex probes 3.5× for 0.316 B/node; native regex 2.90× over the UDF boundary; trigram rejected;
- word FTS (srcfts 1.34 B/node) deleted because unread;
- **budgets: tape 6 B/node; all secondary indexes ~5 B/node** (kidx 1.4 + vidx 0.9 + flags 0.25 + trigram 0.9 + 1.55 spare); a new index must fit or replace something ≥ its size; measured SQLite secondary 6.59 B/node;
- line checkpoints every 4 KiB: 0.004 B/node.

### Part 2: spec
**§2.0 Principles** (:131-138):
1. **One store**: everything in the content store keyed by `tape_id`; DLRE's private store retired (ftsk → search_kind_fts, kfreq → kind_freq, vidx/vlex → search_term_fts; decoded_*, ovf, file copies and fk deleted);
2. per-document granularity;
3. **indexes prune and estimate; they never decide**: a clean Absent may skip, anything else is Unknown;
4. statistics in SQL, read at plan time only (compiler batched lookup + xBestIndex); no run-time count(*), no Rust rebuild;
5. **no per-node postings in SQL on the hot path; inside a document the transducer scans the kind column**; positions are kept only while the fallback and Q2 need them.

**§2.1 Global codes** (:140-148): `gcode = lang_tag*1024 + local`; token `k%06x`; the tape keeps local codes; the compiler maps (dialect, kind name) → gcode.

**§2.2 Kind index** (:150-174):
- reuses `search_kind_fts` plus `search_kind_vocab` (fts5vocab 'row'), one row per document;
- `POSITIONAL_INDEX_VERSION = 3`;
- `detail=full` kept for now (vocab row gives `doc` and `cnt`; detail=none would lose cnt and need `kind_freq`);
- Q2 decides positions;
- migration v2→v3 is backlog-driven; older binaries get Unknown.

**§2.3–2.4 Scope** (:176-209): a `file_scope` view (primary + derived documents); a `workspace_docs` temp view over snapshot_files minus overlay plus overlay.

**§2.5 `edge_freq`** (:211-229): `(parent_gcode, child_gcode, field) → edges`, STRICT, WITHOUT ROWID, with a child index; a per-document `edge_counts` LEB128 blob in `search_index_sets` for exact subtraction on GC. Q3: if the blob exceeds 0.15 B/node, go add-only and rebuild above 25% drift.

**§2.5b** (:231-241): `kind_size(gcode, nodes, sum_size)`; `backend_costs(backend, device, op, ns, measured_at)` from install-time calibration; `plan_feedback(program_digest, selectivity, cost_ns, at)`.

**§2.6** `kind_freq` is conditional on Q2.

**§2.7 `search_term_fts`** (:252-269): contentless, detail=none, ascii; the distinct words of named leaves, prefixed `t{lang:02x}`, leaves > 96 bytes skipped (`term_complete = 0`). It replaces vidx/vlex and serves document pruning plus value-anchor statistics. Q4 budget ≤ 1.3 B/node.

**§2.8** (:271-275): per-node probes in TEXT side programs; per-document required words via 2.7; no trigram.

**§2.9 Compiler statistics** (:277-299):
- `stat_docs` view;
- point lookups on `search_kind_vocab` (0.27 ms), `edge_freq`, `search_term_vocab`;
- workspace subset counts only if Q6;
- one batched statement per compile;
- decisions: rarest mandatory variable (ROW_NUMBER argmin), seeded vs scan, reverse/edge anchor, value-before-kind, probes only when selective (~2/3 rejected);
- never scan the vocabulary.

**§2.10 xBestIndex** (:301-307): `est_matches_per_doc = cnt/doc` of the anchor gcode; `estimatedRows = min(|IN|, est_docs) * est_matches_per_doc`; ordering declared only on doc.

**§2.11** (:309-312): `PRAGMA optimize=0x10002` at open; optimize after a publication batch touching > 10%; `analysis_limit=1000`; never ANALYZE temp; don't trust SQLite's planner on CTEs, recursive CTEs or TVFs.

**§2.12 `compiled_plans`** (:314-330): (rule_hash_id, compiler_version, grammar_set_hash_id, index_version) PK; plan BLOB, decisions BLOB (stat key, value, valid range), compile_ms; batched re-validation.

**§2.13 Publication** (:332-343):
- background: one transaction per ≤ 64 tapes (set row, kind FTS row, term FTS row, edge_freq merge);
- edit: synchronous publication for the edited document; scope Unknown until its row exists; one document's stat changes are ignored for plan validity;
- Q7 p95 latency;
- delete: triggers plus edge_freq subtraction in Rust GC.

**§2.14 Budget** (:345-364): total ≤ 4.1 B/node (kind 2.6 + term 1.3 + edge 0.15 + sets 0.01).

**§2.15 Q1–Q8** (:366-377).

**§2.16 Deleted** (:379-390): DLRE store tables, the run-time count(*), ANALYZE temp, SSC PlannerStats (logic kept, fed from SQL), N1562 constants.

### OPEN CONFLICT: positional anchor postings
- `spec-indexes.md` §2.0.5 and :5 (written 2026-10-25 under 0004) say per-node positions in SQL serve only the fallback, the transducer scans the kind column inside a document, and DLRE's private store is retired.
- Decision 0006 (2026-10-25, accepted, after the user rejected making the index opt-in) says the positional anchor index (kind postings, value postings, FTS kind index in `matcher/anchors/sql`) is always built, never optional, and stays on disk. Its basis is spike topics 3, 5 and 7 (kidx, "vidx is mandatory", rarest anchor, value before kind).
- `spec-transducer.md` §2.6 LEAF and §8: set-stage candidates come "from the kind postings (the index) or a kind scan".
- shape-fixes §14–§16 and 0007 continued optimising the DLRE anchor SQL lowering.

Resolution needed, and **not mine to make silently**. A consistent reading:
- per-node postings (kidx-style, columnar fid + preorder deltas; vidx value postings) are always built and live in the one content store (§2.0.1), not a private temp store;
- they feed the set stage's LEAF candidates and the planner's anchor choice;
- the "kind-column scan" is the alternative the planner may choose per plan when cheaper (Q2 measures it);
- what is retired is the private store's copies (`file`, `decoded_*`, ovf) and run-time count(*), not the postings.

To check with the user, citing 0006 and the user's memory note `anchor-index-mandatory`.

## spec-fusion.md (326 lines), read in full

### Constraints (:16-20)
- findings are a set;
- no CPU/JIT fallback from Metal; unbounded work uses GPU-driven continuation (C2); exhaustion is `Unknown(resource)`; **the planner may resubmit on Metal with a smaller batch or a split group**;
- the CPU interpreter is the reference and oracle, not a fallback; the CPU JIT is first-class for CPU-only deployments.

### §4 Fusion algorithm (:30-176)
The input is the per-rule programs assigned to one batch, each with its scope digest. The output is groups with set stages plus a demux table.

- **§4.1 Normalisation:** lower; peepholes; canonical test order (KIND, FIELD, FLAGS, NTH, SIDE_TEST, SIZE, TEXT, ASSERT); canonical capture numbering; dead-code removal; digest. **Equal digests are deduplicated into one program with several labels.**
- **§4.2 Grouping:** partition by (scope_digest, grammar, backend_profile); split by label budget (≤ W on Metal) and by code size (≤ 65,536). Identical scope only (1.45× against 0.86×).
- **§4.3 Prefix tries** per root kind: share the longest common prefix (identical word and table content); j−1 FORKs at each divergence; sub-program regions are shared only when identical by digest.
- **§4.4 Boolean fusion:**
  - OR = FORK chain with EMIT per branch;
  - AND = ascending cost/(1−sel), where a conjunct binding captures read later is inlined, else wrapped in an EXISTS barrier;
  - NOT = NOT_EXISTS;
  - inline vs barrier is a legality decision.
- **§4.5 Utilities:** recursive → FIX + IN_SET; relation target → LEAF/HAS/INSIDE set; `s·n ≤ 256 words` or `n ≤ 8` → inline; else a CALL region shared by digest.
- **§4.6 Relation lowering cost:**
  - cursor `Σ E|axis(r)|·c_step`;
  - kind scan `Σ E|axis(r)|·c_scan`;
  - set `n·c_pass/T + |Roots|·c_test + cost(LEAF)`;
  - the set stage is mandatory if the inner rule has an unbounded axis in a sub-program at nesting > 2, or is a recursive utility;
  - T = targets packed per pass, so packing makes the set stage cheaper as more rules share it.
- **§4.7** Deduplicate set definitions by (type, params, inner digests); strata; pack passes per stratum by (type, depth, stop, field); dense labels; R(pc) by backward dataflow.
- **§4.8 Cost model** (:122-147):
  - n from `workspace_docs ⋈ node_count`;
  - N_k, D_k from `search_kind_vocab`;
  - f from `edge_freq`;
  - s_k from `kind_size`;
  - sel from `plan_feedback` EWMA, else the prior `min(1, D_k'/D_k)`, else 0.5;
  - c_step, c_scan, c_test, c_pass, t_task, D_disp, δ_div from `backend_costs`.

  Decisions: AND order; fuse a trie when separate > fused × 1.1; postings when `bound(S)·32 < n`, else bitset; bound propagation (HAS/INSIDE ≤ outer kind count, AND = min, OR = sum). `plan_feedback(program_digest, backend, selectivity, ns_per_root, samples, updated_at)`.
- **§4.9 Plan-cache key:** SHA-256(sorted member digests ‖ fusion_version ‖ ir_version ‖ grammar_set_digest ‖ backend_profile ‖ stats_bucket_vector), with log2 buckets of every statistic read.
- **§4.10 Demux:** t_match/t_export/t_outcome per (group, label, doc), joined to `labels(group, label, rule, fragment)`. Outcomes are per label, so one label's budget doesn't poison the others.
- **§4.11 Priority, revised by M2** (:166-176):
  1. interned text tests and L5 indexed joins;
  2. recursive tries with conjunct ordering (2.95 → 0.26 ns/pn); a single-level trie fails at 326M evaluations;
  3. hash-consed sub-rules;
  4. memo columns only for expensive non-text sub-rules (~94 ns per memo hit vs 5.9 ns per mask lookup; promote to a mask when lookups > nodes × cell/lookup);
  5. masks last.

  The fusion-model order was: the hash-consed graph with lazy per-file memo columns (1.4–3.1×, "required"); masks above threshold; tries 0.9–1.2× on data-layer.

### §5 Legality table (:178-199)
Each transform preserves Out as a set and may change multiplicity, order, witness and work. Illegal cases and responses:
- first emit / reach pruning: illegal for EXPORT ALL and COUNT;
- EXISTS barrier: illegal if a later conjunct, export or capture predicate reads a capture bound inside, so inline instead;
- OR short circuit: illegal for ALL exports;
- conjunct reorder: respect capture def-before-use;
- NOT_EXISTS of an Unknown sub-run is Unknown;
- relation → set stage: illegal if the inner rule binds captures used outside (e.g. a metavariable shared between the outer pattern and the has target), so keep it inline as a cursor;
- utility as set target: illegal if it binds captures the caller reads;
- COUNT → COUNT_SET: illegal if sub reads outer captures;
- ordered semantics are expressed as `A ∧ NOT_EXISTS B'` positionally.

**Captures needed above the file (t_export) are the main legality constraint**: an exported capture pins the path, and the label must be ALL unless the schema is a function of the root alone.

### §6 Limitations (:203-267)
- **§6.1** 65,536 words and table entries; K=512 measured 1,750 words; split by root kind; nesting > 8 → set stage; strata > 16 → split across two dispatch rounds with an intermediate set kept resident.
- **§6.2** Labels: bitsets of 64/128/256; split above; dedup-in-consolidation is not used on Metal (it loses early stop and pruning).
- **§6.3 Metal divergence:** δ_div bounds live branches; **tasks within a dispatch are sorted by root kind so that a threadgroup executes one trie**; split sub-tries by second-level kind or field when divergence exceeds gain.
- **§6.4 Register pressure:** per-thread pc, node, a cursor per enumeration level (≤ MAX_SUB_DEPTH+2), ≤ 16 live captures (Tempo's capture arena for more), label bitset, ≤ 4 counters. The verifier computes max live state; over budget, split or move to the set stage (device memory, no per-thread state); threadgroup memory only for scan tiles.
- **§6.5** Nesting d needs d strata (`d·n·c_pass/T`, selectivity-independent) or d inline cursor levels (selectivity-weighted), chosen per level.
- **§6.6** In-document capture joins: all 29 per-file joins in the relational clauses are expressible. Use EXISTS + TEXT_EQ/SIDE_EQ from the reaching side, else L5 (a shared join is a catalog fact), chosen by cost, because a per-candidate EXISTS over a function body can go quadratic. Across documents: a SQL hash join over t_export.
- **§6.7 Cross-document is never fused.** A group runs within one document (DOC_* within a file only); cross-file is SQL over t_export, fact_* and graph vtabs.
- **§6.8 Non-monotone recursion** is rejected by the verifier; "all branches raise" becomes a native producer or derived fact (`py.flow_exit`) via SIDE_TEST; rules are restated (bounded unrolling depth, or an engine fact).
- **§6.9** Value programs are ordered last among tests; DFA tables are deduplicated per group on Metal.
- **§6.10 Per-label budgets:** steps are charged to every label in R(pc); an exhausted label goes Unknown and leaves R; the task continues. A whole-task arena exhaustion marks unfired labels Unknown; **the planner resubmits that (group, document) set on Metal with the group split, never rerouting to CPU**; single-program exhaustion is reported as a defect in the executor's memory model.
- **§6.11 Per-edit runs:** a single file × all rules gives small n, so the cost model selects fused dispatch without an index plus set-stage passes for the one document. Plans are cached per (scope digest = {file}) bucket in a per-edit plan class. Statistics from the file's own tape may refine decisions per run without recompiling (the plan stores alternatives keyed by bucket).
- **§6.12** Executor nondeterminism affects only multiplicity and order.

### §7 Obligations (:271-311)
- **Differential invariants:** Out(fused) = ⋃ Out(members) per label; all backends identical; set-stage lowering = inline lowering. Matrix: conformance packs, every data-layer rule, random fused groups of 1/2/8/64/512.
- **Fusion fuzzer:** random canonical programs from the AstMatcher grammar; shrinks to minimal fixtures; CI on CPU, nightly on Metal.
- **`dump-group`:** header, tries as trees with kind and field names, listing, set definitions by stratum with representations and bounds, cost decisions with stats and intervals; `--format json`, `--diff`; generated from the serialised group.
- **Counters** per label (tasks, paths, steps, emits, exhaustions), per set (pass time, size, representation) and per dispatch (time, threadgroups, arena used/reserved), exported as `t_profile(group, label|set, metric, value)` into `plan_feedback`. Flame attribution by labels; shared prefixes apportioned by reach count.
- **Verifier fixtures** positive and negative per rule, with specific error codes.

### §9 Implied changes (:321-326)
`kind_size`, `backend_costs`, `plan_feedback` tables; the E10 routing-to-CPU language removed from spec §6 and §8.

### Note: what "batch" means in the spec
- A batch is a set of documents × the program groups assigned to them (§4 input: "the per-rule programs that the planner assigns to one batch").
- Within a batch, tasks are (group, document, root), and set-stage passes run per document over the batch.
- **"A group runs within one document"** (§6.7) means a task never crosses documents. Dispatches cover all documents of the batch: spec.md §8, "Dispatches are O(nesting depth) per batch, in one command buffer".
- For an audit, batches are cut by resident bytes (spec.md §11, §18), never by file count.
- Per-edit is the batch that arrives (edited file × all rules, §6.11), with the same groups and plan class.

---

## spec-tooling.md (313 lines, read in full) — proposed spec.md §18

Source: `Pulling-it-all-together/docs/rule-engine-direction/spec-tooling.md`. Governing positions (:3-6): there is **no Metal → CPU/JIT fallback**; resource exhaustion is reported as `Unknown` and counts against coverage; memory limits, debugging, testing and profiling are first-class.

### Inventory verdicts (Part 1)
- Reuse from main/DLRE (:18-35):
  - `code-rule-profile` feature;
  - `profile::trace` spans (`RERAIL_PROFILE`, `RERAIL_TRACE <json>`), with phase names replaced by a fixed taxonomy;
  - `Timing{wall,cpu}` with `CLOCK_THREAD_CPUTIME_ID`;
  - `profile/memory.rs` (getrusage, jemalloc);
  - `profile/sql.rs` statement profiler with deferred EXPLAIN;
  - `estimated_bytes` on every cache object;
  - jemalloc;
  - `scripts/profiling/run.py/report.py/compare.py`;
  - DLRE `full_repository.py` as the measurement gate.
- Budgets change meaning: they only ever produce `Unknown(budget)` and are never semantics (:32, decision 0001).
- Reuse from Tempo:
  - `validate` (`transducer.rs:1534`) as the verifier core (:41);
  - launch-guard accounting and its bypass tests (`metal_launch.rs:18-185,296-376`), but the fixed caps go: 200/600 launches and `METAL_BUFFER_LIMIT = 2_000_000_000` (:42);
  - the semantic conformance suite (:45).
- Reuse from N1562 (:43-44):
  - the per-task `Status`/`Evaluation` shape (error, pc, arena, task, workUsed);
  - the `reserve_shared/transient` atomic quota idiom.
  - Its **14 per-path arena kinds go away** with path enumeration (cites `research/metal-recursion.md`).
- Absent (:47-48,56,60): IR dumps/disassembler, Metal counters/GPU capture, `dbstat`, fuzz/property tests.

### §2.1 Memory limits (:66-92)
- Every evaluation allocation belongs to exactly one budgeted pool. Limits are computed at plan time from statistics. Exceeding a limit **never reroutes**: it gives `Unknown(resource: pool)`.
- Pools (:70-80):

  | Pool | Scope | Sized from / enforced by |
  |---|---|---|
  | `tape.resident`, `source.resident` | process, shared host/device, no-copy slabs counted once | Σ node_count × v9np widths, Σ raw_length of the snapshot and edit classes (§20.7). Residency waves; never Unknown for non-residency |
  | `tape.derived` | — | par, cidx, line starts, text hashes; evicted before their tapes |
  | `batch.doctable` | per batch | the planner splits the batch |
  | `set.<stage>` | per batch, device | postings 4 B/elem or bitmap n/8, whichever is smaller; `reserve_*` |
  | `output.<relation>` | — | count → scan → scatter, so overflow after the count is impossible; a count above the limit gives Unknown(resource) for those documents |
  | `queue.<program>` | — | unbounded programs only (device-side enqueue); bound ≤ nodes × reachable states, recorded by the verifier |
  | `facts.<family>` | per document digest, host | memo LRU under a global cap |
  | `sql.temp` | — | SQLITE_FULL → Unknown(resource: sql) |
  | `plan.cache`, `consolidation` | — | LRU |

- Global caps (:82):
  - `engine.limits.device_bytes` = 50% of `recommendedMaxWorkingSetSize`;
  - `host_bytes` = 25% of RAM;
  - the planner packs batches so that Σ pool limits ≤ cap.
- Rules (:84-87):
  - no hard-coded pool sizes;
  - every cache object has `estimated_bytes()`;
  - Unknown(resource) carries (pool, limit, requested);
  - CI fails above a per-corpus Unknown threshold.
- Gates (:89-92): peak RSS and device bytes; Unknown rate per reason; reported dispatch count == the launch guard's observed count.

### §2.2 Debugging (:94-151)
- `rir` text IR (:96-113):
  - stable, sorted, diffable;
  - shows programs (`KIND`, `EXISTS @sub`, `EMIT L0`), subs (`DESCEND_KIND yield stop=@stop_scope`), groups (scope digest, members, dispatch table by kind, labels, trie) and **set plans** (strata, e.g. `S0 = postings(kind=yield)`, `S1 = subtree_has(S0, stop=anc(...))`).
  - This confirms that set-stage strata run off kind postings.
- Commands:
  - `rerail engine dump`;
  - the assembler, round-trip exact, which runs the verifier (:118);
  - `explain`, which prints the `decisions` blob of `compiled_plans` with statistic values and validity intervals: anchor choice and alternatives' counts, seeded vs scan, fusion groups, mask vs per-root scan, batch formation, pool limits; with `--path`, the scope result, demand row and outcome (:120-128).
- Executors (:130-137):
  - the interpreter is the reference, with `--trace` and `--step`;
  - JIT `--dump-jit`;
  - Metal debug mode keeps the per-task status records, compares dispatch counts, reads back arena high-water marks, and supports `--gpu-capture <batch>` (MTLCaptureManager).
- Repro bundles (:139-147):
  - triggered by Unknown(resource), verifier rejection, executor error or a differential mismatch;
  - contents: rir, tape columns + source, demand rows, stats snapshot, plan decisions, expected/actual keys;
  - `rerail engine replay` runs with no store.
- `rerail engine tape` and `index --check` (recounts vocab and edge_freq against the tables) (:149-151).

### §2.3 Verifier (:153-168)
- Runs at compile, in the assembler, and at load under debug_assertions.
- Obligations:
  - table bounds;
  - progress on every cycle;
  - bounded CALL depth (E6);
  - an acyclic rule graph, except monotone single-direction recursion;
  - set mode only;
  - every loop bounded by a batch constant, otherwise the program is **unbounded** and must declare a queue pool bound;
  - capability intersection, where **every program must be Metal-capable**: a non-Metal program is a compiler bug, not CPU-routed;
  - plan pool sizes cover static needs.

### §2.4 Testing (:170-218)
- Differential pairs on finding-key sets, comparing only (rule, file) pairs that both sides report Complete (:172-184):
  - interp ↔ JIT ↔ Metal;
  - fused ↔ unfused;
  - vtab ↔ pure-SQL binding;
  - engine ↔ ast-grep matcher;
  - engine ↔ data-layer oracle.
  - Every mismatch writes a bundle.
- Corpora: fixtures for the inner loop; the gates are data-layer (9,021 files) and CPython (335 patterns) (:185).
- Goldens: rir dumps and explain output against frozen stats, updated by flag only (:187-190).
- Property/fuzz testing with proptest and cargo-fuzz (:192-198):
  - random programs, interp = JIT = Metal;
  - asm round trip;
  - verifier mutation;
  - v9np encode/decode identity and v7 ↔ v9np node equivalence;
  - set stage = per-root walk.
- Tape (:200-202): v7 ↔ v9np round trip over the gate corpora (v8 checksum method); Blade compiled-view tape vs tree-sitter-php.
- Targeted per-module commands (:206-218); full suites, gates, fuzzing and Metal-on-gate run at phase gates only, once, in the background.

### §2.5–2.7 Outcomes, profiling, CLI (:220-284)
- `t_outcome` reasons: `complete | budget | resource(pool) | parse | unsupported | timeout | executor_error`. The coverage report counts them per (rule, reason) (:222-226).
- Fixed span taxonomy (:234-247): `compile` (separate), `plan.validate` (stat point lookups), `scope`, `demand`, `upload`, `program`, `setstage`, `queue`, `facts`, `sql`, `consolidate`, `rehydrate`.
- Always-on counters (:249-252):
  - interpreter/JIT: instructions, nodes, paths;
  - Metal: dispatches (reported vs observed), command buffers, waves per queue, GPU time per command buffer, arena high-water, occupancy, bytes up/down;
  - fused groups: per-label attribution.
- SQL: `sqlite3_stmt_status` VM steps, full-scan steps, sorts, autoindex; EQP on first sight; `dbstat` bytes (:254-257).
- One JSONL profile with record types run, span, counter, memory, outcome-summary, stmt, decision and gate (:259-269).
- Proxy metrics are marked `diagnostic`. The gate reads only end-to-end CPU, wall, RSS, device bytes, parity, Unknown rate and compile time (:271).
- CLI: dump, asm, explain, run, replay, tape, index --check, diff (:275-284).

### Removed (:309-313)
Tempo's fixed caps; N1562's 14 arena kinds; SSC's duplicate EXPLAIN; the authored-SQL budget scripts.

### Pertinence to the final shape
- No fallback plus "every program Metal-capable" means the IR and lowering must keep every program in set mode with bounded or queue-declared loops. A per-file CPU path is not an escape hatch.
- Pools sized from statistics tie tooling to the index/stats layer: kind postings give `set.<stage>` bounds.
- The `tape.resident` pool is defined over **no-copy slabs counted once**, consistent with §20. Any design that copies columns into a batch arena would double-count and violate it.
