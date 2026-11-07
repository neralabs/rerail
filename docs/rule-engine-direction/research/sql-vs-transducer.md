# SQL lowering versus the transducer: inventory and boundary

Read-only investigation, 2026-10-25. Sources:

- SSC `jeremy/sql-capture-demand-spikes` at `b87eba3b`, plus its uncommitted fragment pivot in the worktree named after Moti's sql-set-composition branch.
- DLRE as merged into `Pulling-it-all-together`.
- N1562 `jbarnes/ner-1562-sql-rerailer` at `8b6576f0`.
- Moti's `sql-dl60`, `facts-engine` and `facts-plan`.
- The ast-index-spike decision indexes (`docs/rule-engine-direction/prior-work/`).

Line counts are `wc -l` of the Rust files, tests included unless stated otherwise.

## 0. Verdict up front

The user's suspicion holds.

Most SQL-lowering code in these streams computes things that are **tape-local, per document**: structural navigation, `inside`/`has`/`precedes`/`follows` with stops, `nthChild`, captures and their equality, value tests, repeats, utility calls and recursion, per-file counts, scope/ownership, and candidate selection within a file. The N1562 transducer IR (`packed::native_ir::StructuralOperation`) already has the primitives for most of these:

- `Kind`, `Field`, `Flags`, `Supertype`, `AbsentField`
- `Child`, `Next`, `First`, `Last`, `Parent`, `Descend`, `Following`
- `SaveCursor`/`RestoreCursor`
- `Fork`, `Goto`
- `RepeatBegin`/`RepeatBump`/`RepeatEnd`
- `Capture`, `Text`/`Assert` (value and capture functions), `Emit`
- output programs and reductions

Measured evidence says the boundary is in the wrong place:

- **DLRE first-50 full run:** anchor selection takes 537 s and index preparation 137 s, against 58 s of actual AST matching. The whole run is 958.6 s, against 125.8 s for the pre-SQL native path.
- **SSC's own pivot:** native fragments behind one batched vtab ran at native cost, 287.5 ms against 288.2 ms. The SQL state-machine emitters it removed ran at about 3x native.
- **ast-index-spike:**
  - SQL per-row machinery costs 110 ns/node, and recursive climbs 765 ns/touch (design index topic 19, D S152).
  - Moving the child walk into a native table function gave about 390x on the hot query (topic 8, D S058/S062).
  - A query-as-data interpreter in SQL was 12x slower (history index dead end D73).
  - Temp tables were 57–115x slower (D50).

**Estimate:** about 85–90% of current SQL-lowering code falls into categories T, T+ or D. About 10% is legitimately SQL: file-level candidate scope, cross-file composition, closures over cross-file graphs, stats-driven planning, and consolidation.

The spike lowered structure to SQL because its constraint C1 forbade native extensions on PG/MariaDB (topic 8). That constraint does not apply to this engine, except for the low-priority pure-SQL fallback.

## 1. What the transducer executes per document today (N1562, enough to classify)

- **IR:** `packed/native_ir.rs` (370 lines). It defines `StructuralOperation`, value and capture programs (`packed/value.rs`, 870 lines), an output program (`packed/output.rs`, with `MatchProperty`, `CaptureProperty`, `Offset`, `Strip`, `SelectAdjacent`, `Downcase` and `LanguageFromMime`), and reductions (`native_cpu_reduction.rs`, 433 lines).
- **Executors:**
  - CPU interpreter and scheduler: `native_cpu*.rs`, about 13k lines, including regex (2.6k) and Lua (0.9k).
  - Metal: `packed/metal_*`, `transducer/metal_backend.rs`, about 2.4k lines.
  - arm64 JIT: `transducer/jit.rs`, 173 lines.
