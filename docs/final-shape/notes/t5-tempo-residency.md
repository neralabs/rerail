# T5 notes: Tempo tape residency, workspace store, arenas, anchor indexing

Research track T5 for the final Rerail rule-engine shape. Read-only extraction.
Source root (abbreviated `TD/` below): `/Users/jeremy/orca/workspaces/nera-landing-2/ner-1562-scale-test-the-re-railer/tempo/docs/`.
Native source root (abbreviated `TN/`): `/Users/jeremy/orca/workspaces/nera-landing-2/ner-1562-scale-test-the-re-railer/tempo/native/src/`.
Citations are `file:line`. Measurement marks: **M** = measured in the source, **D** = derived/computed from measurements in the source, **A** = aspiration/projection/estimate/target (not measured). "Logical estimate" byte counts from Tempo's profile feature are marked M(est) because the source itself says they are accounting estimates, not allocator/RSS bytes.

---

## 1. TD/phase-a-anchor-indexing.md (629 lines)

### What Phase A is
- Goal: revision-correct structural index selects where an AST policy could match; existing matcher runs only on candidates; preserves findings/captures/failures (:7). Execution optimization, not new language/matcher (:9).
- Before Phase A: `make_file()` filters policies by scope+language; `root_kinds()` extracts possible root kinds; `Candidates.by_kind` dispatches checks per node; `unrestricted` holds checks w/o kind constraint; `evaluate_ast_file()` still traversed entire tree (:15-21).
- Out of scope: replacing resident trees with tapes (Phase B), matching directly on tapes (Phase C), joins/counts/absence/cross-file (Phase D+), finding deltas, DBs/network in edit path, 1 GB memory target (:78-85).

### What is indexed (current "predicate-aware index", :91-118)
- Hybrid positional value/structure index, not kind-only and not file-word filter (:95).
- Spike sections adopted: S073/S256 per-kind distinct-value vocabulary + node-aligned postings; predicate resolved once per distinct value, `(regex, kind)` results cached within the file pass (:97). S163/S166 necessary text constraints derived from regex HIR via `regex-syntax` (min byte length; max length only with both absolute anchors; bounded alternatives for absolute-start literal prefix) (:98). S274/S275 kind/relation-scoped text domain; small values in ordered case-exact vocabulary; large values keep node positions + one 8-Unicode-char prefix posting per node when a predicate requests prefix indexing (:99). S207/S209/S216/S217 concrete Smart-pattern parent paths, reject wrong kinds while climbing, conjunct ordering by estimated positional work, compare vocabulary scan vs checking small candidate domain (:100).
- In-memory over Tempo's existing tree, not SQL/tape (:102). Vocabulary includes interior spans as well as leaves. Values >256 bytes go to explicit residue with full-text confirmation; domains of ≤8 nodes use direct text checks instead of vocabulary (:102). These cutoffs "select an execution method, not supported input" (:102).
- Not added: file-level word filtering, S265 dictionary-trigram, Bloom probes, persistent cross-file dictionaries, replacement matcher (:104).
- Code: `policy_dsl/anchors.rs` (planning + temporary index), `policy_dsl/anchors/text.rs` (HIR analysis) (:106). `AstMatcher` regex arm holds same `regex::Regex` as `RegexMatcher` (:106).
- Lifetime: "All file-local positions, vocabularies, and predicate caches are discarded after evaluation" (:118). "No structural index is retained across file evaluations" (:141). Decision: "Build one temporary, borrowed-node index per evaluated file, shared across that file's checks. Drop it after evaluation ... retaining indexes for untouched files has no demonstrated benefit and would worsen resident memory" (:228). Rebuild changed file's index after reparse, no incremental posting surgery (:229).
- Index contents per evaluated file (historical first implementation): borrowed tree-sitter nodes, parent/subtree intervals, kind postings, requested exact-terminal postings (:530).
- Correctness contract: candidate set must be superset (:57); `any` with unanchored branch → unrestricted (:62, :292); `not` contributes no positive anchor, and negation subtracts only exact plans (:116); incomplete tree/index never supplies absence proof (:112).
- Kind IDs are grammar-scoped; TS `property_identifier` kind 377 vs Python `identifier` kind 1 (:517).

### Schema 12: query-independent FTS persistence (:159-179) — important, differs from in-memory story
- "Schema 12 replaces the Schema 11 file-by-predicate postings. Each search-index set uses an immutable `TapeId` and an explicit index version" (:161-162). Identity prevents preorder ordinals crossing source/grammar revisions (:163).
- Kind index = contentless FTS5 table with one token per preorder node; instance offsets are exact preorder ordinals (:165-166).
- Exact-value index stores each bounded `(kind, value)` term once per tape; selects files, does not reconstruct value positions (:167-168).
- Source-word index selects files for required regex words and comment searches (:169).
- Value lookup returns one completion row per tape and sparse positive file/program rows, not full file×program cross product (:171-172). Complete miss = exact file rejection; missing index set returns `Unknown` and widens to terminal scanning (:173-174).
- CPU execution builds kind roots only for selected files. "Metal execution converts SQL eligibility to per-file program masks. One Metal dispatch finds exact roots and materializes terminal captures. Parent, sibling, and subtree verification remains in the tape executor." (:176-179)
- Contradiction note: :104 says no persistent cross-file dictionaries / no word filter; :159-179 (later section) adds a persisted source-word index and SQL/FTS5 storage. The Schema 12 section is newer than the "Predicate-aware index" body text.

### Measurements (all M unless marked)
- Corpus: 5,725 files, 95,697,853 bytes, 2,527,237 newlines (data-layer `fce4dc7`) (:392). Subset 538 files, 8,616,974 bytes (:393). Parseable supplement 5,722 files, 95,326,563 bytes (:446). Machine Apple M4 Max, 16 logical CPUs, 128 GiB (:394).
- Parity: 159,145 policy/file pairs, 120,753 ordered findings, zero mismatches; 16 parser-incomplete pairs (:122). All-policy workspace: 185 docs compiled, 100 registered, 169,014 initial findings (:123).
- Predicate vs previous kind/literal index, 538-file sample, worker thread CPU (scan+update): 2327.928→885.049, 2269.769→905.226, 2276.447→875.607, 2258.505→895.169 ms; median reduction 60.95% (:130-137). Experiment 20.47 s (:128).
- Matcher calls 4,500,255 → 9,718; visited nodes 1,506,737 unchanged; rule/file combos rejected 2,499 → 9,865 of 10,435 (:139).
- Temporary index peak estimate 8,513,872 → 11,704,304 bytes (8.12 → 11.16 MiB) — M(est), excludes allocator, source, tree, plans, scratch; not RSS (:141).
  - D: ~11.7 MB / 8.6 MB subset ≈ 1.36 bytes of peak temp index per source byte of the subset — but it is a *peak per-file* temporary estimate, not a sum; do not treat as per-byte ratio. (My derivation; source gives no per-byte figure.)
- Edit 34.51–39.13 ms before vs 37.43–39.16 ms after; dominated by whole-result serialization (:143).
- Baseline (pre-Phase-A, uninstrumented) (:454-456): subset 2.416 s native initial, edit p50/p95 36.15/40.39 ms, peak RSS 753.4 MiB; parseable 22.414 s, 440.41/499.40 ms, 6,908.8 MiB; full 22.394 s, 6,921.9 MiB.
- Diagnostic attribution (:474-480): subset parse 356 ms, AST traversal/matching 1,837 ms, regex 79 ms, serialization 41 ms; parseable parse 4,376 ms, matching 15,572 ms, regex 810 ms, serialization 478 ms; matcher invocations 4,667,890 / 54,083,554; visited nodes 1,506,737 / 18,492,155.
- AST dispatch ≈71% of parseable initial apply (:484). Edit: serialization >99% of native edit apply (396.589 of 396.901 ms); initial result 81,300,513 bytes re-serialized each edit (:484).
- First candidate (kind/literal) full-supplement: index construction 1.280 s, selection 29.6 ms, AST dispatch 13.437 s; matcher calls 54,083,554 → 52,097,433; 37,641 of 159,145 combos rejected; 12,199 unrestricted fallback; peak index estimate 8,694,544 bytes (:559).
- Second candidate: subset native initial 2,439.080 → 2,324.872 ms (−4.682%); supplement 22,291.479 → 21,361.294 ms (−4.173%); RSS subset 779.484 → 738.688 MiB; supplement 6,927.984 → 6,882.094 MiB (:573-576).
- Kind/literal vs pre-Phase-A worker CPU (538 files): 2,457.343→2,316.302 ... median reduction 4.670% (:598-605); indexed documents 443; 2,499 of 10,435 rejected, 795 unrestricted fallback; peak temp index 8,513,872 bytes (:609-612).
- Coverage: 80 AST queries; 76/80 root-kind restricted; 17 have patterns; 13 identifier opportunities (:423-432).
- Spike prior art: tape_scan 567 queries, 199,469 files, 1.45x overall matching speedup, 1.63x constrained, 110 fallback, index build 251.47 s (:195). Predicate postings probe 3,018 ms → 38 ms (:198).
- PR #2558 follow-up: candidate cache follows live file ownership; 8,191-subset churn probe kept one live candidate set; 500 language changes bounded (:151-155). (Cache lifecycle behavior, not perf.)

---
## 2. TD/phase-b-tape-residency.md (200 lines)

### What Phase B is (residency/lifetime change, unchanged matcher)
- Stages 1–4 delivered 2026-09-23; stage 5 (snapshot/delta transport, removal of retained results) "not authorized or implemented" (:3-5). 1 GB resident memory and 500 core-ms/edit are "roadmap aspirations", not gates (:5) — A.
- Stage 1: `TN/policy_dsl/tape.rs` encodes an **all-CST-node tape** (anonymous punctuation included) with grammar-scoped identities, source-bound byte spans, overflow checks, cancellation and validated restoration (:9). `TapeDoc` implements ast-grep's `Doc`/node interface — restores existing ast-grep matching over the tape, **not** a reconstructed `tree_sitter::Tree` (:9, :99). Tree-sitter 0.27 API exposes no constructor from node records; `from_raw` is not deserialization (:88).
- Stage 2: anchors.rs runs existing conservative plans + positional selection + unchanged matcher over the restored doc (:10).
- Stage 3 ownership/lifecycle (:11): resident per file = immutable source as `Arc<String>`, revision-bound tape, shared grammar, current snapshot bookkeeping. **Parser and native tree are per-file transient and dropped *before* restored `TapeDoc` + temporary `FileAnchorIndex` evaluation**; line index transient/evaluation-local. Changed source/language **fully invalidates** old tape, witnesses, findings — no old-tree changed-range witness reuse. Close releases idle state; budgets unchanged.
- Unchanged resend skips parsing and doc restoration; restoring a cached tape on forced unchanged reevaluation is internal only (:12). Serialization still full snapshots (:12).
- Phase A's deviation from deck: "retains no structural index between file evaluations" (:112). Deck p.36 proposed `tree: Option<Tree> → tape: Vec<u8>`, `anchors → candidate file → decode tape → rebuild Tree → match → drop`, "one tree live at a time" (:73).

### Spike tape codec facts recorded here (from ast-index-spike `tape_v2.py`) (:84-86)
- All concrete syntax nodes in preorder. `rec` = **6 bytes/node**: LE packed kind/field, parent delta, subtree size. Separate delta/length varints for byte spans; checkpoints bound span lookup; `0xFFFF` parent/size sentinels → overflow entries. Source text separate; spans are views into it.
- `pack_kf`: kinds ≤10 bits, fields ≤6 bits; 3-byte checkpoint fields. Checkpoint stride is **16** in S008/code (S047 prose says 128 — stale).

### Measurements (M unless marked)
- Full corpus parity: 5,725 files, 159,145 comparisons, 120,753 findings, zero mismatches (:22).
- Subset (538 files) B vs A: initial process CPU **+17.45%**, applyAsync wall +17.63%, initial high-water RSS **−29.98%**; completed edit per-pair CPU −0.095%, per-revision +0.173% (:23). I.e. tape residency cost CPU and saved RSS.
- 185-policy landing context: 10,820 files, 8,962 findings, CPU +7.24%, wall +7.13%, whole-process peak RSS −19.81% (:24). Data-layer 7,182 files failed scans (16 known failures), no completed-cost median (:24).
- Completed edit native CPU p50/p95: A 36.912/41.57475 ms, B 37.043/39.63225 ms (:25). Harness end-to-end p50/p95: A 65.55/75.30 ms, B 65.33/71.62 ms (:26).
- Profile-only **logical retained estimates** (source, tape, grammar, dispatch, results) at initial: subset **33,780,720**, landing **240,999,900**, data-layer **437,013,202** bytes; of which results **10,109,388 / 6,349,477 / 122,480,109** bytes (:30). M(est) — "not allocator bytes, native live bytes or RSS".
  - D: subset non-result retained ≈ 23.67 MB for 8.62 MB source ≈ 2.75 B/source byte (source+tape+grammar+dispatch). Data-layer non-result ≈ 314.5 MB; the data-layer source size for the 7,182-file context is not given in this doc (not found), so per-byte ratio cannot be derived there.
- Transient navigation/line/index peaks: **16,914,072 / 16,244,017 / 18,501,041 bytes** (:30) M(est).
- Owner counters: peak native-parser owners ≤1, restored-doc owners ≤1, zero at revision end (:32). Data-layer initial: 5,506 parser acquisitions, 5,503 restored-doc acquisitions (:32).
- Tiny-file post-GC RSS 80,134,144 → 80,658,432 bytes after close (:32); active-close attempt INCONCLUSIVE (:32).
- Inherited Phase A whole-repo comparison: native-apply CPU 27,525.1 → 14,294.8 ms (−48.07%), harness wall 28,809.0 → 15,742.7 ms, peak RSS 5.387 → 5.344 GiB (:156-158).
- Codec review: 64 files, 9,927 nodes, 180,001-node overflow fixture, 1,160 matcher/selector pairs, 780 truncated-index cases, 62 image mutations (:42).
- Deck aspirations (A): 1 GB resident for B, 500 core-ms/edit for C, 100 ms edit p50 / 500 ms p95; p.36 estimates **190–380 MB tapes/postings + one transient tree** from spike ratios; historical 3.78 GB/95.8 MB resident measurement (deck) has different boundary than 5.34 GiB (:176).
- Budgets: `FILE_TIME_BUDGET` 500 ms, inline budget 5 ms (:129).

