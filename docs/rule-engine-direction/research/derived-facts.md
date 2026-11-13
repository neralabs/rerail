# Named derived facts: design report

The normative text derived from this report is `spec.md` §21 and decision 0005. Part (e) lists the spec edits that were applied.

**Sources and short forms**
- **PR** = nera-data-layer PR 2539 at tip `281f500d7c`, read from the checkout `/Users/jeremy/orca/workspaces/nera-data-layer-10/DL-PR-2564`. `dd/` means `docs/design/`.
- **FE** = `origin/Moti-Velz/facts-engine` at tip `685db3f7`.
- **HEAD** = the Pulling-it-all-together worktree.
- Gate-corpus row counts come from `/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Pulling-it-all-together/3adb237a-22e7-4e0c-9a7a-33e109d2ab63/scratchpad/rule-profiles/out/facts.sqlite`, `.../flow.sqlite` and `.../measure.txt` in the same folder, plus `research/heavy-rule-profiles.md`.

---

## (a) What PR 2539 and the facts-engine designed

### PR 2539

The PR designed four layers. Shared intermediate facts belong to fact materialization, which is framework-owned, generic and named. They are not part of linter execution.

1. **Pass structure.** `dd/LINTER_GENERIC_FRAMEWORK_FACTS_DESIGN.md@281f500d7c:93-147` defines four passes: Pass 0 paths, Pass 1 AST primitives, **Pass 2 "Generic Derived Facts"**, and Pass 3 YAML policy queries.
   - Pass 2 "Runs after primitive collection over the per-file facts… may derive reusable facts… This pass still belongs to fact materialization, not linter execution" (`:126-137`).
   - Pass 3's only policy inputs are atom sets, path sets, signed allowlists, ratchets and messages (`:139-147`).
2. **SQL-only boundary.** `dd/DECLARATIVE_LINTER_SQL_ONLY_BOUNDARY.md@281f500d7c`:
   - Python may only discover, parse, "Write generic, framework-owned AST/source rows", compile and execute (`:8-19`).
   - Forbidden: linter-specific collectors, linter-specific SQL adapter relations, Python-side materialization for linter logic, and "Precomputed linter-specific candidate tables" (`:21-29`).
   - The YAML structural operators are `from, join, left_join, recursive, group_by, aggregate, project, union, materialized, and named queries` (`:67-71`).
   - "Any relation named after a linter, rule, or finding class is suspect unless it is the final projected query output" (`:105-106`).
   - "Shared call graph and reachability helpers must be generic, parameterized by YAML labels" (migration step 3).
   - "Projection invalidation follows declared dependency edges and updates only changed files plus the affected dependency cone" (`:136-143`).
3. **"Process Failure"** (`:169-186`, the only occurrence in any of the docs). Verbatim, it warns that "Python collectors were allowed to produce linter-shaped facts", that "SQL adapter branches were added for linter-shaped relations", and that "Some YAML specs expressed only the last step of a pipeline whose real work had already happened in Python". The implementation drifted into "**Python prepares convenient rows, YAML queries them**", which does not meet the bar.
4. **Catalog relations and dependency maps.** Shared derived relations are SQL over framework tables, dispatched from `_relation_source` (`scripts/lint_lib/fact_query/sqlite_executor.py@281f500d7c:4113-4460`). Examples:
   - `python_method_pair`: a sync/`_async` self-join on (path, class_name, name), at `:2745-2770`. It is used by `missing_async_pairs` and `async_sync_delegation`.
   - `python_type_descendant`: a recursive CTE, at `:2858-2882`.
   - `python_function_call_edge` (`:3560-3654`), `python_definition_flag` (`:2793-2855`) and `python_function_summary` (`:3492-3557`).

   Dependencies are declared, not inferred:
   - `DECLARATIVE_RELATION_EXTRACTORS` maps relation to extractor (`scripts/lint_lib/fact_query/default_catalog.py@281f500d7c:54-129`).
   - `DECLARATIVE_PHYSICAL_RELATION_DEPENDENCIES` maps relation to relation, e.g. `"python_type_descendant": ("python_class_base",)` (`:131-141`).
   - Rule-to-fact demand is computed from the YAML graph (`fact_query/demand.py@281f500d7c:35-80`) and folded into each extractor's `cache_version` (`linter_plugin.py@281f500d7c:40-43, 72-130`).
   - `DerivedFactSpec` is a "Versioned fact computed from other materialized per-file facts" with `input_facts` (`linter_plugin.py@281f500d7c:133-138`).
   - Project-scoped facts "materialize… once per dependency key and expose them through the same fact-query catalog" (`dd/LINTER_EXPRESSION_FACT_QUERY_DESIGN.md@281f500d7c:6177-6203`).
   - `SqliteProjectFactCache` caches "derived project facts keyed by contributing fact content" (`project_fact_cache.py@281f500d7c:13-54`).
   - The memo plan has `Dependency(kind, identity, fingerprint)`, with the note "Filesystem metadata alone is not a validity proof" (`dd/LINTER_MEMOIZATION_AND_DEPENDENCY_PLAN.md@ab2e3f714c:83-121, 242-258`).
