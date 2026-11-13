# Index and statistics specification (engine spec, part: indexes)

Status: draft for the engine spec. All paths are relative to the merged worktree (`Pulling-it-all-together`) unless another ref is named. Spike citations use the prior-work indexes: **DI** is `prior-work/ast-index-spike-design-index.md` (topic numbers) and **HI** is `prior-work/ast-index-spike-history-index.md`. Source lines are in ast-index-spike `DESIGN.md` (D) at `9449260`.

The index layer has one job under decision 0004 (the transducer is the concrete IR). That job is file- and document-level candidate scope plus statistics for plan choice. Finding candidate nodes inside a document is the transducer's work: it scans the kind column. Per-node positional postings in SQL serve only the low-priority pure-SQL fallback.

---

## Part 1. Inventory of what exists today

### 1.1 main (origin/main `5cf4246e`), unchanged in the merged worktree

**Content store DDL** (`native/src/content_store/schema.rs`).

| Object | DDL (abridged to the relevant columns) | Line |
|---|---|---|
| `tape_objects` | `tape_id PK, source_id, canonical_hash_id, grammar_id, grammar_version, tape_format_version, codec_id, raw_length, frame BLOB, created_generation, UNIQUE(source_id, grammar_id, grammar_version, tape_format_version)` | 73-86 |
| `file_documents` | `(tape_id, ordinal) PK WITHOUT ROWID, parent_ordinal, role_id, via_id, document_tape_id, source_map BLOB, host_errors BLOB`, plus index `file_documents_by_document(document_tape_id)` | 92-103 |
| `snapshots` / `snapshot_files` | `snapshot_files(snapshot_id, path_id, source_id, tape_id NULL, PK(snapshot_id, path_id)) WITHOUT ROWID`, plus indexes by source, tape (partial) and path | 123-141 |
| `workspace_overlay` | per-workspace overlay of path → source/tape, with indexes by source, tape and path | 156-166 |
| `search_index_sets` | `tape_id PK → tape_objects ON DELETE CASCADE, index_version, node_count >0, complete 0/1, has_errors 0/1` | 245-251 |
| `search_kind_fts` | `CREATE VIRTUAL TABLE search_kind_fts USING fts5(token, content='', contentless_delete=1, detail=full, tokenize='ascii')` | 252-254 |
| trigger `delete_tape_search_index` | `BEFORE DELETE ON tape_objects BEGIN DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id; END` | 255-257 |

`candidate_checks`, `local_check_results`, `context_check_results` and `completed_*` are result caches, not indexes over tapes (`schema.rs:182-238`). No other FTS tables exist. No `fts5vocab` table exists on main.

**Token format.** `kind_index_token(kind: u16) = format!("k{kind:04x}")` (`code_rule_dsl/transducer.rs:525-527`).
- There is one token per preorder node, written in preorder (`transducer.rs:529-548`), so an FTS5 instance offset is the preorder ordinal.
- `kind` is `TapeDoc::transducer_kind`, the decoded local kind (`tape.rs:1973`). Local codes are 10 bits (`KIND_LIMIT = 1024`, `tape.rs:31`; `kf = code | field << 10`, `tape.rs:825`). An ERROR node decodes as `ERROR_SYMBOL = u16::MAX` (`tape.rs:39`), so its token is `kffff`.
- `POSITIONAL_INDEX_VERSION = 2` (`transducer.rs:505`).

**Writer and invalidation.**
- A background backlog runs `TAPES_WITHOUT_SEARCH_INDEX_SQL` (`content_store/search_index.rs:18-28`). It selects tapes of this build's grammar version and tape format that have no `search_index_sets` row of *any* version, in tape-id order.
- Each tape is decoded and `persist_search_index_batch` writes the set row and the FTS row (`search_index.rs:62-110`; `workspace/storage.rs:170-200`), at most 64 tapes per transaction.
- A tape the build cannot decode gets an incomplete set (`put_unavailable_search_indexes`, `search_index.rs:33-60`).
- The invalidation key is the tape id, which is content-addressed: source × grammar version × tape format. The delete trigger keeps FTS in step with garbage collection.
- Derived documents are ordinary tape rows, so each document is indexed under its own tape id.

**Reader: one query, file-level presence only** (`search_index.rs:202-260`, called from `workspace/persisted.rs:330-380`):

```sql
WITH requested AS MATERIALIZED (SELECT ... position, tape_id, version, query FROM json_each(?1))
SELECT r.position, s.complete, s.has_errors,
       EXISTS(SELECT 1 FROM search_kind_fts WHERE rowid=r.tape_id AND search_kind_fts MATCH r.query)
FROM requested r LEFT JOIN search_index_sets s ON s.tape_id=r.tape_id AND s.index_version=r.version
ORDER BY r.position
```