---

## 3. TD/content-addressed-workspace-store.md (819 lines)

### Targets (A) (:8-14, :775-782)
~100 concurrent workspaces mostly identical source; steady-state Tempo RSS < 1 GiB; total store < 10 GB; initial-index worker peak RSS < 500 MiB; edit p95 < 500 ms.

### Boundary
- JS never touches DB; Rust owns SQLite, SQL, caches, GC (:22-33). `indexRepositoryAsync()`: Rust owns discovery, path selection, source reads (clean tracked sources from Git ODB; dirty via repo descriptor, no symlinks), batching, publication (:35-41). `applyBufferAsync()` for unsaved/generated content (:41).
- Schema 9: completed clean Git trees recorded (tree hash, logical prefix, snapshot, semantic catalog identity); tree not eligible if overlay below prefix; restore only maximal matching prefixes (:43-46).
- Schema 10: **result pages per clean Git tree + logical prefix**, ≤64 direct file paths per page (findings + deterministic failures), each page independent zstd-3 frame (:47-49). Matching subtree attaches result segments **without per-file restoration**: no file state, no source load, no hash, no tape restore, no decode of local results (:51-52). Global-check policies still use per-file state (:54).
- Bulk restoration: one JSON-backed joined query for Git aliases; bulk chunks for manifests/results; dirty files need source read + hash; publish batches ≤64 files; tape frames loaded in bulk before worker execution; restore helper cannot query SQLite; missing frames → reparse without point lookup (:56-64).

### Measured workload (M)
- Full data-layer: 5,725 files, 95,697,853 source bytes, **160.38 MB raw compact tapes**, **56,079,347 bytes zstd-3 frames** (source+tape), 169,014 findings (:75-81).
- "source and tape ratio ≈ 2.68 raw bytes per source byte" (:83). D: tape alone ≈ 160.38/95.70 ≈ **1.68 tape bytes per source byte**; zstd-3 combined = 21.9% of raw (:84) → ≈0.586 compressed bytes per source byte (D).
- Independent-frame file access p50 59–77 µs, p95 374–497 µs (:86-87).
- Zstd curve (:91-96): L1 58,263,964 B / 466 ms; L3 56,079,347 / 590 ms; L5 53,583,594 / 1,269 ms; L7 51,590,489 / 2,113 ms. Default L3.
- Machine inventory 2026-09-24 (:104-111): 45 Pi procs, 44 workspaces, 12.19 GiB Pi RSS, 327,565 Py/TS files, 5.05 GB path-addressed source, 1.47 GB globally unique source; ~71% cross-workspace reduction (:116). Content-addressing keyed by (language identity, source digest) (:113).
- 58 GB of source-shaped files under `tmp` dirs; 35 workspaces with scanner-visible temp files; 10 exceeded 512 MiB source limit; 5 exceeded 100,000-file limit (:121-126).
- Storage alternatives (:150-163): path-addressed hybrid ≈3.0 GiB (44 ws), scan-limit upper ≈6.8 GB; CAS ≈0.87 GiB payload + 0.1–0.4 GB manifests; 100 ws est. 2–4 GB (A).
- RAM-only table (44 ws) (:170-176): compressed frames path 2.8–2.9 GiB / CAS 0.85–0.95 GiB; **restored source+tapes path 13–15 GiB / CAS 3.8–4.2 GiB**; path restored after overhead 15–18 GiB; >30 GiB with temp trees. (These are D/A estimates from inventory.)
- "Content addressing reduces RAM only when processes share mappings or one process owns the object cache. A digest name alone does not remove duplicate buffers" (:178-179). Selected: CAS on disk, bounded hot data (:181).
- SQLite vs Turso 0.7.2 (5,725 rows, 56 MB, WAL, 8 MiB cache) (:207-214): bulk load 265.2/177.5 ms; seq scan 22.0/42.9; 10k random reads 76.9/101.4; 500 changed-row txns 18.6/59.2; 4-worker reads 100.8/77.5; bulk-load peak RSS 143.9/227.4 MB. SQLite chosen. PRAGMAs: WAL, synchronous NORMAL, cache_size −16384 (16 MiB), journal_size_limit 16 MiB, busy_timeout 250 (:190-197).

### Keying / content addressing
- Hashes as 4 signed i64 lanes (SHA-256 BE lanes; SHA-1 in 2.5 lanes zero-padded), algorithm id distinguishes (:279-298). Strings interned by (domain, content hash, collision ordinal) with text compare (:322-348). Paths interned as full normalized repo-relative paths, one `path_id` across snapshots/workspaces (:352-366).
- `source_objects(canonical_hash_id UNIQUE, raw_length, codec_id, frame)` (:375-381); Git blob ID is an alias (`source_aliases`), algorithm-namespaced; first encounter reads source and computes canonical SHA-256 while parsing+compressing; later workspaces use alias without re-read (:387-404).
- **Tape identity**: `tape_objects` UNIQUE(source_id, grammar_id, grammar_version, tape_format_version) plus its own canonical_hash_id, codec, raw_length, frame (:408-428). "A tape depends on source identity, grammar identity, grammar version, and tape format" (:408).
- Source and tape frames independent zstd frames; fetch one without the other (:430-431). Look up identities before compression; compress only missing objects; **four symmetric workers** share parse/match/tape/hash/compress; publication transaction ≤64 files; busy writer → complete retry, counted as `workspaceProfile.storageRetries` (:432-438).
- Git: `gix`, no git binary (:442-448). Git tree IDs used for snapshot identity, skip unchanged walks, reuse manifests, find blob aliases (:455-464). Ignore: `.gitignore` then `.tempoignore`; always exclude `.git .nera node_modules target tmp __pycache__` (:489-504). Content-transforming attributes (filters, eol, working-tree-encoding) force read+hash of workspace bytes; spans bind to evaluated bytes (:514-518).
- Snapshots immutable; identity (root_tree_id, scanner_version, grammar_set_hash, tape_format_version); `snapshot_files(snapshot, path) → (source_id, tape_id)`; 100 ws on same commit share one snapshot (:526-554). Workspace = snapshot + overlay (`workspace_overlay(workspace, path) → state, source_id, tape_id`); deleted path has no refs (:556-580).

### Invalidation on edit
- "A normal edit transaction changes one overlay path and its object references" (:606). New content → new immutable source/tape objects (:580). Frames created/compressed before write txn; insert-or-find handles duplicates (:600-604).
- Validation before frame use: codec, raw length, canonical hash, grammar identity/version, tape format version; failure → evict from use, rebuild from source (:743-753).

### GC / liveness
- DB relationships are liveness authority, not refcounts (:612-613). Live roots: active workspace roots, retained snapshots, overlays, explicit pins from indexing workers and in-flight edits (:620-626). Leases with owner token + expiry (:628-630). Order: retire → overlays → snapshots → tapes → sources → paths/strings/hashes/trees → WAL/free-page (:634-642). Batches of hundreds/low thousands, `BEGIN IMMEDIATE`, few ms (:644-645). Liveness rechecked per delete; creation generation protects own-cycle objects (:653-663). Incremental vacuum (:670-673). Disk: 7 GB soft, 9 GB hard, 10 GB absolute (:681-688).

### Memory plan — the in-RAM caching forms
- CAS stays on disk; daemon does not keep all compressed frames in RAM (:695-696). Bounded regions: SQLite page caches, hot compressed frames, restored source+tapes, active policy runtimes, GC scratch, result delivery (:698-705). Inactive workspace runtimes leave memory (:707). Share compiled policy plans by identity (:710). Initial parse/index in a **disposable worker**; process exit releases parser/allocator high water (:713-714).
- One `ContentStore` per DB path per process; shared decompressed-object cache; one SQLite connection per worker thread per DB path (:716-722).
- **Decompressed-object cache: 32 MiB logical limit, 30-second sliding lifetime; decompressed sources and tapes stored in one 64 MiB anonymous mapping**; GC copies live entries into a replacement mapping; old mapping readable until readers release; ≤2 mappings during collection; `MADV_DONTNEED` before removing a mapping; oversized source/tape uses bounded transient heap instead (:725-732). (No mention of pinned/page-locked memory or Metal here.)

---
## 4. TD/workspace-memory-implementation-plan.md (185 lines)

- Goal: steady-state memory < 1 GiB for full data-layer workload, incremental p95 < 500 ms (:5-6) — A target. Covers NER-1621..1625 + indexed SQL file filter from NER-1562 (:8-9).
- Rules: native ast-grep remains matcher + correctness oracle; **SQL selects candidate files only**; filter may false-positive, never reject a native match; source and tape frames independent (:13-18).
- Completed investigation includes "Replace resident Tree-sitter trees with validated tapes", "Select a bidirectional `file_check` index" (:24-34).
- Phase 1.2 tape persistence: versioned binary codec for `TapeImage`; encode all arrays + source-binding metadata; decode without parsing source; reject malformed lengths, noncanonical side tables, wrong grammars, stale source hashes; byte-stability vectors (:51-56).
- Phase 1.3: "Add bounded read and decompression caches" [x] (:65).
- Phase 2: `file_check(check_id, file_id)`; `PRIMARY KEY(owner_id, check_id, file_id)` query-first; `INDEX(owner_id, file_id, check_id)` changed-file; initial candidate rows built in SQL from scope, grammar, kind, exact-token facts; replace one file's rows per edit txn (:92-96). Open: stream candidates to native tape matching; candidate parity for whole catalog; candidate order not changing finding order (:97-101).
- Phase 3 compact results: fixed-width witness/capture records; removed retained JSON findings; render only at transport; changed-file replacements on edit; **native retained result state < 10 MiB**; **result-transport peak < 64 MiB** (:105-115) [x].
- Phase 4: N-API buffer-batch input; release JS buffers before next batch; send only changed source on edit; **"Retain only compressed source after ingestion"** (:119-124).
- Phase 5: disposable initial worker, separate allocator, publish atomically, exit; worker peak RSS < 500 MiB [x]; main-process RSS < 400 MiB after opening result [ ] (:129-138).
- Phase 6: "Replace resident `FileState` source and tape ownership with store handles" [x]; **"Expand only the active file during matching"** [x]; evict inactive workspace runtimes [x]; remove obsolete full-snapshot and resident-payload paths [ ] (:142-148).
- Phase 7 certification all [x] including steady-state < 1 GiB, incremental p50 < 100 ms / p95 < 500 ms, store < 10 GB for 100 similar workspaces (:152-164) (but journal Entry 14 later says certification gaps existed — see §5).
- "It releases resident source and tape data after each evaluation" (:170).
- M: stored-runtime checkpoint 2026-09-25 final RSS **362,414,080 B**; 5 edits p50 1.78 ms, p95 2.36 ms; 5,725-file full scan 23.23 s (:174-176). Four-worker checkpoint: full scan **9.78 s wall, 19.23 CPU-s, final RSS 458,031,104 B**; no retries (:179-185).

---

## 5. TD/workspace-store-runtime-journal.md (1531 lines)

Chronological; entries supersede each other. Key facts per entry (all M unless marked):