5. **Storage and invalidation.**
   - Base facts are per file, owned by `bundle_key`, keyed by content hash plus `cache_version`. They are deleted and rewritten in one transaction (`dd/LINTER_EXPRESSION_FACT_QUERY_DESIGN.md@7c4469493d:609-630`; `fact_store.py@281f500d7c:1614, 3473-3510`).
   - Query identity is `query_id / schema_version / implementation_version`.
   - "Persist query results only when measurement shows a query is expensive" (`@7c4469493d:621-623`).
   - Later the PR reversed from a skinny entity/slot schema to typed per-family physical tables (`DECLARATIVE_LINTER_FACT_ROW_WIDTH_PLAN.md` Steps 5–6). That took 500 files from 47.1 s to 18.0 s.

**Where the tip fell short of its own design.** This is not a disagreement with your account; the design has the intent, but the tip doesn't implement it.
- Catalog relations are inlined as CTEs in every query, so they are never cached or shared across linters. The design says "use per-query CTEs so unused logical relations cost nothing" (`LINTER_SKINNY_FACT_SCHEMA_REDESIGN.md@281f500d7c:346-347`).
- YAML `materialized: true` is only a CTE hint (`declarative_linter_sqlite.py@281f500d7c:250-252`).
- The dependency-cone tables (`lint_projection_dirty_queue`, `lint_projection_row_owner`, `lint_*_dependency_edges`, `fact_store.py@281f500d7c:6682-6747`) are created but never read.
- `SqliteProjectFactCache` has no callers.
- `DerivedFactSpec` is rejected on the declarative path (`fact_engine.py@281f500d7c:1800-1802`).
- The one global derived table, `lint_function_transitive_summary`, is rebuilt in full on every save (`fact_store.py@281f500d7c:8788`). It also hard-codes lazy-architecture tails (`:8861-8899`), which is a Process Failure case, as are `serverplan_step_class` and `pure_lazy_body_event`.

### Facts-engine (Moti)

- **Today: one definition for both engine and compiler.** Derived relations are SQL views written in `docs/facts/SCHEMA.md` (`py_owner`, `py_function`, `py_call`, `py_parameter`, `cg_def`, `cg_call`, …, `:84-284`). The engine reads them with `include_str!` (`spikes/facts-a/src/views.rs:1-27`). Any change bumps `SCHEMA_VERSION`.
- **Rules don't name facts yet.** A rule stage binds to a view only when its text is byte-identical, and only under a subtree-closure side condition (SCHEMA.md §2.3).
- **The memo is per clause, not per fact.** `facts_memo.rs:2` keys it by "(content hash, plan digest, fact schema version[, path])". Content-pure clauses use the empty path, so identical content is evaluated once (`:7-10, 36`). The plan digest is blake3 over engine version, schema, rewrites, rule, inputs and the physical plans of the stages (`spikes/facts-a/src/memo.rs:81-98`). Invalidation is purely by address, with `ON DELETE CASCADE` on the source (`schema.rs:281-310`). Measured: warm restart in 486 ms with 1.05M clause runs served from the memo.
- **Proposal, not scheduled** (`docs/facts/DECLARED-FACTS.md`):
  - A `facts` block with `over: content|membership`, `columns`, `key`, `fd`, `define: {sql | match}` and `version`.
  - Identity is `(fact digest, schema version)`, "where the digest covers the definition's algebra after lowering, not its text" (`:53-54`).
  - A duplicate key is a named refusal.
  - Motivation: "34.9 s of byte-identical stage text recomputing the same relations per clause" becomes "12 declared facts computed once per content".

### DLRE and N1562 on HEAD

- **Producer catalog.** `producers/mod.rs` is staged but not committed. A binding's identity is contract name + inputs + typed options + domain, compared as a full definition: "No digest can establish sharing" (`:61`). It caches nothing across evaluations (`docs/shared-producer-contracts.md:55-56`).
- **Shared syntax facts (c4bd76bf).** Cold CPU went 12.62 → 3.19 s (−74.7%) once one shared preparation replaced per-rule `nodes`/`ownership` rebuilds. Before that, each ownership materialization ran exactly 5,299,816 VM instructions three times (`shared-sql-performance.md:85-98`).
- **The counter-lesson.** "Adding names to already shared rows alone does not establish a performance improvement" (`shared-producer-workplan.md:271`). Sharing pays only where the same work actually repeats.
- **Deferred SQL facts (f52817fc).** These are view-computed columns over a stored stage table. They are a physical detail, not a fact model.
- **N1562 `semantic_facts.rs`.** These are per-anchor rows computed inside the packed producer, with no cache and no cross-rule identity. SQL joins them to nested-rule relations.

**Conclusion.** All three agree that shared intermediates must be generic and named. They should be computed once per content (per file) or per dependency key (repository), and joined in SQL. The unbuilt pieces are the same everywhere: a persistent per-fact cache keyed by a definition digest, and dependency-cone invalidation.

---

## (b) Normative design: derived fact relations

### B1. Three producers, one fact namespace

Every relation a rule may name is a fact. Each fact has exactly one producer:

| Producer | What it computes | Language | Example |
|---|---|---|---|
| **Fact program** | Base facts that come from tree structure, rule-independent | Transducer IR, compiled from the ordinary AST rule language, emitting `EXPORT` labels into a declared schema | `py.def`, `py.call`, `py.param`, `py.import`, `py.annotation_atom` |
| **Native producer** | Base facts that need a resolver or evaluator | Reviewed Rust behind a relation contract | `py.scope`, `py.binding`, `py.value`, `py.flow_exit` |
| **Derived fact** | Relations over other facts: joins, unions, anti-joins, aggregates, recursion | The declarative relational algebra, compiled to SQL and executed in SQLite | `py.def_family`, `py.call_edge_local`, `py.import_res`, `py.subclass_closure` |

"Shared" does not by itself mean "SQL". A shared analysis that walks the tree, such as annotation atoms (about 18 rules) or receiver chains (about 12 rules), is a fact program: named, cached and computed once per document, but still run by the transducer. SQL computes what is relational: name-keyed joins, fixpoints over names, aggregates, and anything that crosses files.

### B2. Where definitions live, and in what language

**Decision: a declarative algebra, not authored SQL, in fact-definition documents separate from rules.**
- `engine/facts/catalog/*.facts.yaml` (`rerail.facts/v1`): language-level facts owned by the engine (`py.*`, `php.*`).
- `packs/<pack>/facts/*.facts.yaml`: domain facts owned by a pack, under the `<pack>.*` namespace.
- Rule documents never contain definitions. They reference facts by qualified name.

The operators are those PR 2539 settled on: `from`, `join`, `left_join`, `anti_join`, `union`, `group_by` + `aggregate`, `project`, `recursive` (positive and stratified), and literal policy relations. Scalars use the existing `Predicate` AST and typed column references. There are no expression strings and no UDFs.

Why the algebra rather than reviewed SQL files:
1. **Decision 0002's reasons still apply.** Materialising once removes the sharing objection, but the compiler still has to:
   - choose the physical form. Wording alone gave 22× and 34× swings (`heavy-rule-profiles.md` §4: reachability 0.80 s vs 17.8 s; scoped resolution 0.73 s vs 24.8 s). With the algebra the compiler emits the good form and records it;
   - **prove the partition** (B4). "Per document" is a property the compiler can check over the algebra (every join is an equi-join on `doc`, and every input is per-document). Over SQL text it is only a claim;
   - **derive dependencies and invalidation** from the definition. PR 2539 declared its dependency maps by hand and never wired its dependency-cone tables. Lineage derived from the algebra cannot drift;
   - give each relation its second binding (spec §1.3) and push it down behind a vtab.
2. **FE's own proposal digests "the algebra after lowering, not its text"** (DECLARED-FACTS.md:53). That only works if a canonical algebra exists.
3. **Escape hatch:** anything the algebra cannot state becomes a native producer. There is no SQL tier. The generated SQL of each derived fact is dumped and stored with its plan (`spec-tooling.md`), so review happens on output that is visible and recorded.

**Admission rules (the Process Failure guard).** A derived fact or fact program is admitted only if:
- (a) its name denotes a language or domain concept, never a rule, finding or linter. `serverplan_step_class` becomes `py.subclass_of` with the atom `ServerPlanStep` supplied as policy data;
- (b) it is used by at least 2 admitted rules, or it is a catalog family named in spec §9;
- (c) its estimated rows per document are at most the document's node count, so it is never a tree copy (L4);
- (d) it carries no rule-specific seeds. Rule-specific closures use the rule algebra's `reaches` operator over a generic edge fact.

### B3. Identity and versioning

```
fact_digest(F) = H("rerail.fact/v1" ‖ canonical lowered algebra (or fact-program digest / native producer id+revision)
                   ‖ output schema (columns, types, key, FDs) ‖ sorted fact_digest of every input ‖ fact compiler version)
```
- `version:` in the document is human-facing only. The cache never trusts it.
- A change to any input's digest changes every dependent's digest transitively. A native producer must bump its revision whenever its output changes. This is FE's `ENGINE_VERSION` rule.
- Two packs that declare the same name with different digests is a compile error. The producer catalog's full-definition comparison (`producers/mod.rs:72-86`) is kept as the clash check.

### B4. Partition, materialization and demand

**Partition.** The compiler infers it and refuses a mismatch with what the document declares, as in FE's `over`.

| Partition | Condition | Unit of storage and recomputation |
|---|---|---|
| `document` | All inputs are per-document, and every join, group and recursion is keyed by `doc` | rows owned by `doc` (`tape_id`) |
| `repository` | Otherwise: membership-dependent, such as import resolution or the class closure across files | one relation per workspace snapshot, maintained by dependency cone (B6) |

**Physical form.** The planner chooses per admitted plan:

| Form | When |
|---|---|
| **Table** (default for derived facts) | 2 or more consumers in the admitted plan, or recursive, or `repository` partition |
| **Inline** (expanded into its single consumer) | Exactly 1 consumer, non-recursive, `document` partition. This is PR's "unused relations cost nothing" |
| **Vtab** over native columnar data | Base facts from fact programs and native producers (M3 decides; heavy-rule-profiles measured real tables at 16.4 s for 7.9M rows and estimated about 1 s for a vtab) |