- `r.query` is `k.... OR k....` over a clause's `AnchorPlan::root_kinds`.
- A file is skipped only if every AST clause answers `Absent` (`persisted.rs:368-378`).
- Errors, an incomplete set or a version mismatch answer `Unknown`, so nothing is narrowed. A clause that crosses documents is never narrowed (`persisted.rs:352-355`).

**Problems.**
1. Positions are stored (`detail=full`) but never read. Only presence is read.
2. Tokens are language-local. A per-tape lookup is still exact, because `rowid=` scopes it to one tape of one grammar. Any corpus-wide statistic (`fts5vocab`) or corpus-wide positional query mixes languages, though: `k0042` is a different kind in each grammar. The spike measured this collision as 43.9% foreign instances and a 13% tax (DI 5, DI 21; D S071 L4187-4240).
3. There is no statistic of any kind: no vocabulary table, no kind frequency, no edge counts, and no `ANALYZE` or `PRAGMA optimize` on the store. (`ANALYZE` is used only on DLRE temp tables; see 1.2.)
4. There is no value or term index and no text prefilter at file level. Regex probes (`matcher/anchors/text.rs`: min/max length and literal prefixes from `regex_syntax`) are per-node confirm checks, not an index.

### 1.2 DLRE (merged; `matcher/anchors/sql/`, ~7.0k lines; no uncommitted index changes in the DLRE worktree)

DLRE built a **second, private SQLite store** following ast-index-spike's `tape_store.py`.
- Its location is a `tempfile` database per corpus (`store.rs:44-55`), opened with `schema.sql` (`user_version` 2).
- It copies source and tape bytes in again (`file.src`, `file.rec`, ...), so it duplicates the content store.

| Object | DDL | Purpose |
|---|---|---|
| `ftsk` | `fts5(k, content='', contentless_delete=1, detail=full)` (default unicode61 tokenizer) | positional kind tokens |
| `ftsk_inst` | `fts5vocab('ftsk','instance')` | declared, but bypassed: "fts5vocab cannot push file constraints into its corpus-wide instance stream" (`lower.rs:299-300`) |
| `lang`, `kind`, `field` | catalogs | names |
| `file` | `fid, nodes, path, digest, lang, src, rec, spans, checkpoints, parb, sizb, flags` | copy of the tape and source |
| `fk` | `(fid, lang, kmask BLOB(128))` | 1024-bit kind-presence bitmap per file |
| `file_kind_counts` | `(fid, code) PK WITHOUT ROWID, n, postings BLOB` (delta-varint preorder list) | per-file per-kind count and positions; used as the anchor source (`lower.rs:301-304`) |
| `kfreq` | `(lang, code) PK, n` | running kind aggregate; maintained on put and remove (`store.rs:236`, `:317-321`), and **read by no planner** |
| `parovf`, `sizeovf` | escape tables | the tape's overflow tables, as rows |
| `vidx` / `vlex` / `vcov` | `vidx(lang, term, fid, kind, postings) PK WITHOUT ROWID`; `vlex(lang, term, n)` maintained by triggers; `vcov(fid, kind, bad, residue, missing)` | value postings over case-exact leaf text (`TERM_CAP = 96`, `values.rs:7`), with a vocabulary and coverage, as in the spike's vidx |
| `vocabulary_predicate/term/pending` | regex over vocabulary, filled by `nera_term_matches` (`vocabulary-fill.sql`) | the vocabulary-seed path |
| `decoded_properties/spans/children/files` | one row per decoded node | sidecar caches. This is the spike's deleted design (D S089; DI Top-15 #1) |

**Tokens.** `kind_token(lang*1024 + local)` in base 36 with a `k` prefix (`store.rs:11-29`, `:164`). These are the spike's Option B global codes.

**Statistics.** Shape is chosen at **run time**: each alternative is priced with `SELECT count(*) FROM <relation>` inside the statement and the cheapest is kept (`lower.rs:520-548`). This is spike trap T18. The range estimate is a constant (the DLRE journal reports 32 rows). In DLRE's full run, anchor selection took 537 s and index preparation 137 s, against 58 s of matching (`research/sql-vs-transducer.md`).

`ANALYZE temp.*` runs after each materialized relational stage (`relational/storage.rs:68`, `reachability.rs:87,144`, `forest.rs:159`). That is spike dead end D50: temp tables plus ANALYZE, 57-115x slower.

