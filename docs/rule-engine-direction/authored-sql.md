# Authored SQL in Rerail rules (DLRE): what it is, why it exists, how to remove it

Status: proposed as decision 0002 (`decisions/README.md`).

**Decision.** A rule document carries no SQL, no code and no expression strings that are evaluated as code. Rules state which relations they want, using declarative operators over named relations. The engine decides how those relations are computed.

The `relational` clause's authored fields are frozen now and removed once the 10 rules that use them are re-expressed. Those fields are `query`, `stages[].query`, `stages[].indexes`, `stages[].deferred`, and the `rx_*` UDF surface. The executor behind them may survive only as an internal lowering target.

A computation that cannot be stated declaratively becomes an engine producer behind a relation contract. That is reviewed engine code, not rule text.

**Why it appeared.** The data-layer port bar required exact reproduction of imperative Python oracles. Authored SQL was the escape hatch that made exact reproduction possible. Under `findings-contract.md` that bar is gone. About half the authored SQL (categories A and J below) existed only to meet it.

**Why it is incompatible with the architecture.** The compiler cannot see inside authored SQL, so it cannot:

- choose a statistics-driven shape for it;
- share it across a batch;
- push it down behind a virtual table;
- bind it to the GPU.

It is also where DLRE's runtime went: 733 of 760 CPU samples in batch six were in relational evaluation.

Scope: DLRE branch `jeremybarnes/Data-layer-rule-expressivity` at `d3470da2`, as merged into the Pulling-it-all-together worktree. All paths are relative to the repo root.

## 1. Mechanisms by which a rule document can carry SQL or code

There is exactly one SQL-bearing clause kind, `relational`. Several sub-fields of it carry SQL text. No other clause kind carries SQL.

| Mechanism | What the rule author writes | Schema | Compiler | Executor |
|---|---|---|---|---|
| `relational.query` | A final `SELECT` that produces findings: path, start_byte, end_byte, identity fields, optional code | `src/engine/code-rule/code-rule-schema.ts:1497-1524` (zod); `src/engine/code-rule/code-rule.ts:189-207` (type); `CLAUSE_KINDS` at `code-rule-schema.ts:72` | `native/src/code_rule_dsl/compile/relational.rs:30` (`compile`), dispatched from `compile/mod.rs:546`; it validates every statement against empty schemas before any source is read | `native/src/code_rule_dsl/relational.rs` (`evaluate` :975, `evaluate_batch` :999, final result read :822) |
| `relational.stages[].query` | Ordered, materialized `SELECT`s, with joins, windows, `GROUP BY` and `WITH RECURSIVE` CTEs. Each stage is a temp table | same | same | `relational.rs:265` (`create_stage`), materializing as `INSERT INTO`/`CREATE TEMP TABLE … AS` |
| `relational.stages[].indexes` | The physical indexes to build on a stage | same | same | `relational.rs` / `relational/schema.rs` |
| `relational.stages[].deferred` | A map from column name to **SQL expression**, evaluated lazily through a view | same | same | `relational/deferred.rs` |
| `relational.stages[].reachability` | A declarative closure (roots, edges or syntax, batchSize, graph: directed\|forest). The edge and root relations it reads are themselves authored SQL stages | same | same | `relational/reachability.rs`, `relational/forest.rs` (native) |
| `relational.syntax` | A request for a native CST node table: columns, fields, excludeKinds, edges, ownership, owners. Declarative, no SQL. It is a native pushdown of the CST that rules had been re-encoding in SQL | same | same | `relational/syntax.rs`, `relational/syntax/packed.rs` (vtab) |
| `relational.references` | Native declared-reference edges (`reference_edges` table), with sibling-link and constructor-field conventions | same | same | `relational.rs` / inputs |
| `relational.inputs` | Named AST evidence queries with `project` maps. These are declarative and become the SQL tables | `codeRuleRelationSchema` | `compile/evidence.rs` (`compile_relation`) | `relational/inputs.rs`, `relational/insert.rs` |
| SQL dialect surface available to authors | SQLite core plus `json_each`/`json_tree`, plus the UDFs `rx_matches`, `rx_extract` and `rx_replace` | — | — | UDFs at `relational.rs:~127-190`; whitelist authorizer at `relational/authorizer.rs` |
| `$option` switches around any of the above | `{"$option": "entryPoint"\|"oracleAnalyzer", "cases": {…SQL…}}` selects different SQL per oracle entry point or analyzer mode | options machinery (`compile/options.rs`) | — | — |

Other clause kinds are declarative and carry no SQL:
- `forbid`, `require_each` (subject/evidence anti-join), `budget` (group_by + count), `select_when` (semi-join), `define.report_when`;
- the `Predicate` AST (all/any/not/eq/lt/…);
- `NameClosure` (seeds/edges/extends/partition_by);
- AST rules with `calls`/`resolves`/`binds`, regex state machines and `transform`.