- Start (2026-09-25, `44e4989d4`): 4 symmetric workers, ≤64-file txns; full scan 5,725 files 9.78 s wall, 19.23 s CPU, final RSS 458,031,104 B; incremental p50/p95 wall 1.57/3.16 ms, CPU 1.59/2.20 ms (:20-38).
- E2 batched CAS lookup: one read phase per batch before compression; compress only missing; counters `reusedSources/reusedTapes/compressedSources/compressedTapes/storageRetries` (:66-84).
- E3 **read-through tape reuse**: before parsing a changed input, check stored source identity; restore compatible stored tape when source hash + tape metadata match; workers do lookup, decompression, validation, matching; SQLite locks no longer cover zstd or SHA-256 (:109-112). Warm full scan restored 5,721 tapes, reused 5,725 sources / 5,722 tapes, 0 compressions; **7.46 s wall, 10.35 s CPU, AST parse 25.36 ms**; −46% CPU vs 19.23 s (:114-127).
- E5 representative edits by size bucket (:155-160): min 0 B 1.75–1.87 ms wall; p50 7,986 B 3.75–3.78 ms; p95 54,886 B 10.91–10.97 ms; max 718,905 B 185.07–210.09 ms wall / 153.10–156.95 ms CPU.
- E6 compact witnesses: each span six fixed-width `u32`; captures share name + boxed span array; JSON only at response (:175-178).
- E7 `applyBufferAsync(Buffer)`: 8-byte marker + metadata length + JSON metadata + raw UTF-8; reject >40 MiB (was 16 MiB; policy source limit 32 MB) (:193-213).
- E8 result pages: caller limit, hard 64 MiB; enforcer uses 8 MiB pages; no N-API response contains complete finding set (:219-228).
- E9 disposable initial-index child Node process; **worker peak 380,174,336 B** (:234-247).
- E10 `maintainStorage()` renews lease 120 s; maintenance owner collects ≤256 rows/pass; called every 30 s (:255-262).
- E13 "final certification": warm store scan 5.41 s wall, 8.75 s CPU, main RSS 507,920,384 B, worker peak 380,174,336 B, page 8 MiB, **store size 96 MiB**, edits p50 2.26 ms, p95 78.82 ms, max-file 78.82 ms wall / 77.58 ms CPU; restored 5,722 tapes (:319-340).
- E14: completion review found gaps in items 2–10; "supersedes earlier statements that described the complete plan as finished" (:357-359). (Contradiction with the all-[x] Phase 7 in §4.)
- E15: Git blob aliases validated against evaluated bytes; clean files promoted to immutable snapshot; inactive runtimes release files, grammars, candidate maps, result state; next request restores tapes/candidates from store (:365-377).
- E16 candidate-restore regression: loading full manifest per 64-file request → 21.93 s scan / 102.50 ms edit p50. Fix: one snapshot+overlay manifest per active runtime, updated in memory after publication, dropped on eviction (:383-389). After: 7.78 s / 9.63 CPU-s / p50 15.25 / p95 123.50 ms; with shared snapshot candidates **4.59 s wall, 8.01 CPU-s, p50 4.80 ms, p95 77.73 ms** (:391-407).
- E17 schema 3 completed revisions; cancellation before prep/retry/commit; clean overlay promoted per ≤64-path batch (:415-425).
- E18 schema 11 `policy_run_result_pages` (execution store, 8 MiB pages) (:431-437).
- E19 sparse-file tests for 7/9/10 GB; corrupt zstd fails closed; 4-process same-source → one canonical object (:448-459).
- E21 repeated warm distribution (5 scans, 20 edits): full-scan p50 6.64 s, p95 8.66 s; incremental p50 16.16, p95 173.58, p99 185.16 ms; final RSS 532,627,456 B; peak 562,642,944 B; optional 400 MiB main target failed (:481-495).
- **E22 SQL candidate facts + decompressed-object arena** (:502-557): Schema 4 source-anchor facts and candidate requirements over grammar, node kinds, root kinds, exact terminal values, required text trigrams; completion markers distinguish empty vs nonexistent; SQL facts = necessary conditions. **"Persisted AST candidates now control tape restoration. A file with no eligible AST checks does not load its tape."** Regex checks outside AST candidate set; active catalog 39,056 persisted file-check rows (:506-515).
  - Rejected: one mapping per large tape (page-table + VM-zone metadata per mapping); also separate heap allocations for source cache (:517-519).
  - Final: decompressed sources + tapes in **one shared arena, one 64 MiB mapping for a 32 MiB logical limit, 30-s sliding lifetime**; collection copies live entries into one replacement mapping; readers finish on old; no third mapping while old has readers; transient heap when rotation unsafe; `MADV_DONTNEED` before removing old mapping (:520-529).
  - Manifest rebuild regression 12–19 s fixed by keeping loaded manifest (:531-534).
  - Run: full scan 4.83 s, 8.61 CPU-s, p50 8.43, p95 86.86 ms; restored candidate files 5,725; **tape decompressions 5,663; avoided 59; native AST matcher calls 524; tape-cache hits 0** ("full scan read each tape once"; filter weak for current catalog) (:536-552).
- **E23 one-process 46-workspace pressure** (:559-637): first attempt one store/cache/connection/maintenance thread per workspace → failed at ws 14, peak 1.30 GB, "content store writer is busy". Fix: one `ContentStore` per DB path per process (one connection, one decompressed cache). Run: 333,597 files, 5,155,063,652 source bytes; wall 821.93 s, CPU 1,180.66 s; per-ws p50 19.50 s p95 24.04 s; peak RSS 1,984,848 KiB; after all ws 1,902,297,088; after 35-s idle 1,863,139,328; after all closes 1,770,651,648 B. "Closing all workspaces did not return most pages to the operating system. The daemon requires runtime eviction and a process boundary" (:596-597).
  - Disk: SQLite 1,015,746,560 logical; WAL 5,487,872; **sources 55,677 objects / 302,141,842 frame bytes; tapes 55,207 objects / 522,980,152 frame bytes**; 2,046,886 candidate rows; raw_length sums: sources 1,491,676,179, **tapes 2,432,984,906**; source frames 20.3% of logical, tape frames 21.5% (:599-615).
    - D: unique tape raw / unique source raw = 2,432,984,906 / 1,491,676,179 ≈ **1.63 tape bytes per source byte**; compressed tape ≈ 0.35 B per unique source byte; compressed source+tape ≈ 0.55 B per unique source byte.
  - Warm second run on an already complete worktree: 8,577 sources / 8,479 tapes reusable, still 13.21 s, **decompressed 8,478 tapes; decompressed cache supplied 1 tape**; candidate filter avoided 53 loads. Phases (4 workers overlapping): **storage reads + decompression 37.60 worker-s**, regex 1.45, line indexes 0.14, AST parse 0.25, native AST matching 0.003, manifest load 0.09, publication 0.93 s (:617-630). "native matching is not the warm-scan bottleneck ... reads and decompresses almost every tape for each workspace ... a similar worktree is a storage reuse case, not an evaluation no-op" (:632-637).
- **E24 combined-process memory attribution** (:639-683): warm 46-ws in one process reached 4,207,820,800 B; 4,154,015,744 after; V8 heap 13,076,408, external 4,865,938, array buffers 2,241,325. Native estimates 652,117,217 B: **file state 348,877,890; candidate manifests 295,324,222**; dispatch 6,018,784; witness 1,896,321; 2,052,185 manifest rows; 333,776 files. Policy-only open of 46 ws: 186.61 → 239.80 MiB (~53 MiB). `vmmap`: 3.7 GiB footprint, 3.8 GiB peak; default malloc zone 3.3 GiB resident small allocations; large empty malloc regions 202.2 MiB; **shared 64 MiB decompressed arena had only 32 KiB resident after expiration**; stacks 1.6 MiB; page tables 29.8 MiB. After close RSS 3,722,362,880. "most memory after close was dirty malloc high-water memory ... Repeated tape decompression and source ingestion grew this high-water state" (:675-677). "The disposable initial-index process boundary remains mandatory. A long-lived daemon must not scan all complete workspaces again" (:682-683).
  - D: native file state 348,877,890 / 333,776 files ≈ 1,045 B/file; candidate manifests ≈ 885 B/file; ≈144 B/manifest row.
- **E25 canonical source/check result reuse** (schema 5 `local_check_results`): row keyed by canonical source + candidate-check identity; compressed payload = findings, clean negatives, or deterministic failures; catalog identity = policy digest + check id + raw capability doc + grammar identity + scanner version + tape format version + local-result format + ast-grep runtime identity (:689-699). "A warm restore does not load a tape when all eligible checks have stored results" (:701). 5,725 files → 677 findings, 27 failures, hash `159a0a08…` (:703-708). Landing ws worker cold 18.04 s / 273,399,808 B peak; warm 1.97 s / 175,407,104 B (:712-715).
- E26 warm 47 active ws: 329,260 files, 5,131,032,469 logical bytes; 329,069 restored from results; 34 source + 34 tape frames compressed; **did not decompress a stored tape**; aggregate wall 44.22 s, per-ws p50 829.19 ms, p95 1.81 s, max 2.23 s; CPU aggregate 34.81 s; **peak RSS 499,499,008**, 477,986,816 after idle; store 1,023,475,712 B (:729-751).
- E27 shorter write txns: 150 result rows/statement; 128 cached prepared statements; cold 5,725-file write-lock 1,478.13 ms, storage-write path 3,765.98 ms, complete index 14,919.30 ms (:769-790).
- E28 baseline: median index 10,390.09 ms, CPU 23,016.31 ms, storage-write 2,655.42, write-txn 1,502.22, peak RSS 474,873,856 (:803-810).
- E29 thread-local connections (one per worker thread per DB path); schema 7 hash index on `(algorithm_id, lane0)` only, other lanes checked; CROSS JOIN to fix planner (89.62 s pathological); ≤10 data statements per pass, measured max 9 (:825-847). Medians: index 12,243.89 ms, CPU 28,867.70 ms, write txn 1,324.04 ms, peak RSS 484,212,736 (:858-864). 4 workers concurrency 4.66–5.11 s vs 9.11–10.57 s (:871-875).
- E30 one maintenance thread per process+store; renews all leases with one statement; no collector election (:904-915). Cold fill 4 procs: 47 ws, 331,345 files, 5,156,171,339 B; elapsed 225.63 s; sum index 667.06 s; CPU 771.82 s; 17 retries; largest worker RSS 664,961,024 B; sum of four peaks 2,528,100,352 (upper bound) (:917-931). Warm in one process: 331,344 files restored, 36.52 s aggregate, p50 836.06 ms, p95 1.18 s, CPU 32.83 s, **peak RSS 524,746,752, after idle 511,295,488** (:933-940); store 1,006,682,112 B (:948).
- E31 cold profile: 331,348 files 183.63 s; setup 60.06 s (fs enumeration 30.00, gix 28.36, filtering 1.69); index 574.21 s; CPU 968.54 s; worker-seconds: **persisted-file restoration 657.72**, parsing 103.26, publication 108.33, result-manifest load 41.56, regex 25.80, writer-lock 38.79, line-index 1.48, **structural matching 0.02** (:968-986). 92,974 data statements in 14,813 passes; 2,827,287 local check results restored; 46,897 sources compressed, 266,628 reused (:990-995). Subtree reuse analysis: 304,937 tracked occurrences, 138,331 (45.36%) coverable by unchanged subtrees at same prefix (:1002-1005).
- E32 native repository indexing (schema 9): data-layer clone cold 11.59 s (5,725 clean blobs); second clone reused root tree, read 0 files, restored all in **610 ms**; 677 findings / 27 failures (:1040-1046). 47 ws: restored 316,411 below clean trees, read 24,655; elapsed 48.72 s; peak RSS 398,802,944; p50 976 ms, p95 5.39 s, max 30.81 s (:1048-1056).
- **E33 stored subtree result segments** (schema 10): no `FileState` for attached files; skips source reads, hashes, tape restoration, local-result decoding, parse, match, compress, publish (:1076-1080). Complete-root reuse 668.59 ms + 5,725 file states + 9,286,770 B est → ten runs median **34.11 ms**, p95 36.96 ms, 0 file states, 909,203 B est (−94.90% time, −90.21% memory) (:1089-1093). 47 ws: elapsed 48.72 → 13.57 s; p50 976 → 111 ms; p95 5.39 → 1.29 s; attached 323,405 files; peak RSS rose 398,802,944 → 677,888,000 B ("allocator and shared store caches require more analysis") (:1095-1106).
  - D: 9,286,770 B / 5,725 files ≈ 1,622 B of per-file workspace state (estimate) before segments.
- E34 bulk restoration for dirty overlays: elapsed 10.91–11.85 s; p95 653–778 ms; p50 124–136 ms; peak RSS 639,238,144–644,218,880 (:1128-1136). Audit found individual tape lookup/fetch fallbacks in `restore_file_storage()`; "bulk-only claim was too broad" (:1138-1139).
- E38 fixed-word transducer (`TN/policy_dsl/transducer.rs`): 5,725 files, 8 identifier patterns, 12,967 ranges; medians tree ast-grep **12,608.58 ms**, tape ast-grep **817.68 ms**, tape transducer **359.22 ms** (35.10× vs trees, 2.28× vs tape ast-grep); parsing 8,653.04 ms; **tape encoding + restoration 4,574.91 ms** (:1206-1214).
- E39 batched anchors / JIT / Metal: `execute_batch` many programs × many tapes, optional ordered preorder anchors per (file, program) (:1222-1225). Unanchored CPU 380.07 ms; exact kind+value anchors → **4.93 ms** (77.04×); anchor construction 301.35 ms outside medians (:1236-1239). CPU JIT 5.14 ms (+0.07 ms compile). Generic Metal interpreter 49.86 ms, specialized Metal 33.15 ms — "GPU transfer, submission, synchronization, and reconstruction costs exceeded kernel savings"; generic Metal compile 181.02 ms; specialized 1.74 ms warm shader cache (:1241-1246). "Both Metal routes use shared buffers" (:1230).
- **E40 mapped Metal batches** (:1251-1287): "Preparation packs sources and tasks into **page-backed mappings**. Metal reads those mappings through **no-copy shared buffers**. Prepared batches retain the command queue, program buffers, result buffer, and compiled pipeline. Repeated runs do not pack inputs or create buffers again." (:1253-1257). GPU anchor kernel: one thread per node writes 32-bit program mask; host converts to ordered preorder lists; GPU==CPU anchors for all 5,725 files (:1259-1262). Schema 11 anchor postings (delta-coded preorder ordinals per tape × predicate; complete empty proves absence; missing → Unknown) (:1264-1268). **Warm mapped Metal interpretation 1.47 ms; shape-specific kernel 1.28 ms; cold-buffer 39.12 / 37.26 ms** — "persistent mapped inputs removed most measured GPU setup cost" (:1276-1279). Warm GPU anchor scan 68.71 ms vs CPU tape scan 295.57 ms; GPU anchor prep 352.31 ms (compile, pack, map, buffer creation) (:1281-1284).
- **E41 sparse GPU anchors + SQL vtab** (:1289-1329): kernel emits only positive hits (global node ordinal + literal-group id) via one atomic counter; host sorts by ordinal. Literals grouped by kind and length, byte trie per group; pipeline cache keyed by device identity + source SHA-256; input = separate kind, span, eligibility-mask, source buffers; one dispatch. Median 68.71 → **2.61 ms**; dispatch+sync 1.31 ms; sparse reconstruction 0.86 ms for 12,967 hits; anchor + terminal capture materialization **5.64 ms**; CPU anchor scan 266.61 ms (:1302-1307). **GPU preparation 284.72 ms** (compile, node packing, mappings, buffer creation); "Prepared mappings remain necessary for low warm latency" (:1310-1312). Per-file arbitrary-length program bit set from SQL file filter (:1314-1316). `tempo_matches(batch)` SQLite vtab, one `xFilter` per batch, `OnceLock` prevents re-execution; complete negative pair has explicit status row (:1318-1322). Production runtime does not use this path yet (:1326).
- **E42 Schema 12 contentless FTS** (:1331-1369): kind index **22,278,144 B = 1.172 B/node** (matches spike 1.14 B/node); first bare value index one token per node 59,314,176 B, positional lookup 2,229.20 ms (rejected); final value index (distinct (kind,value) per tape) **32,239,616 B**; sparse SQL selection of all 5,725 files × 8 programs **7.60 ms**; source-word FTS **2,387,968 B**, `TODO` lookup 118.82 ms → 25 files. **All three + metadata 56,983,552 B = 2.998 B/node = 0.595 B/source byte; build 9.21 s for 95,697,853 B and 19,007,624 nodes** (:1354-1356). CPU path: SQL lookup + root selection 50.09 ms, terminal 60.87 ms. Metal path: SQL lookup + mask 7.80 ms, warm fused terminal 5.06 ms (one dispatch), **combined warm 12.85 ms**; cold Metal prep 337.63 ms (:1358-1366).
  - D: 19,007,624 nodes / 95,697,853 B ≈ **0.199 nodes per source byte** (~5.03 source bytes/node). With 160.38 MB raw tapes (§3) ≈ **8.44 tape bytes per node** (D; mixes two measurements from different docs, same corpus).
