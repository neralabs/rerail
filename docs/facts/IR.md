# Rule IR (Phase 2)

Status: **v1, interface.** Types: `facts/src/ir.rs` (crate `rerail-facts`, module `rerail_facts::ir`). Entry point: `rerail_facts::compile::compile`. Owner: S1. Changes are announced to S2, S3, S5 before they land.

## 1. Crate

Phase 2 code lives in `facts/` (package `rerail-facts`), a workspace member next to `native/`.

- Not in `native/`: that crate is the shipped engine (a cdylib for the Node addon), and Phase 2 changes it by visibility only.
- Not in `spikes/`: the spike crates are deleted once the Phase 2 crate passes their gates.
- The crate starts with Spike A's leaf files that carry no evaluator: `sql/value.rs` (SQLite values, affinity, comparison), `sql/parse.rs` (the SQLite-subset parser) and `policy.rs` (`$option` resolution, scopes). They were moved by `git mv`; facts-a re-exports them, so its paths are unchanged.
- The backends move in as their slices land; until then they consume the IR from here.

## 2. Program

`compile(document, config, leaves) -> Result<Program, Unsupported>`:

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
- The leaf executor (Spike A's `ast.rs` over facts, through facts-a) compiles it. Its refusal is the construct name in `Unsupported`.
- A regex clause is `InputKind::Regex`. The front end checks its keys: `lines.state` and `lines.context`, the `content` gate and non-python terminators are refused by name.
- Rows: one per witness (with `each`, one per distinct binding), anchored as spec-rel § 1.4–1.6, fields in name order.
- Identical evidence objects within a rule share one input.

## 7. Classification

Declared:

- `relational.partitionBy` is `file` or `repository` (absent = `repository`, `$option` resolved).
- `forbid` is per file.
- A `budget` is per file iff its `group_by` contains a field projected from `file.path` and every closure stage's `partition_by` does too. Otherwise its groups or closures span files.

Verified, by construction: the classifier maps each unclassified relation (`Rel<AnySource>`) to `Rel<FileSource>` (`classify::file_source`) or `Rel<RepoSource>`.

- `FileSource` has no variant that reaches another content: `Input`, `Syntax` and `Stage` of the same plan only, plus constants (`Lit`, `Unit`, `json_each` over constants).
- A per-file plan that names any of the following is a compile error (`Refusal` "cross-file read in a per-file plan: …"):
  - `reference_edges` (relational `references`);
  - a pattern leaf with a `calls` atom (the reference graph);
  - a name closure spanning files;
  - a group spanning files.
- The leak test is `cargo test -p rerail-facts leak`. A seeded `partitionBy: file` clause reading `reference_edges` must fail to compile. A `compile_fail` doctest on `ir.rs` shows that `Rel<FileSource>` cannot hold `RepoSource::Exported`.

Repository clauses are split:

1. **File-locality.** A relation is file-local when it is the union over contents of the same relation computed from one content. Rules:
   - Inputs are file-local, keyed by `__path`.
   - Filter, project and `json_each` preserve file-locality.
   - A join of file-local relations is file-local when its `ON`/`WHERE` equates their path columns. Joining with a constant relation also preserves it.
   - `Exists`/`InRel`/`Scalar` subqueries keep it when correlated on the path column.
   - Aggregate, window, `Distinct` and set `Union` keep it when the path column is a key, partition or output column.
   - A fixpoint is file-local when init and step are and the step joins on the path.
   - `Sort`/`LIMIT` keep it only inside a path-correlated scalar subquery.
   - Stages are analysed in order.
2. **Cut.** A file-local stage, and every file-local subtree of a later stage, moves to the per-file side. Each boundary relation the repository side reads becomes an `ExportedRelation`.
   - Projection: only the columns the repository side reads.
   - Distinct when every read is set-insensitive: `DISTINCT`, set `UNION`, `EXISTS`/`IN`, `min`/`max`, `GROUP BY` keys, reachability.
   - `ordered` when a read reaches `row_number()`, `LIMIT` or `group_concat`.
3. The repository plan scans only `RepoSource::Exported` and its own stages. `source_text` in a repository plan is a cross-file read of file text; it is pushed to the per-file side with its rows, or refused.

## 8. Known order and identity caveats (carried from Phase 1)

- `row_number()` ties, `LIMIT 1` under a partial `ORDER BY`, and `group_concat` over an unordered input are resolved by input order. That order is deterministic per § 4, but it is SQLite plan order only where Spike A matched it.
- In repository plans these constructs are listed by the census: the dataflow has no input order. A construct whose order keys are not provably total is a kill-criterion case for S3. It is refused there, recorded, and narrowed with Jeremy.
- `json_each` over an object enumerates keys sorted (serde_json), not in text order.

## 9. Unsupported, named

Refused at compile time with the construct named:

- SQL outside § 4: `BETWEEN`, blob literals outside `CAST(X'..' AS TEXT)`, window functions other than `row_number()`, functions outside § `Func`, table functions other than `json_each`, `NATURAL`/`USING` joins, `EXCEPT`/`INTERSECT`, compound `ORDER BY` expressions;
- recursive CTEs mixing `UNION`/`UNION ALL` or holding `ORDER BY`/`LIMIT`/`WITH`;
- pattern rule keys the leaf executor does not implement (named by the executor);
- regex `lines.state`, `lines.context`, `content`;
- relational `references`;
- a `calls` atom in a pattern rule.

The census (`docs/facts/reports/phase-2-s1-ir.md`) lists every refusal with counts.