The relational engine is about 7,000 lines of Rust: `relational.rs`, `relational/**` and `compile/relational.rs`. The rule JSON is produced by about 2,400 lines of Python generators in `scripts/dsl/data_layer_six/*.py` and `scripts/dsl/data_layer_relational_five.py`. These scripts write the SQL; it is not hand-written per rule.

## 2. History and stated rationale

- **Introduction.** `7dd15e1f` (2026-10-19), "Add bounded project relations and declared reference contracts", introduced the clause. Then followed:
  - `6cb8dc06`: port of batch six with bounded relational analysis;
  - `d97e1239`: batched reachability;
  - `f52817fc`: deferred SQL facts and native forest;
  - `c4bd76bf`: shared syntax facts.
  - 19 commits on the branch touch the relational engine.
- **Stated rationale.** From `docs/data-layer-batch-five.md:86-99` and `docs/data-layer-engine-gaps.md:144-158`: "Actual ports proved two further execution shapes… Relational clauses join projected source facts, group exported families, and compute recursive reachability with native reference edges." The clause was justified by the need to reproduce what the Python oracle scripts do.
- **The governing requirement.** From `docs/data-layer-rule-port.md:83`: "Model the exact state tracked by the oracle." Rules are admitted "as exact only after its diagnostics and coverage match the oracle" (`:264`). Batch five boasts: "No oracle source, expected findings, or existing AST work limit changed to obtain parity" (`batch-five.md:117`).
  - The root-cause table in batch five shows the cost of that bar. One example: "Dead-code parity had 42 extra and one missing diagnostic | Plain imports vanished through SQL null arithmetic…" (`:114`). The SQL had to reproduce the oracle's own quirks.
- **Consequence.** The oracle scripts are imperative Python analyzers: scope tracking, import resolution, class hierarchy, call graphs and taint propagation. Exact-parity porting therefore demanded a Turing-ish escape hatch, and the escape hatch became SQL. The branch then spent its performance work on SQL materialization:
  - `batch-six.md:148`: "733 of the active worker's 760 samples in relational evaluation. Of these, 716 included SQL stage materialization."
  - This is the "subset takes >20 min" path.