Hard rules:
- **One physical owner.** A fact is materialised at most once per (document or snapshot, `fact_digest`) per evaluation, and never re-expanded into several statements. This is lesson 16: one statement held 710 CTEs, and one definition was repeated across 61 and 90 statements.
- Tables are narrow and typed, with atoms interned as u32. There is no EAV and no union table (lessons 11 and 12, spec §9 "Forbidden").
- Indexes are chosen by the planner from consumer access paths and recorded in the plan.
- **Demand-driven.** The plan computes the closure of facts reachable from admitted rules. A per-document fact is computed only for documents inside the demanding rules' scopes. A fact no rule demands costs nothing.

### B5. Cache keys and storage

- **Per document:** `(fact_digest, tape identity = (source digest, grammar_id, grammar_version, tape_format_version)[, path if the definition reads path])`. Content-pure facts carry no path, so identical content is computed once (FE `CONTENT_PURE`).
- **Repository:** `(fact_digest, snapshot generation vector of its input facts)`.
- **Catalog tables in the content store:**
  - `derived_facts(name, digest, table, partition, form, created)`;
  - `derived_fact_parts(digest, doc_key, complete, reason, rows)`;
  - one physical table per (name, digest-prefix), so versions coexist during rollout.
- **Reclamation:** rows cascade with their source object, as in FE.
- **Completeness is per (fact, document).** Absent rows mean false only inside a complete domain. An incomplete input makes its dependents `Unknown` for the affected (rule, file) pairs, through `t_outcome` (spec §9).

### B6. Incremental invalidation by dependency cone

- **The dependency DAG comes from the algebra.** Fact nodes are the vertices, and recursion is folded into its own strongly connected component. Each rule's plan records the fact digests it read.
- **Edit to document d:**
  1. Recompute d's base facts (fact programs and native producers): under 2.5 ms typical.
  2. Recompute the demanded per-document derived facts for d only: delete d's rows, then insert.
  3. For each `repository` fact in the cone, compute the input delta Δ (d's old rows against its new rows). Then:
     - non-recursive operators: semi-naive delta rules, touching only rows whose join keys (atoms) appear in Δ;
     - recursive components: delete-and-rederive (DRed), or a full recompute when |Δ| crosses a planner threshold. A full reachability recompute measured 342 ms (`heavy-rule-profiles.md` §5).
  4. Re-run only rules whose read set contains a changed (fact, key) pair.
- **Per-edit serving.** The edited document's per-document facts are computed synchronously. Repository facts are the snapshot plus the overlay delta. The target is M8: one file through facts, resolution and reachability at 1 s or less p95.

### B7. Statistics for the planner

- Every materialisation records exact counts in `fact_stats(digest, column, rows, ndv, max_rows_per_doc, null_frac)`.
- For physical tables, the engine writes `sqlite_stat1` rows directly from those counts, which are exact and cost nothing. Derived tables never run `ANALYZE` (spec §5).
- `xBestIndex` estimates, and decisions that read fact sizes, store the statistic keys they used in `compiled_plans`, with validity intervals (spec §11).

### B8. How rules reference facts

- **Closed rule algebra:** `from: {fact: py.call_edge}`, `join` / `anti_join` on typed columns, and `reaches: {from: <rule relation>, via: {fact: py.call_edge_local}, to: …}`.
- **AST rules:**
  - the semantic keys (`resolves`, `binds`, `calls`, `arguments`, `value`) are bound to catalog facts, as today;
  - one new predicate: `fact: {name: py.def_family, match: {sync: $F, async: $G}}`, a semi-join of captures against a fact (and `not: {fact: …}` for the anti-join);
  - capture projections such as `capture.F.qualname` read a fact column through its declared key.

### B9. How transducer programs consume derived facts

The pipeline is stratified per document:

```
fact programs / native producers (base facts) → SQL derived facts → rule programs → SQL composition
```

There are three bindings, chosen by the planner:
1. **`SIDE` columns** (`spec-transducer.md` §2.8). Used when the fact is keyed on (doc, node) with a functional dependency node → value, such as `enclosing_scope.owner`, `qualname_id` or `binding.target`. The table is scanned in doc order into a resident u32 column, defaulting to NONE.
2. **Fact sets.** A new set-stage definition, `FACT f [column = const]`, gives `{y : (doc, y) ∈ f}`, loaded as a posting list and read by `IN_SET`/`COUNT_SET`. It replaces what `KEYS`/`IN_KEYS` delivered.
3. **Demand seeds** (E7), or a **join in SQL** above the transducer over `t_match`/`t_export`.

The verifier rejects any plan with a cycle across the boundary: a rule program cannot feed a fact that the same program reads.

### B10. How transducers produce base facts

- A fact program is a rule-independent AST query in the catalog whose `EXPORT` labels carry the fact's schema.
- It fuses with other fact programs under the ordinary fusion rules, with the same candidate-scope digest per family.
- It is cached by `(fact_digest, tape identity)`, and it is subject to L4: exported rows per document must stay at or below the node count.

---

## (c) The boundary, balanced

Test: put each condition where it is simplest to compute once and share. A program should read like a structural pattern; anything that reads like a query goes to a derived fact.

**Stays in the transducer:**
- kind, field, pattern and text tests;
- `all`/`any`/`not`;
- tree relations with every `stopBy` mode, `nthChild` and bounded `COUNT`;
- non-recursive utilities;
- **E14 monotone single-axis FIX** (receiver chains, annotation walks, destructuring);
- `TEXT_EQ`/`SIDE_EQ` inside `EXISTS` when one side reaches the other by navigation;
- **E16 `ORDER`**: one comparison, local, used by L2 and LEG-085;
- reads of `SIDE` and `FACT` sets.

**Moves out.** Each change below follows the same reasoning: E15 and E17 add machinery to every backend and to the verifier, while the equivalent work is cheap and shareable in SQL or native code.

| Construct | Verdict | Replacement | Reason |
|---|---|---|---|
| **E15 `KEYS`/`IN_KEYS`** (joins between independent matches in one document) | **Remove** | Lowering rule L5: the compiler splits the rule into two exporting programs whose rows are `(doc, key_atom, node)` for candidates only. SQL semi-joins them on `(doc, key)`. The result is either findings or a `FACT` set fed back to the outer program | M7's quadratic case (0.451 s → 3.6 ms with keyed sets) is solved just as well by an indexed join. Candidate exports are small (906 functions in `generated_facades.py`). Removing it deletes a sort, scatter and binary-search kind from every backend, and its key-source typing from the verifier. Where the join is shared, it is a named fact (`py.def_family`, the PR's `python_method_pair`) |
| **`KEYS` inside `FIX`** (recursion through names: intra-file call graphs, `raw_shapers`, `eager_reach`, `bad_configs`, executor owners, local value flow, taint) | **Remove** | The derived fact `py.call_edge_local` (85,560 rows), plus `reaches` with rule seeds. Data flow is `py.flow_edge` (a fact program) plus a recursive derived fact | This is fixpoint machinery over names, the typical relational case. Measured: recursive SQL taint 0.49 s, against 0.94 s for a native worklist (`heavy-rule-profiles.md`). Call-graph closure 5 ms |
| **E17 `FOLD_UP`** (only use: "always raises" in quality-conditional-1, 4 files) | **Remove** | Native producer `py.flow_exit(doc, node → exits: never/always/maybe)`, read via `SIDE_TEST`. Also reusable for missing-return and unreachable-code checks | It added the only negation-through-recursion exception to the verifier, spec §6 and spec-fusion §6.8. A generic control-flow summary is a bounded reverse-preorder pass in reviewed native code, with nothing new for Metal |
| "Everything within one document lowers to the IR" (spec §6, §7, decision 0004) | **Replace** | "Local, navigational conditions lower to the IR. Per-document relational conditions lower to derived facts (partition `document`)" | Your requirement |
| Shared tree-walking utilities (annotation atoms, receiver chains, store targets, parameters) | **Make them named fact programs** | `py.annotation_atom`, `py.receiver_token`, `py.binding_site`, `py.param` | Today they are re-derived in about 18 rules, about 12 rules, and so on, as private recursive utilities. Naming them moves no work out of the transducer and removes the repeats |