### 1.3 N1562 (`jbarnes/ner-1562-sql-rerailer` @ `8b6576f0`)
- **Schema:** none new. Its `content_store/schema.rs` is older than main (it predates `file_documents`).
- **Estimates:** constants. `set_estimated_rows(100_000)` or `i64::MAX` in `rerail_matches` `xBestIndex` (`transducer/sql_backend.rs:2461-2468`).
- **Design:** `docs/sql-scope-and-operational-semantics-design.md:609-627` specifies SQL-visible statistics: documents by language, nodes by document and kind, kind frequencies "from `fts5vocab` over `search_kind_fts`", candidate counts per scope recipe, demand counts, multiplicity and failure counts. "Virtual tables use these values for `estimatedRows`." None of this is implemented. `query/plan/` has identities and grouping, no statistics.

### 1.4 SSC (`jeremy/sql-capture-demand-spikes` @ `b87eba3b`)
- `sql/relational/planner_stats.rs:8-95` holds in-memory `PlannerStats { kinds: BTreeMap<u16,u64>, edges: BTreeMap<(u16,u16),u64> }`.
- It is built by walking every node of the batch per run (`from_batch`). It picks the anchor as the minimum summed kind count, and it uses direct-edge counts for edge decisions.
- It is the right statistics (kind plus edge, as in the spike), held in the wrong place: Rust, rebuilt per batch, local codes per dialect. That is against principle 3 and the README rule that Rust-held statistics need justification.

### 1.5 Moti `facts-plan` (Y1, `docs/facts/reports/explore-y1-graph.md`)
- An in-memory Rust reference graph: keyed tables plus memoised keyed queries with recorded read sets, layered base → cluster overlay → worktree delta.
- 72 MB base; 0.73 / 2.40 ms p50 / p95 per edit; exact against native (145,431 edges).
- It is a cross-file **fact relation**, not a statistic or a tape index. It belongs to phase 7 and the relation-contract layer. It is out of scope here, except that its edge counts could feed `xBestIndex` estimates for graph vtabs.

### 1.6 ast-index-spike: index designs and measurements
- **Two-stage execution.** Postings generate candidates and the tape confirms them. An index may over-produce and must never under-produce (DI 3; D S016 L426-436).
- **FTS structural tokens.** Bare-kind tokens cost 2.60 B/node. Positions equal preorder, and phrase queries are exact. NEAR is not recall-safe. File-level pruning saturated at 1.03x *in its SQL executor* (DI 3; D S032-S038 L2211-2651).
- **kidx** is a positional kind posting per (kind[, batch]) with columnar fid and preorder deltas. It cost 1.61 B/node on PG and 2.40 on MariaDB, and took an `identifier` anchor from 1.8 s to 64 ms. Absolute preorder was rejected (+2.6 to 4.2 B/node for 10 ms) (DI 3; D S053 L3319-3348).
- **Option B global codes.** `global = lang_id*1024 + local` is used in the *index only*. The tape stays local, since widening `kf` to 7 B would cost +16.7% on every file. Global codes removed the 13% tax (565 → 533.5 ms) and made the language semijoin unnecessary (D S071 L4187-4240; DI 21).
- **Statistics.**
  - `t_kfreq` holds kind cardinality.
  - `efreq(lang, parent, child, field)` is a direct-edge aggregate with no file id or positions. It costs 0.039 B/node and drives the reverse anchor (1,061 → 844 ms) (D S277 L7103-7129; DI 4).
  - Planning statistics must be per-file or running aggregates that survive a restart, never a corpus scan (constraint C7; D S124 L512-592).
  - Planning is done in one statement with a `ROW_NUMBER() OVER (PARTITION BY qid ORDER BY n, vpos)` argmin (D S241 L6385-6471).
  - Runtime timeout probes and magic cardinality thresholds were rejected (DI 4).
- **Anchor rule.** Use the rarest mandatory concrete variable: 112 → 33 s. Anchoring is "finished" at 1.05 rows read per answer. Put a value anchor before a kind anchor when a text predicate exists (DI 5; D S049-S050, S151).
- **vidx / value postings.**
  - Keyed by case-exact text, with literal lookup or a vocabulary-seed path, and a coverage mask (D S073 L4286-4377).
  - Layouts on PG: per-term TEXT 2.24 B/node, linear hashing 0.91 (shipped) (D S074-S076).
  - In SQLite, RUNS postings took secondary storage from 10.57 to 5.67 B/node (D S128-S129).
  - Exact-token predicates were 15.8x faster on SQLite.
  - The vocabulary must be per language (rust paid for Python vocabulary: 42 → 410 ms) (DI 4b, DI 21).
  - vidx was 87-89% of indexing time; size-tiered runs fixed the per-file insert cost (DI 20; D S132, S140, S146).
