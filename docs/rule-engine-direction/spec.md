# Unified rule engine: specification

Status: draft for review. Open measurements are listed per section. Every decision here is recorded in `decisions/` with citations to prior work (`prior-work/README.md`). Supporting research is in `research/`.

This document is normative: it states what the engine is. `README.md` holds the direction and evidence, and `architecture.md` holds the integration plan.

---

## 0. Targets and scope

All three are required:

1. **Per-edit checks.** One edited file × all rules, eventually while typing, which needs the GPU.
2. **Full-repository audit** in 1–2 s.
3. **Everything runnable inside an outer SQL query.** Every rule also has a pure-SQL lowering on SQLite. This is lower priority.

Out of scope for the engine, and owned by the integration: presentation, ordering, ids, history and payload rendering (§12).

## 1. Principles

1. SQL drives execution. Native code runs only behind relations the planner can see.
2. **The transducer is the concrete execution IR for local structure.** It stays small and backend-uniform. Local, navigational conditions lower to it; intermediate relations that several rules share, joins between independent sets, and fixpoints over names are **named derived facts** defined separately from rules and cached in SQL (§21); base facts that need a resolver come from native producers. A new transducer operation must be local, cheap on every backend and used by several rules (`spec-transducer.md` §0).
3. One relation, two bindings: a native vtab and a pure-SQL definition, differentially tested.
4. Query shape comes from statistics held in SQL (§5). Rust reads statistics; it does not own them.
5. Work is batched across files, and across rules only within identical candidate scope.
6. Compiled plans are cached. Compile time is reported separately.
7. Executors (interpreter, JIT, Metal) are swappable behind the same IR.
8. Findings are a set (§12). No executor reproduces order, witnesses, spans, messages or operational behaviour.
9. Rules carry no SQL or code (§10).
10. Decisions cite prior work.

## 2. Layers and data flow

```
 rule documents (rerail.policy/v1, declarative only)
        │ compile (cached by rule digest + compiler version + stats decisions)
        ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ plan: per-document programs (transducer IR, fused per §7)    │
 │       + cross-file composition (SQL over relation contracts)  │
 └──────────────────────────────────────────────────────────────┘
        │ outer SQL statement(s)
        ▼
 SQLite ── demand(program_group, doc[, root][, exports]) ──► transducer vtab
   │                                                         │ CPU interp / JIT / Metal
   │ ◄── t_match, t_export_*, t_count, t_outcome, fact_* ────┘
   │ joins, anti-joins, repository counts, cross-file closures, allowlists
   ▼
 finding keys + per-(rule, file) outcome ──► consolidation (§12) ──► consumers
```

## 3. Tape: v9np (decision 0003)

Measured in `research/tape-v8-measurements.md` over 24.2M nodes: CPython 3.13 Lib, nera-data-layer `impl/`, and rerail's TS, Rust and PHP. Code is in `experiments/tape-v8/`. All variants decode to identical checksums.

**Content: structure only.** The tape holds exactly the parser's tree for one document in one grammar, and nothing else:

| Column | Width | Definition |
|---|---|---|
| `kf` | u16 | 10-bit **folded kind code** plus 6-bit field. Named, extra, missing, error and contextual aliases are distinct codes in the kind space, assigned once per grammar at registration. This takes 194–277 codes per grammar of 1,024, and `extra` is uniform per kind except 1–2 nodes |
| `size` | u16 or u32, per document | subtree size. u32 only when the document has ≥ 65,535 nodes (24 of 6,251 files) |
| `base` | u32 per 16-node block | `start(i) = base[i/16] + off[i]` |
| `off` | u16 (u32 if any block spans ≥ 64 KiB; never seen) | offset of the start within the block |
| `tail` | u16 (u32 per document if needed; never seen) | `end(i) = start(i+size(i)-1) + tail[i]`: the end measured from the start of the node's last descendant (for a leaf, its length) |
| `par` | optional, derived | not stored at rest. Rebuilt with a stack in 4.1–5.1 ns/node, once per resident tape, as a sidecar (§20.6). Store it (+2 B/node) only if SQL parent climbs prove hot: a climb is 1.6–3× faster than interval containment |

- **Layout at rest:** structure-of-arrays (separate `kf`, `size`, `base`, `off`, `tail` columns), plus a one-byte width descriptor and the node count.
- **Not in the tape:**
  - the digest and source length, which belong to the store row;
  - alias string tables and flag lists, which become folded codes;
  - line tables, which are recomputed once per resident source (§20.6);
  - derived-document data, which lives in `file_documents`.
- **Size:** 8.25–8.55 B/node logical, 0.93–1.01× v7; 0.94–1.06× v7 after zstd. The 30% growth of the first v8 proposal is gone. That proposal (absolute u32 start/len) measured 1.24–1.59× v7, and 1.47–1.90× after zstd, because files of 64 KiB or more hold 47% of CPython's nodes and absolute positions compress poorly.

**Benefits measured against v7:**
- **Rust random access** to (kind, parent, size, span, named): 113–490 → 23–199 ns. Random span alone: 33–364 → 13–57 ns.
- **SQL**, using the ast-index-spike pure-SQL technique on SQLite 3.53.2:
  - span extraction is 2.7–3.8× faster in columns, and 5.6× interleaved (v7 6.2–9.5 s → v9 1.7–2.5 s);
  - structure queries are no faster, since v7's rec6 is already regular. Time there is dominated by which blob SQLite reloads: interleaved is about 2× faster for single-load anchors (`has`), and columns 3–4× faster for recursive re-joins (`nthChild`);
  - kind+size layout is chosen on the real anchored workload.
- **JIT/Metal:** columns are bound in place from residency slabs (§20); there is no upload on Apple UMA. The 44-byte `MetalNode` restore (3.7–7.8 ns/node) is deleted. Stratified range execution (§8) reads `kf`, `size` and the derived `par` directly.
- **No gain:** sequential scans. v9 recomputes ends at 8.8–24 ns/node, against v7's 3.8–4.3. A sequential walker carries ends on a stack, which is not yet measured.

**Readers:** the checkpoint walk, the overflow and flag binary searches, the alias tables and DLRE's `decoded_*` caches are deleted. `TapeDoc::restore` becomes optional; ast-grep's first/next/prev links are derived in one stack pass.

**Blade stores exactly what tree-sitter-php stored:**
- **The compiled-view document's tape is the plain tree-sitter-php tape of the view text.** Today nodes over synthetic placeholder segments are dropped, so directive calls have no `function` child. Skipping synthetic text becomes a matcher and evidence concern, applied through the segment map's omit flag (`SegmentMap` flags bit 2).
- **Errors are stored as tree-sitter emits them, the same way ast-index-spike does:** ERROR nodes carry the ERROR folded code and their real span, and MISSING nodes are zero-width nodes with a MISSING folded code. The Forte primary tape keeps its lexer-error leaves, provided they are generic ERROR nodes in the same form (the ERROR code and a real span) and need no Blade-specific decoding. Any structure Blade stores must be expressible in the generic tape. No grammar-specific tables are allowed.
- **The view text, segment map and host errors** are a derivation sidecar keyed by (template digest, via), recomputable from the template.

**Before locking:**
- re-measure the code space and aliases on Rerail's patched grammars (the spike used the crates.io grammars: 0 aliases, 0 missing);
- measure the sequential walker with stack-carried ends;
- choose kind+size layout for SQL on the anchored workload;
- decide whether to keep `par`, from SQL climb frequency.

## 4. Documents and sidecars