**What remains new in the IR:** E16 and the `FACT` set definition, which is a posting-list load. `MAX_STRATA` and the "positive within a stratum" rule hold with no exception.

**Effect on `research/heavy-inputs.md` §4.** Of the 48 per-document clauses:
- About 38 stay purely in the IR: the 28 dedupe-only clauses, authority-sites, quality-contracts-2/3, quality-local-taxonomy, direct-execution-lines, state-projection-recovery-2, and most of step-contracts and legacy.
- About 10 now read derived facts:
  - step-contracts-3/4/5 (`raw_shapers`, `column_derived`, `static_exception_functions`);
  - STEP-019/026/028/039, which become L5 joins;
  - legacy `eager_reach`;
  - executor-provenance owners;
  - runtime-reference-summaries;
  - state-flow taint and call_graph;
  - state-signatures-1 `bad_configs`;
  - quality-contracts-1 (scope via `SIDE`);
  - quality-conditional-1 (`py.flow_exit`, plus the class closure).

**Risk:** an extra crossing of the boundary per document on the per-edit path. That is the gate for new measurement M10 in (e).

---

## (d) Inventory: intermediate facts the 60 rules share

The gate corpus is 9,010 files and 28.23M nodes.

**Base facts: fact programs (FP) and native producers (N)**