- E43 full ast-index corpus: 210,176 files 1.32 GB; indexed 202,130 files, 1.20 GB, 265,511,978 nodes; rejected files > 4 MB; search indexes 737,382,400 B = 2.777 B/node = 0.613 B/source byte; 452.11 CPU-s (:1373-1381). Scala alias failures 4,024 (:1385-1387). Positional value `fts5vocab(instance)` access-path bug (:1403-1406).
  - D: 265.5M nodes / 1.20 GB ≈ 0.22 nodes/source byte.
- E44 tape format 3: sparse contextual alias overrides for kinds + named flags (:1420-1423). Scala 4,175 files, 45.2 MB, 8,199,956 nodes. 8-part index 203,986 files 1.21 GB 740,618,240 B = 2.770 B/node (:1425-1428). (SQL timings here measure spike, not Tempo — :1416.)
- E45 Go worktree parity: 12,113 eligible files; 39,995,635 ordered nodes compared; parse 12.10 s (summed), **tape encoding 6.88 s, serialization+decoding 0.13 s, restoration 1.78 s** (:1491-1504). P-256 table (264,216 nodes): parse 86.57 ms, encode 53.91 ms, ser/de 4.35 ms, restore 10.75 ms (:1507-1508). `$F($$$ARGS)` 1,081,460 matches: native 3,285.28 ms vs restored-tape 695.70 ms (:1511-1513).
- E46 tape format 4: sparse hidden-supertype memberships per visible node from parse cursor (:1527). 5,521 SCM patterns attempted, 5,438 grammar-compatible, all lowered (:1531).

---

## 6. TD/sha-indexed-transducer-execution.md (321 lines)

- Status: historical survey + implemented fixed-word executor + positional store (:3). Schema 12 query-independent FTS replaces Schema 11 (:9-10). **"The Metal backend supports reusable no-copy buffers over page-backed host mappings. It also supports a GPU exact-value anchor scan for the current compiler subset."** (:11-13)
- Result-page attachment still reads and decodes stored pages into resident values; still enumerates paths; "not a lazy disk-backed result cursor" (:43-47). Tape frames bulk-loaded before worker execution; missing rows → parse without point queries (:49-54).
- Regression fixture hash `159a0a08…` for 5,725 files, 677 findings, 27 failures (:56-62). ~678 MB process peak exceeded 500 MiB worker target (:72) M.
- Phase A index "is temporary and file-local. It can reduce matching inside a loaded file but cannot alone avoid fetching that file's tape" (:95-96). CAS already persists `source_anchor_facts` and `candidate_anchor_facts` = coarse conservative bit tests (:102-103).
- Tape types: `TapeImage`, `TapeDoc`, `TapeNode` (`SgNode` impl) in `tape.rs` (:108-111).
- Identity rules (:166-173): source-only facts keyed by canonical source SHA; **structural facts keyed by source SHA + grammar identity + parser version + tape format + index version**; a tape-content SHA identifies the immutable artifact; path scope is a separate workspace/revision relation; rule identity includes capability semantics + compiler version.
- Proposed relations (:177-183) `workspace_paths`, `artifact_status`, `kind_presence`, `value_dictionary`, `value_postings`, `structure_postings`, `eligible_pairs`.
- Schema 12 tables (:187-190): `search_index_sets` (tape identity, index version, node count, completion), `search_kind_fts`, `search_value_fts`, `search_text_fts`. Kind index returns preorder positions through `fts5vocab('instance')` (:192). Three stages: select artifacts without decompression → root positions inside survivors → exact terminal program (:202-204).
- Proposed `TapeView` (immutable validated artifact view) + separate `SourceView`; node identity = artifact identity + ordinal; cheap independent cursors; "An execution backend can build a temporary structure-of-arrays view when its measured cost justifies expansion. The iterator API must not force a resident object or navigation allocation for every node." (:210-220). Flags: named, extra, missing, error (:222).
- Many programs × many inputs: sparse work relation (program, artifact, candidate-root range, scope context), not full product; scheduler groups by grammar, shared predicate, **input residency**; "bulk-loads required frames before matching and reuses each tape across eligible programs" (:273-276). Columnar output; offsets reference immutable source views (:278-281). GPU needs explicit capability set; "No GPU speedup is established yet. Measurements must include transfer, tape expansion, scheduling, output compaction, and CPU fallback" (:283-287) — note this predates E40–E42.
- SQL boundary: SQL owns composition (joins, grouping, counts, absence after completeness); batch table-valued operator/vtab preferred; scalar UDF per pair "would recreate the bottleneck"; interpreter must not do nested SQLite queries (:294-301).

---
## 7. TD/shared-sparse-bump-arena-design.md (278 lines) — read before §8 because §8 supersedes it

- Superseded in part: shared-arena-direct-publication supersedes per-task scratch ownership with shared `(file, Program)` evaluation ownership (:5-6). One semantic task per `(file, Program, root)` (:12).
- **Measured problem (M)**: 92 KB Python fixture → **46,765 tasks for 335 Programs** (:16). Current Metal batch declared **≈7.77 GB of mutable buffers**, most records never written (:18). Largest: pending paths 2.07 GB, raw+final results 1.60 GB, final captures 1.44 GB, query buckets 720 MB, queued states 502 MB, capture links 481 MB (:22-27).
- **Metal System Trace: 213.3 ms before GPU execution; GPU ran 6.1 ms** (:29). Second dispatch reusing same `MTLBuffer` objects: host wait **230.5 ms → 3.8 ms** (:31) (residency evidence only — dirty scratch, no semantic evidence, :33). "This result confirms that **first-use resource mapping or pinning** causes most of the Metal delay. Shader execution is not the primary cause." (:35)
- Requirements (:41-50): same offsets/limits/outcomes CPU JIT & Metal; typed handles; atomic reservation; failed reservation exposes nothing; capacity failure → no retry/fallback; physical order ≠ result order (task ordinal + local ordinal define order); reuse buffers without full clear; one kernel invocation per production Metal batch; store gets only validated records.
- Typed ABI: `ArenaOffset<Tag>{u32}`, `ArenaRange<Tag>{start, capacity}`, `ArenaCursor<Tag>{range, used}` (:57-74). Reserve = checked_add, capacity test, bump (:88-96). Reserve all related records before writing (:98-100).
- Arena classes (:106-138): task scratch (pending paths, capture links, cursor links, repeat links, value-program work; power-of-two capacity classes; compact strided region per class); program output arenas per `(file, Program)` (results, final captures, properties, transformed bytes; limit from witness limit × verified per-witness facts; overflow fails whole evaluation); query sets (buckets = fixed hash tables; members + queued = bump arenas; classes chosen independently).
- Generation-safe reuse (:152-174): "A persistent workspace cannot clear gigabytes before each dispatch"; append arenas reset cursor; hash tables occupied iff `entry.generation == workspace.generation`; increment before dispatch; gen 0 invalid; wrap → clear small hash tables.
- **Metal workspace pool** (:176-197): bounded pool of completed mutable buffers; lease removes one workspace; select smallest fitting; increment generation; one kernel; validate/decode; return; evict oldest idle at byte limit. **"The pool does not call `madvise(MADV_DONTNEED)` when it retains a workspace. That call removes the residency benefit."** (:193). `MTLResidencySet` possible on supported macOS but `metal` crate doesn't expose it (:195). "Persistent `MTLBuffer` objects remain the portable first implementation. The repeated-dispatch measurement proves that this method preserves the important mapping state." (:197)
- **Pre-pinning strategy** (:199-221): daemon may prepare common workspace classes before first interactive request via one attended warm-up command outside production execution (:201-203); must obey initial-worker memory limit; only measured common classes; large uncommon classes stay cold; memory-aware scheduler permits **one large cold workspace at a time** (:207-209). **"`mlock` is not the selected method. It locks CPU virtual pages but does not guarantee a persistent Metal GPU mapping."** (:211). **"The JIT cache retains immutable Program buffers with the compiled pipeline. A tape cache retains node, child, supertype, line, and source buffers by tape identity."** (:215). Per-dispatch: task records, headers, small offset tables (:217). "This arrangement retains nearly all large Metal resources. It also avoids repeated pinning for unchanged Programs and tapes." (:219)
- CPU JIT: same arena directory; each worker leases **demand-zero storage** for its capacity classes and reuses it (:227); Cranelift code receives raw bases, descriptors, generation, no allocator (:231).
- **Implementation status (M)** (:250-266): tagged offsets + checked reservations done; Metal retains **one bounded mutable workspace**; generation values for query/queued/greedy; **process-wide Metal dispatch lease prevents concurrent batches from pinning two large workspaces**; waiting batches reuse the retained workspace (:254-256). Second dispatch on two identical production files reused generation 3: wait **110.6 ms → 2.4 ms** (:260). Logical mutable workspace **7.96 GB → 2.72 GB**, hash unchanged (:262). Max resident **524,599,296 B** = 311,296 B over 500 MiB; memory gate not complete (:266). (Note: 7.77 GB at :18 vs 7.96 GB at :262 — two different "before" figures; not reconciled in the source.)
- Remaining order includes "Retain immutable Program and tape buffers in Metal caches" (:275) — i.e., the tape cache of :215 was **not yet implemented** at time of writing.

---

## 8. TD/shared-arena-direct-publication-design.md (778 lines)

### Status (:3-12)
- Packed execution ABI v4; v3 removed. **24-byte task record** (no arena ranges). CPU shares physical storage by evaluation; **Metal dispatches one worker per evaluation** and uses batch-global typed output arenas + evaluation-local lazy scheduler blocks (:7). CPU and Metal backends pass suite + four-backend parity (:8).
- Production compacts backend output into `PackedResultPage`, read via `PackedCaptureView`, written via `PackedNormalizedSink`; SQLite publication consumes repeatable `NormalizedRowSource`; no nested `Output`/`NodeMatch` reconstruction (:10).
- **"Stored workspaces retain only normalized evaluation summaries and result identities after response serialization. Match, capture, and property payloads are released. Subsequent full responses reload typed rows from SQLite"** (:12).

### Measured constraints (M) (:23-37)
- 640-file profile: initial publication 52.48 s; SQLite publication 29.34 s; reopen >180 s originally; **Schema 20 summary restore → reopen 3.17 s**; **retained Metal workspace reached 3.19 GB RSS** (:25-29).
- Three coarse memory controls failed: **periodic workspace eviction caused cold Metal mapping and more time-budget failures**; **`MADV_DONTNEED` did not reduce Metal residency**; 64-Program batches increased wall to 224 s (:31-35). "These results prohibit eviction and coarse Program partitioning as the primary memory design." (:37)