- **One tape per document.** A file has a primary document and zero or more derived documents (embedded programs, generated views). Each has its own v9np tape in its own grammar: the segment model (ast-index-spike S070). v5's splice into one tape was the rejected design.
- **Document table:** `file_documents`, which exists today (`schema.rs:92`). Each row is `(ordinal, role, via, parent_ordinal, tape_id, source_id, source map, host errors)`.
- **Source maps:** fixed `(doc_start, parent_start, len, flags)` triples. They are used only to normalise finding locations (§12), to apply host suppressions, and to skip synthetic segments (flag bit 2).
- **Sidecars**, keyed by tape or template digest, optional and recomputable:
  - line starts;
  - host-error and lexer-error ranges;
  - the Blade view derivation.
- **Execution batches** reference resident documents through a batch document table (§20.5). Columns are never concatenated or copied into a batch. Every column starts at a 16-byte offset in the at-rest blob, so one decompression into a slab yields aligned columns (§20.2).

## 5. Indexes and statistics

Fixed now:

- **5.1 Global kind codes.** Kind tokens in every index use a code space that is unique across languages, so vocabulary statistics never mix languages. ast-index-spike Option B, `lang·1024 + local`, measured −5 to −22% latency and removed foreign hits exactly. Main currently emits language-local tokens (`k{kind:04x}`, `native/src/code_rule_dsl/transducer.rs:525`). SSC's `fts5vocab` reader had to treat counts as estimates because those tokens overlap across dialects (`docs/sql-matcher-fragment-transition.md:225` on its branch).
- **5.2 Kind index.** Contentless FTS5 with positions equal to preorder ordinals. It provides kind frequencies per document and per file through `fts5vocab(…,'row')`, and positions through `'instance'`. Measured: a point lookup takes 0.27 ms and a workspace subset 6.6 ms. Scanning the whole vocabulary takes 348 ms, so it is forbidden at plan time.
- **5.3 Exact specification:** `spec-indexes.md` Part 2 is normative. Part 1 inventories what exists today. In summary:

| Index | Form | Purpose | Budget (B/node) |
|---|---|---|---|
| `search_kind_fts` v3 | existing contentless FTS5, tokens `k%06x` of the global code, one row per document | kind presence per document; `search_kind_vocab` (`fts5vocab 'row'`) gives kind node and document counts | ~2.6 (Q2: `detail=none` + `kind_freq` if positions are unneeded) |
| `edge_freq` | aggregate table (parent, child, field) → edges, with a per-document `edge_counts` blob for exact deletion | edge and reverse anchors (spike `efreq`, 0.039 B/node) | ≤ 0.15 |
| `search_term_fts` | new contentless FTS5, `detail=none`, language-prefixed words of named leaves | document pruning for literal text predicates; value-anchor statistics | ≤ 1.3 |
| `workspace_docs` | view over snapshot, overlay and `file_documents` | membership and demand source | 0 |
| `compiled_plans` | table keyed (rule digest, compiler version, grammar set, index version), with stored decisions and their valid intervals | plan cache and statistics re-validation | negligible |

  - **All secondary indexes ≤ 5 B/node** (spike C4). No trigram index; regex probes run in the transducer.
  - `PRAGMA optimize` on base tables only; never ANALYZE temp tables.
  - `xBestIndex` estimates come from the plan's stored statistics, never constants.
  - DLRE's private anchor store is retired.
  - Open measurements: Q1–Q8 (`spec-indexes.md` §2.15).

## 6. Transducer IR

**Normative text: `spec-transducer.md`.** It states the lowering principle (local, navigational conditions lower into the transducer; shared and relational intermediates are named derived facts, §21), the machine model, every operation with encoding and semantics, value/capture/output programs, text and regex handling, the complete lowering of the rule language, the verifier, inputs and outputs, and backend obligations. This section summarises it.

Base: Tempo's IR (`research/transducer-ir.md` §1). That is one 32-bit word per instruction, path-forking execution over one document from root nodes, and typed output arenas, with:

- **tests:** `KIND`, `FIELD`, `FLAGS`, `TEXT`/`TEXT_EVAL`, `SIZE`, `SUPERTYPE`;
- **navigation:** `CHILD`, `NEXT`, `PARENT`, `DESCEND`, `FOLLOWING`;
- **cursors, captures and control:** `SAVE_CURSOR`/`RESTORE_CURSOR`, `CAPTURE`, `REPEAT_*`, `FORK`, `GOTO`, `EMIT`;
- **side programs:** value (regex as byte NFA, with DFA forms), capture-predicate and output programs.
- **text:** token-text regexes are evaluated once per distinct text and read by `TEXT_ID`; other node text uses `TEXT_DFA`; whole-file text clauses over code are rewritten as AST rules (`spec-transducer.md` §5).

Changes adopted (decision 0004):

| # | Extension | Purpose |
|---|---|---|
| E1 | **Set mode** | No output order and no capture-containment reduction. Multiplicity and witness choice are free (§12) |
| E2 | `EXISTS p` / `NOT_EXISTS p` sub-programs from the current node | `not`, `stopBy: <rule>`, nested rules, semi- and anti-joins inside a document |
| E3 | Axes `PREV`, `PRECEDING`, `ANCESTOR`; read child index | `precedes`, `inside`, `nthChild` |
| E4 | Bounded `COUNT p` along an axis, compared to a constant | relation counts |
| E5 | ast-grep pattern opcodes | patterns lower to IR instead of a separate interpreter |
| E6 | Bounded `CALL`/`RETURN` for utilities, with a static depth bound | `matches`, utility reuse |
| E7 | Seeded starts and capture-bearing demand | demand from SQL with bound exports |
| E8 | `SIDE f` reads of per-node fact columns, and enclosing-scope facts | owner and qualname, references |
| E9 | Document switch for derived documents | embedded and host rules |
| E10 | Per-operator capability declarations | the verifier proves each admitted program executable on every backend, Metal included. There is **no backend fallback** |
| E11 | A completeness bit and reason per (program, doc) | budgets and hard resource limits yield `Unknown(reason)`, never a silent reroute |
| E12 | Lazy range enumeration for `DESCEND`/`FOLLOWING`/`ANCESTOR`, plus `*_KIND` superinstructions (`DESCEND_KIND k` scans `kf` over `[i+1, i+size)`) | `DESCEND` becomes a one-entry cursor. Pushed paths fell 55×; it moves out of Metal's dynamic-frontier class |
| E13 | Program-group header: a dense root-kind → entry-pc table, a label table, per-pc reachable-label masks | multi-rule tries and demultiplexing |
| E14 | **Set stage:** batch passes that materialize per-node sets (bitset columns or sorted postings), read with `MASK t` / `IN_SET t`. Includes subtree, ancestor and `stopBy`-gated passes, one per nesting stratum | relations and nested rules as set operations (§7, §8). Measured 25–54× on CPU, and 0.005–2 ms on Metal |
| E15′ | **`FACT` set:** a set-stage definition loaded from a fact keyed on (doc, node) as a posting list, read by `IN_SET`/`COUNT_SET` | results of joins and fixpoints over names computed as derived facts (§21). Replaces the withdrawn keyed sets (E15) and bottom-up folds (E17, now the native producer `py.flow_exit`); decision 0005 |
| E16 | **Order against a capture:** `ORDER cmp, capture` compares preorder position with a captured node | "assigned before use", earliest anchor, alias imported earlier |

The following are explicitly excluded: first-match binding, three-valued logic as semantics, exact work charges, and memo-replay accounting (decision 0001).

Verifier obligations:
- The rule graph must be stratifiable. Recursion is positive within a stratum, over set-stage axes and local tests only, in any direction. Negation and counting are allowed only between strata (`research/metal-enqueue.md` §3).
- Bounded recursion (E6), and progress on every cycle.
- Every operator declares its implementation on every backend. A program is admitted only if every operator it uses is implemented on all backends, Metal included.
- Strata, live sets and arena size bounded at plan time.
- Set semantics only.
- Every loop bounded by a batch constant (depth, fan-out or range).
- Table sizes within the encoding limits.

## 7. Lowering and fusion