| Fact | Producer | Columns | Rules using it (examples) | Rows |
|---|---|---|---|---|
| `py.def` | FP (qualname via N) | doc, node, kind, name, qual, scope, is_async | 16 clauses rebuild `functions`, 11 `classes`, 6 `methods`; authority, dead-code, public-api, legacy, required-contracts | 194,281 |
| `py.call` | FP | doc, node, scope, callee, last | 14 clauses `calls`, 11 `arguments`, 10 `keywords`; runtime-reference-summaries | 993,650 calls; 482,606 keyword args |
| `py.param` | FP | doc, fn, idx, name, kind, annot, default | 11 clauses; `pos-param` (the only utility shared verbatim: missing-async-pairs and sync-async-signatures); transaction-patterns `connection-function` (root of all 58 programs) | about 400–500k (estimate) |
| `py.annotation_atom` | FP (E14 FIX) | doc, annot_root, atom, node | about 18 rules (transaction-patterns, sync-async-signatures, plan-scope-ownership, forwardref, grammar-*, QC1, executor-provenance, required-contracts, dead-code) | 0.8–1.2M (estimate) |
| `py.receiver_token` | FP (E14 FIX) | doc, expr, root, token | about 12 rules (async-patterns, state-tripwires ×8, serverplan-steps, legacy, state-flow) | 773,510 edges on the 11.77M-node scope; about 1.9M across the corpus (estimate) |
| `py.binding_site` | FP (destructuring FIX) | doc, scope, name, stmt, value | 12 clauses `assignments`; utilities `store-target`, `local-target` and others | 411,275 |
| `py.import` | FP | doc, scope, local, module, name, level | dead-code, public-api, legacy, QC1, grammar-fragment-usage and 11 more import-kind rules | 226,642 |
| `py.class_base` | FP | doc, cls, base_text | dead-code, public-api, QC1/2, quality-conditional-1, step classes, required-contracts, 6 `superclasses` rules | 22,061 |
| `py.decorator` | FP | doc, def, text, root | dead-code, public-api, legacy, imperative-private-functions | 38,934 |
| `py.export_name` | FP | doc, name | dead-code, public-api | 16,740 |
| `py.return` | FP | doc, fn, node, kind | 6 clauses | 126,609 |
| `py.scope`, `py.binding` | N | per node: owner, top_fn, cls, binding target | D1 in 27 rules, through nearest-enclosing `stopBy`; QC1 scope chains; forwardref | as `SIDE` columns, no rows |
| `py.value` | N | per node: value_kind, value_const | state-flow string feeders, which were 325,944 exported rows and cost 20.0 s | `SIDE` column |
| `py.flow_exit` | N | per statement/block: exits | quality-conditional-1 (replaces E17) | `SIDE` column |
| `py.flow_edge` | FP | doc, src, dst, kind | state-flow ×2, step-contracts-4, executor-provenance | 773,510 on the state-tripwires scope |
| `file.module` | Pass-0 path fact | doc, path, module | dead-code, quality-conditional-1 (today via regex UDFs) | 9,010 |

**Derived facts (algebra compiled to SQL)**

| Fact | Partition | Definition sketch | Rules using it | Rows | Measured cost |
|---|---|---|---|---|---|
| `py.def_family` | document | `py.def` self-join on (doc, scope, base name) for `_async`/`_lazy` transforms | async-sync-delegation, missing-async-pairs, sync-async-signatures, legacy `valid_boundary`, `siblingLinks` in dead-code and imperative-private-functions | 2,421 sync/async pairs; 3,288 `_lazy` defs | — |
| `py.call_edge_local` | document | `py.call` ⋈ `py.def` on (doc, last name / `self.`) | state-flow `call_graph`, legacy `eager_reach`, step `raw_shapers` and `static_exception_functions`, QC1 `preop_functions`, grammar-fragment-usage, state-signatures `bad_configs` | 85,560 | closure 5 ms |
| `py.module_const` | document | `py.binding_site` at module scope ⋈ `py.value`, with recursive alias chains | state-flow ×3 `string_values`, module-level-caches, transaction-closures, facade-purity, compat-aliases, grammar-allowlist-burndown | 27,481 module-level assignments | replaces the 20.0 s of feeders |
| `py.local_flow` (taint) | document | recursive over `py.flow_edge` from source kinds | state-flow, executor-provenance, runtime-reference-summaries, step `column_derived` / `payload_values` | 51,626 tainted nodes | 0.49 s recursive SQL |
| `py.import_res` | repository | `py.import` ⋈ `file.module` ⋈ `py.def`, then the re-export closure | dead-code, public-api, QC1, quality-conditional-1 `class_bases` | 120,229 (123,309 with the suffix variant) | 0.43–0.68 s |
| `py.call_edge` | repository | `py.call` resolved through `py.import_res` or a unique local name | dead-code `reach_edges`, imperative-private-functions `calls {graph: references}` | 352,111 scoped (368,187 by unique name) | 0.73–1.08 s (24.8 s in the bad form) |
| `py.subclass_closure` | repository | `py.class_base` ⋈ `py.import_res`; recursive with a cycle guard | dead-code, public-api, QC1/2, quality-conditional-1, step classes as `subclass_of ServerPlanStep` (268 classes), required-contracts, sql-grammar-escape-hatches, grammar-callable-constructors | 10,112 resolved; 16,561 closure rows | 0.28 s |
| `py.export_closure` | repository | `py.export_name` ∪ re-exports through `py.import_res` | dead-code, public-api `exported-families` | ≤ 16,740 plus re-exports | — |

A whole-repository reachability fact is **not** cached as a fact. `reaches` over `py.call_edge` plus containment (100,618 rows) with rule seeds stays rule-level (174,914 reachable, 9,371 dead; 0.80 s), and its result is memoised per plan as FE does.

**Removed, not turned into facts:**
- `descendants`: about 4.4M rows over step-contracts-1..6, up to about 16.7M in QC1; becomes interval tests on the tape;
- `nodes`/`edges`/`fields` rows: a vtab only;
- `attributes` as rows: AST rules read them directly;
- document gates: lowering rule L1;
- first/last-definition-wins: lowering rule L2.

**Estimated load (assumed, not measured).** All derived facts together are under about 0.7M rows. Per-document derived facts add under 0.1 s cold. Repository facts are about 1.5–2.5 s cold in the good form (the sum of the measured B1/B2/B6 queries), and milliseconds per edit through deltas. Fact-program rows (about 4–5M) are where vtab versus copy matters most (M3).