- **Boundary:** `transducer/sql_backend.rs` (2,903 lines) is the `rerail_matches` vtab. There is one xFilter per batch token, with output rows typed by `row_kind`. Its `ProducerKind` values are `Terminal`, `Pattern`, `Navigation`, `ValueFact`, `SemanticFact`, `ParseFact`, `FileFact` and `TransformFact`.
- **The limitation, by stated design.** `semantic_facts.rs:1-5` says "A producer ... never executes a nested user rule. SQL joins the rows to separately lowered relational operands." `ParseFactDescriptor` likewise keeps nested rules as SQL operands.
  - N1562 therefore pushes only *leaves* into the transducer and composes `all`/`any`/`not`/`inside`/`has`/`nthChild`/`count` in SQL (`query/relational_ir.rs` `RelationNode`).
  - That is the boundary set too low.

## 2. Inventory and classification

Legend:

| Verdict | Meaning |
|---|---|
| **T** | Push into the transducer. The IR can express it today. |
| **T+** | Push into the transducer, but it needs a named IR extension (section 3). |
| **S** | Stays in SQL. |
| **D** | Delete, because it is superseded or is authored SQL under decision 0002. |

### 2.1 SSC `native/src/code_rule_dsl/sql/**` (105,140 lines, 352 files)

| Item | Lines | Computes | Tape-local? | Evidence | Verdict |
|---|---|---|---|---|---|
| `sql/relational/` compiler core: `ir.rs`, `lower.rs`, `emit.rs`, `leaves.rs`, `pattern_leaf.rs`, `pattern_capture.rs`, `span_capture.rs`, `row.rs`, `witness.rs` | about 3.3k | One rule's AST pattern lowered to SQL set relations over `(file, node)`. The `Set` operators are `Kinds`, `Regex`, `Pattern`, `Intersect`, `Union`, `Complement`, `Preimage(Parent/Child/Previous/Next)`, `Within`, `Ancestors` and `SearchRoots` | Yes | The frame/SQL executor ran at about 3x native (retired by its author). The pivot shows native fragments at about 1.0x | **T** for execution. Keep the emitter only as the **pure-SQL fallback emitter** for transducer programs (low priority, section 5) |
| `sql/relational/outcomes/` | 43,375 (180 files, about half tests and spikes) | Per-rule semantics in SQL: bindings (`binds`, `bound`), `nth`, `field`, `repeat`, `nested`, `capture`, `constraint`, `predicate`, `call`, `format`, `parse` (embedded and parsed regions), `memo`, `recursive*`, `utility_projection`, and `sim905`/`b615` spikes | Yes; all per document | The pivot deleted about 3.7k lines of these emitters (finite, mixed-recursive, parse-recursive, utility-projection adapter). 11 grammars that the SQL compiler refused run natively | **T** for structure, captures and repeats. **T+** for utility calls with memo, parsed sub-documents, and semantic bindings. **D** for the state-machine emitters (D73) |
| Memo, continuation and peak machinery: `memo_store.rs`, `continuation_store.rs`, `peak_tracker.rs`, `sibling_state.rs`, `recursive_chain.rs`, `recursive_downward.rs`, `recursion.rs` | about 2.1k | Reproduces native operational semantics in SQL: memo replay, continuations, depth peaks, sibling cursors, recursive search | Yes | Exists only to match exact work accounting (`docs/sql-matcher-fragment-transition.md`, "Open semantics") | **D**. Decision 0001 removes the need. Sibling and recursive search become **T** (`Next`/`Following`/`Descend` plus `Fork`) |
| `demand.rs`, `file_guard.rs` | 522 | Demand propagation and file guards | Mixed | — | Demand-to-the-transducer is **S** (it becomes the demand relation). In-file guards are **T** |
| `parse_facts.rs`, `parse_export.rs`, `format_facts.rs` | about 430+ | Parse and format facts | Yes | — | **T+** (sub-document switch, extension 7) |
| `planner_stats.rs`, `physical.rs`, `job.rs`, `shared_job.rs` | 693 | Statistics read, physical plan, job batching, sharing | No; planning | Superseded by N1562 `query/plan/` (workstream scan) | **S**, taken from N1562 instead |
| `sql/frames*`, `sql/emit*`, `sql/fused`, `sql/large`, `sql/exec.rs`, `sql/lower.rs`, `sql/lower/` | about 18k | The frame/stage set-composition executor (Moti's base) | Yes | Retired by its author at about 3x native. It encodes an interpreter in SQL (D73) | **D** |
| `sql/nodes.rs` | 1,164 | `node` vtab, one row per tape node | Yes | Per-node rows cost 110 ns/node of SQL machinery (spike topic 19). The old callback engine took 1,141 s against 10 s native | **D** as an execution path. A narrow node relation survives only for the pure-SQL fallback (an address-based TVF in the style of `nera_kids`, topic 8) |
| `sql/terminals/` | 2,571 | Batched native terminal matches behind a vtab | Yes | Works; one xFilter per batch | **T**. Merge into the transducer vtab boundary |
| `sql/nearest.rs`, `sql/recursion.rs`, `sql/texts.rs`, `sql/shapes.rs`, `sql/skip.rs`, `sql/scan.rs` | about 2.6k | Nearest enclosing ancestor, recursion, text access, shape choice, skip | Yes | — | **T** (nearest is **T+**, extension 4). Shape choice is **S** (planner) |
| `sql/crossfile.rs`, `sql/select_when.rs` | 816 | Cross-file and select_when composition | No | — | **S** |
| SSC pivot: `fragments.rs`, `fragment_job.rs` (uncommitted) | 614 (+419 tests) | Native matcher fragments behind one batched vtab filter; typed captures; explicit unknown; file-isolated failure | Boundary | 287.5 ms against 288.2 ms native across 11 grammars and 597 files; one batch per fragment | **Keep as boundary pattern**. It is the right shape, but the executor behind it should be the transducer, not `AstMatcher` directly |
| `sql/spikes`, `sql/tests` | 15.5k | Spikes and tests | — | — | Mine for parity tests, then **D** |

**SSC totals.** Excluding spikes and tests, about 89k lines. Of that, about 85k is T, T+ or D, and about 1.5–3k is S (planning, crossfile, select_when, demand).

### 2.2 DLRE (merged): `matcher/anchors/sql/` (9,070), `relational/` (5,443), `relational.rs` (1,009), `compile/relational.rs` (547), `producers/` (349)

| Item | Lines | Computes | Tape-local? | Evidence | Verdict |
|---|---|---|---|---|---|
| `matcher/anchors/sql/` lowering of in-file candidates: `lower.rs`, `lower/{children,ownership,lifetime,waves,render}.rs`, `graph*`, `values.rs`, `decode.rs`, `functions.rs`, `execute.rs`, `reader.rs`, `nodes.rs` | about 5.5k | Lowers `AnchorPlan`s to SQL sets of `(file, preorder)` per file, then the matcher validates. It builds per-file node, posting and value tables and stages them | Yes; selects roots within a file | **The 537 s anchor-selection and 137 s index-preparation cost**, against 58 s of matching. `Lower::pattern` runs `count(*)` at run time to price arms (trap T18), with a constant 32-row estimate | **T**. Candidate roots within a file come from the transducer's start instruction, `Descend` plus `Kind`, which is a tape scan. With a seeded start set (extension 1), the transducer can also start from positions SQL hands it |
| `matcher/anchors/sql/` file-level scope: `corpus.rs`, `store.rs`, `store/kinds.rs`, `postings.rs`, `plans.rs`, `session.rs`, `candidates.rs` | about 2k | Publishes per-file kind postings into the FTS store and chooses which files can match | No; cross-file pruning | `fts5vocab` point lookup 0.27 ms (README, Statistics) | **S**. Keep and simplify into the planner's candidate-scope step (statistics and FTS file postings) |
| `matcher/anchors/sql` tests and spikes | about 2k | — | — | — | Keep the parity tests |
| `relational/syntax.rs`, `syntax/packed.rs` | about 1.3k | Native CST node table for authored SQL (columns, fields, edges, ownership, owners) | Yes | It exists because rules re-encoded the CST in SQL (authored-sql category C) | **D** as a rule-facing table. Ownership and owners become **T+** facts (extension 4) |
| `relational/{insert,inputs,batch,schema,storage,pool,deferred,admission,table,producer,producer/completed}.rs`, `relational.rs`, `compile/relational.rs` | about 4.5k | Authored-SQL stage execution: CTAS, deferred SQL columns, evidence inputs into temp tables, admission, pools | Mixed | 733 of 760 CPU samples in batch six were relational evaluation | **D** under decision 0002. `inputs` (AST evidence projected into tables) becomes the transducer's typed capture-export relation (**T**) |
| `relational/reachability.rs`, `relational/forest.rs` | 443 | Batched closure over reference and call edges; native forest | No; repository graph | Worked in DLRE | **S**. Keep as a native graph producer behind a relation contract (a vtab, not the transducer) |
| Reference edges, declared-reference extraction | in `relational.rs`/inputs | Extracts declared references per file, then resolves them across files | Extraction is tape-local; resolution is not | — | Extraction **T** (fact emission); resolution **S** |
| `producers/` | 349 | Producer contract registry (ordering, multiplicity, identity, completion) | — | — | **S**. It is the relation catalog. Ordering and multiplicity fields relax under decision 0001 |

### 2.3 N1562 `query/` (in `8b6576f0`)

| Item | Lines | Computes | Tape-local? | Verdict |
|---|---|---|---|---|
| `query/relational_ir.rs` + `query/relational.rs` | 8,746 | `RelationNode` composition in SQL: `Navigate` (ancestor/descendant/sibling with neighbor/end/rule stops), `NthChild`, `Constrain`, `Count`, `Compare`, `Value`, `Semantic`, `Parsed`, `Utility`/`RecursiveUtilitySpec` (recursive CTE), `SelectWhen`, `RequireEach`, `Budget`, plus `SqlOperation`s (`AntiJoin`, `OrderedUnion`, `WindowRank`, `MinPosition`, `RecursiveCte`, `CrossFileJoin`, …) | Mostly yes | In-file nodes (`Navigate`, `NthChild`, `Constrain`, `Count`, `Compare`, `Value`, `Utility`, `Semantic`, `Parsed`; in-file `AntiJoin`/`Union`) are **T/T+**, about 80%. Cross-file nodes (`SelectWhen`/`RequireEach` across files, repository `Budget`, `CrossFileJoin`, file filters) are **S**, and seed the composition layer |
| `query/compact_output_sql.rs` | 981 | Hand-written SQL that reassembles `rerail_matches` rows (status, roots, related matches, captures) into findings with `row_number`/`distinct` | Yes | **D**. Most of it reassembles per-document structure SQL should never have received; the rest becomes the findings consolidation step |
| `query/plan/` (`mod`, `identity`, `scope`, `grouping`, `semantics`, `utility`) | 1,396 | Semantic plan identities, exact candidate-scope identities, grouping by identical scope, utility dedup by semantic identity | No; planning | **S**. Keep as `engine/plan/`. Utility dedup now dedups transducer sub-programs |
| `transducer/sql_backend.rs` | 2,903 | The `rerail_matches` vtab | Boundary | **Keep, reshaped.** Use narrow typed outputs per section 4 instead of one 35-column union. Report estimated rows from statistics, not a constant 100,000. Add demand through `sqlite3_vtab_in` |

### 2.4 Moti's branches

| Item | Lines | Verdict |
|---|---|---|
| `sql-dl60` `sql/{frames,emit,lower,exec,nodes,nearest,rec}.rs` | about 12.6k (shared base with SSC's frames/emit) | **D**. A SQL interpreter of tape-local matching over a node vtab (D73). facts-engine's "23% of native" came from *removing SQLite from this tape-local work*, which supports moving it into native |
| `facts-engine` `content_store/facts_memo.rs` | — | **S** (storage). A content-hash memo of per-document outputs. It fits caching transducer outputs per (document digest, program digest) |
| `facts-plan` Y1 (maintained reference graph, 72 MB, 0.7/2.4 ms p50/p95 per edit) and Y2 (rules as local queries over it) | — | **S**. These are the cross-file relations the SQL layer composes over. Per-document reference extraction feeding Y1 is **T** |

### 2.5 ast-index-spike

- **In SQL:** everything structural. This was forced by C1, no native code on PG/MariaDB (design index topic 8).
- **Native, on SQLite only:** decode scalars, then the `nera_kids` table function as the default. That gave 493.6 s → 153.5 s on a 3 MB sweep, and about 390x on the hot query once it took an address rather than a value.
- **Its own per-touch analysis:** SQL row and union bookkeeping, not decoding, dominates. "A batch or columnar executor that avoids per-row machinery attacks the main cost" (topic 19, D S152).
- **Its linter analysis:** 77 of 99 rules are single-file. Anti-join, counting and closure are written by the caller around structural fragments (LINTER_ARCHITECTURE §5, §7).
- **Implication:** the per-file structural fragment, the transducer, is the unit, and the caller (SQL) composes around it.

## 3. Transducer IR extensions needed for T+ items

These are named against N1562's IR. Tempo's newer IR may already have some of them; the other investigation covers that.

1. **Seeded start set.** Begin at preorder positions supplied by demand: SQL roots, FTS `instance` positions when selective, or roots bound by a cross-file join. N1562 has a sparse task grid; this generalises it into an input relation.
2. **Capture-bearing demand.** Inputs carry pre-bound capture values (node ids or text) that `Assert` checks. SSC's pivot lists this as open ("relation-handle contract").
3. **Sub-program call with a boolean result.** `Call{program}` / `Not{program}`, for `not`, `inside`/`has` with a rule `stopBy`, `matches` (utilities), and constraints. Nested rules execute inside the transducer instead of becoming SQL operands. Today `Fork` gives alternatives but not negation or lookaround over a sub-program.
4. **Enclosing and ancestor-stack facts.** "Nearest ancestor of kinds K" and an accumulator along the ancestor chain (owner function, owner class, qualname built by an output-program `Concat`). This covers DLRE ownership/owners, SSC `nearest.rs` and authored-sql category D.
5. **Counted sub-search.** `CountMatches{program, axis, stop, op, n}` for `count` and the per-file `budget` part. It is a reduction over a sub-program per root; N1562 reductions are per program today.
6. **Utility calls with parameters, recursion and memo.** `Call{utility, args}` with a bounded stack and a per-(utility, node, args) memo arena. The memo is a performance device only; under decision 0001 its accounting is not observable. On Metal it may stay CPU-only (capability flag below).
7. **Document switch.** Run a sub-program on a derived or embedded document, or a parsed region, and map nodes back to the host document. The `document_id`/`mapped_document_id` columns exist; this makes it an instruction (`ParseFact` today is a leaf).
8. **Per-document fact families.** Rule-independent programs that emit typed fact rows once per document version, cached by digest: declared references, imports, definitions with qualname, call sites, scope bindings. They feed `binds`/`resolves` locally and the cross-file graphs (Y1). This generalises `SemanticFact`/`ValueFact`, which are per-predicate today.
9. **Backend capability flags per program.** Metal, JIT or CPU. A program using 3 or 6 can be routed to CPU while Metal keeps the regular subset. Tempo's full-corpus failure on containment memory (workstream scan) argues for this escape valve rather than for keeping composition in SQL.
10. **Completeness only.** Output a per-(program, document) `Complete`/`Unknown` bit. `work_used` stays as a diagnostic only (decision 0001).

Per-function data flow (authored-sql category H) could become a T+ per-document fixpoint program. Cross-function and cross-file flow stays S. This is open and is decided per rule.

## 4. The proposed boundary

### 4.1 Transducer → SQL (relations the transducer binding emits)

All are narrow and typed, with set semantics. Each row carries `(batch, program, doc)`, plus a per-(program, doc) outcome row.

| Relation | Columns | Notes |
|---|---|---|
| `t_match` | `program, doc, root` | Keys only. Enough for findings (decision 0001). The integration rehydrates payloads |
| `t_export_<schema>` | `program, doc, root, <declared typed exports>` | Only captures the rule *declares* it needs above the file: join keys (`require_each` keys, `select_when` keys), identity tags, count group keys. One schema per export shape, never a 35-column union |
| `t_outcome` | `program, doc, complete, reason` | `Unknown` per decision 0001 |
| `t_count` | `program, doc, group…, n` | Per-document reductions, for per-file `budget` and `count` |
| `fact_<family>` | e.g. `fact_ref(doc, node, name, kind)`, `fact_def(doc, node, qualname, kind)`, `fact_import(doc, module, name, alias)`, `fact_call(doc, node, callee_name)`, `fact_scope(doc, node, owner, qualname)` | Rule-independent, computed once per document digest, cached (facts memo), incrementally maintained. These are engine relation contracts (authored-sql D/E/F/G) |

The binding is one `xFilter` per (program group, batch). Programs are grouped by identical candidate scope (N1562 `plan/grouping`). `estimatedRows` comes from statistics.

### 4.2 SQL → transducer (demand)

`demand(program_group, doc [, root] [, bound exports…])`, received through `sqlite3_vtab_in`.

- Usually `doc` only. The file set comes from the planner's candidate scope: FTS file postings and `fts5vocab` counts, intersected with the incremental change set for per-edit runs.
- `root` and bound exports appear only when a cross-file join must drive matching. Extensions 1 and 2 cover that.

### 4.3 What the SQL layer above owns

- Cross-file composition: `require_each`/`select_when` across files, anti-joins across files, joins on exported keys, unions.
- Repository-level counts and `budget`.
- Closures over cross-file graphs: reachability, class hierarchy, import resolution. These use a recursive CTE, or a native graph producer vtab (DLRE `reachability`/`forest`, facts-plan Y1). Neither is a transducer.
- Policy data: allowlists.
- **Statistics-driven planning:** which documents to run, which program groups, batch formation, and per-program start-shape choice (seeded versus scan) from `fts5vocab` and edge statistics, at plan time and cached (README, Statistics).
- Incremental maintenance: which cached `fact_*` and `t_*` outputs are invalid for a change set.
- The outer query, and findings consolidation into the key set (`findings-contract.md` §4).
- The **pure-SQL fallback**, low priority: each transducer program lowered to *specialised* SQL over a tape-reading TVF. SSC's relational emitter is the starting point. A generic transducer interpreter in SQL is forbidden (D73). It is differential-tested against the transducer.

## 5. Conflicts with the current `architecture.md` module map, and the revised map

**Conflicts:**

- "Lowering | `engine/lower/pattern/` | Structural pattern compiler: grammar → typed relations → SQL fragments (SSC)" puts tape-local structure into SQL. **Wrong boundary.**
- "Lowering | `engine/lower/anchors/` | Candidate selection lowered to SQL (DLRE)" puts in-file candidate selection into SQL. That is the 537 s cost. **Wrong boundary** for the in-file part; right for file-level scope.
- "Execution | `engine/exec/packed/` … behind the vtab boundary" makes the transducer one executor among several. **It should be the primary IR**, with `AstMatcher` as one source that lowers into it.
- Phase order:
  - Phase 2 (SSC) loses most of its value. Take it for history, the fallback emitter and the fragment-boundary pattern; expect most of `outcomes/` to be deleted.
  - Phase 3 (N1562) becomes the key merge.
  - The Tempo IR (phase 6) defines the IR and should be read before phase 3's restructure.

**Revised map** (under `native/src/code_rule_dsl/`):

| Module | Responsibility | Source |
|---|---|---|
| `engine/transducer/ir/` | The transducer IR: structural, value, capture, output and reduction programs, plus extensions 1–10 | Tempo Metal IR, N1562 `packed/native_ir.rs`, `value.rs`, `output.rs` |
| `engine/transducer/lower/` | `AstMatcher` (and utilities, embedded documents, semantic predicates) lowered to transducer programs. In-file composition happens here | N1562 lowering into `packed`, plus the in-file parts of N1562 `relational_ir` and SSC `outcomes`, rewritten as IR lowering |
| `engine/transducer/exec/{cpu,jit,metal}/` | Executors | N1562 `packed/native_cpu*`, `transducer/jit.rs`; Tempo Metal |
| `engine/transducer/facts/` | Rule-independent per-document fact programs (`fact_*`) | N1562 `semantic_facts.rs`, DLRE ownership/reference extraction |
| `engine/vtab/transducer.rs` | The boundary: demand in, narrow typed relations out, statistics-based estimates | N1562 `transducer/sql_backend.rs`, SSC pivot `fragments.rs`/`fragment_job.rs`, SSC `terminals/` |
| `engine/vtab/graph.rs` | Cross-file graph producers (closure, forest) | DLRE `relational/reachability.rs`, `forest.rs`; facts-plan Y1 |
| `engine/compose/` | Declarative rule operators above the file (`require_each`, `select_when`, `budget`, join/union/anti-join, `reaches`, allowlists) lowered to SQL over the relations | Cross-file nodes of N1562 `relational_ir`; SSC `crossfile.rs`, `select_when.rs` |
| `engine/plan/` | Candidate scope, grouping, sub-program dedup, shape choice from statistics, plan cache | N1562 `query/plan/`; DLRE file-level anchor scope (`corpus`, `store`, `postings`, `plans`) |
| `engine/stats/` | SQL views over `fts5vocab` and edge counts | as before |
| `engine/relations/` | Contract catalog for `t_*`, `fact_*` and graph relations | DLRE `producers/` |
| `engine/fallback/` | Pure-SQL binding: transducer program lowered to specialised SQL (low priority) | SSC `sql/relational/` emitter core |
| `engine/findings/` | Consolidation (`findings-contract.md`) | unchanged; absorbs N1562 `compact_output_sql.rs` |
| deleted | SSC frames/emit/fused/large/exec and state-machine `outcomes`; Moti's frame engine; SSC `nodes.rs` as an execution path; DLRE authored-SQL relational machinery; DLRE in-file anchor SQL lowering; memo, continuation and peak SQL machinery | — |

## 6. Share of effort

| Stream | SQL-lowering lines (excl. tests/spikes, approx.) | T/T+ | D | S |
|---|---|---|---|---|
| SSC `sql/` | about 89k | about 45k (relational core and outcomes structure) | about 41k (frames/emit/exec, state-machine emitters, memo SQL) | about 3k |
| DLRE anchors-sql + relational | about 14.5k | about 5.5k (in-file anchors) + extraction | about 5k (authored-SQL machinery) | about 2.5k (file scope, reachability/forest, producers) |
| N1562 `query/` | about 11k | about 7k (in-file `RelationNode`s) | about 1k (`compact_output_sql`) | about 3k (plan, cross-file nodes) |
| Moti frame engine | about 12.6k (overlaps SSC base) | — | about 12.6k | — |

Overall, about 85–90% of the SQL-lowering code computes tape-local things, or exists only to reproduce exact operational semantics, and belongs in T, T+ or D. Measured effort follows the same split:

- DLRE's dominant costs (anchor SQL 537 s, index preparation 137 s) are tape-local.
- SSC's remaining open items (work accounting across fragments, refused grammars) disappear once composition within a file lives in the transducer and decision 0001 holds.

## 7. Risks

- **Metal regularity.** Utility recursion, memo and sub-program calls are data-dependent. Keep them CPU/JIT-capable first, and route by capability flag (extension 9). Do not let Metal's limits push composition back into SQL.
- **Statistics-driven shape inside a program.** The start kind or seed choice must be a plan-time parameter of the program (compiled per stats bucket and cached), not a runtime `count(*)` (trap T18).
- **Fact families become a versioned schema.** PR 2539's lesson applies: keep them generic, and suspect any relation named after a rule.
- **The pure-SQL fallback** cannot be a transducer interpreter (D73). It needs per-program SQL emission, so its coverage will lag.