**Normative text: `spec-fusion.md`.** It gives the formal set-mode semantics; exact encodings of every new operation in Tempo's word layout; the group header format; the fusion algorithm in pseudo-code; the cost model with every parameter sourced; legality per transformation; limitations with the planner's response to each; and the test and debug obligations. The evidence is measured on CPU in `research/fusion.md` (8.54M-node Python corpus; code in `experiments/fusion/`). The Metal side is in `research/metal-recursion.md`.

**Two-level execution model.**
- **Instruction programs** handle node-local tests and bounded paths (Tempo classes 1–3).
- **The set stage (E14)** handles relations and nesting: an inner rule is materialized as a per-node set by a pass over the regular tape, and the outer rule tests membership.
- Both the Metal stratified-range design and the CPU mask passes are this model.

**Frontends.**
- `AstMatcher` (ast-grep-compatible rules) is one frontend; SCM is kept as a test frontend.
- If a rule feature cannot lower, it becomes a derived fact (§21) or a native producer. The IR is extended only under `spec-transducer.md` §0 rule 3.
- **Program boundaries follow shared state:** capture rollback, recursive utility calls and ordered searches that share state stay in one program (SSC `docs/sql-matcher-fragment-transition.md:54-56`).
- Local constructs within one document lower to the IR: `all`/`any`/`not`, every relation and `stopBy` mode, `field`, `nthChild`, non-recursive utilities and monotone single-axis recursion, capture equality within a match, counts along an axis, transforms. Joins between independently matched sets and recursion through names lower to derived facts (§21; lowering rule L5).

**Fusion forms:**

| Form | Lowering | Measured (CPU) |
|---|---|---|
| OR / `any` | `FORK` + `EXISTS` branches; shared prefix; first emit ends the task (set mode) | 1.4–2.4× |
| AND / `all` | `EXISTS A; EXISTS B; EMIT` with early exit, conjuncts ordered by `cost/(1−selectivity)` | 1.5–2.0× against separate programs + join; wrong order costs up to 1.8× |
| NOT | `NOT_EXISTS`; also gives `stopBy: <rule>` | 1.3–1.9×; 0.84–1.07× when A barely filters |
| `has`/`inside` | lazy cursor → `DESCEND_KIND` → set-stage mask pass, chosen by the cost model | mask pass 25–54× for 210 programs |
| Multi-rule batches | root-kind dispatch + tries with shared prefixes and labels | K=512: 2.3–2.5×, 14.3× fewer tasks; K≤32 break-even |
| Counts | `COUNT` with early stop; prefix counts per target | — |
| Utilities | inline by digest when small; bounded `CALL` when large and shared; a mask target when used as a relation target; recursive ones run as set-stage fixpoints with device-side enqueue on Metal (§8) | — |
| Capture equality across matches | within a document: `EXISTS` + `TEXT_EQ` when one side reaches the other; otherwise L5 (candidate exports, SQL semi-join on `(doc, key)`, optionally fed back as a `FACT` set); across documents: SQL hash join over `t_export` | — |

**Measured on the 60 data-layer rules** (`research/fusion-model.md`, 28.2M nodes, identical findings in every mode). These results set the priority order:

1. **Shared sub-rule evaluation is the primary fusion.** All 535 queries compile into one hash-consed rule graph (279,204 expansions → 6,821 operations). Each shared node is evaluated once per document into a lazy per-file memo column. Measured gain: 1.4–3.1×. The gain comes almost entirely from regexes no longer re-scanning large nodes: 2.59 TB of text scanned → 3.1 GB.
2. **Masks only for range-scanning relations** (`has`/`inside` with `stopBy: end` or a rule, sibling ranges), and only when the cost-model threshold in `spec-fusion.md` §4 holds. Measured: break-even to 1.3× over item 1. Eager masks for every relation are slower (0.74–1.5×), and neighbor relations never qualify.
3. **Root-kind dispatch and tries** are kept but are not expected to pay on the data-layer ruleset: 0.9–1.2×. The largest group is 166 queries (`call`), and candidates are already selective. On the shipped Python catalog the evidence points the other way: fragments with no matches anchored on very common kinds take 39% of native time in the top 10 alone (SSC `largestNativeFragments`). Tries are re-evaluated per ruleset from `plan_feedback`.
4. **Rewriting the relational inputs** (decision 0002) is the largest single item. All 191 relational-clause inputs take 146.6 of the 171.5 s `sep` total (about 85%), mostly copying the tree into rows, dedupe-only inputs and literal feeders. Rewritten as transducer programs (with `ORDER` (E16), `FACT` sets, derived facts (§21) and lowering rules L1–L5), the six heaviest groups are estimated at 0.40–1.29 s instead of 139.6 s, and exported rows fall from 6.7M to essentially zero (`research/heavy-inputs.md`). Part of the dedupe saving overlaps with item 1.

- **Per edit:** fused dispatch without an index plus masks for the one file. That is as fast as anchored execution with prebuilt postings. Measured: one file × 535 queries, parse included, p50 2.9 ms, p99 189 ms, worst file 1.3 s, with a cached plan. A cold plan compile takes 5–6 s, so the plan cache is mandatory for per-edit use.
- **Legality:** every fusion above relies on the findings contract (§12): set semantics, free witnesses, budgets → `Unknown`. A rule whose meaning is ordered becomes `A AND NOT_EXISTS B`.

## 8. Executors

- Rust interpreter (reference), CPU JIT (arm64), Metal interpreter, Metal JIT. **Every admitted program runs on every backend. There is no fallback from Metal to CPU/JIT.** A construct Metal cannot execute is either implemented on Metal, including through device-side enqueue for unbounded programs (§8), or rejected by the verifier with a reason. A hard device limit yields a reported `Unknown(resource)`, which counts against the Metal coverage metric and its CI gate.
- Executors read tape columns directly. There is no per-batch repacking.
- **Data-dependent recursion on Metal: stratified range execution** (`research/metal-recursion.md`, measured on an M5 Max over 5.4M nodes).
  - Tree relations are range arithmetic over the preorder tape: subtree = `[r+1, r+size[r])`. `stopBy` uses a subtree skip or a nearest-pruning-ancestor column. `inside` is a parent walk bounded by maximum depth.
  - Nested rules (`EXISTS`/`NOT_EXISTS`, `stopBy: <rule>`, nested patterns, non-recursive utilities) are strata: the inner rule is materialized as a sorted node set (a posting list or a bitmap), and the outer relation is a range query or set operation over it. Dispatches are O(nesting depth) per batch, in one command buffer with GPU-written indirect arguments.
  - **Recursion uses device-side enqueue** (`research/metal-enqueue.md`, measured). The set stage is stratified Datalog. Positive recursion in any direction terminates in ≤ n waves per stratum, because each node enters the frontier once. Negation and counting are allowed only between strata.
  - **Execution: hybrid waves scheduled by the GPU.**
    - Device-wide waves use indirect dispatch sized by a one-thread `prep` kernel. An empty slot is a no-op.
    - At ≤ 1,024 frontier nodes, the GPU hands off to a single-threadgroup `threadgroup_barrier` loop, which has guaranteed progress.
    - The CPU only encodes slots and submits.
    - Measured: 1.2–1.6 µs per wave; 100,000 waves in 113 ms in one command buffer; a 27-wave fixpoint over 5.4M nodes in 1.57 ms. Per-wave CPU round trips cost 386–890 µs.
  - **GPU-encoded ICB commands** are used only when the next kernel depends on the data (18–30 µs per wave). A GPU write to an ICB slot is not visible within the same `executeCommandsInBuffer` call (measured), so each such slot gets its own execute call.
  - **Live frontier at the end of a command buffer:** chain command buffers with resident state (about 6.4 µs per wave). The CPU sees only "not done".
  - **Measured:** postings with range search took 0.005–0.05 ms per query. The interval scan took 0.1–10.6 ms (0.86 ms with a SIMD-group scan on huge roots). Stacks overflowed at cap 64. A persistent-threads queue is rejected because Metal gives no forward-progress guarantee within a SIMD group.
  - **Memory** is sized at plan time from the bounds in the kind statistics. There are no per-path arenas.
  - **No CPU fallback** (superseded): unbounded programs use device-side enqueue, designed in `research/metal-enqueue.md`. Hard limits yield `Unknown(resource)`, reported and gated.
  - The 600-dispatch and 2 GB guards are replaced by bounds reported at plan time. The arena is exact, because frontiers cannot overflow. Batches are cut to fit the device budget, and subtree-local programs chunk along preorder ranges.
  - **Resource outcomes:** `doc_exceeds_device`, `batch_split_exhausted`, `device_lost`/`command_buffer_error`, `plan_bound_violation`.
    - Each is reported per (rule, document) and feeds `metal_coverage`.
    - The CI gate requires 1.0 for every admitted rule on the gate corpora.
    - The verifier rejects unstratifiable recursion, recursion through anything other than set-stage axes and local tests, and live sets that cannot fit the largest corpus document. Each rejection carries a reason.
    - Constructs with no lowering yet are tracked compiler gaps, never executed elsewhere.