---

## (e) Spec text to add or replace

**1. New section, `spec.md` §21 "Named derived facts":**

> ## 21. Named derived facts
>
> **21.1 Producers.** Every relation a rule may name is a *fact* with exactly one producer: a **fact program** (a rule-independent transducer program exporting a declared schema; base facts from tree structure), a **native producer** (reviewed engine code; base facts needing a resolver or evaluator), or a **derived fact** (a definition in the declarative relational algebra over other facts, compiled to SQL and executed in SQLite). Shared does not imply SQL: shared tree-walking analyses are fact programs; relational analyses (name-keyed joins, fixpoints over names, aggregates, anything membership-dependent) are derived facts.
>
> **21.2 Definitions.** Facts are defined in `rerail.facts/v1` documents, never in rules: engine-owned families in `engine/facts/catalog/`, pack-owned families in `packs/<pack>/facts/` under the `<pack>.` namespace. Derived facts use the closed algebra (`from`, `join`, `left_join`, `anti_join`, `union`, `group_by`/`aggregate`, `project`, stratified `recursive`, literal policy relations) with the `Predicate` AST. No SQL text, expression strings or UDFs (decision 0002). Whatever the algebra cannot state is a native producer. The generated SQL is dumped and stored with the plan.
>
> **21.3 Admission.** A fact is admitted only if (a) its name denotes a language or domain concept, never a rule, linter or finding (PR 2539 `DECLARATIVE_LINTER_SQL_ONLY_BOUNDARY.md` :105, "Process Failure" :169-186); (b) at least two admitted rules use it, or it is a catalog family in §9; (c) its estimated rows per document do not exceed the document's node count (L4); (d) it carries no rule-specific seeds or constants, which are passed as policy data or handled by the rule algebra (`reaches`).
>
> **21.4 Identity.** `fact_digest = H(canonical lowered definition ‖ output schema with key and FDs ‖ input fact digests ‖ compiler version)`. A fact program contributes its program digest; a native producer contributes its id and revision, which must change whenever its output changes. `version:` is informational. The same name with a different digest is a compile error.
>
> **21.5 Partition.** The compiler infers `document` (all inputs per-document; every join, group and recursion keyed by `doc`) or `repository`, and rejects a declaration that disagrees with it.
>
> **21.6 Materialization.** The planner chooses, per admitted plan: **table** (default for derived facts; required when there are two or more consumers, when the fact is recursive, or when it has repository partition), **inline** (one consumer, non-recursive, document partition), or **vtab** over native columnar data (base facts; M3). Each fact has one physical owner per (document or snapshot, digest) per evaluation, and is never re-expanded into several statements. Tables are narrow and typed, with atoms interned as u32; no EAV or union tables. Indexes come from consumer access paths and are recorded in the plan. Computation is demand-driven: only facts reachable from admitted rules, and per-document facts only for documents in the demanding scopes.
>
> **21.7 Cache.** Per document: `(fact_digest, tape identity[, path if read])`, with content-pure facts path-free. Repository: `(fact_digest, input generation vector)`. Content-store tables `derived_facts` and `derived_fact_parts(digest, doc_key, complete, reason, rows)`; rows cascade with their source. Completeness is per (fact, document); an incomplete input makes dependents `Unknown` for the affected (rule, file) pairs.
>
> **21.8 Invalidation.** The fact dependency DAG is derived from the definitions. An edit to document `d` recomputes `d`'s base facts and demanded per-document derived facts. Repository facts in the cone are updated by semi-naive delta over the changed join keys; recursive components use delete-and-rederive, or a full recompute when the delta exceeds the planner threshold. Only rules whose recorded read set intersects a changed (fact, key) pair re-run.
>
> **21.9 Statistics.** Each materialization records `fact_stats(digest, column, rows, ndv, max_rows_per_doc, null_frac)`. For tables these are written to `sqlite_stat1` (never `ANALYZE`). Plan decisions that read them store the keys and validity intervals in `compiled_plans`.
>
> **21.10 Consumption.** Rules name facts in the rule algebra (`from: {fact: …}`, joins, `reaches … via: {fact: …}`), and in AST rules through the semantic keys, the `fact:` semi-join predicate and capture projections. Transducer programs read facts as `SIDE` columns (key `(doc, node)` with node → value), as `FACT` sets (§6, `spec-transducer.md` §2.6), as demand seeds (E7), or SQL joins facts with `t_match`/`t_export`. Execution per document is stratified: base facts → derived facts → rule programs → composition. Plans with a cycle across the boundary are rejected.
>
> **21.11 Catalog.** The initial families and measured sizes are in `research/derived-facts.md` (the inventory in this report).

**2. Other `spec.md` edits**
- **§6 summary.** Replace "(everything decidable from one file lowers into the transducer; SQL only above the file)" with "(local, navigational conditions lower into the transducer; shared and relational intermediates are named derived facts, §21)".
- **§6 E-table.**
  - Delete E15 and E17.
  - Add **E15′ `FACT` set**: a set-stage definition loaded from a fact keyed on (doc, node) as a posting list, read by `IN_SET`/`COUNT_SET`. Used for results of joins and fixpoints over names computed as derived facts.
  - In the verifier list, delete "except for `FOLD_UP` (E17), which is well-founded by construction".