- **Diagnosis.** The clause is a symptom of the over-strict parity contract (the user's item 3), not of a missing declarative primitive.

## 3. Census

10 of the 60 ported data-layer candidate rules use `relational`, with 52 clauses between them. In total the clauses hold 191 evidence inputs, 431 SQL stages and about 926 KB of authored SQL text.

| Rule | relational clauses |
|---|---|
| data-layer-serverplan-state-tripwires | 32 |
| data-layer-serverplan-steps | 7 |
| data-layer-serverplan-quality | 6 |
| execution-authority-bypass, impl-direct-execution, internal-dead-code, legacy-execution-surfaces, public-api-triples, serverplan-dispatch-architecture, serverplan-target-architecture | 1 each |

Also: `fixtures/host/data-layer-port/profiling/{batched-sql,evidence,native-relations-metrics}.json` and `test/scripts/test_sql_variants.py`.

### What the SQL computes, by semantic primitive

Categories are assigned per clause; one clause can fall in several. Counts come from stage names and query features (script in this session).

| # | Primitive | Clauses (rules) | What the SQL does |
|---|---|---|---|
| A | **Output dedupe / pick-first only** | 28 (1) | 27 state-tripwire clauses have no stages at all. Their AST evidence rule does all the matching, and the SQL is just `row_number() OVER (PARTITION BY path,line,scope,terminal ORDER BY start) = 1`, which reproduces the oracle's one-finding-per-line identity. `impl-direct-execution` does the same per line. |
| B | Count / threshold per group | 11 (7) | `count(*) >= 2` per file or per project (taxonomy duplication), `HAVING count(*)=1` for unique resolution, `group_concat` for qualnames |
| C | **CST re-encoded as rows** | 15 (5) | The generators (`scripts/dsl/data_layer_six/ast_relations.py`) capture *every* node with 17 optional field captures. SQL then rebuilds `nodes`, `edges`, `fields`, `descendants`, sibling `ordinal` (via `row_number`) and `ownership` (recursive CTE). The tape already holds all of this. The later `relational.syntax` native table is a pushdown of exactly this. |
| D | Lexical scope / qualname / ownership | 13 (6) | Nearest enclosing function or class by interval containment (`p.__start<f.__start AND p.__end>=f.__end ORDER BY … LIMIT 1`); recursive qualname building (`a.<locals>.b`); module name from path via regex |
| E | Import / name resolution | 8 (5) | Tokenizing `import` text with a recursive CTE over `substr/instr`, relative-import resolution, alias maps, `resolved_names` fixpoint, re-export via `__all__` |
| F | Class hierarchy closure | 5 (3) | `base_edges` (resolving a base-class name through imports, then same module, then a global unique name); recursive `hierarchy` with cycle guard; MRO-ordered `public_methods` |
| G | Call graph / reachability | 5 (4) | Resolving call candidates to unique functions (`HAVING count(*)=1`); recursive `reaches`/`flow`; dead-code roots (exports, public methods, dynamic imports); dispatch through overrides |
| H | Value / data flow | 11 (5) | Taint propagation over expression edges, receiver token propagation, callback carriers, string-value constant folding, branch variables, self-stores |
| I | Authored allowlist literals | 6 (5) | `json_each('[[path, qualname], …]')` signed exceptions; `p.path NOT IN ('…py', …)` |
| J | Oracle-mode switching (`$option` entryPoint / oracleAnalyzer) | 50 (8) | Different SQL or inputs per oracle script entry point and analyzer flag, so that each standalone checker and the plugin path are reproduced exactly |

Raw SQL feature counts across clauses:

| Feature | Clauses (rules) |
|---|---|
| `row_number` window | 47 (9) |
| anti-join | 20 (10) |
| regex UDF | 18 (9) |
| string tokenizing | 18 (7) |
| `WITH RECURSIVE` | 17 (8) |
| interval containment | 13 (7) |
| path literal tests | 13 (8) |
| identity string building with `\|\|` | 13 (5) |
| aggregates | 12 (7) |
| JSON literal tables | 5 (4) |

## 4. Declarative replacement for each category, and whether the architecture can lower it

Shared principle: the rule states *what relation it wants*, and the engine owns *how it is computed*. Semantic facts (scope, symbols, references, hierarchy) are **engine-produced relation contracts** (architecture.md, "Relation contracts" and "Virtual tables" layers). They are computed once per file or snapshot and shared by every rule, rather than re-derived per rule in SQL. All of the following lower to SQL fragments over the contracts, or to vtab pushdown, so the compiler keeps stats-chosen shapes, batching and GPU bindings. Authored SQL defeats all three: the compiler cannot see inside it, cannot share it, and cannot re-shape it.

| Category | Declarative construct | Exists? | Lowering |
|---|---|---|---|
| A dedupe | **None in the rule.** Findings are a *set* keyed by a declared identity (`violation` identity fields, e.g. the existing `key`). Collapsing several matches on one line into one finding happens in the integration's normalization step. | Mostly: `key` exists. Delete the SQL. | Trivial: a `DISTINCT` on the identity, or done by the consumer |
| B count | `budget` (group_by, count lte/distinct); `define.report_when.count.gt`. Extend with `gte`/`min` and `scope: file\|repository` | Mostly | `GROUP BY … HAVING` |
| C CST rows | **Delete.** Use AST rules (`inside`/`has` with `stopBy: end`, `field`, `nthChild`, `precedes`/`follows`). Structural joins between captures go through engine `node`/`kids`/`ancestor` contracts. Containment is an interval test on tape extents (ast-index-spike LINTER_ARCHITECTURE.md §7 G1). | Yes | Native matcher per file, or the `node` vtab |
| D scope/qualname | An engine relation `scope(file, node, owner_fn, owner_class, top_fn, qualname)`, exposed as capture projections (`capture.X.qualname`, `capture.X.owner`) and as `inside: {owner: function}` | Partly (`syntax.ownership/owners` is the native seed) | A per-file native producer behind a relation contract |
| E name resolution | `resolves` / `binds` / `calls {graph: references}` and the declared-reference graph, backed by an engine relation `symbol_ref(file, node, target)` plus `import(module, name, target)` | Largely (declared-reference graphs, batch five) | A repository-scoped relation; incremental by dependency cone |
| F hierarchy | `NameClosure` (`extends`, `edges`, `partition_by`), or an engine `class_base` relation plus a closure operator | Partly | A recursive CTE with cycle guard, or the native forest |
| G reachability | A first-class operator, e.g. `reaches: {from: <rule>, via: calls\|references, to: <rule>}`. The native `reachability` stage already exists; it only needs to take *declarative* roots and edges instead of authored stages. | Engine yes, surface partly | A recursive CTE, or the native batched closure (`relational/reachability.rs`) |
| H data flow | The genuine residual. Either (a) an engine-owned def-use / value-flow relation with a `flows: {from, to, through}` operator, or (b) loosening the findings to site-level candidates. ast-index-spike declined this as "a resolver, not an index" for 2/99 specs (§7 G3). | No | (a) a native producer behind a contract |
| I allowlists | Policy *data*: an `allowlist:` relation from a declared (signed) artifact, joined by an `except: {allowlist, on: [path, qualname]}`. Path lists go in the existing `paths` scope. | No surface; the host has signed-allowlist verification | A plain table join |
| J oracle modes | **Delete.** Keep only `$option` cases that are product options; drop the ones that emulate an oracle script's entry point | n/a | n/a |

General composition (join two evidence relations on a captured value, union, anti-join) needs only a small closed relational algebra over named evidence relations, with typed column references and the existing `Predicate` AST. Examples: `require_each`, `select_when`, plus a `join`/`union` node. No SQL text. This is what PR 2539 landed on, and what ast-index-spike argued from the consumer side.

**PR 2539** (`pr2539/docs/design/DECLARATIVE_LINTER_SQL_ONLY_BOUNDARY.md`) took this route:
- Generic, framework-owned facts: `python_definition` (with qualname, bases and decorators), `python_function_body_call` (with resolved symbol), imports and parameters.
- Plus YAML structural operators: `from`, `join`, `left_join`, `recursive`, `group_by`, `aggregate`, `project`, `union`, named queries.
- Scalar predicates in a restricted `expr:` dialect, and signed allowlists as policy relations.
- The YAML compiles to one SQL statement per query.
- Crucially, scope and name resolution were *ingestion facts*, never per-rule SQL.

Its explicit "Process Failure" section warns against "Python prepares convenient rows, YAML queries them" and against rule-shaped relations. DLRE failed in the mirror direction: rules re-derive generic semantic facts in SQL, once per rule.

**ast-index-spike** (`LINTER_ARCHITECTURE.md` §5, §7, §8, §10):
- 77 of 99 rules are single-file syntax questions.
- Anti-join (16 specs), counting (24) and closure are written by the **caller** around structural fragments.
- The real gaps are containment-at-depth (expose extents), child ordinal (project the ordinal onto captures) and data flow (declined).
- In this architecture the "caller" is the engine compiler composing declarative operators, not a SQL string inside the rule.

**Update (decision 0005).** Categories D–H are now fact programs plus named derived facts in `rerail.facts/v1` (`spec.md` §21): D → `py.scope`/`py.def`; E → `py.import_res`; F → `py.subclass_closure`; G → `py.call_edge(_local)` + `reaches`; H → `py.flow_edge` + `py.local_flow`. Category C (tree copies) is deleted outright; see `research/heavy-inputs.md` §1 for why the generators produced it.

## 5. Recommendation

1. **Freeze, then remove, `relational` as an authoring surface.**
   - Freeze: no new rules may use it, and the schema marks `query`, `stages[].query` and `deferred` as deprecated.
   - Keep the executor only as an internal lowering target while the rules are re-expressed.
   - Then delete `query`, `stages` and `deferred` from the schema. Delete the `rx_*` UDF surface and the authorizer once nothing authored reaches them.
2. **Replace the parity contract first (item 3).**
   - Categories A and J (28 + 50 clauses) exist only because of exact oracle identity and mode reproduction. Under a set-of-findings contract with integration-side normalization, A disappears immediately: 27 of the 52 clauses become plain `forbid` clauses with an identity key.
3. **Promote the generic parts to engine relation contracts.**
   - Most of the native work already exists: `syntax`, `ownership`/`owners`, `reference_edges`, `reachability`/forest.
   - The new contracts are `node`/`kids` (C), `scope` (D), `symbol_ref`/`import` (E), `class_base` (F) and the reachability operator (G).
   - Each is computed once per file or snapshot, shared across rules, and bound to SQL or a vtab under the architecture's two-binding rule.
4. **Add the minimal declarative surface:**
   - capture projections for scope and qualname;
   - `reaches`;
   - `budget` extensions;
   - an `except`-allowlist relation;
   - a small join/union node over named evidence relations, with the `Predicate` AST.
5. **Re-express the 10 rules** by rewriting the generators (`scripts/dsl/data_layer_six/*.py`), not the JSON by hand. Measure each against the loosened findings contract.
6. **Residual: category H, data flow** (11 clauses, in state-flow, steps and dispatch). Either build a scoped engine value-flow relation behind a contract, or report candidate sites with looser semantics and accept the recall difference. Decide this per rule with numbers.
   - It is not an argument for keeping authored SQL. A rule that truly needs arbitrary computation belongs in a native producer behind a relation contract, which is reviewed engine code, not rule text.

**Risks:**
- Engine-owned semantic facts become a schema that needs versioning. This is PR 2539's lesson: keep them generic, and suspect any relation named after a rule.
- The loose contract can hide real regressions unless the normalization step is explicit and itself tested.
- Data-flow rules may lose recall.
- Removing `$option` oracle modes changes which entry point a finding is attributed to; the integration must accept that.