### Data flow / ownership
- Six stages: verifier builds `EvaluationArenaPlan` per `(file, Program)` → host packs tasks + evaluation directories (ABI v4) → generated CPU/Metal code enforces quotas, reserves from batch-global arenas → backend validates + compacts → `PackedNormalizedSink` publishes typed rows in one file transaction → paging reads normalized rows (:43-48). No `Vec<Vec<Vec<Output>>>` (:50).
- Evaluation owns all tasks for one (file, Program); shared resources: pending paths, capture links, cursor links, repeat links, query buckets/members, queued, greedy, candidates, selected results, final captures, properties, transformed bytes (:54-70). One failed task invalidates evaluation (:72).
- ABI v4 batch header fields (:84-98); `EvaluationArena` record per (file, Program) with limit/used pairs per arena (:106-147); limits = logical quotas, used = atomic u32 (:149-150). `GlobalArena` directory: one physical range + atomic cursor per record type; all tasks reserve from global cursors; **no untyped byte allocator across record types** (:152-155). Directory lives at start of scheduler buffer because **kernel already uses all 31 Metal buffer positions** (:157-159).
- Task record: evaluation, batch_ordinal, file, program, root, work_limit — **136 → 24 bytes; 46,765-task fixture saves ≈5.0 MiB** (:163-179).
- Outcomes 0 pending, 1 success, 2 invalid, 3 work limit, 4 capacity, 5 predicate error, 6 witness limit (:185-193).
- Two-level reservation: evaluation quota (CAS loop with overflow-safe check) then global cursor; global exhaustion = planner/ABI error, fails batch (:203-231). Task-local physical chunks for high-frequency append arenas; quota counts exact records (:220-226). Global capacity = checked sum of evaluation limits + max chunk-tail slack (:228). Cranelift emits native CAS, no Rust calls; Metal `atomic_compare_exchange_weak_explicit` relaxed; command-buffer boundary supplies host visibility (:233-236). Typed `ArenaOffset<Tag>/ArenaRange<Tag>` in Rust (:245). Native IR gets `ArenaKind` + `ReservationSite`, not serialized opcodes (:252-261). Offsets stored +1, zero = empty list (:263-265).
- Capacity failure: atomic outcome change; others stop at next loop boundary; scratch ignored; store gets one failed evaluation + zero rows (:269-275). Witness-limit range exhaustion → code 2 (witness budget) (:280-282). Output bundle reserves all four ranges before any write (:289-294).
- Shared query tables: per-task hash tables "caused most of the sparse workspace"; v4 = one logical query table per evaluation, lazily allocated block from global arena via atomic state word (`block_start + 2`; `u32::MAX` failure) — avoids unavailable Metal acquire/release orders (:302-312). Keys include task_ordinal (:314). Open addressing + 31-bit generation + lock bit (:317-322). Capacity = next pow2 above 2× verified admission count (load ≤0.5) (:333-335).
- Capacity planning: `ProgramCapacityFacts`; scratch budget = sum of verified op demand across roots (cyclic → work limit); output budget = witness limit × captures per output, etc. (:353-375). Move witness enforcement into generated execution (:377-378). Batch admission: sum budgets, `BackendCapacity` if exceeding process budget; no retry/split (:382-386).
- Workspace layout: keep existing physical buffers (pending paths, capture/cursor/repeat links, scheduler, results, statuses, value work, final captures, output); each = one batch-global compact arena (:390-408). **"The physical buffer capacity equals the checked sum of evaluation quotas. Only the compact used prefix becomes resident during execution."** (:410-411) **"This layout retains the same `MTLBuffer` objects across dispatches. It avoids the failed page-eviction strategies."** (:413-414)
- **Quantitative arena model (M from per-task high-water)**, 92 KB fixture, 46,765 tasks, every task at max (:418-431): pending 81.17 MiB, capture links 18.20, cursor 3.75, repeat 4.28, query members 87.06, candidates 60.65, final captures 54.59, properties 4.28; **total 313.97 MiB excl. hash tables**. Actual selected output ≈ **1.58 MiB** (:434).
- Generation reuse identical to §7 (:441-449). CPU: interpreter is semantic oracle; Cranelift gets raw bases + directory; CPU worker pool leases same bounded workspace classes; CPU & Metal identical capacities/outcomes (:453-461). Metal: one thread per semantic task; one command buffer + one kernel invocation per packed batch; host writes zero cursors + generation into small evaluation directory before dispatch (no grid-wide barrier); host doesn't clear payloads (:465-473).
- Packed results: backend validates while holding workspace lease, copies only used successful output into `PackedResultPage` (outcomes, results, final captures, properties, transformed bytes); returns workspace to pool after compaction; **SQLite publication never holds a Metal workspace lease** (:477-490). Compactor steps incl. sort refs by (file, Program, task ordinal, local ordinal); payloads not moved (:492-504). Result views borrow captures/properties/bytes/**source**/Program strings; ordinary capture text from source spans (:519-520); messages/fixes rendered **before the source document leaves evaluation scope**; page keeps only rendered message/fix text + spans (:525-527).
- Direct normalized publication: `NormalizedRowSource` (repeatable indexed access so a retried txn doesn't rebuild output) (:531-538); `FileState` holds `PendingPackedPublication` until `persist_files` completes; no Metal lease (:543-548). Publication order (:550-558).
- **Metal atomic spike (M)** 46,765 threads × 35 records, 20 dispatches after warm-up (:596-607): direct unique writes 258.0 µs; one global atomic per record 407.2; evaluation + global atomics 439.5; **evaluation atomics with one global chunk per task 240.8 µs**. Naive two-level +181.5 µs.
- **SQLite spike (M)**: 200,000 typed match rows into WITHOUT ROWID; Rust vtab and raw FFI loop rejected (slower than cached prepared rows) (:614-616). 12-column relation, 10 runs: cached single-row 245.80 ms; raw single-row 237.57; 8-row 223.29; 32-row 216.34; 128-row 215.24; **512-row 212.24 ms** (13.7% faster); plateau ≈1,024 rows (:622-632). Integrated 640-file: storage 40.35 → 40.11 s (0.6%); identifier resolution, B-tree work, row construction dominate (:634-636). Decision: cached fixed-width typed statements per relation, widths 128/512/1,024, power-of-two tail; "Do not use JSON or a virtual table" (:642-656).
- Measurement phases to record separately include "Cold Metal mapping" and "Reused Metal execution" (:749-750); per typed arena record logical bytes, **resident bytes**, high-water, capacity failures (:761).
- Selection gates (A): four-backend equality; full parity; no capacity failure; failed evaluations zero child rows; one Metal invocation and zero host events per batch; initial-worker RSS < 500 MiB; steady-state < 1 GiB; full command < 5 min; reopen/paging preserve hash; projected 100-workspace store < 10 GB (:765-776).

---

## 9. TD/sparse-rule-file-grid-design.md (204 lines)

- Replaces per-file backend submission; command = sparse grid of rules × files; "A fixed file count must not define a command" (:5-7).
- Problem: one file at a time; `WORK_POOL` has **four threads** → coordinator usually combined only four files (host scheduling artifact) (:11-13).
- Model (:19-35): N deduplicated Programs, M prepared files, sparse set of eligible `(file, Program)` evaluations, one task per `(file, Program, root)`. No empty tasks for ineligible cells; evaluation record retained when persistence needs explicit empty result (:26). Stable ordinals: file = normalized path order; Program = policy/check order; root = tape preorder; local result = Program execution order; physical lane/arena order doesn't change them (:28-35).
- Phase 1 prepare files (4-thread `WORK_POOL`, Rayon one work item per file, `rayon::yield_now()` while waiting): **restore or create the tape; restore the line index; build the anchor index; select roots per eligible Program; record non-Program matcher results; submit owned request** (:43-54). (So per-file anchor index is still built during preparation — not persisted as positions.)
- Phase 2: coordinator combines ready requests with **same complete Program identity**; ≤64 files per dispatch group; each file supplies one anchor row with N entries (empty = no task) (:58-62).
- Phase 3: never split an evaluation; close command before exceeding packed input bytes, ABI offset ranges, aggregate task work, lane scratch, durable result pages, Metal buffer limits; any file count 1..prepared limit (normally 64); oversized evaluation runs alone else explicit capacity error; no retry (:66-83).
- Phase 4: fixed physical lane grid; lane claims next task from global atomic cursor (`atomic_fetch_add`) (:87, :125-131); one validated result page; file-scoped views without copying; sort; publish ≤64 files per SQLite txn; "A command partition and a publication transaction are separate units" (:89-93).
- Packed directories: Program[N], File[M], Evaluation[K], Task[T], FileProgramRange[M], Root[R], GlobalArenaDirectory, Lane[L] (:99-109).
- Lane count fixed per device, independent of N/M/K/T; each lane owns reusable transient scratch; durable output uses shared bounded page pools (:119-121). Transient capacity depends on lane count + Program facts, not evaluation count (:139). **Metal durable page limits: 4,194,304 results; 8,388,608 captures; 2,097,152 properties; 64 MiB transformed text** (:143-148). CPU backends retain disjoint evaluation arenas (:150). (Note: this lane model differs from §8's "Metal dispatches one worker per evaluation" / "one thread for each semantic task" — later design moved to fixed lanes with atomic task cursor. Recorded as an evolution, both in sources.)
- Determinism sort key `(file ordinal, Program ordinal, task ordinal, start node, first capture node, local ordinal)` (:166-168).
- **Measured (M)**: CPython corpus, 335 Programs, **274.52 s**; 37,432,364 Program outputs, no failures; **SQLite store 2,836,709,376 B**; reopen 8.05 s restoring 681,725 evaluations without matching (:186-190).

---
## 10. TD/cpython-335-seconds-workplan.md (249 lines)

- Objective: complete 335-Program CPython workload in a few seconds (:5) A; current **219–275 s**; **Metal execution + SQLite publication = 91.74% of wall** (:8) M.
- Baseline (M) (:14-21): **2,079 evaluated files; 335 Programs; 82,899,054 semantic tasks; 37,432,364 Program outputs; 85,070,707 normalized SQLite rows; 68 Metal commands; 100.38 s Metal execution; 100.69 s SQLite publication.**
  - D: ≈39,875 tasks per file; ≈1.22M tasks per Metal command; ≈30.6 files per command (2,079/68).
- Invariants (:29-37): one identity per (file, Program, root); native Tree-sitter structural oracle; same admission rules all backends; failed evaluation no partial output; ordinals define order; normalized typed rows; **reopen executes no matcher/Program**; publication and Metal command boundaries independent; ABI v4 free of v3.
- Workstreams: cardinality oracle (:39-55); fuse root traversal across Programs — grouped root record referencing bounded Program list, load file+root once (:57-75); Metal scheduling/state reset — chunked lane scheduling, lane state reused with generation ranges (:77-100); pipeline execution and publication with bounded channels limiting resident pages (:102-121); replace row-oriented SQLite ingestion (:123-142); reduce durable row volume (:144-159).
- **Spike results (M)** (:174-237):
  - 82,899,054 tasks → **9,632,501 distinct file-root pairs; 8.61 Programs per pair** (:180).
  - Tasks with ≤16 work units = **88.04%** of tasks (:186). 8-task consecutive reservation didn't finish in 340 s (lane imbalance from correlated adjacent tasks) (:188).
  - Ideal ordered pipeline projects 143.80 s from 219.16 s; max gain 1.52× (:194) (D/A projection).
  - 5M-row `INSERT SELECT` into PK shape 2.79 s; projected all rows 47.47 s; heap load 2.17 s, +unique index 3.66 s (:200-203). 512 → 2,048 rows/VALUES increased publication 100.69 → 187.89 s (:205). Typed vtab 144.99 ms for 200k rows vs prepared 124.27 ms (:207).
  - Payload sharing: duplicate Program sources 6.57%; capture sample 1.29× max; match sample 1.03×; no dictionaries (:213-217).
  - Top-12 recurring Programs = 41.22% of interpreter work; simple kind/token choices ≥25.16% (:221-222).
  - Native Tree-sitter: 324 unique queries over 1 MB in 11.94 s, 911,772 matches; complete native run >300 s (:228-230).
  - Conclusion: few seconds not feasible with current task and row materialization volumes; needs 40–55× (:234-237).
- Nothing about tape residency/pinned memory in this doc (not found).

---

## 11. TD/distribution-model.md (304 lines)

- Relevance to residency is low. Facts: one customer-neutral local runtime; policy kernel and latency-sensitive evaluation run on the customer machine; network not in evaluation path (:5-7). Local runtime layout `~/.tempo/` with `versions/`, `daemon.json` (:102-111). Multiple profiles keep **separate credentials, policy caches, session storage, diagnostics** under `~/.tempo/profiles/<customer>/<deployment>/`; "Customer data must never be combined implicitly" (:280-284). Policy evaluation needs no network round trip (:276).
- Implication (restating, not designing): the CAS of §3 is "machine-wide" below `TEMPO_HOME` (§3 :253); distribution-model's per-profile separation of caches is not reconciled with machine-wide CAS in either doc (not found).
- No tape/memory/mmap content (not found).

---
## 12. Other Tempo docs (grep pass for pinned / residen / uncompressed / tape cache / TapeDoc / mmap / page-locked / unified memory / eviction / invalidat)

Grep hit counts were dominated by "pinned" meaning *pinned corpus/catalog/toolchain* (e.g. metal-execution-cutover-phase3/4/5 "all 335 pinned patterns compile") — not memory pinning. Relevant passages:

- **TD/packed-result-bottleneck-profile.md** (150 lines): 640 files, 335 Programs, schema 19; 212,055 evaluations; 15,693,949 normalized rows; 15,693,231 roots examined; 7,223,179 matches (:5-9). Initial publication wall 52.48 s, SQLite 29.34 s (55.9%), parse 0.87 s, normalization 1.62 s, **peak RSS 3,186,753,536 B**, reported workspace-state memory **15,014,358 B** (:16-24) M. Non-SQL hot symbols include `madvise` and Metal I/O (:31-38). Reopen >180 s: reads every match/capture/property row, builds nested vectors, sorts (:44-52). **Memory (:62-71): "The retained Metal workspace caused a 3.19 GB peak RSS after 634 file batches. The two-file profile reported only 524.6 MB. The same Metal buffers receive different sparse task ranges for each file. New dispatches fault new pages into the retained buffers. A generation reset changes hash validity but does not release resident pages. The workspace-state memory report excludes this Metal residency."** Rejected controls (M): 8-dispatch generation limit → peak 2.36 GB but wall 74.83 s and time-budget failures 4,737 → 9,414 (:96); **tail-page `MADV_DONTNEED` → peak still 3.19 GB, wall 84.94 s, "Metal treated later page use as new resource mapping"** (:98); 64-Program batch → RSS 1.25 GB, wall 224.00 s, 43,166 budget failures (:100); 4,096-record query cap → peak 4.99 GB (smaller hash tables had larger resident fraction) (:102). Plan item 5 "Cache immutable packed tables: Retain packed Program tables with each Metal JIT cache entry. **Cache tape buffers by tape identity.**" (:123-131). 395 capacity failures (640 files), 3,122 partial full corpus (:138-139).
- **TD/packed-result-throughput-design.md**: compact record-capacity spike 851 MB logical workspace; host buffer allocation + init **93.5 ms**, kernel submit+complete **26.7 ms**, complete backend call 164.7 ms (:83-91) M. **Demand-zero no-copy Metal spike** (original logical capacities): exact hash; **max resident 474 MB; buffer allocation 3.4 ms; kernel time increased to 407.2 ms**; "Sparse allocation fixes physical memory use. It does not fix sparse-table locality." (:97-107) M. Metal System Trace (92 KB fixture): CPU commit-to-completion 221.1 ms, **GPU queue delay 213.3 ms, GPU active 6.1 ms**, encoder CPU 0.216 ms (:111-118). ≈7.77 GB mutable Metal resources (:120). Decision: "Use demand-zero no-copy mappings for mutable Metal buffers. Do not create a zero-filled host `Vec` and copy it into each Metal buffer." Pool mutable buffers by capacity class and device (:239-247). Report "Logical workspace bytes" vs "Resident workspace bytes" (:480-481).
- **TD/metal-production-grid-cutover-design.md**: "The complete input must remain resident when checked buffer limits permit it" (:109). **Input residency (:451-476): one Metal execution input, not one per 32/64-file publication batch; upload once: files+nodes, children+supertypes, source bytes+line starts, Programs+signature tables, root universe+selectors, evaluation spans+blocks. "Use private Metal storage for large immutable buffers. Use bounded staging buffers, and release each staging buffer after its copy command completes. Report private Metal bytes and process RSS separately. The initial-worker RSS limit still applies during upload. Incremental execution builds the same structures for changed files only. It can reuse resident Program buffers and unchanged corpus buffers."** Memory budget (:502-519): root universe 9.6M records; ~324,000 execution blocks; ~681,725 evaluation spans; ~12.2 MB block-span count records; excludes 82.9M `MetalTask`s, per-thread/per-evaluation scratch, per-publication-batch corpus copy. Fewer than 40 Metal invocations target (:108, :492) A.
- **TD/metal-optimal-execution-shape.md**: temporary hard limits **600 launches per evaluation, 2,000,000,000 application Metal-buffer bytes** (:5). Hardware **Apple M5 Max, 40 GPU cores, SIMD 32, max threadgroup 1,024, recommended Metal working set 115,448,725,504 B** (:21-27) M. Production kernel dispatched 64 threads in two SIMD groups (:29). Input residency: "Pack the complete CPython input into one Metal-resident corpus when checked buffer limits permit it ... production working-set limit remains much smaller than this hardware limit"; contents packed nodes, child/supertype data, source bytes, file prefixes, Program data, anchor postings or compressed root ranges, work blocks/spans; "Do not rebuild these buffers for each 32-file publication batch" (:273-290). Gates: Metal < 5 s, host prep + validation < 2 s (:379-380) A.
- **TD/metal-execution-cutover-plan.md**: "Publication boundaries do not control execution partitions or rebuild resident input" (:209); target structure `resident input: roots, selectors, semantic spans, signature blocks` (:219); "Resident input | Immutable shared buffers and exact signature blocks | [prohibited:] Tuple task expansion or publication-sized repacking" (:233).
- **TD/metal-execution-cutover-handover.md** / **phase4** / **phase5** (resource observations, M): pinned (=frozen) full run 557 launches, **387,776,512 tracked Metal bytes peak**, wall 37.85 s, compiler RSS 539,475,968 (phase4:101-103); initial pinned run once requested a **9,758,228,480-byte Step buffer** (phase4:47); frozen full run peak tracked 1,982,038,016 B, **owned RSS 3,094,740,992**, compiler RSS 761,380,864 (handover:21-23); later 3,585,277,952 owned RSS (handover:33); 288 MiB sealed-relation bound (handover:27); five-file differential 402 dispatches / 142,098,432 tracked bytes; aggregate observer 3,120,459,776 owned / 790,249,472 compiler bytes; **resource contract: 600 dispatches, two billion tracked Metal bytes, four GiB application RSS, one GiB compiler RSS** (phase5:1635-1639). "Source and string arrays now share the immutable resident upload" (phase4:476). "Immutable residency: observes two NFA buffers ... green fixture observes one table across both value rounds" (phase4:913). Resident validator now checks parent/child/sibling/descendant agreement before dispatch (phase3:259-272). "The three-partition fixture preserves complete state order and one resident upload" (phase3:303).
- **TD/rerailer-ast-grep-sql-investigation.md** / **TD/rerailer-benchmark.md** (pre-tape baseline, M): data-layer 5,736 files, 95,783,744 source bytes, 2,530,133 LOC; RSS before source 203,653,120, after source 313,982,976, **after workspace load 4,095,492,096**; workspace added 3,781,509,120 B = **39.48× source** (40.63× baseline-to-resident) (rerailer-ast-grep-sql-investigation:233-256; rerailer-benchmark:122-126). Old `FileState` retained `input, previous, tree: Option<Tree>, candidates, witnesses, failures, findings: Vec<Value>` (:263-273). Spike durable index ratios: PostgreSQL 1.97×, MariaDB 2.72×, SQLite 3.75× source; row-per-node PG 14.51× (:283-292).
- **TD/unified-packed-runtime-investigation.md**: schema 12 `local_check_results` one compressed JSON doc per source×check; 501,471 rows, 337 checks, 2,085 sources, **13,130,218,219 uncompressed JSON bytes → 806,063,608 compressed**; largest JSON 28,217,223 B (:74-87) M. Migration invalidates old local results and tree-result segments (:522).
- **TD/callback-free-production-completion-plan.md**: schema 12→13 migration "must preserve sources, tapes, snapshots, anchors, and search indexes. It must invalidate old structured results without decoding their frames" (:598-599). Memory attribution owners to measure separately include "Source and tape decompression caches", "Packed immutable batches", "Mutable workspaces", "SQLite page cache" (:763-773).
- **TD/sql-driven-transducer-execution.md**: "Incremental invalidation joins by tape and catalog identity" (:155).
- **TD/scm-production-execution-plan.md**: matches become `NodeMatch<TapeDoc>` then `Witness` (:169); measure initial-index peak RSS, steady-state RSS, post-eviction RSS (:284).
- "unified memory", "page-locked", "pinned memory", "uncompressed tape" as phrases: **not found** in TD or TN. Closest: `metal_backend.rs:169-171` comment "Apple Silicon's shared storage can read this allocation directly".

---

## 13. Native source (TN) — core structs relevant to residency (read-only verification, HEAD `4c94cf77e`)

- **Tape image (compact, retained form)** `TN/policy_dsl/tape.rs`: `WIDTH = 6` (:16), `TAPE_FORMAT_VERSION = 4` (:18), magic `TMPTAPE4` (:20), `KIND_LIMIT 1024`, `FIELD_LIMIT 64` (:21-22), `SENTINEL u16::MAX` (:23), `CHECKPOINT_STRIDE 16` (:25), `CHECKPOINT_LIMIT 0x00ff_ffff` (:26), `MAX_NODE_SUPERTYPES 8` (:28). `struct TapeImage { rec, spans, checkpoints: Vec<u8>, par_overflow, size_overflow: Vec<(u32,u32)>, extra, error, missing: Vec<u32>, kind_aliases, kind_overrides, supertypes, grammar, language, source_len, source_digest: [u8;32] }` "Owns only compact structure and a binding to the exact source and grammar" (:347-370).
- **Restored/expanded form** `TapeDoc { source: Arc<String>, grammar, data: Arc<Vec<Rec>>, lines: Arc<LineIndex>, kind_aliases, kind_overrides, supertypes }` (:1111-1119). `Rec { kind u16, field u16, start, end, size, parent, first, next, prev, children: u32, extra, error, missing: bool }` (:1032-1046) — D(code): 2×2 + 8×4 + 3 = 39 B → 40 B/node with alignment (my computation; source does not state it). `restore_with_lines` rejects grammar/language/source_len mismatch and **recomputes `source_digest(&source)` (SHA-256) on every restore** (:1144-1159).
- `TapeSource { source: Arc<String>, grammar, image: Arc<TapeImage>, lines: Arc<LineIndex> }` — "Immutable source-bound tape input. Expanded navigation exists only while this file is packed or its result page is normalized." (:1121-1129); `restore()` builds a `TapeDoc` (:1131-1141).
- **FileState** `TN/policy_dsl/workspace.rs:187-210`: `input: FileInput`, `source_digest`, `source_revision`, **`tape: Option<(u32, Arc<tape::TapeImage>)>`** (revision-bound compact image), `search_index`, `stored: Option<StoredFile>`, `anchor_facts`, `tape_cache_hit`, `tape_decompressed`, `candidates: Arc<Candidates>`, `normalized_results: PackedNormalizedSink`, ... After store publication: `file.stored = Some(stored); file.tape = None; file.search_index = None;` normalized match/capture/property vectors emptied; then `release_allocator_pages()` (:1309-1323). `release_allocator_pages` calls `mi_collect(true)` (mimalloc), `malloc_zone_pressure_relief` on macOS, `malloc_trim(0)` on Linux (:75-88). Whether `input.source` is released after publication: not verified (not found in the passages read).
- Tape restore from store: `tape_manifest.get(&tape_id)` gives `(bytes, cache_hit)`; `TapeImage::from_bytes(bytes, &grammar)` then optionally `TapeDoc::restore_with_lines` (:2147-2160).
- **Decompressed-object cache** `TN/content_store.rs`: `DEFAULT_READ_CACHE_BYTES = 32 MiB`, `DEFAULT_DECOMPRESSED_CACHE_TTL = 30 s` (:48-49). `enum CacheKey { Source(i64), Tape(i64) }` — **keyed by SQLite row id, not content hash** (:968-972). `CachedBacking::{Arena{Arc<MmapArena>, offset, length}, Heap(Box<[u8]>)}` (:978-985). `MmapArena::new` = `mmap(PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANON)` (:1000-1022); `Drop` = `MADV_DONTNEED` + `munmap` (:1065-1077). `DecompressedReadCache { byte_limit, arena_bytes = 2×byte_limit, ttl, used_bytes, cursor, active, retired: Weak, order: VecDeque, entries: HashMap }` (:1116-1141); `get` refreshes TTL (sliding) and LRU (:1191-1200); `compact` copies live entries into a new arena unless the retired arena still has readers (:1202-1229); `insert_bytes`: oversized (> byte_limit) → heap, evict LRU until fits, 8-byte aligned bump, fallback heap if compaction impossible (:1231-1271). Source read path: `zstd::decode_all` into a heap `Vec`, length + SHA-256 check, `insert_bytes` copies into arena, then **returns `source.to_vec()`** (another copy) (:2326-2342). Bulk tape fetch returns `HashMap<TapeId, (Arc<CachedBytes>, bool)>`, checks cache first, decodes misses with `decode_all` then inserts (:2383-2446); "A missing row is a cache miss: callers parse again rather than issuing a point lookup" (:2430).
- **Metal buffers** `TN/policy_dsl/transducer/metal_backend.rs`: process-wide ledger `METAL_BUFFER_LIMIT = 2_000_000_000` "shared by anchor selection, execution, and caches. Reserve before mmap/copy/Metal allocation" (:20-22); separate private-bytes ledger (:24); `METAL_UPLOAD_CHUNK_BYTES = 8 MiB` (:27). `MappedRegion` — "A page-backed host allocation retained for the complete lifetime of a no-copy Metal buffer. **Apple Silicon's shared storage can read this allocation directly; repeated dispatches do not repack sources, tasks, programs, or anchor nodes.**" (:169-176); page-rounded anonymous mmap (:181-214); `new` copies bytes in (:216-222); drop `MADV_DONTNEED`+`munmap` (:225-236). `MappedBuffer::from_mapping` → `new_buffer_with_bytes_no_copy(..., StorageModeShared, deallocator)`; deallocator captures `Arc<MappedRegion>` and `retain_buffer_owner` attaches a second owner via `objc_setAssociatedObject` (:344-382). `PrivateInputUpload`: one 8 MiB shared staging buffer, batched blit copies, `wait_until_completed` per flush (:243-336). `private_from_slice` / `private_capacity` → `StorageModePrivate` (:425-505). `METAL_EXECUTION_LOCK` process-serial execution (:562-577). Thread-local pipeline/library caches keyed by SHA-256 of device registry id + name + constant + source; libraries ≤16, pipelines ≤512 (:579-662).
- **Production resident input** `TN/policy_dsl/transducer/metal_register_execution.rs:28-161`: `upload(device, &ResidentInput)` derives relations (first/last named child), fields, sorted supertypes, span positions; lays out ~25 arrays (nodes, files, roots, selectors, selector_data, evaluation/block spans, blocks, words, programs, longs, relations, fields, supertypes, semantic order, positions, first, limits, children, source, strings, string bytes, line starts, assertion data, witness limits, values, assertions, child field/kind nodes, named order, value prefixes) behind a 41-word header, each array 16-byte aligned; **allocates one `private_capacity` buffer and copies via `PrivateInputUpload`** (blit through 8 MiB staging) (:146-160). `ResidentInput { input: MetalGridInput, witness_limits }` "One immutable input for the complete evaluation, independent of publication batches" (`metal_grid_plan.rs:174-192`). Built per call: `execute_shared_corpus_pages` → `MetalGridInput::build_streamed_with_upload` → `executor(...)` (`metal_program_backend.rs:78-94`); no cross-evaluation retention found.
- **Packed batch** `TN/policy_dsl/transducer/packed_batch.rs:59-84`: `PackedBatch { nodes: Vec<MetalNode>, children: Vec<u32>, source: Vec<u8>, line_starts: Vec<u32>, supertypes, files, words, longs, string_bytes, strings, values, nfa_*, lua_*, assertions, labels, programs, tasks, evaluations, global_arenas }`. `build_from_tapes` per file: restore TapeDoc (streamed path restores from `TapeSource`, :178-196), append whole source, rescan for `\n` to build line starts, emit one `MetalNode` per node + global children (:216-291). `MetalNode` = 11 × u32 = **44 B** (parent, next_any, next_named, child_start, child_count, descendant_end, source_start, source_end, kind, field, flags) (`metal_batch_abi.rs:32-46`). D(code): device-input node form ≈ 44 B/node + 4 B per child edge, vs compact image ≈ 8.4 B/node (D from §3/§5).
- `tape_cache_hit` is a per-file metric reported as `tapeCacheHits` (workspace.rs:197, :2150, :3257, :4070).
- No `mlock`, `MTLResidencySet`, `MTLHeap`, huge-page, or "unified memory" code found in TN (grep).

---
## 14. Review of PIAT `research/tempo-residency.md` (316 lines): what it got, missed, or got wrong

PIAT file: `/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/docs/rule-engine-direction/research/tempo-residency.md`. It read Tempo at HEAD `d737fad16`. The checkout is now at `4c94cf77e` (20+ later commits, all transducer/Metal test and perf commits per `git log d737fad16..HEAD`). The residency code paths I checked (content_store cache, metal_backend, metal_register_execution upload, metal_program_backend per-call build) still match PIAT's description.

**Verified correct** (I re-checked against code/docs): the 64 MiB / 32 MiB / 30 s arena cache and its compaction/MADV_DONTNEED/heap fallback (content_store.rs:48-49, :968-1279); row-id cache keys (:968-972); `get_source` returns `to_vec()` (:2342); per-restore SHA-256 re-hash (tape.rs:1153-1159); 44-byte `MetalNode` repack plus whole-source concatenation and `\n` rescan per batch (packed_batch.rs:216-291, metal_batch_abi.rs:34-46); no-copy `MappedRegion` with deallocator plus associated-object retention (metal_backend.rs:169-382); production resident input = private buffer filled through 8 MiB staging blits, rebuilt per call (metal_register_execution.rs:146-160; metal_program_backend.rs:78-94); `METAL_BUFFER_LIMIT` 2e9 ledger; `METAL_EXECUTION_LOCK`; schema version 22; `cache_size −16384`, `wal_autocheckpoint 4096` (content_store.rs:31, :1335-1337); the pinning evidence (213.3 ms queue vs 6.1 ms GPU; 230.5→3.8 ms; 110.6→2.4 ms); the rejected-controls list; `mlock` rejected; `MTLResidencySet` deferred; the tape cache keyed by tape identity is designed, not implemented (shared-sparse-bump-arena-design.md:215, :275).

**Wrong or overstated:**
1. "**No measurement of resident-input upload cost or of private vs shared storage was found**" (PIAT A.5 :129). Partly wrong. Shared no-copy input residency **was** measured, cold vs warm: E40 "Preparation packs sources and tasks into page-backed mappings. Metal reads those mappings through no-copy shared buffers ... Warm mapped Metal interpretation took 1.47 ms ... The cold-buffer medians were 39.12 ms and 37.26 ms ... persistent mapped inputs removed most measured GPU setup cost" (journal:1253-1279). GPU anchor preparation (compile, packing, mappings, buffer creation) 352.31 ms (:1284), 284.72 ms (:1310), 337.63 ms (:1366). Host buffer allocation + init 93.5 ms vs kernel 26.7 ms (packed-result-throughput-design.md:89-91). What is genuinely absent is a measurement of private-storage + blit versus shared no-copy for the *same* production input, and of the cost of the current per-call private upload. Not found.
2. A.2 "The cache was too small to get hits" / "Lesson: a TTL cache much smaller than the working set does nothing" (PIAT :20, :65). The source gives a different reason for the 0 hits: "The cache had no hits because this full scan read each tape once" (journal:549). Small size plus TTL is PIAT's inference. The E23 warm run's single hit across 8,478 tapes (:619-620) supports PIAT's reading for *re-scans of a second workspace*. But Tempo's own conclusion pointed at result reuse, not a bigger tape cache (:634-637). See omission 1.
3. A.3 "`FileState` keeps ... `tape` ... resident". This holds only before store publication. After a store publication the code sets `file.tape = None` and keeps `stored` handles (workspace.rs:1309-1323). The memory plan states "Replace resident `FileState` source and tape ownership with store handles", "Expand only the active file during matching", and "It releases resident source and tape data after each evaluation" (workspace-memory-implementation-plan.md:142-143, :170). Phase B's resident tape (phase-b:11) is the pre-store, non-persisted mode.
4. A.0 headline "Tempo has real page-allocated, uncompressed caches and real pinning". The uncompressed cache is real but bounded to 32 MiB logical with a 30 s TTL. Tempo's stated retention policy is the opposite of keeping uncompressed tapes resident: "**Retain only compressed source after ingestion**" (plan:123), and the CAS "does not retain all compressed frames in RAM" (CAS:695-696). "Pinning" in Tempo means retained mutable `MTLBuffer` workspaces (shared-sparse-bump-arena:176-197, :254-256) plus SQL GC pins (`object_pins`, CAS:620-626). No pinned uncompressed tape store exists in Tempo.

**Missed (relevant to residency and the final engine):**
1. **Result reuse replaces tape residency for warm workspaces.** Schema 5 `local_check_results`, keyed by canonical source + check/catalog identity: "A warm restore does not load a tape when all eligible checks have stored results" (journal:689-701). E26 warm pass over 47 workspaces: "did not decompress a stored tape", peak RSS 499,499,008 (:731-748). Schema 9/10 Git-subtree result segments attach whole clean subtrees with no `FileState`, source read, hash, or tape restore (journal:1066-1093; CAS:43-54). Complete-root reuse fell from 668.59 ms to 34.11 ms (median) (journal:1089-1093). Schema 20 summary-only reopen: 3.17 s (shared-arena:28), and 8.05 s restoring 681,725 evaluations (grid:190). PIAT A.6 cites result reuse in one line but does not connect it to residency.
2. **Persisted AST candidates gate tape loading**: "A file with no eligible AST checks does not load its tape" (journal:512-513). Avoided tape decompressions measured 59 (:545) and 53 (:620).
3. **Metal no-copy over page-backed mappings was the earlier production-grade path** (E40-E42, sha-indexed:11-12). The later grid cutover deliberately chose **private storage + bounded staging** for large immutable buffers (metal-production-grid-cutover-design.md:469-473). PIAT notes the private choice. It does not note that the no-copy alternative had a measured warm/cold profile (point 1 above), or that the switch to private storage has no recorded justification in the docs (not found).
4. **Demand-zero no-copy spike**: max resident fell to 474 MB, allocation took 3.4 ms, but **kernel time rose to 407.2 ms** from 26.7 ms (packed-result-throughput-design.md:97-107). This is direct evidence that first-touch page faults inside a kernel are expensive. It bears on any "fault on demand" residency plan.
5. Metal resource contract and observed RSS: 600 dispatches, 2e9 tracked Metal bytes, **4 GiB application RSS, 1 GiB compiler RSS** (phase5:1639). Observed owned RSS was 3.09–3.59 GB in frozen full runs (handover:23, :33). This conflicts with the CAS/memory-plan 1 GiB steady-state target (CAS:11; plan:5).
6. M5 Max recommended working set 115,448,725,504 B (metal-optimal-execution-shape.md:27), and "Pack the complete CPython input into one Metal-resident corpus when checked buffer limits permit it" (:275). Phase A measurements were on an M4 Max (phase-a:394), so machines differ across docs.
7. The Schema 12 FTS index sizes, which form part of the resident and at-rest footprint: kind 1.172 B/node, total 2.998 B/node = 0.595 B/source byte; 19,007,624 nodes for 95.7 MB (journal:1337-1356). Full 1.2 GB corpus: 2.777 B/node, 0.613 B/byte (:1378-1379).
8. Allocator: Tempo calls `mi_collect(true)` (mimalloc) plus `malloc_zone_pressure_relief` after publication (workspace.rs:75-88). Its daemon-level lesson was "Closing all workspaces did not return most pages to the operating system. The daemon requires runtime eviction and a process boundary" (journal:596-597, :682-683). PIAT cites the 3.3 GiB figure but not the conclusion that a process boundary is mandatory.
9. "One large cold workspace at a time" scheduler rule and process-wide Metal dispatch lease (shared-sparse-bump-arena:209, :256).
10. Tape format details beyond v4: format 3 sparse contextual alias overrides (journal:1420-1423); format 4 sparse hidden-supertype memberships (:1527).
11. Edit-path numbers with store-backed tapes: incremental p50/p95 4.80/77.73 ms (journal:402-403), 2.26/78.82 ms (:332-333), 8.43/86.86 ms (:541-542). By size bucket: the max 718,905 B file took 185–210 ms (:160).

**PIAT's own derived numbers** (Part C footprint table, 59–77 µs × 5,725 cold-load estimate, 4 ms pin extrapolation) are PIAT derivations from spike v9np B/node and Tempo latencies. They are not Tempo measurements. PIAT labels the pin extrapolation as unmeasured, correctly.

---

## 15. Synthesis for the final engine (restating only what the sources establish)

### Q1. Tape caching: forms, sizes, keys, load, pin, share, evict

**Forms that exist in Tempo:**

| Form | Where | Encoding / size | Lifetime | Cite |
|---|---|---|---|---|
| At-rest compressed | SQLite `tape_objects.frame`, `source_objects.frame` (machine-wide CAS under `TEMPO_HOME`) | One independent zstd-3 frame per object; raw_length + canonical hash outside the frame | Until GC (relationships + leases) | CAS:375-438, :612-663 |
| Compact image in RAM | `Arc<TapeImage>` (rec6 + varint spans + checkpoints every 16 + overflow/side tables) | ≈1.68 tape B per source byte; ≈8.4 B/node (D) | Per FileState revision; dropped after store publication | tape.rs:347-370; workspace.rs:1317; CAS:79-83 |
| Decompressed-object cache | one 64 MiB anonymous `mmap` arena (32 MiB logical), sources + tapes | Raw decompressed bytes, 8-B aligned | 30 s sliding TTL + LRU; two-mapping compaction; MADV_DONTNEED on drop | content_store.rs:48-49, :968-1279; CAS:725-732 |
| Expanded navigation (CPU / ast-grep) | `TapeDoc { data: Arc<Vec<Rec>> }` | AoS `Rec`, ≈40 B/node (D code) | Transient, "only while this file is packed or its result page is normalized" | tape.rs:1111-1141 |
| Executor-packed (CPU JIT + Metal input) | `PackedBatch` heap Vecs: 44-B `MetalNode` + children + concatenated source + line_starts | 44 B/node + 4 B/child edge (D code) | Per batch/evaluation | packed_batch.rs:59-291 |
| Device view | One `StorageModePrivate` buffer per evaluation, blitted through 8 MiB shared staging (current); earlier no-copy `StorageModeShared` over page-backed mmap | Same packed arrays | Per call; not retained across evaluations | metal_register_execution.rs:146-160; metal_backend.rs:169-382; metal_program_backend.rs:78-94 |
| Designed, not implemented | "A tape cache retains node, child, supertype, line, and source buffers by tape identity" (Metal) | — | — | shared-sparse-bump-arena:215, :275; packed-result-bottleneck-profile:123-131 |

**Sizes (M unless D):**
- Data-layer corpus: 95,697,853 source B, 160.38 MB raw tapes, 56,079,347 B zstd-3 for source + tape (21.9%) (CAS:75-84).
- 46-workspace store: unique source raw 1,491,676,179 B, tape raw 2,432,984,906 B (D ≈1.63 B/source byte). Frames are 20.3% and 21.5% of raw (journal:605-613).
- Nodes per source byte ≈0.199 (D: 19,007,624 / 95,697,853; journal:1356).

**Keys:**
- Tape object identity is UNIQUE(source_id, grammar_id, grammar_version, tape_format_version) plus the tape's own canonical hash (CAS:408-428).
- Source identity is the canonical SHA-256, with Git blob IDs as aliases (CAS:375-404).
- Structural facts are keyed by source SHA + grammar + parser version + tape format + index version (sha-indexed:166-169).
- Search index sets use an immutable `TapeId` + index version (phase-a:161-163).
- The in-process cache is keyed by row id (content_store.rs:968-972).

**Load:**
- Bulk-load tape frames before worker execution. A missing row means "parse again", with no point lookups (CAS:56-64; content_store.rs:2430). The audit later found individual fallbacks in `restore_file_storage()` (journal:1138-1139).
- Decompression and SHA-256 run outside SQLite locks, on 4 workers (journal:109-112).
- Restore validates codec, raw length, canonical hash, grammar id/version and tape format (CAS:743-753). `TapeDoc` restore re-hashes the source (tape.rs:1153-1159).

**Pinning:**
- Retained mutable `MTLBuffer` workspaces, with one bounded workspace and a process-wide dispatch lease (shared-sparse-bump-arena:254-256).
- SQL `object_pins` from indexing workers and in-flight edits (CAS:620-626; content_store.rs:153).
- `mlock` rejected (shared-sparse-bump-arena:211). `MTLResidencySet` deferred (:195).

**Sharing:**
- One `ContentStore` and one decompressed cache per DB path per process, shared by all workspaces (CAS:716-722; journal:572-575).
- Snapshot sharing across workspaces on the same commit (CAS:553).
- Across processes only via SQLite + OS page cache (CAS:144). "A digest name alone does not remove duplicate buffers" (CAS:178-179).
- Each tape is reused across eligible programs inside a batch (sha-indexed:276).

**Eviction:**
- Cache: TTL/LRU/byte limit (content_store.rs:1143-1271).
- Inactive workspace runtimes are evicted and later restore tapes/candidates from the store (journal:376-377; plan:144).
- Periodic Metal workspace eviction was rejected: cold remapping and budget failures (shared-arena:31-37; packed-result-bottleneck-profile:96).
- Store GC: relationship-driven, bounded batches (CAS:610-663).

### Q2. Pinned-memory uncompressed tapes
- A pinned uncompressed tape store is **not found** as an implemented Tempo feature.
- What exists and why:
  - Metal first-use mapping/pinning dominates dispatch latency: 213.3 ms before GPU vs 6.1 ms GPU execution (shared-sparse-bump-arena:29; packed-result-throughput:113-118). Reusing the same `MTLBuffer`s cut host wait from 230.5 to 3.8 ms (:31), and later from 110.6 to 2.4 ms (:260). Persistent mapped inputs: warm 1.47 ms vs cold 39.12 ms (journal:1276-1279). "Prepared mappings remain necessary for low warm latency" (:1312).
- How Metal reads:
  - Earlier path: `new_buffer_with_bytes_no_copy` over page-rounded anonymous mmap, `StorageModeShared`. "Apple Silicon's shared storage can read this allocation directly" (metal_backend.rs:169-176, :372-377). This is zero-copy for the GPU, but the bytes are first copied into the mapping (`MappedRegion::new`, :216-222).
  - Current production resident input: a private buffer filled by blit (metal_register_execution.rs:146-160). The grid design says to use private storage for large immutable buffers with bounded staging (metal-production-grid-cutover-design.md:469-473). The docs record no comparative justification (not found).
- Lifetime and pinning rules from the sources:
  - Never `MADV_DONTNEED` a retained workspace (shared-sparse-bump-arena:193; packed-result-bottleneck-profile:98).
  - Keep the same `MTLBuffer` objects across dispatches (shared-arena:413-414).
  - Only one large cold workspace at a time (shared-sparse-bump-arena:209).
  - Warm-up pre-pinning happens outside production commands and within the worker memory limit (:201-207).
  - The deallocator plus associated object keep the mapping alive while a command buffer holds the resource (metal_backend.rs:365-380).
  - Mutable Metal buffers use demand-zero no-copy mappings, not zero-filled host Vec copies (packed-result-throughput:239-241). That spike's kernel time was 407.2 ms vs 26.7 ms (:97-107).
- How big: the Metal application buffer cap is 2,000,000,000 bytes (metal_backend.rs:22; metal-optimal:5). Measured working sets are recorded under Q6 below.
- Sizes for a pinned tape cache: not found (never implemented).

### Q3. Anchor indexing (Phase A)
- In-memory, per evaluated file, temporary. It is built after parse/restore and dropped after evaluation; no structural index survives (phase-a:118, :141, :228, :613).
- Contents: preorder / parent / sibling / subtree relations; kind postings; per-kind exact-value vocabularies (≤256 B values, with interior spans included); large-value residue; 8-char prefix postings; predicate caches. Plans are compiled once from the matcher (phase-a:95-106, :530).
- Cost: index construction 1.280 s vs 13.437 s AST dispatch on the 5,722-file supplement, for the first kind/literal version (phase-a:559). Peak temporary index estimate 8.12 → 11.16 MiB (:141).
  - Win: worker CPU −60.95% vs kind/literal on 538 files (:137); matcher calls 4,500,255 → 9,718 (:139).
  - Whole-repo vs pre-Phase-A: native CPU −48.07% (phase-b:156).
- Persisted, query-independent indexes (Schema 12, contentless FTS5) select files before any tape decompression:
  - kind (one token per preorder node; offsets = ordinals);
  - exact value (distinct (kind, value) per tape; file selection only);
  - source word.
  - Sizes: 1.172 B/node kind; 56,983,552 B total = 2.998 B/node = 0.595 B/source byte; build 9.21 s (journal:1337-1356; phase-a:159-179; sha-indexed:185-205).
- The sparse grid still "build[s] the anchor index" per file during preparation (sparse-rule-file-grid:47).
- Metal: one dispatch on a GPU exact-value anchor kernel, 2.61 ms warm; prep 284.72 ms (journal:1302-1312).

### Q4. Workspace store
- Content addressing:
  - Source by canonical SHA-256 with Git blob aliases.
  - Tape by (source, grammar, grammar version, tape format).
  - Snapshot = (root tree, scanner version, grammar set hash, tape format).
  - Workspace = snapshot + overlay.
  - Paths are interned once across snapshots (CAS:352-580).
- File identity: a repo-relative interned `path_id` plus source/tape object references in `snapshot_files` or `workspace_overlay` (CAS:544-577). Spans bind to evaluated workspace bytes; transforming attributes force a read + hash (:514-518).
- Invalidation on edit:
  - The edit transaction changes one overlay path and its object refs (CAS:606).
  - Changed source or language fully invalidates tape, witnesses and findings, with no old-tree reuse (phase-b:11).
  - Candidate rows for the file are replaced in one transaction (plan:96).
  - Results are keyed by canonical source + catalog identity (journal:689-699).
  - Subtree results are ineligible if any overlay lies below the prefix (CAS:45).
- Cached per file:
  - Store: source frame, tape frame, candidate rows (`file_check`), anchor facts, local check results / normalized evaluations.
  - Process: `FileState` with revision-bound `Arc<TapeImage>` until publication, then store handles (workspace.rs:187-210, :1309-1323).
- Cached per workspace:
  - One in-memory snapshot + overlay candidate manifest per active runtime, updated after publication and dropped on eviction (journal:387-389).
  - Normalized evaluation summaries and result identities only (shared-arena:12).
- Memory plan:
  - Bounded regions: SQLite caches, hot frames, restored source/tapes, policy runtimes, GC scratch, result delivery (CAS:698-705).
  - Disposable initial worker (plan:127-137).
  - Retain only compressed source after ingestion; expand only the active file (plan:123, :143).
  - Native retained results < 10 MiB; transport < 64 MiB (plan:113-114).
- Measured memory:
  - Pre-tape baseline: 3.78 GB added, 39.5× source (rerailer-benchmark:122-126).
  - Phase B: −29.98% RSS (phase-b:23).
  - Store-backed single workspace: 362,414,080 B (plan:174); 458,031,104 B (plan:180).
  - Distribution: 532,627,456 B final RSS, 562,642,944 B peak (journal:491-492).
  - Worker peak: 380,174,336 B (journal:245).
  - 46 workspaces held in one process: 1.98 GB cold / 4.2 GB warm; malloc high-water dominates (journal:589-592, :645-677).
  - 47 workspaces with result reuse + eviction: peak 499,499,008 / 524,746,752 B (journal:748, :939).
  - Subtree segments: peak 677,888,000 B (journal:1104).
  - Metal-path peak: 3.19 GB from retained Metal workspace page faults (packed-result-bottleneck-profile:64-69).

### Q5. Arenas, publication, sparse grid
- Bump arena (superseded ownership):
  - Typed `ArenaOffset<Tag>`, checked bump.
  - Classes: task scratch, program output, query sets.
  - Generation-tagged hash tables avoid clears.
  - Bounded Metal workspace pool, with no `MADV_DONTNEED` on retained workspaces (shared-sparse-bump-arena:52-197).
- Shared arena (ABI v4):
  - (file, Program) evaluation owns quotas; batch-global typed physical arenas; two-level reservation.
  - Task-local chunks for hot append arenas (240.8 µs vs 439.5 µs naive).
  - Lazy per-evaluation query tables.
  - 24-B task record.
  - The compact used prefix is the only resident part.
  - `PackedResultPage` compaction releases the workspace before SQLite publication.
  - `NormalizedRowSource` gives repeatable typed rows; fixed-width 512-row statements (shared-arena:3-656).
- Sparse grid:
  - N programs × M files; only eligible (file, Program) cells; one task per (file, Program, root); stable ordinals.
  - Fixed lane grid with a global atomic task cursor.
  - Command partition independent of the ≤64-file publication transaction.
  - Metal durable page limits 4,194,304 results / 8,388,608 captures / 2,097,152 properties / 64 MiB text.
  - Measured CPython 335 programs: 274.52 s, 2,836,709,376 B store, reopen 8.05 s (sparse-rule-file-grid:19-190).
- CPython workplan facts:
  - 82,899,054 tasks → 9,632,501 distinct (file, root) pairs, 8.61 programs each.
  - 88.04% of tasks do ≤16 work units.
  - Metal 100.38 s + SQLite 100.69 s = 91.74% of wall.
  - A few seconds was judged infeasible without root grouping, specialization, and new ingestion (cpython-335:8-237).
- Later grid design: one resident corpus input, <40 invocations, no per-tuple records (metal-production-grid-cutover-design:95-519; metal-optimal:258-290).

### Q6. Consolidated key numbers (selection; all cited above)

| Number | Mark | Cite |
|---|---|---|
| 160.38 MB raw tapes / 95,697,853 B source (≈1.68 B/B tape) | M / D | CAS:78-79 |
| 56,079,347 B zstd-3 source + tape (21.9%) | M | CAS:80-84 |
| zstd L1/L3/L5/L7: 58.26/56.08/53.58/51.59 MB, 466/590/1,269/2,113 ms | M | CAS:91-96 |
| Frame access p50 59–77 µs, p95 374–497 µs | M | CAS:86-87 |
| Tape raw 2,432,984,906 B vs source 1,491,676,179 B (46 workspaces, unique) | M | journal:609-610 |
| 19,007,624 nodes / 95.7 MB (0.199 nodes/B) | M / D | journal:1356 |
| FTS indexes 2.998 B/node, 0.595 B/source byte | M | journal:1354-1355 |
| rec6 = 6 B/node + varint spans + checkpoint/16 | code | tape.rs:16-25 |
| `Rec` ≈40 B/node; `MetalNode` 44 B/node | D(code) | tape.rs:1032-1046; metal_batch_abi.rs:34-46 |
| Decompressed cache 32 MiB logical / 64 MiB mapping / 30 s | code + doc | content_store.rs:48-49; CAS:725-726 |
| Tape-cache hits 0, decompressions 5,663 (full scan) | M | journal:544-547 |
| Warm re-scan: 8,478 decompressions, 1 cache hit, 37.60 worker-s storage + decompress vs 0.003 s matching | M | journal:619-628 |
| Warm 47 workspaces via result reuse: no tape decompressed, peak RSS 499,499,008 B | M | journal:732, :748 |
| Metal first use 213.3 ms vs 6.1 ms GPU | M | shared-sparse-bump-arena:29 |
| Buffer reuse 230.5 → 3.8 ms; 110.6 → 2.4 ms | M | shared-sparse-bump-arena:31, :260 |
| Mapped no-copy warm 1.47 ms vs cold 39.12 ms | M | journal:1276-1278 |
| GPU prep (compile, pack, map) 284.72 / 337.63 / 352.31 ms | M | journal:1310, :1366, :1284 |
| Demand-zero no-copy: resident 474 MB, alloc 3.4 ms, kernel 407.2 ms (vs 26.7 ms) | M | packed-result-throughput:89-105 |
| Logical mutable workspace 7.77/7.96 GB → 2.72 GB | M | shared-sparse-bump-arena:18, :262 |
| Retained Metal workspace peak 3.19 GB RSS | M | packed-result-bottleneck-profile:64 |
| Arena model 313.97 MiB at per-task max vs 1.58 MiB actual output | M (model) | shared-arena:418-434 |
| Phase B subset: CPU +17.45%, RSS −29.98% | M | phase-b:23 |
| Phase A predicate index: −60.95% worker CPU; temp index 11.16 MiB peak estimate | M / M(est) | phase-a:137, :141 |
| Pre-tape workspace 39.5× source RSS | M | rerailer-benchmark:126 |
| Edit p50/p95, store-backed: 4.80/77.73 ms | M | journal:402-403 |
| Targets: < 1 GiB steady, < 500 MiB worker, < 10 GB store, edit p95 < 500 ms | A | CAS:11-14 |
| Metal resource contract: 2e9 tracked bytes, 4 GiB app RSS, 1 GiB compiler RSS | A (limit) | phase5:1639 |
| Deck: 190–380 MB tapes + postings + one transient tree | A | phase-b:176 |

### Contradictions recorded
1. **Memory targets.** The CAS and memory plan target < 1 GiB steady-state RSS (CAS:11; plan:5). The Metal cutover resource contract allows 4 GiB application RSS (phase5:1639), and the observed Metal-path owned RSS was 3.09–3.59 GB (handover:23, :33).
2. **Plan vs journal.** The memory plan marks Phase 7 certification all done (plan:152-164). The journal E14 says the completion review "found gaps in items 2 through 10" and "supersedes earlier statements that described the complete plan as finished" (journal:357-359).
3. **Metal input storage.** The earlier path used no-copy shared buffers over page-backed mappings (journal:1253-1257; sha-indexed:11). The later grid design and current code use private storage with staging blits for large immutable input (metal-production-grid-cutover-design:469-473; metal_register_execution.rs:146-160). No reconciling measurement was found.
4. **Phase A text vs Schema 12.** Phase A body text says no persistent cross-file dictionaries, no word filter, and a temporary per-file index only (phase-a:104, :228). The Schema 12 section, later in the same doc, adds persisted FTS kind/value/source-word indexes (phase-a:159-179).
5. **Before figures for the mutable Metal workspace.** 7.77 GB (shared-sparse-bump-arena:18; packed-result-throughput:120) vs 7.96 GB (shared-sparse-bump-arena:262).
6. **Who owns a Metal thread.** Shared-arena says "Metal dispatches one worker for each evaluation" (:7) and "one thread for each semantic task" (:468). Sparse grid uses fixed lanes plus an atomic task cursor (:119-131). The later grid cutover deletes `task_cursor` and persistent lanes (metal-production-grid-cutover-design:618). These are three successive designs.
7. **Bulk-only claim.** The repository loop uses bulk manifests, but `restore_file_storage()` retains individual tape lookup and fetch fallbacks (journal:1137-1139).
8. **Checkpoint stride in the spike.** 128 in spike S047 prose vs 16 in code/S008 (phase-b:86). Tempo uses 16 (tape.rs:25).
9. **Cache sharing scope.** The distribution model keeps policy caches and session storage per profile under `~/.tempo/profiles/<customer>/<deployment>/` (distribution-model:280-284). The CAS is machine-wide under `TEMPO_HOME` (CAS:253). Neither doc reconciles the two (not found).
10. **GPU speedup claims.** sha-indexed says "No GPU speedup is established yet" (:286), while journal E40-E42 record warm GPU wins on the exact-literal subset. The sha-indexed historical outline predates those entries.

### Explicitly not found
- A pinned uncompressed tape cache, implemented.
- "Unified memory" or "page-locked" wording.
- A measured comparison of private vs shared Metal storage for the same input.
- Per-node bytes stated in prose for `TapeDoc`/`MetalNode` (only derivable from structs).
- Data-layer source size for Phase B's 7,182-file context.
- Whether `FileState.input.source` is dropped after store publication (code not traced to that point).