- **§7 Frontends.** Replace "Everything within one document lowers to the IR: …" with "Local constructs within one document lower to the IR: … Joins between independently matched sets and recursion through names lower to derived facts (§21; lowering rule L5)." Also replace "If a rule feature cannot lower, the IR is extended" with "…it becomes a derived fact or native producer. The IR is extended only under `spec-transducer.md` §0 rule 3".
- **§7 Fusion forms, "Capture equality across matches" row.** "Within a document: `EXISTS` + `TEXT_EQ` when one side reaches the other; otherwise L5 (candidate exports, SQL semi-join on `(doc, key)`, optional `FACT` set fed back). Across documents: SQL hash join."
- **§7 item 4.** Change "with E15–E17 and lowering rules L1–L4" to "with E16, `FACT` sets, derived facts (§21) and lowering rules L1–L5".
- **§9 relation table.** Add a row: `d_<name>` derived facts, typed per definition, partition `document`/`repository`, cached by `fact_digest` (§21). Rename the `fact_<family>` row's use to "base facts: fact programs and native producers".
- **§10 "SQL owns".** Add "named derived facts (§21)".
- **§11.** Add "Plans schedule fact strata before the rule programs that read them. `FACT`/`SIDE` inputs are part of the fused-group cache key through their `fact_digest`."
- **§14.** Replace "`fact_*` relations and cross-file graphs are maintained incrementally" with "Base and derived facts are maintained by dependency cone (§21.8)".
- **§16.** Add `engine/facts/catalog/` (definitions), `engine/facts/derived/` (algebra → SQL, cache, invalidation, statistics), and `engine/facts/programs/` (fact programs).
- **§17.** Add decision 0005, "Named derived facts", and record that it amends 0004.

**3. `spec-transducer.md` edits**
- **§0.** Keep it. Add rule 7: "Shared tree-walking analyses are named **fact programs**: cached and computed once, but still transducer programs." Add the L5 lowering to rule 2.
- **§2.6.** Add the set definition `FACT | {fact digest, optional column = const} | {y : (doc, y) ∈ f} | posting list loaded from the fact cache`.
- **§2.8.** Change "Fact columns are produced by…" to "…by native producers, fact programs, or derived facts with key (doc, node) and node → value (`spec.md` §21.10)".
- **§2.11.** Retitle to "Order against a capture (E16)". Keep only `ORDER`. Delete `KEYS`, `IN_KEYS`, `FOLD_UP` and "KEYS inside FIX", and add a note recording why they moved (§0; decision 0005).
- **§6.1 table.**
  - Capture-equality row, set-stage column: "L5 + `FACT`".
  - "All branches" row: "`SIDE_TEST` on `py.flow_exit` (native producer)".
- **§6.2.** Add the row `flow_exit | exits per statement/block | SIDE_TEST | native control-flow summary`.
- **§6.5.** Add **L5, relational split**: "Capture equality between independently matched nodes, and recursion through names, are split into candidate-exporting programs plus a derived fact or semi-join. Shared joins must use a catalog fact."
- **§7.** Delete the `KEYS`/`IN_KEYS` and `FOLD_UP` rows, and add `FACT`.
- **§9 item 4.** Delete the `FOLD_UP` exception.

**4. `spec-fusion.md`**
- **§6.6.** Replace the `KEYS` bullet with the L5 bullet.
- **§6.8.** Delete the `FOLD_UP` exception, and point to the native producer or derived fact.
- **Note at :168.** Change "Keyed sets (`KEYS`)… turned M7's quadratic join into a lookup" to "the keyed lookup in M7 is provided by L5's indexed SQL join (M10)".

**5. `authored-sql.md` §4.** Restate categories D–H as "fact programs + derived facts in `rerail.facts/v1` (`spec.md` §21)": D → `py.scope`/`py.def`; E → `py.import_res`; F → `py.subclass_closure`; G → `py.call_edge(_local)` + `reaches`; H → `py.flow_edge` + `py.local_flow`.

**6. Decision 0005 and measurements**

Decision 0005 (record): "Named derived facts. Intermediate relations shared by several rules, and relational per-document analyses, are named facts defined in `rerail.facts/v1` (algebra compiled to SQL), separate from rules, cached by `fact_digest` and invalidated by dependency cone. E15 and E17 are withdrawn from the IR. Amends 0004." Prior work to cite:
- PR 2539: the boundary doc :8-29, :67-71, :105-106, :136-143, :169-186; Pass 2 in the framework-facts doc; the dependency maps in `default_catalog.py`;
- FE: DECLARED-FACTS.md :53-54 and `facts_memo.rs` keys;
- DLRE: c4bd76bf (−74.7%) and `shared-producer-workplan.md:271`;
- ledger lessons 11, 12, 14 and 16.

Add measurements to `architecture.md`:
- **M10:** L5 against keyed sets on M7's inputs and on step-contracts. Pass if total CPU is within 1.5× and per-edit p99 is unchanged.
- **M11:** cold materialization of all derived facts in (d) on the gate corpus. Pass if it takes 2.5 s or less on one core, with plan-recorded good forms.
- **M8** gains the delta path from §21.8.