- **Inputs:** executors bind residency slabs, never per-batch copies (§20.4, §20.5). Batches are width-homogeneous (u16 or u32 `size`), with kernels specialised by function constants.

## 9. The vtab boundary

- **Demand in:** `demand(program_group, doc[, root][, bound exports…])`. One `xFilter` per (program group, batch). `estimatedRows` comes from statistics (§5).
  - Scalar demand (roots only) uses `sqlite3_vtab_in`.
  - **Capture-bearing demand uses a demand handle.** An aggregate function `demand_<group>(request, doc, root, inputs…)` builds an immutable typed request relation (node, list, count and span cells, each carrying its document identity) and returns a single-use handle. The vtab receives it through an equality constraint on a hidden `demand` column. Handles expire at batch end, are cleared on success and on failure, and reject a second consumption. An empty aggregate is an empty batch, never a scan. Transport is bounded (SSC: 1M requests / 128 MiB). Source: SSC `sql/fragments/demand.rs` (`research/ssc-fragments.md`).
- **Relations out,** all narrow and typed, with set semantics:

| Relation | Columns | Use |
|---|---|---|
| `t_match` | `program, doc, root` | finding keys |
| `t_export_<schema>` | `program, doc, root, <declared exports>` | join keys for cross-file composition; one schema per export shape |
| `t_count` | `program, doc, group…, n` | per-document reductions |
| `t_outcome` | `program, doc, complete, reason` | Complete or `Unknown` |
| `fact_<family>` | e.g. `fact_ref(doc,node,name,kind)`, `fact_def(doc,node,qualname,kind)`, `fact_import(doc,module,name,alias)`, `fact_call(doc,node,callee)`, `fact_scope(doc,node,owner,qualname)` | base facts from fact programs and native producers; rule-independent, per document digest, cached |
| `d_<name>` | typed per definition | named derived facts (§21), partition `document` or `repository`, cached by `fact_digest` |

- **Completion is per domain.** Absent rows within the requested domain mean false; only `t_outcome` reports incompleteness. A resource failure discards that (program group, document)'s provisional rows; other documents in the batch publish normally.
- **Forbidden:** rows per node or per navigation edge, wide union tables, and per-node or per-term callbacks.
- **Measured cost.** With completed, typed output only, SQL plus boundary overhead was 4.06 s of 83.56 s of matching (4.9%; about 55 ns per candidate root, 1.2 ms per `xFilter`) over 73.2M candidate roots and 3,289 `xFilter`s on 5,725 files, with exact finding parity (SSC `benchmark/sql-matcher-fragment-integration.json`). The SQL state-machine emitters it replaced ran at about 3× native. The boundary is cheap; the executor behind it decides speed.

## 10. SQL composition and the rule language

- SQL owns:
  - named derived facts (§21);
  - cross-file joins and anti-joins (`require_each`, `select_when`);
  - repository counts and budgets;
  - closures over cross-file graphs (recursive CTE, or a native graph vtab: DLRE reachability/forest, facts-plan Y1);
  - allowlists as policy relations;
  - incremental invalidation;
  - the outer query.
- **Rule language (decision 0002).** Rules are declarative only. Cross-file composition uses a small closed algebra (join, union, anti-join, group/count) over named relations, with the `Predicate` AST. Generic analyses are engine relation contracts (`fact_*`). The authored-SQL `relational` clause is frozen, then removed (`authored-sql.md`).

## 11. Planning and plan cache

- Source: N1562 `query/plan/` (semantic plan identities, exact candidate-scope identities, grouping by identical scope, utility dedup).
- **Execution budget.** A job has an explicit budget of matching statement executions, checked before execution and shared across producer and consumer passes. Statement count follows SQLite compound limits (SSC: 128 arms per `UNION ALL`), never file or root count. A statistics refresh re-prepares only when a stored decision changes (SSC `engine.rs:333-345`).
- **Batches are cut by resident bytes,** not by file count (§18 resident-batch pool). Prepared plans are reused across batches.
- Shape choices (anchor, seeded versus scan, fusion) are made at plan time from statistics, never by `count(*)` at run time (DLRE spent 537 s doing that; history-index trap T18).
- **Fusion grouping:** only programs with the same candidate-scope digest and grammar. Within a group: tries by root kind, set-stage passes for shared relation targets, and boolean fusion by the cost model.
  - Fuse roughly when 16 or more programs share a root kind, or 2 or more share a relation target.
  - Use masks when the summed per-root subtree scans exceed `nodes · c_pass / targets_per_pass`.
- **Cost-model inputs:** `fts5vocab` root counts, the edge and size aggregates (`spec-indexes.md` §2.5), and `plan_feedback(program_digest, selectivity, cost, at)`.
- **Fused-group cache key:** (sorted member digests, fusion-plan version, IR version, grammar, statistic buckets used). `FACT`/`SIDE` inputs enter the key through their `fact_digest`.
- Plans schedule fact strata (base facts → derived facts) before the rule programs that read them (§21.10).
- **Demultiplexing:** `t_match(program_group, label, doc, root)` joined to the label table `label → (rule, fragment)`. An `Unknown` group is split and re-run.
- Cache key and re-validation: `compiled_plans` (`spec-indexes.md` §2.12). Each decision stores the statistic keys it read and the interval over which it holds. On use, one batched point lookup re-checks them, and the plan recompiles only when a value leaves its interval.

## 12. Findings contract and consolidation

Normative text: `findings-contract.md`. In short:
- The engine returns a set of keys `(ruleId, code, path, line[, column|tag])` plus a per-(rule, file) `Complete`/`Unknown`.
- One shared consolidation step normalises locations, collapses keys, applies suppressions, rehydrates payloads with the native matcher, derives ids and order, and diffs by key.
- Boundary outputs carry no order. An ordinal column exists only to key multiple witnesses of an `each` rule; consolidation, not the engine, sorts.
- **Outcome classes.** Matched; logical unknown (a completed verdict from a three-valued frontend, which never counts against coverage); resource `Unknown(reason)`. Decision 0001 excludes three-valued semantics, but current frontends still produce logical unknowns (`three_valued` grammars). Until they stop, the boundary publishes a logical unknown as no match and keeps it only inside in-document composition. SSC found that treating it as incomplete coverage was a bug. To be recorded as a decision.

## 13. Pure-SQL fallback

Each transducer program is lowered to specialised SQL over a TVF that reads the tape. A generic interpreter in SQL is forbidden (dead end D73; SSC retired its own interpreters in `e32b280b`). The fallback is differentially tested against the transducer. It is SQLite only and low priority.

