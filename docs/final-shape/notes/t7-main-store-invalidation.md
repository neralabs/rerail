# T7 notes: main (wolfeel = main 5cf4246e) store, invalidation, tape, transducer docs

Paths are relative to `/Users/jeremy/orca/workspaces/rerail/wolfeel/`. "CAWS" = `docs/content-addressed-workspace-store.md`.

---

## docs/content-addressed-workspace-store.md (892 lines, read in full)

### Limits and boundary (:8-15, :21-51)
- Machine targets: ~100 concurrent, mostly identical workspaces; **steady RSS < 1 GiB; store < 10 GB**; audits stop at 100,000 files or 512 MiB of source; an index holds at most one batch of source (32 files, reading stops at 1 MiB, plus at most one 32 MB file); **edit p95 < 500 ms**.
- **JavaScript never touches the database.** Rust owns connections, SQL, transactions, migrations, caches and GC; JS calls coarse domain operations (:23-34).
- Repository audit (:36-50): `indexRepositoryAsync()` one bounded chunk per call. Rust walks the workspace root like the JS walk:
  - symlinks never followed;
  - host ignores apply; `.gitignore` is followed by default;
  - `.rerailignore` and built-in ignores decide only what may be restored, never what is evaluated.
  - It restores completed clean directories from the store, then reads one batch per chunk, publishes it, and **releases its sources and tapes before the chunk returns**.
  - Bounds: 100,000 files, 512 MiB, 20 failures, five minutes.
  - A complete index records its clean trees for reuse (`completeStorageRevisionAsync()`); that write is a cache.
- Publication stores a file under its Git blob alias when its evaluated bytes equal the blob; only clean paths are hashed and promoted into the snapshot (:53-55).

### Result reuse by clean Git tree (:57-78) — **main's primary warm-path invalidation mechanism**
- Schema 9 records completed clean Git trees (tree hash, logical path prefix, snapshot, **semantic catalog identity**). A tree is ineligible if Git reports an overlay below its prefix; only maximal matching prefixes are restored.
- **Schema 10 stores result pages per clean Git tree and logical prefix**: ≤ 64 direct file paths per page, violations and deterministic failures, each page an independent zstd-3 frame.
- "A matching subtree attaches all of its result segments **without per-file restoration**. The runtime does not create file state, load sources, hash sources, restore tapes, or decode local check results." Result paging merges attached violations with dirty overlays.
- **"Code rules with global clauses continue to use per-file state because they require complete fact evidence"** (:68). Cross-file rules are not reusable by tree on main.
- Bulk restoration: clean Git aliases resolve in one JSON-backed joined query; dirty files need reads and hashes; published in batches of ≤ 64; tape frames load in bulk before workers; missing frames go back to parsing without a point lookup.

### Evidence (:84-142, M)
- 5,725 files, 95.7 MB source, 160.38 MB raw compact tapes, 56.08 MB zstd-3 frames, 169,014 violations; raw (source + tape) ≈ 2.68 B per source byte; 21.9% after zstd. File access p50 59–77 µs, p95 374–497 µs. The zstd level curve is as in tempo-residency (L3 default).
- Machine inventory 2026-09-24: 45 Pi processes, 44 workspaces, 12.19 GiB RSS, 327,565 files, **5.05 GB path-addressed vs 1.47 GB globally unique source (71% cross-workspace reduction)**; 58 GB of source-shaped files under `tmp` dirs.
- Alternatives (:155-190):
  - path-addressed ~3.0 GiB (scan-limit estimate 6.8 GB) would approach the 10 GB limit at 100 workspaces;
  - **content-addressed ~0.87 GiB (+0.1–0.4 GB manifests); 100 workspaces 2–4 GB. Selected**;
  - RAM-only: compressed 0.85–0.95 GiB CA; restored 3.8–4.2 GiB CA (13–15 GiB path-addressed).
  - "Content addressing reduces RAM only when processes share mappings or one process owns the object cache." Selected: CAS on disk + bounded hot data.
- SQLite vs Turso 0.7.2 (:211-232): SQLite wins sequential/point reads, changed-row writes and bulk memory; Turso rejects UPDATE on WITHOUT ROWID. SQLite selected. Pragmas: WAL, synchronous=NORMAL, foreign_keys, busy_timeout=250, `cache_size=-16384`, journal_size_limit 16 MiB (:199-206).

