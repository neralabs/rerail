# Rule IR (Phase 2)

Status: **v1, interface.** Types: `facts/src/ir.rs` (crate `rerail-facts`, module `rerail_facts::ir`). Entry point: `rerail_facts::compile::compile`. Owner: S1. Changes are announced to S2, S3, S5 before they land.

## 1. Crate

Phase 2 code lives in `facts/` (package `rerail-facts`), a workspace member next to `native/`.

- Not in `native/`: that crate is the shipped engine (a cdylib for the Node addon), and Phase 2 changes it by visibility only.
- Not in `spikes/`: the spike crates are deleted once the Phase 2 crate passes their gates.
- The crate starts with Spike A's leaf files that carry no evaluator: `sql/value.rs` (SQLite values, affinity, comparison), `sql/parse.rs` (the SQLite-subset parser) and `policy.rs` (`$option` resolution, scopes). They were moved by `git mv`; facts-a re-exports them, so its paths are unchanged.
- The backends move in as their slices land; until then they consume the IR from here.

## 2. Program

`compile(document, config, leaves: &mut dyn LeafCheck) -> Result<Program, Unsupported>` (and `compile_report`, which keeps the clauses that compiled and lists every refusal):

- `document` is a policy (`rerail.policy/v1`); `config` holds the option values (`define.config`, as `replay.ts` writes them).
- `leaves` is the pattern leaf executor's compile check (today Spike A's `ast.rs`, via facts-a).
- The result is a `Program`, or every refused construct of the rule, each naming the construct and where it occurs.

| field | meaning |
|---|---|
| `inputs: Vec<InputDef>` | leaf relations evaluated over one content: pattern queries and regex clauses, with projections, anchor, `each`, `paths` |
| `per_file: Vec<Plan<FileSource>>` | plans that read one content's facts. A clause declared `partitionBy: file`, every `forbid`, a `budget` whose groups and closures are per path, and the per-file side of every repository clause (its `Sink::Export`) |
| `repository: Vec<Plan<RepoSource>>` | plans that read only exported relations (and their own stages) |
| `exported: Vec<ExportedRelation>` | the relations between them: per content, produced by `per_file[producer]`. Typed columns; `distinct` when the repository plan reads them as a set; `ordered` when it observes their row order |
| `report_when` | `define.report_when.count.gt`: a repository-level gate on the rule's finding count |
| `runtime_refusals` | constructs that compile but that the leaf executor refuses per content at run time (failure for that content, never an approximation) |

A `Plan<S>` holds:

- `syntax`: the clause's `syntax` spec;
- `stages`: in order;
- `sink`: `Witnesses` (forbid), `Budget`, `Findings` (relational final query) or `Export` (the per-file side of a repository clause).

## 3. Values and columns

- Values are SQLite's: NULL, INTEGER (`i64`), REAL (`f64`), TEXT. In the IR, literals are `ir::Lit`. Executors hold `sql::value::V`, which is `Rc`-based.
- A compiled `Program` is immutable and `Send + Sync`: one copy is shared by every worker through an `Arc`. No per-worker copies of constants, regexes or indexes.
- A column has a name and an affinity: TEXT, NUMERIC, INTEGER, REAL or none.
- Input tables: `__path TEXT, __start INTEGER, __end INTEGER`, then projected fields in name order, with no affinity.
- Syntax tables: as spec-rel § 2.3, `__path TEXT, __start/__end INTEGER`, the rest none.
- Stages: CTAS's expression-affinity rule. A column reference keeps its column's affinity, `CAST(x AS T)` has T's, a scalar subquery has its first column's, anything else none. Deferred columns have none.
- Compound selects: the left-most arm's names and affinities.
- Reachability stages: `(__path TEXT, root, node)`.
- Comparisons carry their operands' comparison affinities, resolved at compile time: `Expr::Cmp(op, l, r, (aff_l, aff_r))`, `InList`, `InRel`, and `Case` with an operand (per `WHEN`). Affinity application and ordering are `value.rs`'s (`comparison_affinity`, `compare`, `cmp`). Unary `+` keeps the value and drops the affinity (`Expr::Plus`).

## 4. Relations

Every operator is a bag of rows with a defined order. Schemas:

| operator | output | semantics |
|---|---|---|
| `Scan { src, schema }` | the source's | source order (below) |
| `Local { id }` | the binding's | a `With` binding; inside a fixpoint's step, the previous level |
| `Unit` | one empty row | `SELECT` without `FROM` |
| `JsonEach { input, arg, on, outer }` | input ++ `key, value, type, atom, id, parent, fullkey, path` | per input row, the elements of `json_each(arg)` satisfying `on`; array elements in order, object members in key order (serde_json, sorted; SQLite uses text order, see § 8); `outer` keeps an input row with no element once, NULL-extended |
| `Join { kind, left, right, on }` | left ++ right | nested loop: for each left row in order, matching right rows in order; `Left` NULL-extends a left row with no match. Equality conjuncts are a backend's index probes, never a reorder |
| `Filter { pred }` | input | rows where `pred` is true (NULL and false drop) |
| `Project { exprs, schema }` | `schema` | per row |
| `Aggregate { keys, aggs }` | representative input row ++ aggs | groups in first-seen order of `keys` (grouping equality: `value::dkey`; NULLs group together); no keys and no rows: one group with an all-NULL representative row. Representative row: the row a lone `min`/`max` picked, else the group's last row (SQLite bare columns). `HAVING` and the select list are a `Filter`/`Project` over this. Aggregates: `count`, `count(*)`, `min`, `max`, `sum`, `total`, `group_concat` and `json_group_array` (both in input order), each optionally `DISTINCT` |
| `Window { funcs }` | input ++ one column per func | rows stay in input order. `row_number()` numbers in the total order (partition values, order keys, **input position**). `min(x)`/`max(x) OVER (PARTITION BY …)` without `ORDER BY` give the whole partition's extremum; with `ORDER BY` (a running frame) they are `Unsupported` |
| `Distinct { prefix }` | input | first occurrence of each distinct prefix (`ORDER BY` keys ride behind the prefix) |
| `Union { all, left, right }` | left's schema | `all`: concatenation; set: first occurrences of the concatenation |
| `Sort { keys, limit, offset }` | input | stable sort by input columns (`value::cmp`, NULLs first; `desc` reverses), then `OFFSET`, `LIMIT` (constant; negative = none) |
| `With { bindings, body }` | body's | bindings evaluated once, in order |

Subqueries are expressions:

- `Exists` and `InRel` are the semijoin and the antijoin. `InRel` follows SQLite's NULL rules: NULL on the left, or no match with a NULL on the right, yields NULL.
- `Scalar` is the first column of the first row, or NULL.
- `Expr::Col { depth: d }` with `d > 0` reads an enclosing row. Depth 1 is the row of the operator whose expression holds the subquery. That is correlation.

A correlated scalar subquery with `ORDER BY … LIMIT 1` is `Scalar(Sort { …, limit: 1 })`.

### Lowering of a `SELECT`

1. FROM items fold left in text order. A table, subquery or CTE becomes `Join`; `json_each(...)` becomes `JsonEach` over the items before it (lateral when its argument reads them).
2. An inner item's `ON` goes into its `Join.on`, and a `LEFT JOIN`'s `ON` into its join.
3. `WHERE` is one `Filter` above the joins.
4. Aggregates or `GROUP BY` produce `Aggregate`, with `HAVING` a `Filter` over it. Windows produce `Window`.
5. The select list becomes a `Project`, with the `ORDER BY` keys appended behind it.
6. `DISTINCT` becomes `Distinct { prefix: select-list width }`.
7. `ORDER BY`/`LIMIT` become `Sort`, then a `Project` that drops the keys.
8. A compound uses `Union` arms; its `ORDER BY` resolves against output columns.

Row order is therefore the order Spike A's executor produces. That is the order the 9 exact batch-6 rules were measured against.

## 5. Recursion

`Binding::Fix { init, step, set }`:

- The result starts with the rows of the `init` arms.
- Each round evaluates every `step` arm with `Local(id)` bound to the **previous round's new rows** (level by level). Its rows are admitted in order.
- `set` (`UNION`): a row already in the result is not admitted. Termination: rows are drawn from a finite domain (values of the inputs and finitely many function results), so the result is finite.
- Bag (`UNION ALL`): every derived row is admitted. Termination: the derivation chains are finite (the step's graph over the inputs is acyclic, or the step strictly consumes a bounded quantity such as a string suffix). A cycle does not terminate. The backends cap a fixpoint at 50 M rows and fail the clause (native fails too, on its work budget); that is a failure, never a truncated answer.
- Mixing `UNION` and `UNION ALL` between arms of one recursive CTE, or `ORDER BY`/`LIMIT`/`WITH` inside it, is `Unsupported`.

Reachability stages (`StageDef::Reach`) are reflexive transitive closures per `__path`, with set semantics:

- `Directed` and `Forest`: over the edge query. A forest with two parents for a node, or with a cycle, fails the clause.
- `Syntax`: descendants over the `syntax` node table. It is an id-range test on facts (`subtree_end`), restricted to visible rows.

## 6. Inputs and the pattern front end

- A pattern query (`forbid.ast`, `evidence.query.ast`) is a leaf relation, `InputKind::Pattern`.
- Its JSON is resolved for the configuration, with `when` evaluated and removed: a clause whose `when` fails is not compiled.
- The leaf executor (Spike A's `ast.rs` over facts, through facts-a's `leaf.rs`) compiles it. Its refusal is the construct name in `Unsupported`.
- Rewrite R1 runs before that check: `file: { path: C }` becomes `all: [{ filePath: C }, rule]` when the query sets `syntax`.
- A regex clause is `InputKind::Regex`.
  - Native keys are accepted: `source`, `pattern`, `include`, `exclude`, `paths`, `content{include, exclude}`, `lines{include, exclude, first, terminators, state, indentation}`.
  - The leaf executor (`LeafCheck::regex`) refuses what it does not implement, by name.
- Rows: one per witness (with `each`, one per distinct binding), anchored as spec-rel § 1.4–1.6, fields in name order.
- Identical evidence objects within a rule share one input.

## 7. Classification (`facts/src/classify.rs`)

Classification is a type, inferred from the leaves (INVARIANTS I6). The declaration is checked against it, never trusted:

- `relational.partitionBy`: `file` or `repository` (absent = `repository`, `$option` resolved).
- `forbid`: implicitly per file. A cross-file leaf in one is a refusal.
- `budget`: per file iff its `group_by` contains a field projected from `file.path` and every closure stage's `partition_by` does too. Otherwise it is a repository plan over an exported copy of its input rows.

**Per-file plans are typed `Plan<FileSource>`.** Every source enters through one function, `classify::file_source` (mutant 7), whose `FileSource` has no variant that reaches another content: `Input`, `Syntax` and `Stage` of the same plan only, plus constants (`Lit`, `Unit`, `json_each` over constants).

A per-file plan that names any of the following is a compile error, refusal "cross-file read in a per-file plan: …":

- `reference_edges` (relational `references`);
- a pattern leaf with a `calls` atom (the reference graph);
- a `file.module`/`file.package` projection (package markers of other contents).

The leak test is `cargo test -p rerail-facts leak`: a seeded `partitionBy: file` clause reading `reference_edges` must fail to compile. A `compile_fail` doctest on `ir.rs` shows that `Rel<FileSource>` cannot hold `RepoSource::Exported`.

**Repository clauses are split by file-locality** (`classify::Loc`: `Const` | `Local(path columns)` | `Bound(path columns)` inside a correlated subquery | `Global`):

1. **Leaves.**
   - Inputs and syntax tables are `Local([0])`; cross-file inputs and `reference_edges` are `Global`.
   - A Filter/Join/JsonEach block is `Local` when every per-content leaf is connected by path equalities (union-find over `ON`/`WHERE` equalities). It is `Bound` when connected to the enclosing row's path.
   - Subqueries must be `Const` or `Bound`.
   - `Project` keeps the path columns it passes through.
   - `Aggregate` and `Window` stay `Local` only with a path key or partition; `Distinct` only with a path column in its prefix; a set `Union` only with a common path column.
   - `Sort` stays `Local` only without `LIMIT`/`OFFSET`.
   - A fixpoint is `Local` when init and step are.
   - A reachability stage is `Local` when its roots and edges are.
2. **Cut (rewrite R3).** Every maximal `Local` subtree with no free outer reference, whose `Local` bindings are all bound inside it, becomes an `ExportedRelation`, defined per content by that subtree (`Rel<FileSource>`, produced by a per-file plan with `Sink::Export`).
   - Projection (R4): only the columns the repository side reads. The repository side re-widens it with `NULL` literals, so its column indices do not change.
   - Distinct (R5): when every reader above is set-insensitive.
   - `ordered`: when a reader observes row order.
   - Per-file stages no export reads are pruned.
3. **Repository side.** It scans only `RepoSource::Exported` and its own stages. A per-file leaf left inside a correlation is refused, and so is `source_text`.
4. **Order audit.** Repository plans have no input order, so these are refused unless candidate keys prove a total order:
   - `row_number()` whose `PARTITION BY` + `ORDER BY` columns contain no key;
   - `LIMIT`/`OFFSET` under an `ORDER BY` that is not a key, or without one;
   - `group_concat`/`json_group_array` not fed by a total sort;
   - aggregate bare columns;
   - bag (`UNION ALL`) recursion.

   Candidate keys come from exports read as sets, `DISTINCT`, set `UNION`, `GROUP BY` keys, `row_number` partitions with the row number, reachability, and inner joins (key ∪ key). Per-file plans keep input-order tie-breaking: PHASE-2 accepts it, and it is deterministic per § 4.

## 7a. Invariant notes (for `docs/facts/INVARIANTS.md`)

| Invariant | How the IR and compiler hold it | Where it is checked |
|---|---|---|
| I1 SQLite semantics | One algebra with SQLite's values, affinities and NULL rules. Row order is defined only where SQL observes it (§ 4), and observed order without a total order is refused in repository plans (§ 7.4) | S5's reference evaluator over this IR; `Expr::Cmp` carries resolved affinities |
| I2 declared structure | Planned: relation metadata from SCHEMA.md § 1.2.1 (keys, FDs, the preorder interval, cardinalities) attached to sources. Key inference already runs over the algebra (`classify::KeyEnv`) | `docs/facts/reports/phase-2-s1-ir.md` § 7 |
| I3 only algebra after the parser | `front::sql` is the only consumer of SQL text. `Plan`, `Rel` and `Expr` hold no SQL, stage names or rule ids except in diagnostics (`Stage::name`, `Plan::clause`). Pattern leaves are resolved DSL queries until the pattern front end lowers them | types |
| I4 rewrites registered | `docs/facts/REWRITES.md`: each row has side conditions checked in code, a test and a mutant | `cargo test -p rerail-facts`; `FACTS_MUTANT=<n>` |
| I5 planned once, `EXPLAIN` | `compile` runs once per rule and configuration. `explain::explain(&Program)` prints stable text; `phys::{explain, audit}` print and audit S2's physical nodes, refusing inner scans | `facts-a explain`; `cargo test -p rerail-facts audit` |
| I6 classification is a type | `Rel<FileSource>` vs `Rel<RepoSource>`, the declaration checked | leak tests, mutant 7 |
| I9 exact or refuse | Every unsupported construct is a `Refusal { clause, at, construct }`. `compile` returns all of them; `compile_report` keeps the clauses that compiled | census |

## 8. Known order and identity caveats (carried from Phase 1)

- `row_number()` ties, `LIMIT 1` under a partial `ORDER BY`, and `group_concat` over an unordered input are resolved by input order. That order is deterministic per § 4, but it is SQLite plan order only where Spike A matched it.
- In repository plans these constructs are listed by the census: the dataflow has no input order. A construct whose order keys are not provably total is a kill-criterion case for S3. It is refused there, recorded, and narrowed with Jeremy.
- `json_each` over an object enumerates keys sorted (serde_json), not in text order.

## 9. Unsupported, named

Refused at compile time with the construct named:

- SQL outside § 4: `BETWEEN`, blob literals outside `CAST(X'..' AS TEXT)`, window functions other than `row_number()`, functions outside § `Func`, table functions other than `json_each`, `NATURAL`/`USING` joins, `EXCEPT`/`INTERSECT`, compound `ORDER BY` expressions;
- recursive CTEs mixing `UNION`/`UNION ALL` or holding `ORDER BY`/`LIMIT`/`WITH`;
- pattern rule keys the leaf executor does not implement (named by the executor);
- regex leaf features the executor lacks (`content`, `lines.include`/`exclude`/`state`/`indentation`, `lines.terminators: lf`);
- relational `references` in a per-file plan, or in any plan until `reference_edges` has a definition over exported facts;
- a `calls` atom in a pattern rule;
- order-dependent constructs and bag recursion in repository plans (§ 7.4).

The census (`docs/facts/reports/phase-2-s1-ir.md`) lists every refusal with counts.