- **Starting points:** SSC `sql/engine/structural.rs` and `sql/fragments/candidates.rs` for the structural subset (kinds, all/any/not, relations with `neighbor`/`end` stops, unbound counts as specialised extent and parent joins), and the surviving non-recursive `sql/relational/` emitters.
- **Sequential contracts** (recursive utilities, capture rollback, ordered search, parsed regions) have no pure-SQL binding. The fallback reports them as uncovered, with a coverage metric, rather than emulating them.
- SSC's structural-to-SQL lowering belongs here only. In the main engine those predicates lower to `EXISTS`/`NOT_EXISTS`, the extra axes, `COUNT` and `DESCEND_KIND` (`spec-transducer.md` §6.1), never to SQL over a node relation.

## 14. Per-edit and incremental

- The batch is one edited file × all rules. Repository facts come from the snapshot.
- **Residency:** the edited document is parsed and its v9np tape, source and line starts are encoded directly into a pinned edit slab (§20.8): zero upload and zero first-use mapping on the edit path.
- Base and derived facts are maintained by dependency cone (§21.8). facts-plan's Y1 measured 0.7/2.4 ms p50/p95 per edit, exact.
- Index publication on edit: the edited document is published synchronously. Its scope reads `Unknown` until its row exists. A single document's statistics changes never invalidate plans (`spec-indexes.md` §2.13, Q7).

## 15. Measurement gate

Adopt the SSC checker pattern (`benchmark/check-matcher-fragment-integration.py`): explicit ceilings (cold CPU ratio, edit CPU ratio, peak RSS, execution count); source, binary and corpus hashes; one process per engine for RSS; an edit control of add / add / restore with retraction checked. Every edit measurement names its scope: matching only, or the whole apply including output.

As in `README.md`: end-to-end CPU, wall time, RSS and parity under §12, on the full data-layer repository, CPython with 335 patterns, and a per-edit check. Compile time is reported separately. Proxy metrics are diagnostics only. DLRE's `scripts/profiling/full_repository.py` implements the full-repository gate.

## 16. Module map

| Module | Role | Source |
|---|---|---|
| `tape/` | regular structure-only tape (v9np), document table, sidecars | main v7 → v9np |
| `content_store/` | storage, indexes, publication | main; DLRE anchor store |
| `engine/stats/` | statistics views and readers | new; N1562 stats tables; SSC `engine/statistics.rs` + `search_kind_counts` (moved to global `k%06x` tokens) |
| `engine/transducer/ir/` | IR, verifier, packed ABI | Tempo; N1562 `packed/` |
| `engine/transducer/lower/` | `AstMatcher` → IR, in-file composition, fusion | N1562 lowering; in-file parts of SSC and N1562 relational IR |
| `engine/transducer/exec/{cpu,jit,metal}/` | executors | N1562, Tempo |
| `engine/vtab/transducer.rs`, `engine/vtab/demand.rs` | the boundary, demand handles, typed cells, outcome codes | SSC `sql/fragments/` at `c17d2e47`; N1562 `sql_backend` |
| `engine/vtab/graph.rs` | cross-file graph producers | DLRE reachability/forest, facts-plan Y1 |
| `engine/compose/` | cross-file operators → SQL | N1562 cross-file nodes, SSC crossfile/select_when |
| `engine/plan/` | scope, grouping, fusion, plan cache, session, execution budget | N1562 `query/plan/`, DLRE file-level scope, SSC `sql/engine/` session and compile cache, SSC resident partitioning |
| `engine/findings/` | consolidation | new; out of `eval/output.rs` |
| `engine/fallback/` | pure-SQL binding | SSC `engine/structural.rs`, `fragments/candidates.rs`, non-recursive `sql/relational/` emitters |
| `engine/facts/catalog/` | fact definitions (`rerail.facts/v1`) | new; PR 2539 catalog relations, facts-engine views |
| `engine/facts/derived/` | algebra → SQL, cache, invalidation, statistics | new; facts-engine `facts_memo.rs`, DLRE producer catalog (clash check) |
| `engine/facts/programs/` | fact programs | N1562 `semantic_facts`, DLRE extraction |
| `matcher/` | native matcher: rehydration and parity oracle | main |

## 17. Decisions

See `decisions/README.md`: 0001 findings contract; 0002 no authored SQL; 0003 regular tape; 0004 transducer as the IR; 0005 named derived facts (amends 0004).

## 18. Memory limits, debugging, testing and profiling

Normative text: `spec-tooling.md`. In summary:

- **Memory.** Every allocation belongs to a pool whose limit is sized at plan time from statistics. The pools are tape columns, set stages, outputs, device-side queues, facts, SQL temp and caches. Host and device caps are configuration. Exceeding a pool yields `Unknown(resource)` with its details, counted against coverage, and is never rerouted. CI gates on peak RSS, peak device bytes and the Unknown rate. Tempo's launch accounting is kept and checked against the plan's dispatch count; its fixed caps are removed.
  - **Resident-batch pool.** CPU batches are cut by resident bytes (decoded tape, navigation and semantic caches), not by file count. SSC measured 760 MB of navigation data retained across 4 whole-job partitions; a 4 MiB-of-source partition target cut peak RSS from 2,933 MB to 1,233 MB at unchanged CPU, with prepared plans reused across partitions.
- **Debugging.**
  - a stable, diffable text dump of IR programs, fused groups, label tables and set-stage plans, with an assembler whose round trip is exact;
  - `explain`, which shows each plan decision with the statistic values it read;
  - instruction trace and single-step on the CPU interpreter;
  - a Metal debug mode with per-task status, arena high-water marks and GPU frame capture;
  - self-contained repro bundles with offline replay;
  - tape and index inspectors;
  - the CLI: `rerail engine dump | asm | explain | run | replay | tape | index --check | diff`.
- **Verifier.** Every admitted program must run on every backend, Metal included. An inadmissible construct is rejected with a reason. Unbounded loops declare a queue bound.
- **Testing.**
  - differential testing on the finding-key set: interpreter vs JIT vs Metal, fused vs unfused, native vtab vs pure SQL, against the native matcher, and against the data-layer oracle;
  - golden dumps;
  - proptest and cargo-fuzz for the IR, assembler, verifier, tape and set stage;
  - a v7 ↔ v9np round trip;
  - a node-for-node Blade vs tree-sitter-php check;
  - index consistency checks;
  - a targeted test command per module. Full suites run only at phase gates.
- **Profiling.**
  - fixed stage names matching the performance model (index/scope, demand, programs, set stage, facts, SQL, consolidation), with compile time separate;
  - always-on executor counters (dispatches, waves, arena high-water marks, occupancy, GPU time);
  - per-rule attribution inside fused groups;
  - SQLite per-statement stats and `dbstat`;
  - one JSON-lines profile format, consumed by `full_repository.py`;
  - proxy metrics flagged as diagnostics only.
- **Reused:** `code-rule-profile` and `profile::trace`, the SQL statement profiler, `full_repository.py`, Tempo's launch guard, N1562's task status records and arena quotas.
- **New:** dumps and assembler, Metal counters, fuzzing, `dbstat`, repro bundles.

## 19. Performance model, heavy-rule profiles and the gap to target

Evidence: `research/heavy-rule-profiles.md` (8 heaviest data-layer rules on the 9,010-file, 28.23M-node gate corpus) and `research/fusion-model.md` (fusion opportunity and the corpus × rules cost model). Tools are in `experiments/rule-profiles/` and `experiments/fusion-model/`. Every estimate is labelled measured, derived or assumed in those documents.

### 19.1 Cost model

Whole-gate cost is the sum of the profiling stages from §18, each with a sourced unit cost:

| Stage | Cost driver | Unit cost (source) |
|---|---|---|
| Parse (cold only) | nodes | 1,232 ns/node (M) |
| Tape encode + kind index | nodes | ≤ 12.4 ns/node (D) |
| Regexes | distinct token texts × patterns | 0.11–0.25 s cold, ≈ 0.01 s warm for all identifier patterns via `TEXT_ID` (M/D, `research/regex.md`); per occurrence 33–69 s, forbidden. Whole-file text stage: 0 after the AST rewrite (today 6.45 s scoped, 9–17 s broad; 55% of it is line splitting) |
| AST programs | Σ program-nodes after fusion | 0.3 / 1 / 3 ns per program-node (A; templates measured at 0.3–0.97) |
| Mask passes | nodes × passes | 6.4–17.3 ns/node per 210 programs (D) |
| Fact extraction | nodes in fact scopes | 54.8 ns/node (M) |
| Fact load | rows | vtab ≈ 0 (A, M3); real tables 16.4 s for 7.9M rows (M) |
| Cross-file SQL | query shape | 0.25–1.1 s per query in the good form; 22–34× worse in the bad form (M) |
| Data flow (category H) | flow edges | 0.5–0.9 s per variant (M) |
| Consolidation | findings | ≤ 1 s (A) |
| Native AstMatcher (current baseline) | candidate roots | 0.5–1.1 µs per candidate root, opt-level 1 (M, SSC catalog and workspace) |
| vtab boundary + publication SQL | candidate roots, `xFilter`s | ~55 ns per candidate root, ~1.2 ms per `xFilter` (M, SSC) |

The planner uses the same model, with parameters read from `backend_costs` and refined by `plan_feedback` (`spec-indexes.md` §2.5b).

### 19.2 Expected range

Two independent estimates exist. They agree on the gaps to close, and disagree on the AST row by about 10×:

| Quantity | Heavy-rule profile (`heavy-rule-profiles.md`) | Fusion model (`fusion-model.md`) |
|---|---|---|
| Method | Unit costs measured per stage, AST cost assumed 0.3/1/3 ns per program-node | 535 queries actually run with exact ast-grep semantics in a harness at 35 ns/evaluation; model fitted to its counters |
| AST / structural, 1 core | 1.9 / 6.3 / 19 s | 56–120 s fused (measured); 129–232 s separate |
| Regex over file text | 1.5 / 3 / 10 s (all regexes) | 9–17 s (4.3 GB scanned). **Superseded by `research/regex.md`:** all 113 clauses lower to AST rules; regex stage ≈ 0.1–0.3 s cold |
| Relational SQL | 2 / 5 / 25 s cross-file SQL + 0.5–30 s fact load + 1–20 s data flow | ≤ 8.5–43 s (row-count upper bound, per-row cost assumed) |
| Whole gate, 1 core, cold, no parse | 8 / 25 / 110 s | 77–167 s fused |
| Whole gate, 16 cores, cached trees | 8 / 15 / 40 s wall at 8–12 cores | 5–17 s; **1.2–3.5 s** with a 3–5 ns/evaluation executor |
| Pure-SQL lowering | AST row becomes 6–190 s | 360–1,760 s (DLRE measured 958.6 s) |
| One-file edit | 0.02 / 0.3 / 2 s, including facts and reachability | AST only: p50 2.9 ms, p99 189 ms |
| Peak memory | 1 / 2 / 4 GB | — |

**M2 result (measured 2026-10-25, `research/m2-m7.md`).** A prototype fused executor (hash-consed sub-rules, recursive prefix trie, cost-ordered conjuncts, kind-postings dispatch, regex results interned per distinct text, keyed sets) produced the native matcher's exact finding set on transaction-patterns and async-sync-delegation at **0.26–0.40 ns per program-node** fused (ASD alone 0.86–1.02). It passes the 1 ns target by doing 20–40× fewer evaluations, not by cheap ones: each evaluation still costs 13–17 ns (target 3–5 ns), and 74% of the time is the interpretive evaluator. The native matcher spent 70–113 s CPU per rule on the same corpus (including parse). This places the AST row near the heavy-rule profile's low/expected estimates, not the fusion harness's. Memo columns did not pay once regex results were interned; masks were almost never promoted.

**Reconciliation.** The difference is the per-evaluation executor cost. The fusion harness is a straightforward interpreter (35 ns per evaluation). The heavy-rule profile assumed the speed of the `research/fusion.md` templates (0.3–0.97 ns per program-node), which were 3–6-operator programs; the real ones average about 150 operators. Executor speed on real heavy programs is therefore the largest single uncertainty in the AST row. A third data point agrees: SSC's full-workload run spent 79.5 of 83.6 s inside the native AstMatcher and 4.9% at the boundary (`research/ssc-fragments.md`), so executor speed, not the boundary, decides the AST row. Pre-merge measurement M2 resolves it. The pass condition stays ≤ 1 ns per program-node, which corresponds to the fusion model's 3–5 ns per evaluation.

**M1 (measured, `research/m1-baseline.md`).** Each heavy rule alone on the current engine (release build of this merge): cold CPU 39 s (async-sync-boundary) to 346 s (serverplan-quality), peak RSS 1.3–10.5 GiB, warm one-file edits up to 25 s CPU (internal-dead-code). Even AST-only rules spend most of their time in SQL anchor selection (serverplan-steps: 20.7 of 25 s ingest), and global relational stages are 68% SQLite. Against M2: transaction-patterns costs 42 s CPU here and 0.03 s in the fused prototype (parse excluded).