### Ownership and model (:234-373)
- Per-worktree state (session DBs, settings, audits) vs **machine-wide CAS under RERAIL_HOME** holding: source/tape frames, canonical hashes, Git aliases, interned strings/paths, Git tree metadata, immutable snapshots, mutable overlays, leases, GC metadata. Multiple daemons open it; SQLite serialises short writes; SQLITE_BUSY retries with jitter.
- Fixed-width relational model: integer keys in hot tables. `hashes(hash_id, algorithm_id, lane0..lane3)` with SHA-256 as 4 big-endian i64 lanes and SHA-1 padded. Static vocabularies are Rust enums. `strings(string_id, domain_id, content_hash_id, collision_ordinal, value)` with collision-safe interning. `paths(path_id, string_id)`, complete normalised repo-relative paths.
- **Note for facts (Q4):** main already has an integer-keyed, collision-safe string interner in the CAS (`strings` by domain). It is a candidate home for u32 atoms / `text_vocab`. Domains so far: paths, components, workspace roots, repository identities. (Spec §21 / spec-transducer `text_vocab` don't reference it: a gap to resolve.)

### Content objects (:377-492)
- `source_objects(source_id, canonical_hash_id UNIQUE, raw_length, codec_id, frame)`; `source_aliases(alias_hash_id → source_id)` (Git blob aliases; first encounter hashes while parsing and compressing).
- `tape_objects(tape_id, source_id, canonical_hash_id, grammar_id, grammar_version, tape_format_version, codec_id, raw_length, frame, UNIQUE(source_id, grammar_id, grammar_version, tape_format_version))`.
- Publication: validate, hash, serialise before the transaction; look up identities before compression; compress only missing objects; **four symmetric workers share parse/match/tape/hash/compress**; ≤ 64 files per transaction; a busy writer retries the whole transaction (`workspaceProfile.storageRetries`).
- **Documents:**
  - every tape = one source, one grammar, one tree (`TMPTAPE7`);
  - a derived document = its own source_objects row + tape_objects row;
  - derived grammar_id = dialect tag | bit 8 | fingerprint of omitted placeholders (:459-460).
  - `file_documents(tape_id, ordinal, parent_ordinal, role_id, via_id, document_tape_id, source_map BLOB, host_errors BLOB, PK(tape_id, ordinal))` keyed by the primary tape, not a path. Single-language files have no rows.
  - Publication writes all in one transaction; loading checks each derived document against its parent; **a missing derived part makes the stored tape absent (re-parse)**.
  - **The search indexer indexes each derived document as its own row in its own dialect; a clause over a derived document consults that document's index** (:491-492).

### Git (:494-586)
- `gix`, no git subprocess. Tree IDs identify snapshots, skip unchanged walks, reuse manifests, find blob aliases.
- `git_trees(tree_id, tree_hash_id, complete)`, `git_tree_entries(...)`. Complete marker written atomically with the entries.
- Ignore: Git rules, then `.rerailignore`; always-excluded `.git node_modules target tmp __pycache__`, but these **shape discovery and storage reuse only; an audit evaluates every file the host walk selects**.
- Content-transforming attributes (filters, EOL, working-tree-encoding) force reading and hashing the workspace file; spans bind to evaluated bytes. Submodules are separate snapshot roots.

### Snapshots and overlays (:587-643)
- `snapshots(snapshot_id, root_tree_id, scanner_version, grammar_set_hash_id, tape_format_version, UNIQUE(...))`.
- `snapshot_files(snapshot_id, path_id, source_id, tape_id, PK(snapshot_id, path_id))`.
- `workspaces(workspace_id, root_path_id, repository_id, snapshot_id, generation, lease_until, state_id)`.
- `workspace_overlay(workspace_id, path_id, state_id, source_id, tape_id)`: staged, unstaged, untracked, deleted.
- 100 workspaces on one commit share one snapshot. **workspace = immutable base snapshot + mutable overlay** (:574). This is the substrate for "snapshot + overlay delta" in §21 per-edit serving and the §20 `snapshot`/`edit` residency classes.
- `workspaces.generation` exists; candidate for the "input generation vector" of repository facts (§21.7). (Check the code for its semantics.)

### Operations, GC, resources, recovery (:645-829)
- Domain operations only: open/renew a workspace, attach a snapshot, apply an overlay batch, fetch a frame, resolve a path, retire, stats, maintenance. Frames are built before write transactions; insert-or-find; **"A normal edit transaction changes one overlay path and its object references."**
- **GC:** liveness = DB relationships (no refcounts). Roots: active workspaces, retained snapshots, overlays, file_documents, worker pins, in-flight edit pins. Leases with owner token + expiry.
  - Order: retire workspaces → overlay rows → snapshots → tapes (cascade file_documents; derived tapes later) → sources → paths/strings/hashes/trees → WAL/free pages.
  - ≤ 256 rows per delete statement; `BEGIN IMMEDIATE`, a few ms; restrictive FKs; every reference probe indexed.
  - Schema compatibility: `user_version` floor (17 dropped exact-value and source-word FTS families, added the tree-complete marker); additive changes raise a revision.
  - Races: liveness rechecked per delete; creation generation protects the current cycle.
  - Incremental vacuum.
- Disk: 7 GB soft / 9 GB hard / 10 GB absolute.
- Memory: bounded regions; inactive runtimes leave memory; share compiled plans by identity; **initial parsing/indexing in a disposable worker (process exit releases allocator high-water)**; one ContentStore per DB path per process; one maintenance thread.
- Decompressed cache: 32 MiB / 30 s / one 64 MiB mapping / two-mapping compaction / MADV_DONTNEED / oversized objects on the heap (:798-805). **§20 replaces this** with pinned snapshot/edit slabs sized to the working set.
- Recovery: validate codec, length, hash, grammar id/version and format **before frame use**; rebuild from source on failure (§20.3 moves this to once per admission).
- Scale test targets: RSS < 1 GiB, store < 10 GB, **initial worker peak < 500 MiB**, edit p95 < 500 ms.

### Tensions with the final shape to resolve
1. **RSS < 1 GiB steady vs §20 pinned snapshot residency.** The gate corpora are ~100–160 MB resident (+par 32–46 MB), which fits. 100 workspaces on *different* commits would not all fit as `snapshot`-class pinned; the §20.7 residency waves and `transient` class cover that. To state explicitly.
2. **Main's warm path is result reuse per clean Git tree** (Schema 10). The final shape must keep result-page reuse for local rules and define it for cross-file rules (currently excluded, :68), via §21 read sets and dependency cones.
3. The interner (`strings`) vs `text_vocab` vs fact atoms: one interner or several? Open.

---

## Main code: content_store schema and invalidation as shipped (read, cited)

### Schema (`native/src/content_store/schema.rs`, 323 lines)
- The doc's "Schema 9/10/17" numbering is historical. The code is **`SCHEMA_VERSION = 1`, file `content-v1.sqlite`** (:24-33). A floor raise renames the file and starts empty ("the store is a cache"); stores are never migrated (:9-23). The managed store lives at `~/.cache/rerail/`; a checkout build uses `<checkout>/.rerail/`.
- Tables beyond the CAWS doc (:176-262):
  - `store_state(generation)`: a store-wide generation counter.
  - **`candidate_catalogs(identity_hash)`** and **`candidate_checks(catalog_id, policy_hash_id, check_string_id)`**: the rule catalog identity and each check (clause) under its policy hash.
  - `completed_workspace_revisions(workspace_id, revision, catalog_hash_id, completed_at)`.
  - **`local_check_results(source_id, check_id, codec, raw_length, frame, PK(source_id, check_id))`**: a per-file result cache keyed by **source content × check**.
  - **`context_check_results(source_id, context_hash_id, check_id, frame, PK(...))`**: results that also depend on a file context (see `file_context` below), cascading on delete.
  - **`completed_directory_reuse(tree_hash_id, prefix, package_hash_id, catalog_hash_id)`**: "A directory can be restored only when both its Git tree and the package-marker chain that determines Python module contexts agree" (:229-238).
  - **`search_index_sets(tape_id, index_version, node_count, complete, has_errors)`** + **`search_kind_fts USING fts5(token, content='', contentless_delete=1, detail=full, tokenize='ascii')`**: "The kind stream contains one token per preorder node, so an FTS5 instance offset is the preorder ordinal; terminal transducers confirm its candidates." The delete trigger is on tape_objects (:240-258).
- **Main already has positional kind postings in the one content store** (`detail=full`, offset = preorder ordinal). This bears on the open anchor-postings conflict (t1): main is consistent with spec-indexes ("postings in the content store").
- Kind token = **`format!("k{kind:04x}")` of the local code** (`code_rule_dsl/transducer.rs:525-527`), with `POSITIONAL_INDEX_VERSION = 2` (:505). **Confirmed: no `lang*1024` global code on main** (the tape-regularity gap stands).
- Use on main: **presence only.** `get_search_kind_presence_bulk` (`content_store/search_index.rs:200-257`) answers per tape whether any requested root kind occurs (`MATCH 'kA OR kB'`) → Present/Absent/SyntaxErrors/Unknown. Missing, stale or error rows are never negatives. Not found on main: `search_kind_vocab`, `edge_freq`, `kind_size`, `plan_feedback`, `compiled_plans` (all spec-indexes additions).
- Publication of indexes is ≤ 64 tapes per transaction (`search_index.rs:40-44, 63-67`). A maintenance backlog indexes tapes lacking a set (`TAPES_WITHOUT_SEARCH_INDEX_SQL`, :17-26).

### Per-file result reuse (`content_store/results.rs`, `workspace/persisted.rs`)
- `LocalResultCache` is in process, keyed by `(catalog, source, context)` HashKeys, with a byte limit; when full it clears entirely (`results.rs:13-60`).
- **`file_context(input)`** = SHA-256("rerail-file-context-v1", language, **path**, Python module, nearest package.json digest) (`workspace/persisted.rs:524-546`): "The inputs besides source content that one file's clause results depend on … Stored results are keyed by content and this."
- Catalog identity: hashed once per code-rule set, memoised by weak refs (`persisted.rs:548-560`). `grammar_set()` = SHA-256 of `GRAMMAR_BUILD_ID` (:520-522).
- **Invalidation on main is therefore by key, never by deletion:** result = f(source content, file context, catalog/check policy hash). A changed file means a new source_id, so a miss. A changed rule means a new catalog/policy hash, so a miss. A moved file or changed module/package means a new context, so a miss.

### Cross-file on main (`workspace/global.rs`, `crossfile.rs`, `calls_state.rs`, `calls/`)
- Two adapters (`crossfile.rs:1-5`): **evidence** (partitions + relational global evaluation) and **`calls`** (summaries + graph closure). "Adapters retain their relation representation." The scheduler orders producer then consumer phases.
- **Evidence / global `forbid`** (`global.rs:1-80`):
  - each regex or AST leaf of a global query becomes a synthetic clause whose per-file witnesses are recorded as evidence (`Partition{witnesses, failure, syntax}`);
  - existence-only leaves stop at the first witness;
  - the global query is answered over the per-file evidence index.
  - On an edit, the file's partitions are recomputed; `replace_global_changes` diffs the global clause's per-path violations and adds the changed paths to `replaced` (`results.rs:647-686`). A pathless difference makes the result non-delta (`complete`).
  - "A global clause's answer changes in files the apply did not touch: they are replaced too" (`api.rs:1098`).
- **`calls`** (`calls/mod.rs:1-5`; `calls/summary.rs:1-40`; `calls/closure.rs:1-50`):
  - per-file **call summaries** are rule-independent and file-local: definitions, what each calls by qualified name, module/class bindings, class bases;
  - flags ASYNC/CLASS/OPEN/DECORATED/SHADOWED;
  - an unknown callee is "open, never absent";
  - `EXTRACTOR_VERSION = 9`.
  - Summaries are linked by name into one graph. Each `calls` term is a **sparse backward reachability walk from its seed definitions**, cost = region reached; the dense SCC closure is the test oracle; MAX_HOPS 16 for alias/star-import hops.
  - **Stamps** (`workspace/calls_state.rs:1-5`): "the closure is rebuilt from them when either changed, and the clauses that read it (consumers) run after it, on every file whose **stamp (what its calls read of the closure)** changed. A consumer's stored result is kept under its file context and stamp, so any workspace whose closure gives the same stamp reuses it." `result_context = calls_context(file_context, stamp)` (`crossfile.rs:86-97`); `refresh()` returns files whose stamp differs (`calls_state.rs:342-401`).
  - **This is a shipped read-set memo with early cutoff**: a consumer re-runs only if the part of the closure it reads changed. Conceptually it is the same as Moti's read-set memo and Typing's interface-digest cutoff (to verify against T9/T10 notes).
- Also cached: summaries are stored as local results (`summary_publication`/`restore_summary`, `crossfile.rs:99-105`).

### Answer so far to Q6 (invalidation on main)
- **Objects:** content addressing. A source change gives a new source row and tape; GC by relationships and leases.
- **Per-file results:** keyed by (source, file context, check policy within catalog). Directory-level reuse by (Git tree, prefix, package chain, catalog).
- **Cross-file:**
  - evidence partitions per file + recomputation of the global answer with a per-path diff;
  - `calls` per-file summaries + sparse closure + per-consumer **stamps** (a read-set digest) so consumers re-run only when what they read changed.
- Not on main: a generic derived-fact cache by fact_digest, semi-naive/DRed deltas, a generation vector (only `workspaces.generation` and `store_state.generation` exist).

---

## docs/content-store-bulk-publication-design.md (212 lines, read in full)
- Properties (:7-17):
  - one SQLite connection per worker thread (thread-local map; `ContentStore` holds no connection);
  - WAL reads overlap writes;
  - **one bulk lookup query with CTEs and joins** returns source+tape+path+catalog+check+overlay+candidate+result state;
  - hash-prefix indexes on (algorithm, lane0) only, with the other lanes compared in SQL;
  - compression and serialisation before the write transaction;
  - a stale read causes a committed correction pass;
  - **≤ 10 data statements per reconciliation pass** (asserted by tests for cold/warm/race/stale/empty/max batches);
  - a completed-revision marker is the visibility boundary.
- ≤ 64 files per publication. Inputs include **"Source and candidate anchor facts"** and compressed local-result frames. JSON params for identities; BLOB params for frames (:62-79).
- Write passes:
  - object pass: hashes → strings → paths → catalogs → checks → sources → aliases → tapes → source anchor facts;
  - relationship pass: overlays → candidate anchor facts → completion markers → candidate file relationships → local results → completed-revision marker.
- Crash recovery by re-reading incomplete state; monotonic progress required (:140-170).
- **Note:** the "anchor facts" tables named here and in sql-driven-transducer-execution (`source_anchor_facts`, `candidate_anchor_facts`, `workspace_file_checks`) are **not in the current schema** (grep found none in `native/src`). The doc is partly historical; the current schema replaced them with `search_kind_fts` presence.

## docs/transducer-design.md (322 lines, read in full) — main's transducer
- Main's IR is the older one: STOP, KIND, FIELD, FLAGS, TEXT, SIZE, CHILD, NEXT, PARENT, DESCEND, CAPTURE, FORK, GOTO, EMIT (:88-103), with 32-bit words `opcode|mode<<8|operand<<16` (:36-50) and four tables (strings, predicates, labels, longs) (:61-80).
  - No SUPERTYPE, FOLLOWING, cursors, repeats or fork policies (Tempo's are newer, per transducer-ir).
  - Path enumeration, depth-first output order, no dedup (:116-132). Cycles must progress (:149-154). Exhausted resources mean incomplete output, never a negative (:207).
- **The ast-grep compiler accepts only exact root-terminal Smart patterns** (:225-227). Benchmark (M): 8 identifier patterns, 5,725 files, 12,967 ranges; ast-grep on trees 12,608.58 ms, on tapes 817.68 ms, **transducers on tapes 359.22 ms**. Parse 8,653 ms; encode+restore 4,575 ms.
- **`execute_batch` (many programs × many tapes in one native operation) exists, but "production runs node-test programs one node at a time"** (:262-264). Root lists per (file, program); an empty list proves no candidates; missing index coverage means unanchored (:267-275).
- Backends: arm64 JIT (root-terminal only), Metal interpreter + Metal JIT (KIND, literal TEXT, CAPTURE, EMIT only) (:277-293).
- **Full-corpus anchored comparison (M, :295-320):**

  | Backend | Median |
  |---|---|
  | CPU interpreter, no anchors | 380.07 ms |
  | CPU interpreter, exact anchors | 4.93 ms |
  | arm64 JIT, exact anchors | 5.14 ms |
  | Metal interpreter, exact anchors | 49.86 ms |
  | Metal JIT, exact anchors | 33.15 ms |

  - **Exact anchors cut interpreted execution 77×.**
  - **Anchor construction 301.35 ms, outside execution timing.**
  - Generic Metal compile 181 ms; specialised 1.74 ms warm.
  - **GPU medians include packing, buffer creation, submission, sync, reads and ordered output reconstruction; "the selective workload is too small to recover those costs."** This is the per-call setup cost that §20 pinned residency removes.

## docs/sql-driven-transducer-execution.md (182 lines, read in full)
- **Status line (:3): "the `rerail_matches` virtual table has been deleted, with `transducer/sql_backend.rs` and its terminals; code-rule evaluation runs no SQL. Regex clauses are scanned natively (`query::regex_matches`) and AST rules are matched by the grammar matcher. … the transducer `Program` interpreter is exercised by tests and the benchmark backends only."**
- So on main today: **production matching = ast-grep grammar matcher over restored TapeDoc per file, with file filtering by kind presence. No transducer and no SQL in evaluation.**
- File filter history: four-lane bit-set "anchor facts" per source/check → `workspace_file_checks` (:5-35) → replaced by FTS5 kind presence (Schema 12: "Only the kind family remains; the exact-value and source-word families were deleted with their readers", :57-59).
- `candidate_dispatch()` builds `Candidates.by_kind` / `unrestricted`; `evaluate_ast_file()` builds a `FileAnchorIndex` per file for the remaining clauses (:45-53). **This per-file anchor index is the in-memory, per-file, rebuilt-each-time form that the anchor-index-mandatory memory refers to.**
- Historical `rerail_matches(batch)` contract (:73-107): eponymous vtab, required hidden batch, one xFilter per (batch, program_index) executing the terminal over every file, a OnceLock cache, a completion row per pair, `complete=0` + error code; SQL inspects completion before using absence.
- Principles still valid (:147-172):
  - scalar functions only for pure per-value work, never one matcher per row;
  - vtabs for batched terminal work and posting expansion;
  - the SQL schema doesn't change when the backend changes;
  - **"A dispatch loop is valid only when a hard device limit requires partitioning … It must not split work to hide contention or setup costs."**

## docs/rule-scoping.md (110 lines, read in full) — bearing on plan and catalog identity
- Rules come from installed packs (read from npm packs) and the user library, attached per repository **folder**, with settings (`on`, `mode`, `suppressions`, `config`) by owner (checkout → containing checkouts → repository), deepest folder, rule before pack (:35-60).
- **`config` is set per rule for the whole repository "because one compiled rule runs in every folder of a checkout"** (:46). One compiled plan per checkout's rule set; folders only change presence/mode/suppression per path.
- Each file is judged by its owning checkout; nested checkouts own their files (:62-64).
- `rerail-ignore` comments are always honoured (:48).
- For the final shape: the **set of (rule, path) pairs is per-folder**, so a plan's candidate scope must include folder attachment as a scope predicate (file-level), not as a separate plan per folder.

## docs/writing-rules.md (109 lines, read in full)
- User-facing: code rules = `rerail.policy/v1` with scope globs and clauses (regex, or syntax query over Python/JS/TS/PHP/Blade); command rules. `rerail rules test` counts matches across the repo. 69-PR replay evidence for rule value.
- Pertinence: low for the engine shape. It confirms the frontends (regex + syntax query) and that **`rerail rules test` runs a rule over a whole repo**, which is an audit-shaped workload.

---

## T7 synthesis: main as the baseline the final shape replaces or keeps

| Concern | Main today | Final shape (PIAT spec) | Keep / replace |
|---|---|---|---|
| At-rest objects | CAS, zstd-3 frames, hash lanes, interning, snapshots + overlays, lease GC | same (§20.1 "at rest") | **keep wholesale** |
| Tape format | v7 `TMPTAPE7`, one tree per tape, `file_documents` | v9np SoA, 16 B-aligned columns, same document model | replace the encoding, keep the document model |
| Read path | 32 MiB / 30 s decompressed cache; ~3 copies to `Rec` AoS | pinned slabs, decompress directly, no copies (§20) | replace |
| Kind index | `search_kind_fts` detail=full positional, local `k%04x` tokens, presence only | same table + global `lang*1024` tokens + vocab/edge/size stats + postings feeding LEAF (spec-indexes) | extend (re-token) |
| Per-file anchors | `FileAnchorIndex` rebuilt per file in memory | positional postings from the store index; set-stage LEAF | replace |
| Matching | ast-grep grammar matcher per file; transducer test-only | transducer set-mode batches across files and rules; Metal/JIT/interp | replace (the matcher stays as the rehydration oracle) |
| Per-file results | `local_check_results`/`context_check_results` keyed (source, context, check) + directory reuse by Git tree | keys = (rule set, finding keys per (rule, file), outcome); same key discipline | keep the key discipline |
| Cross-file | evidence partitions + global re-answer + per-path diff; `calls` summaries + sparse closure + **stamps** | §21 named facts, fact_digest caches, Δ/DRed, read-set re-run | generalise (`calls` stamps are the shipped precedent) |
| Publication | ≤ 64 files/txn, ≤ 10 statements/pass, correction passes, completion marker | spec-indexes ≤ 64 tapes/txn | keep |
| JS boundary | Rust-only DB | — | keep |