- **Regex.**
  - Pattern-derived byte probes (first and last byte sets, length bounds, required literals, class bitmap) gave 3.5x for 0.316 B/node, measured at 0.559.
  - The engine is the lever: a native regex was 2.90x faster than the UDF boundary (DI 4b; D S163-S170).
  - **Trigram rejected.** Only 2.7% of queries carry `#match?`, only 9.2% of regex patterns need it, the comparison behind the earlier claim was wrong, and FTS5 trigram does not support REGEXP (DI 4b; D S144-S161 L1145-1329). One contradiction remains open: SORA and SCPR still propose a document-level trigram index.
- **Word FTS** (srcfts, 1.34 B/node) was deleted because the matcher never read it (DI 3).
- **Budgets.** Tape 6 B/node; all secondary indexes about 5 B/node, with a reference stack of kidx 1.4 + vidx 0.9 + flags 0.25 + trigram 0.9 + 1.55 spare. A new index must fit within that, or replace something at least its own size. Measured SQLite secondary storage was 6.59 B/node (DI 16; SORA C4).
- **Line index.** Byte checkpoints every 4096 bytes cost 0.004 B/node (DI 16).

---

## Part 2. Specification of the new index set

### 2.0 Principles
1. **One store.** All indexes live in the content store (`schema.rs`), keyed by the existing `tape_id`, which is one per document. DLRE's private temp store (`matcher/anchors/sql/schema.sql`) is retired.
   - Its tables either map onto the ones below (`ftsk` → `search_kind_fts`, `kfreq` → `kind_freq`, `vidx`/`vlex` → `search_term_fts`) or are deleted (`decoded_*`, `parovf`/`sizeovf`, `file` copies, `fk`).
   - Spike basis: no sidecars (DI Top-15 #1); a new index must replace one of at least its size (DI 16).
2. **Document granularity.** Every index row is per document (primary or derived) under that document's `tape_id`. File-level answers go through `file_documents`.
3. **Indexes prune and estimate; they never decide.** A clean `Absent` may skip a document. Anything else (an error, an incomplete set, a version mismatch, an unknown token) answers `Unknown` and does not prune. This keeps main's current rule (`persisted.rs:368-378`) and the spike's never-under-produce rule (DI 3).
4. **Statistics are in SQL and are read at plan time only.** Rust reads them in two places: the compiler (one batched lookup) and `xBestIndex`. There is no run-time `count(*)` and no Rust-side rebuild per batch. (README, Statistics; spike C7; trap T18.)
5. **No per-node postings in SQL on the hot path.** Inside a document, the transducer scans the kind column (decision 0004). Positions in the kind FTS are kept only while the pure-SQL fallback and the measurement in open question Q2 need them.

### 2.1 Global kind code space
- `lang_tag: u8` is the existing persisted dialect tag (`language::tape_tag`, `language.rs:411`).
- `local: u16 < 1024` is the tape's kind code. ERROR is local code 0, as already stored in the record (`ERROR_CODE = 0`, `tape.rs:38`). MISSING keeps its own kind and is flagged in `flags`.
- `gcode: u32 = lang_tag * 1024 + local`. This is spike Option B (D S071), unchanged.
- **FTS token format:** `k` followed by `gcode` in lowercase hex, zero-padded to 6 digits (`k%06x`, e.g. `k01402a`). This is ASCII alphanumeric, so the `ascii` tokenizer keeps it whole.
  - Hex over base 36 keeps it greppable and one-to-one with integers.
  - The spike's measured cost is per token, not per digit.
  - [Q1: measure token width vs size.]
- **The tape keeps local codes.** Only the index and statistics speak global codes. The compiler maps (dialect, kind name) → `gcode` through the grammar catalog at plan time. The spike interned the grammar whole at registration to keep bytes order-independent (D S130 L593-622).

### 2.2 Kind index (reuses `search_kind_fts`)

```sql
-- unchanged DDL
CREATE VIRTUAL TABLE IF NOT EXISTS search_kind_fts USING fts5(
    token, content='', contentless_delete=1, detail=full, tokenize='ascii');
-- new: statistics view over it
CREATE VIRTUAL TABLE IF NOT EXISTS search_kind_vocab USING fts5vocab(search_kind_fts, 'row');
```

- **Content.** One row per document, `rowid = tape_id`. One token per preorder node, `k%06x` of the node's `gcode`, space-separated in preorder.
- **Version.** `POSITIONAL_INDEX_VERSION = 3`. The set row records it.
- **Error documents** stay indexed, with `has_errors = 1`. Lookups treat them as `Unknown`, as today.
- **Detail level.**
  - `detail=full` is kept for now. `fts5vocab 'row'` then reports both `doc` (documents containing the kind) and `cnt` (nodes of that kind). With `detail=none`, `cnt` stops counting instances, and the node counts the anchor rule needs would have to come from a separate `kind_freq` aggregate (2.6).
  - Positions also serve the pure-SQL fallback: `fts5vocab 'instance'` with `term = ?`. That form pushes the term constraint but not the document constraint, which is why DLRE added per-file postings (`lower.rs:299-300`).
  - **[Q2: measure `detail=full` vs `detail=none` + `kind_freq` in bytes per node on the gate corpora.** If the transducer's kind-column scan beats posting decode for rare kinds, drop positions.]
- **Reader: presence** is unchanged in shape. It changes only the token function and the version:
  `EXISTS(SELECT 1 FROM search_kind_fts WHERE rowid=?tape AND search_kind_fts MATCH 'k01402a OR k01402b')`.

**Migration from v2.**
1. `TAPES_WITHOUT_SEARCH_INDEX_SQL` changes `NOT EXISTS (SELECT 1 FROM search_index_sets x WHERE x.tape_id = t.tape_id)` to `... AND x.index_version = ?current`. A v2 tape re-enters the backlog.
2. Republication replaces the FTS row (`DELETE` then `INSERT`, as `search_index.rs:93-107` already does) and upserts the set row.
3. An older binary sharing the machine store asks for `index_version = 2`. It gets `Unknown` for v3 rows (`LEFT JOIN ... s.index_version = r.version`), so it never narrows wrongly. Its backlog sees an existing set row and does not republish, so there is no ping-pong.
4. No schema floor raise is needed: the DDL is unchanged and the vocab table is additive (`schema.rs` header comment).

### 2.3 Document and file scope
- **Per document.** Kind presence, term presence, counts and errors are all keyed by the document's `tape_id`.
- **File-level answer.**

  ```sql
  file_scope(file_tape, doc_tape, ordinal, via_id) =
      SELECT t.tape_id, t.tape_id, 0, NULL FROM tape_objects t            -- primary
      UNION ALL
      SELECT d.tape_id, d.document_tape_id, d.ordinal, d.via_id FROM file_documents d  -- derived
  ```

  A clause over document `via` probes only the matching `doc_tape`. This keeps main's `derived_selector_tape` rule: `Unknown` when there are several or none (`persisted.rs:384-400`).
- **Host errors.** A derived document read through host errors (`file_documents.host_errors` non-empty) answers `Unknown`, as today.

### 2.4 Snapshot and workspace membership

These are existing tables; nothing new is stored. Membership is a view:

```sql
CREATE TEMP VIEW IF NOT EXISTS workspace_docs(workspace_id, path_id, file_tape, doc_tape, grammar_id) AS
  WITH files(workspace_id, path_id, tape_id) AS (
    SELECT w.workspace_id, f.path_id, f.tape_id FROM workspaces w
      JOIN snapshot_files f ON f.snapshot_id = w.snapshot_id
      WHERE f.tape_id IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM workspace_overlay o
                        WHERE o.workspace_id = w.workspace_id AND o.path_id = f.path_id)
    UNION ALL
    SELECT o.workspace_id, o.path_id, o.tape_id FROM workspace_overlay o WHERE o.tape_id IS NOT NULL)
  SELECT f.workspace_id, f.path_id, f.tape_id, s.doc_tape, t.grammar_id
  FROM files f JOIN file_scope s ON s.file_tape = f.tape_id
  JOIN tape_objects t ON t.tape_id = s.doc_tape;
```

The overlay's exact column names must be checked against `schema.rs:156-166` when this is implemented. The demand relation handed to the transducer vtab is a subset of `workspace_docs` (2.8).

### 2.5 Edge statistics: aggregate table, not FTS edge tokens
**Choice:** an aggregate over direct edges, following the spike's `efreq`. It cost 0.039 B/node and drove the reverse anchor (D S277). FTS edge or ancestor-pair tokens cost 1.4-6.6 B/node (D S032-S038) and are rejected.

```sql
CREATE TABLE IF NOT EXISTS edge_freq (
    parent_gcode INTEGER NOT NULL,   -- lang_tag*1024 + local
    child_gcode  INTEGER NOT NULL,   -- same lang_tag as parent
    field        INTEGER NOT NULL,   -- 0 = no field, else the tape's 6-bit field code
    edges        INTEGER NOT NULL CHECK(edges >= 0),
    PRIMARY KEY (parent_gcode, child_gcode, field)) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS edge_freq_by_child ON edge_freq(child_gcode, parent_gcode);
-- per-document contribution, so deletion can subtract exactly
ALTER TABLE search_index_sets ADD COLUMN edge_counts BLOB;   -- v3 rows only
```

- `edge_counts` holds the document's distinct `(parent_gcode, child_gcode, field, n)` tuples, sorted, in LEB128 varints (with delta coding on `parent_gcode`).
- **Publish:** merge into `edge_freq` with `INSERT ... ON CONFLICT DO UPDATE SET edges = edges + excluded.edges`, in the publication transaction.
- **Delete:** the Rust garbage-collection path reads the blob and subtracts before deleting `tape_objects`. A SQL trigger cannot decode the blob.
- **[Q3: measure the `edge_counts` blob in B/node.** If it is above 0.15 B/node, drop the blob and make `edge_freq` add-only. Deletion then makes the counts drift. Rebuild the aggregate when the generation counter says more than 25% of indexed nodes have been deleted since the last rebuild. Statistics are estimates and never correctness (spike C7 allows running aggregates).]

### 2.5b Kind size, backend costs, plan feedback (fusion cost-model inputs; `spec-fusion.md` §4.8)

```sql
CREATE TABLE IF NOT EXISTS kind_size (gcode INTEGER PRIMARY KEY, nodes INTEGER NOT NULL, sum_size INTEGER NOT NULL) STRICT;
CREATE TABLE IF NOT EXISTS backend_costs (backend TEXT NOT NULL, device TEXT NOT NULL, op TEXT NOT NULL, ns REAL NOT NULL, measured_at INTEGER NOT NULL, PRIMARY KEY (backend, device, op)) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS plan_feedback (program_digest BLOB NOT NULL, selectivity REAL, cost_ns REAL, at INTEGER NOT NULL, PRIMARY KEY (program_digest, at)) STRICT, WITHOUT ROWID;
```

- `kind_size` gives mean subtree size per kind. It is maintained with `edge_freq` from the same per-document blob (negligible size).
- `backend_costs` is filled by a calibration microbenchmark at install time and again on driver change.
- `plan_feedback` holds per-program selectivity and cost observed at run time. Profile counters fold into it (`spec-tooling.md`).

### 2.6 Kind frequency aggregate (conditional on Q2)
If Q2 drops `detail=full`, add this, mirroring DLRE's `kfreq` and the spike's `t_kfreq`:

```sql
CREATE TABLE IF NOT EXISTS kind_freq (gcode INTEGER PRIMARY KEY, nodes INTEGER NOT NULL, docs INTEGER NOT NULL) STRICT;
```

It is maintained like `edge_freq`, from a per-document `kind_counts` blob in `search_index_sets`. If `detail=full` stays, `search_kind_vocab` provides the same numbers and `kind_freq` is not created.

### 2.7 Term index: word presence per document and term statistics
**Purpose.**
- Prune documents for literal text predicates: `regex` with required word literals, exact identifier matches and `TEXT` equality in transducer programs.
- Supply the value-before-kind anchor statistics (DI 5).
- This replaces DLRE's `vidx`/`vlex`.
- Per-node term positions are not needed: the transducer confirms. The spike's word FTS was deleted only because nothing read it (DI 3). Here it is read by scope.

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS search_term_fts USING fts5(
    token, content='', contentless_delete=1, detail=none, tokenize='ascii');
CREATE VIRTUAL TABLE IF NOT EXISTS search_term_vocab USING fts5vocab(search_term_fts, 'row');
```

- **Content.** One row per document, `rowid = tape_id`. The distinct terms of the document's *named leaf* nodes (identifiers, property names, string fragments, numbers): the text is split by the `ascii` tokenizer rule (runs of ASCII alphanumerics and non-ASCII bytes), case-folded by the tokenizer, and each word prefixed `t{lang_tag:02x}`. Leaves over 96 bytes are skipped and the set row records `term_complete = 0`.
- **Prefix.** The language prefix keeps vocabulary statistics per language, as the spike measured necessary (DI 21). Folding over-produces, and over-production is allowed (DI 3).
- **Lookup.** The compiler reduces a predicate to necessary words, using the existing `TextProbe` extraction (`text.rs`) for required literals. A word that is not alphanumeric gives no pruning.
- **Version.** It lives in `search_index_sets` as `term_index_version` (new column). The term set is query-independent, so it can share the same backlog pass.
- **[Q4: measure size (budget ≤ 1.3 B/node, against the spike's srcfts 1.34) and pruning rate** on the gate rule set's text predicates.]

### 2.8 Regex and text probes
- **Per node:** the existing pattern-derived probes run in the transducer's `TEXT` side programs: min/max length, literal prefixes, first and last byte sets, class bitmap (DI 4b; D S163-S172).
- **Per document:** required word literals only, via 2.7.
- **No trigram index.** DI 4b gives the rejection: 2.7% of queries have regex, 9.2% of those need trigram, and FTS5 trigram does not support REGEXP.
- **Q5 answered** (`research/regex.md` §3): 2 of 109 content clauses (1.8%) have no word-shaped literal, and after the AST rewrite (`spec-transducer.md` §5.1) no data-layer clause is textual. No trigram index. If a future pack has textual clauses, use custom trigram postings (0.127 B per source byte, 0.14 s for 113 queries, 0 misses), not FTS5 trigram (`spec-transducer.md` §5.5).

### 2.9 Statistics views read by the compiler

```sql
-- documents, by language, in scope (store-wide by default; README: per-snapshot only if a shape is measured to flip)
CREATE TEMP VIEW IF NOT EXISTS stat_docs(grammar_id, docs, nodes) AS
  SELECT t.grammar_id, count(*), sum(s.node_count)
  FROM search_index_sets s JOIN tape_objects t USING(tape_id)
  WHERE s.complete = 1 AND s.index_version = 3 GROUP BY t.grammar_id;

-- per kind: nodes and documents (point lookups on term; 0.27 ms each measured, vocab.py)
-- usage: SELECT doc, cnt FROM search_kind_vocab WHERE term IN ('k01402a','k01402b')
-- per edge
-- usage: SELECT edges FROM edge_freq WHERE parent_gcode=? AND child_gcode=? AND field IN (0,?)
-- per term (value anchor)
-- usage: SELECT doc FROM search_term_vocab WHERE term = 't05getusername'
-- workspace-subset count, used only when Q6 says store-wide counts flip a shape
-- usage: SELECT count(*) FROM search_kind_fts f JOIN workspace_docs w ON f.rowid = w.doc_tape
--        WHERE w.workspace_id=? AND search_kind_fts MATCH 'k01402a'     -- 6.6 ms for a common kind (vocab.py)
```

- **Compiler protocol.** One batched statement per compile collects every statistic the rule set's decisions need: a `json_each` of requested keys joined to the three sources.
- **Decisions.** The spike's rarest-mandatory-variable anchor (`ROW_NUMBER` argmin, D S241). Seeded vs scan start, from kind `cnt` against `stat_docs.nodes`. Reverse or edge anchor, from `edge_freq` (D S277). Value-before-kind, from term `doc`. Probe emission only when the probe is selective: about 2/3 rejected on SQLite (D S164). No fitted constant without a recorded measurement (R14b).
- **Never scan the whole vocabulary** (348 ms measured, vocab.py).

### 2.10 Estimates in `xBestIndex`
- The transducer vtab's constraints are `program_group = ?` and `doc_tape IN (...)` (via `sqlite3_vtab_in`), optionally with `root`.
- At plan time the compiler stores, per program, `est_matches_per_doc = cnt(anchor gcode) / doc(anchor gcode)` and `est_docs = doc(anchor gcode)`.
- `xBestIndex` reports:
  - `estimatedRows = min(|IN list|, est_docs) * est_matches_per_doc` when the doc constraint is usable;
  - otherwise `est_docs * est_matches_per_doc`, with `estimatedCost` proportional to the nodes scanned (`stat_docs.nodes` share).
- **No constants:** this replaces N1562's 100,000 / `i64::MAX`. Declare ordering only on `(doc_tape)` when the cursor emits in doc order.

### 2.11 ANALYZE / PRAGMA optimize policy
- **Content store base tables:** run `PRAGMA optimize=0x10002` when a connection opens, `PRAGMA optimize` after any publication batch that changes more than 10% of indexed documents, and `PRAGMA analysis_limit=1000` at connection setup. SQLite's recommendation is opt-in here; the bundled SQLite 3.53.2 has STAT4 enabled (README).
- **Never ANALYZE temp tables.** Delete DLRE's `ANALYZE temp.*` calls together with the materialized stages they serve (D50; DI 11).
- **Do not trust the SQLite planner on CTEs, recursive CTEs or table functions** (DI 9). Shape choice is the compiler's, by 2.9. SQLite's planner chooses only join order over base tables with STAT4.

### 2.12 Plan cache and statistics re-validation

```sql
CREATE TABLE IF NOT EXISTS compiled_plans (
    rule_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),   -- rule document digest
    compiler_version INTEGER NOT NULL,
    grammar_set_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    index_version INTEGER NOT NULL,                              -- POSITIONAL_INDEX_VERSION (+ term version)
    plan BLOB NOT NULL,                                          -- serialized IR programs + SQL composition
    decisions BLOB NOT NULL,                                     -- list of (stat key, observed value, valid range)
    compile_ms REAL NOT NULL,                                    -- recorded separately (README)
    PRIMARY KEY (rule_hash_id, compiler_version, grammar_set_hash_id, index_version)) STRICT, WITHOUT ROWID;
```

- Each shape decision records the statistic keys it read and the interval of values over which it would not change. Example: "anchor A over B holds while `cnt(A)/cnt(B) < 1`".
- **On use,** one batched point lookup (2.9) re-reads those keys. If any value leaves its interval, recompile and replace the row.
- **Cost:** a few point lookups at about 0.27 ms each, against compile time. Rules and statistics change rarely (user clarification).

### 2.13 Publication and incremental update
- **Background:** the existing backlog (`workspace/storage.rs:150-200`), extended to v3. For each batch of up to 64 tapes, one transaction:
  1. upsert `search_index_sets` (v3, node_count, complete, has_errors, edge_counts, term_index_version, term_complete);
  2. replace the `search_kind_fts` row;
  3. replace the `search_term_fts` row;
  4. merge `edge_freq` (and `kind_freq` if Q2 requires it).
- **Edit path, for a per-edit check:**
  - The edited file's new tape is published synchronously for that document only. This is already the store's model: a new source gives a new tape id.
  - Scope for the edited document is `Unknown` until its row exists, so the edit is never wrongly skipped.
  - Statistics changes from one document are ignored for plan validity. Plans re-validate on interval exit only.
  - The spike recorded insert cost as dominated by the number of rows touched per insert, and fixed it with size-tiered runs (D S140, S146). Here each document is one FTS row and a handful of aggregate upserts, so the measured problem doesn't arise. [Q7: measure p95 publish latency per edited document.]
- **Delete:** the existing trigger for `search_kind_fts`, a new trigger for `search_term_fts`, and an `edge_freq` subtraction in the Rust garbage-collection path (2.5).

### 2.14 Size budgets and measurement plan
- **Budget:** all secondary indexes together ≤ 5 B/node (spike C4, DI 16). Source and tape are accounted separately.

| Index | Estimate (B/node) | Basis |
|---|---|---|
| `search_kind_fts` (detail=full) | ~2.6 | spike bare-kind FTS 2.60 (D S032-S038); [Q2] |
| `search_term_fts` (detail=none) | ≤ 1.3 | spike srcfts 1.34 (DI 3); [Q4] |
| `edge_freq` + `edge_counts` | ≤ 0.15 (blob); 0.04 (aggregate only) | D S277 0.039; [Q3] |
| `search_index_sets` | ~0.01 | one row per document |
| `compiled_plans` | negligible | per rule |
| **Total** | **≤ 4.1** | |

- **Measurement plan** (gate corpora: data-layer 9,021 files, CPython):
  - per-object bytes from `dbstat` if it is compiled in, else page counts before and after;
  - publication CPU per node;
  - presence-pruning rate per clause (kind and term);
  - compile-time statistics lookup latency;
  - plan re-validation latency;
  - the Q2 transducer kind-scan vs posting-decode comparison.
  - Proxy metrics stay diagnostics only (README, Measurement gate).

### 2.15 Open questions (measure before locking)

| # | Question | Decides |
|---|---|---|
| Q1 | Token width (`k%06x` vs base 36) vs FTS bytes | token format |
| Q2 | Kind FTS `detail=full` vs `detail=none` + `kind_freq`; transducer kind-column scan vs posting decode for rare kinds | whether to keep positions |
| Q3 | `edge_counts` blob size | exact vs add-only edge statistics |
| Q4 | Term FTS size and pruning rate | whether to ship the term index |
| Q5 | Share of regex predicates with no word-shaped literal | answered: 1.8%; no trigram index (`research/regex.md`) |
| Q6 | Whether store-wide vs workspace-subset counts ever flip a shape | per-snapshot statistics |
| Q7 | Per-edit publication p95 | edit-path design |
| Q8 | Whether `fts5vocab 'row'` counts stay exact under `contentless_delete` republication churn (likely, since FTS5 maintains them; verify) | trusting vocab counts |

### 2.16 What gets deleted
- DLRE's `matcher/anchors/sql/schema.sql` store:
  - `file` copies;
  - `parovf` / `sizeovf`;
  - `decoded_*`;
  - `fk`;
  - `ftsk` / `ftsk_inst`;
  - `file_kind_counts` (unless Q2 keeps positions; even then they come from `search_kind_fts`);
  - `vidx` / `vlex` / `vcov` / `vocabulary_*` (superseded by 2.7; the vocabulary-seed regex path becomes a scan of `search_term_vocab` for the language prefix, which DI 4b supports as the selective-regex path).
- DLRE's run-time `count(*)` choice (`lower.rs:520-548`) and its `ANALYZE temp.*` calls.
- SSC's `PlannerStats::from_batch`, replaced by 2.9. Its decision logic (`anchor`, `incident_edge`) is kept and fed from SQL.
- N1562's constant estimates.