Baselines: pre-SQL native 709 s / 16.1 GiB (structural part 125.8 s, which matches the fusion model's `sep` mode); DLRE SQL path 7,212 s / 15.0 GiB; DLRE per-edit 40–80 s.

**Planner accuracy.** The per-query cost estimate from rule structure alone is off by about 3×. With κ measured from `plan_feedback`, it is within 1.13–1.40× per query. Static estimates are used only for first-run grouping; plans are refined from feedback (`spec-indexes.md` §2.5b).

### 19.3 Gap to the 1–2 s target

The expected cold figure misses target 2 (§0) by an order of magnitude. Stated plainly:

- **Cold is not the target case.** Parse (35 s CPU) and fact extraction are content-addressed and cached (facts-engine memo, phase 7). The 1–2 s target applies to a warm repository: changed files are re-parsed, everything else is read from the store.
- **Warm cost is dominated by the AST row (if the executor stays slow), fact load, cross-file SQL and data flow.** The regex stage drops to ≈ 0.1–0.3 s once text clauses are AST rules and token regexes use `TEXT_ID` (`research/regex.md`). Metal helps the AST row; `TEXT_ID` lookups run on every backend. Reaching 1–2 s needs all of:
  1. facts served by vtab, not copied (M3);
  2. stable good-form query plans for cross-file SQL (recorded plans plus a plan-regression check);
  3. data flow as native relations, at the low end of 0.5 s per variant (M5);
  4. an executor at ≤ 1 ns per program-node (3–5 ns per evaluation) on real heavy programs, with shared sub-rule evaluation (M2), which is 7–10× faster than the fusion harness;
  5. the 113 whole-file text clauses rewritten as AST rules, and token regexes evaluated once per distinct text (`TEXT_ID`). A single multi-pattern pass per file is slower (measured 92.6 s) and is not used;
  6. the ten heaviest relational inputs rewritten under decision 0002;
  7. 16-way parallelism across files and independent strata (assumed, not yet measured).
- **If M2, M3 or M5 fails its pass condition, target 2 is not reachable with this design,** and the result is recorded as a decision before further work.
- **What is credible today:** both models put the warm 16-core gate at roughly 5–17 s with the current executor speeds. Both reach the 1–2 s band only with the executor and the remaining items above in place. The per-edit target is credible now: milliseconds for AST matching, with an expected 0.3 s including facts and reachability.

### 19.4 Pre-merge measurements

M1–M9 and the phase each one gates are listed in `architecture.md` (Integration phases). No phase starts before its entry measurements are recorded.

## 20. Tape residency and caching

Evidence and the Tempo/main survey: `research/tempo-residency.md`. Bare document paths below are in the Tempo tree (`nera-landing-2/ner-1562-scale-test-the-re-railer/tempo/docs/`).

Tempo evidence: first-use Metal mapping cost 213 ms against 6.1 ms of GPU time, and reused buffers cut the wait 30–60× (`shared-sparse-bump-arena-design.md:14-35,260`). A 32 MiB / 30 s decompressed cache got 0 hits on full scans and spent 37.6 worker-seconds decompressing in a warm run. Heap decompression left 3.3 GiB of malloc high-water (`workspace-store-runtime-journal.md:536-681`). Per-object mappings were rejected for VM metadata cost (`:517-520`).

### 20.1 Forms

A document's tape and source exist in exactly these forms:

| Form | Where | Encoding | Owner |
|---|---|---|---|
| at rest | `tape_objects.frame` / `source_objects.frame` | one zstd frame each (level 3), `raw_length` and canonical SHA-256 outside the frame | content store |
| resident | a residency slab (20.2) | the decompressed v9np blob, byte-identical to the frame's content; UTF-8 source bytes | residency manager |
| device view | the slab's `MTLBuffer` | the same pages (20.4) | residency manager |
| sidecars | a residency slab | `line_starts` u32, derived `par`, `cidx`, text-hash and side-fact columns | residency manager, keyed by tape or source digest |

No other copy of tape or source bytes may exist during evaluation. Executors, the native matcher used for rehydration, `source_text`, and the vtab read the resident form in place. `TapeDoc::restore` (array-of-structs) is permitted only for the native matcher and parity oracles, and never feeds an executor.

### 20.2 Slabs

1. Resident bytes live in **slabs**: anonymous `mmap(PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANON)` regions whose size is a multiple of the OS page size (`sysconf(_SC_PAGESIZE)`, which is 16 KiB on Apple Silicon). The default slab is 64 MiB, a configuration value. An object larger than a slab gets its own page-rounded mapping. Per-object mappings are forbidden otherwise.
2. Objects are bump-allocated in a slab at 16-byte alignment.
3. The at-rest v9np blob is laid out so that every column starts at a 16-byte offset from the blob start. One zstd decompression **directly into the slab** (known `raw_length`) therefore produces aligned, ready columns. Decompressing into an intermediate heap buffer is forbidden.
4. A slab is append-only until it is retired. Compaction copies live objects into a **new** slab, publishes new handles, and retires the old slab when its lease count reaches zero. At most two generations of a slab exist at once. If a third would be needed, the load waits.
5. A slab is released with `MADV_DONTNEED` then `munmap`, only after its `MTLBuffer` is gone. Partial `MADV_DONTNEED` on a live slab is forbidden, because Metal treats re-touched pages as new mappings (Tempo: 84.94 s, `packed-result-bottleneck-profile.md:102`).
6. Transparent huge pages, superpages and `mlock`/wired memory are not used. `mlock` does not keep a GPU mapping.

### 20.3 Identity, admission and validation

1. A resident object is keyed by its content identity: source by `(hash algorithm, canonical digest)`; tape by `(source digest, grammar_id, grammar_version, tape_format_version)`. Row ids are never cache keys.
2. **Admission** decompresses the object into a slab, checks codec, `raw_length` and the SHA-256 of the decompressed bytes, and checks that the tape's node count and width descriptor match `search_index_sets`. This validation runs **once per admission**. Restores, batches and evaluations trust the handle and must not re-hash source.
3. A failed validation removes the store row from use, increments `residency_corrupt`, and the document is re-parsed. It never becomes a clean negative.
4. Loads are bulk: the planner asks for the demanded document set, the residency manager issues one batched store read for the misses, and workers decompress outside any SQLite transaction.

### 20.4 Pinning and device views

1. On Apple Silicon each slab has exactly one `MTLBuffer` created with `newBufferWithBytesNoCopy(…, StorageModeShared, deallocator)`. The deallocator and an associated object both own the slab, so the mapping outlives every command buffer that references it.
2. A **pinned** slab keeps its `MTLBuffer` alive across dispatches and evaluations. Pinning means retention. Buffers over resident objects are never created per batch or per evaluation.
3. At daemon start, and after compaction, the residency manager may submit one warm-up command that touches each pinned slab, outside any production command. Its time is reported as `residency.pin`.
4. `MTLResidencySet` (macOS 15+) may be added only after the differential measurement Q-R2. CPU-to-GPU blits of resident objects are forbidden unless Q-R1 shows private storage wins. In that case the private copy is made once per admission, never per evaluation.
5. Executors bind slabs, not documents. A batch references at most the configured number of slabs. If it needs more, it uses an argument buffer of slab GPU addresses (subject to Q-R3).

### 20.5 Batches reference, never copy

1. A batch is a **batch document table** of fixed records `(slab, tape_offset, node_count, width, source_offset, source_len, line_offset, derived_offsets…)`, plus demand. Executors compute document-local indices: `start(i) = base[i/16] + off[i]` uses the document-local `i`. No tape column, source byte or line start is concatenated or copied into a batch.
2. Batches are **width-homogeneous**. The planner groups documents whose `size` width is u32 (24 of 6,251 in the spike) into their own batches, and kernels are specialised by width with function constants. Mixed-width batches are forbidden.
3. CPU executors read the same slabs through the same table.

### 20.6 Source text and sidecars

1. A document's source is resident whenever its tape is resident and any admitted program, rehydration or SQL function can read its text. On UMA this costs no extra device memory, because the pages are shared.
2. `line_starts` is computed once per resident source digest, in the same pass as admission or lazily on first demand. It is never recomputed per batch or per finding.
3. Derived columns (`par`, `cidx`, text hashes) are sidecars keyed by tape digest, built once on first demand (on CPU or GPU), and evictable before their tape. They are charged to `tape.derived`.
4. Derivation sidecars (the Blade view text, segment map and host errors) follow the same rules, keyed by (template digest, via).

### 20.7 Cache policy

Residency classes:

| Class | Members | Eviction |
|---|---|---|
| `snapshot` | documents of active workspace snapshots in scope of any admitted rule | not evicted while Σ ≤ `tape.resident` limit; these slabs are pinned |
| `edit` | the newest overlay version per open path | pinned; superseded versions are freed when their leases reach zero |
| `transient` | everything else (other snapshots, historical versions, oversized one-offs) | LRU by last lease release; no fixed TTL |

1. Sharing is by content identity across all workspaces in the process. Cross-process sharing goes through the store (and Q-R5).
2. If the `snapshot` class exceeds its limit, the planner runs the audit in **residency waves**. Each wave loads, pins, executes and releases. Waves are reported as `residency_waves`, and misses as `residency_miss_bytes`. Being non-resident is never an `Unknown`. A single document larger than the device budget is `Unknown(resource: doc_exceeds_device)` (§8).
3. Residency state may be exposed to the planner as a cost input (load bytes). It is never a correctness input.

### 20.8 Per-edit residency

1. The edited buffer is parsed, and its v9np tape is encoded **directly into an `edit` slab**. The node count is taken from the parse before columns are written, so there is no intermediate tape buffer. Its source bytes and `line_starts` go into the same slab.
2. The edit slab is already pinned, so per-edit execution has zero upload and zero first-use mapping.
3. Compressing and writing the store row happen off the evaluation path. §14's synchronous index publication is unchanged.
4. The previous version keeps its handle until its last lease is released. Edit-slab compaction never runs on the edit path.

### 20.9 SQL access to tapes

1. The transducer vtab (§9) resolves demanded documents to residency handles before `xFilter` dispatch. It never `SELECT`s frames.
2. The pure-SQL binding (§13) and the ast-index-spike techniques read columns through a TVF `tape_column(doc, column)` that returns a blob **pointing into the slab** (`sqlite3_result_blob64` with a no-copy destructor). A statement-scoped residency lease, held by the cursor or function auxiliary data, keeps the bytes valid for the statement's lifetime. The same applies to `source_text`.
3. Uncompressed tape columns are never stored in content-store rows. The captured-SQL laboratory schema is test-only.
4. The FTS5 kind, term and edge indexes stay in SQLite as specified in §5. They are built from the resident columns at publication.

### 20.10 Accounting and outcomes

1. Pools: `tape.resident`, `source.resident`, `tape.derived` (process scope) and `batch.doctable` (per batch). They replace `tape.columns` in `spec-tooling.md` §2.1.
2. Limits are sized from Σ `node_count` × v9np widths and `raw_length`, taken from the store and `search_index_sets`.
3. Shared no-copy slabs are counted **once** against both `host_bytes` and `device_bytes`. Private copies (if Q-R1 adopts them) are counted on the device only.
4. Every slab reserves its page-rounded size in the ledger before `mmap`.
5. The profile records slab count, resident, pinned and derived bytes, admissions, decompressed bytes, hits and misses per class, compactions, `residency.pin` time and `residency_waves`.
6. The always-on Metal counters add `bytes_bound` per command and `slabs_bound`.

### 20.11 Forbidden

Per-batch repacking into node records, such as Tempo's 44-byte `MetalNode`. Per-batch source concatenation or line rescans. Heap-`Vec` decompression. Copying source on cache hit. Re-hashing on restore. Per-object `mmap`. TTL-only caches smaller than the working set. Partial `MADV_DONTNEED` on mapped slabs. `mlock` as pinning. Fixed byte caps in code.

### 20.12 Open measurements before locking

- **Q-R1:** shared no-copy vs a private copy made once per admission, measured on GPU time of the stratified kernels over the gate corpora.
- **Q-R2:** `MTLResidencySet` vs retained buffers alone, measured on first-dispatch latency after idle and under memory pressure.
- **Q-R3:** slab count per batch vs argument buffers (`gpuAddress`); the 31-buffer binding limit.
- **Q-R4:** slab size (16/64/256 MiB) vs VM metadata and compaction cost.
- **Q-R5:** a file-backed, read-only shared mapping of a snapshot image for multi-daemon sharing, and whether GPU no-copy over file-backed pages is safe.
- **Q-R6:** whether no-copy buffers count in `currentAllocatedSize`.
- **Q-R7:** pin cost per GB of tape and source slabs, to check the about-27 µs/MB extrapolation from Tempo's scratch measurement.
- **Q-R8:** cold admission throughput (zstd into slab) at 4/8/16 workers against the 1–2 s audit target.

## 21. Named derived facts

Decision 0005. Evidence and prior work (PR 2539, the facts-engine, DLRE producers) and the initial catalog: `research/derived-facts.md`.

### 21.1 Producers

Every relation a rule may name is a *fact* with exactly one producer: a **fact program** (a rule-independent transducer program exporting a declared schema; base facts from tree structure), a **native producer** (reviewed engine code; base facts needing a resolver or evaluator), or a **derived fact** (a definition in the declarative relational algebra over other facts, compiled to SQL and executed in SQLite). Shared does not imply SQL: shared tree-walking analyses are fact programs; relational analyses (name-keyed joins, fixpoints over names, aggregates, anything membership-dependent) are derived facts.

### 21.2 Definitions

Facts are defined in `rerail.facts/v1` documents, never in rules: engine-owned families in `engine/facts/catalog/`, pack-owned families in `packs/<pack>/facts/` under the `<pack>.` namespace. Derived facts use the closed algebra (`from`, `join`, `left_join`, `anti_join`, `union`, `group_by`/`aggregate`, `project`, stratified `recursive`, literal policy relations) with the `Predicate` AST. No SQL text, expression strings or UDFs (decision 0002). Whatever the algebra cannot state is a native producer. The generated SQL is dumped and stored with the plan.

### 21.3 Admission

A fact is admitted only if (a) its name denotes a language or domain concept, never a rule, linter or finding (PR 2539 `DECLARATIVE_LINTER_SQL_ONLY_BOUNDARY.md` :105, "Process Failure" :169-186); (b) at least two admitted rules use it, or it is a catalog family in §9; (c) its estimated rows per document do not exceed the document's node count (L4); (d) it carries no rule-specific seeds or constants, which are passed as policy data or handled by the rule algebra (`reaches`).

### 21.4 Identity

`fact_digest = H(canonical lowered definition ‖ output schema with key and FDs ‖ input fact digests ‖ compiler version)`. A fact program contributes its program digest; a native producer contributes its id and revision, which must change whenever its output changes. `version:` is informational. The same name with a different digest is a compile error.

### 21.5 Partition

The compiler infers `document` (all inputs per-document; every join, group and recursion keyed by `doc`) or `repository`, and rejects a declaration that disagrees with it.

### 21.6 Materialization

The planner chooses, per admitted plan: **table** (default for derived facts; required when there are two or more consumers, when the fact is recursive, or when it has repository partition), **inline** (one consumer, non-recursive, document partition), or **vtab** over native columnar data (base facts; M3). Each fact has one physical owner per (document or snapshot, digest) per evaluation, and is never re-expanded into several statements. Tables are narrow and typed, with atoms interned as u32; no EAV or union tables. Indexes come from consumer access paths and are recorded in the plan. Computation is demand-driven: only facts reachable from admitted rules, and per-document facts only for documents in the demanding scopes.

### 21.7 Cache

Per document: `(fact_digest, tape identity[, path if read])`, with content-pure facts path-free. Repository: `(fact_digest, input generation vector)`. Content-store tables `derived_facts` and `derived_fact_parts(digest, doc_key, complete, reason, rows)`; rows cascade with their source. Completeness is per (fact, document); an incomplete input makes dependents `Unknown` for the affected (rule, file) pairs.

### 21.8 Invalidation

The fact dependency DAG is derived from the definitions. An edit to document `d` recomputes `d`'s base facts and demanded per-document derived facts. Repository facts in the cone are updated by semi-naive delta over the changed join keys; recursive components use delete-and-rederive, or a full recompute when the delta exceeds the planner threshold. Only rules whose recorded read set intersects a changed (fact, key) pair re-run.

### 21.9 Statistics

Each materialization records `fact_stats(digest, column, rows, ndv, max_rows_per_doc, null_frac)`. For tables these are written to `sqlite_stat1` (never `ANALYZE`). Plan decisions that read them store the keys and validity intervals in `compiled_plans`.

### 21.10 Consumption

Rules name facts in the rule algebra (`from: {fact: …}`, joins, `reaches … via: {fact: …}`), and in AST rules through the semantic keys, the `fact:` semi-join predicate and capture projections. Transducer programs read facts as `SIDE` columns (key `(doc, node)` with node → value), as `FACT` sets (§6, `spec-transducer.md` §2.6), as demand seeds (E7), or SQL joins facts with `t_match`/`t_export`. Execution per document is stratified: base facts → derived facts → rule programs → composition. Plans with a cycle across the boundary are rejected.

### 21.11 Catalog

The initial families and measured sizes are in `research/derived-facts.md` §(d): base facts `py.def`, `py.call`, `py.param`, `py.annotation_atom`, `py.receiver_token`, `py.binding_site`, `py.import`, `py.class_base`, `py.decorator`, `py.export_name`, `py.return`, `py.flow_edge`, `file.module` (fact programs) and `py.scope`, `py.binding`, `py.value`, `py.flow_exit` (native); derived facts `py.def_family`, `py.call_edge_local`, `py.module_const`, `py.local_flow` (document) and `py.import_res`, `py.call_edge`, `py.subclass_closure`, `py.export_closure` (repository). Together the derived facts are under about 0.7M rows on the gate corpus.
