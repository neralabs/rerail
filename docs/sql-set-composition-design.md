# Set-based SQL composition for code rules

Status: experiment on branch `Moti-Velz/sql-set-composition`. The native grammar matcher is the production engine and the parity oracle.

## Goal

Lower every code-rule expression to set operations in SQLite. Native code runs only behind virtual tables, as **terminals executed once per batch of files**. It never runs once per joined row. The experiment succeeds when, on the nera-data-layer benchmark (12 policies, 5,725 files, limits lifted):

1. findings and failures are byte-identical to the native engine, and
2. whole-tree wall time is within 10% of native, on the same machine, over alternating runs.

There is one way to add an operator. A combinator is a lowering. A leaf is a native terminal behind a virtual table plus a lowering that reads it. No operator has both a native and a SQL implementation in production.

## Why the previous SQL engine was slow

`query/` in nera-landing at `1bc2d9de2` compiled each (file, check) to one statement over a one-file batch. Every term was a scalar `tq_*` function of `(file, node, env-handle)`, called per candidate:
- relations were correlated `tempo_nodes` subqueries, re-filtered per outer row;
- utilities were nested statements stepped from inside scalar functions, with one prepared copy per depth;
- patterns were matched per node;
- no join or set operation carried the load.

That design measured 1,589 s against native's 34 s for the same rules. This design inverts it. **Rows are the unit of work, and every statement processes all rows of a batch.**

## Data model

### Batch

A batch is a set of file versions evaluated together: the changed files of a workspace apply, or a slice of the whole tree in the benchmark. Each worker thread owns an in-memory SQLite connection. Statements are prepared once per policy set per connection, and the statement cache is sized to the policy set (lesson D31).

### `node` virtual table

Rows come straight from the batch's restored tapes, with no copies. One row per tape node:

| Column | Meaning |
|---|---|
| `file` | batch-local file index |
| `id` | preorder ordinal (tape node id); the root is 0 |
| `last` | `id + size - 1`; the subtree is `[id, last]` |
| `parent` | parent id (NULL for the root) |
| `kind`, `field` | raw symbol id, field id |
| `named`, `leaf`, `extra`, `missing`, `error` | flags as the matcher sees them; `named` honours contextual aliases |
| `named_ix`, `named_n` | 1-based index among the parent's named children, and their count |
| `prev`, `next` | sibling ids under the tape's byte-positioned `prev()`/`next()` rules (zero-width skip) |
| `start`, `end` | byte span |
| `shash`, `thash` | unification keys (below) |

`xBestIndex` serves:
- `(file, id)` equality and `id` ranges;
- `(file, parent)` equality;
- `(file, kind) IN (...)` from per-file kind postings;
- `ORDER BY id` as consumed.

Walk order is preorder order:
- children left to right: `id ASC`;
- descendants in preorder: `id ASC`;
- ancestors nearest first: `id DESC`;
- later siblings nearest first: `id ASC`;
- earlier siblings nearest first: `id DESC`.

**Unification keys.** `thash` = hash of the node's source text. `shash` = `thash` for a named leaf; for a composite node it is `hash(kind, shash of every child)`. Two bound nodes unify iff:
- they have the same id, or
- both are composite and their `shash` values are equal, or
- at least one is a named leaf and their `thash` values are equal.

This reproduces `does_node_match_exactly`. `$$$X` unifies over the named nodes only; it carries a list hash computed by its terminal.

### Frames

A sub-rule lowers to a function from an input frame to an output frame. A frame is a set of rows:

`(k, file, node, c_1 … c_n, unk, wh)`

- `k`: the caller row the result belongs to.
- `c_i`: the bound node id (or exported text/span pair) for every capture in the clause's static capture set; NULL means unbound.
- `unk`: the three-valued flag. It exists only in clauses the matcher runs three-valued.
- `wh`: the withheld-capture bitmask for three-valued `all`.

The native matcher keeps at most one env per root at every point, because evaluation is first-match without backtracking. Every operator here therefore maps each input row to at most one output row. That is what makes the per-row semantics exactly reproducible as set semantics: an operator is a join followed by an argmin per `k`.

A clause's root frame is the set of candidate roots:
- nodes of the clause's root kinds;
- narrowed by the anchor index when one exists;
- ids below `visits_for(len)`;
- one row each, with every capture unbound.

## Operator lowering

`R(F)` means "rule R evaluated on frame F". Lowering starts from the compiled `AstMatcher` tree, so term order, `order_by_captures`, `Never` folding and grammar gaps stay with the existing front end.

| Operator | Lowering |
|---|---|
| `kind` | filter `node.kind IN (…)` |
| `regex` | the `text_match(term)` terminal: input set = distinct (file, node) of F; returns the nodes whose text matches |
| `pattern` | the `pattern(term)` terminal: candidates = nodes of the pattern's root kinds in the batch. It emits (file, node, bound captures) under a fresh env, then a join unifies them with F's bound captures. A pattern with an ellipsis directly before a metavariable that F may already bind gets the env-input variant instead: the terminal receives F's distinct (node, bound captures) rows, because greedy ellipsis alignment depends on the env. |
| `all` | sequential pipeline over terms in compiled order. Three-valued: a term's unknown rows set `unk` and OR the term's static may-bind mask into `wh`. Later terms whose static mention mask intersects `wh` pass those rows through untouched. A false term drops the row. At the end, `unk` rows are unknown. |
| `any` | branch i runs on the F rows that no earlier branch matched (anti-join on `k`), from the incoming env; results are `UNION ALL`. Three-valued: rows where a may-bind branch is unknown stop as unknown; rows where a non-binding branch is unknown continue and carry the unknown. |
| `not` | inner rule on F with the inner rule's bindings discarded; output = F rows whose `k` has no inner match (anti-join). Inner unknown makes the row unknown. Captures F has not bound match freely inside the probe, because the inner lowering only unifies against bound columns. |
| `has` / `inside` / `precedes` / `follows` | join F with `node` on the axis predicate (child: `parent = f.node`; descendant: `id BETWEEN f.node+1 AND f.last`; parent; ancestor: `f.node BETWEEN id+1 AND last`; siblings via `prev`/`next` for neighbour, or `parent` plus an id range for end). Then run the sub-rule on the joined candidates, carrying F's env. If the sub-rule may bind: keep the walk-order-first match per `k` (argmin/argmax on `id`, using SQLite bare columns with `MIN`/`MAX` or `ROW_NUMBER`). Otherwise use `EXISTS`. |
| `stopBy: rule` | the stop rule runs in a fresh env, so it is context-free: a per-batch stop set S. `has`: a candidate is visited iff there is no s in S with `f.node < s.id < c.id ≤ s.last`. `inside`: ancestor a is visited iff no s in S is an ancestor strictly between a and the node. |
| `field` | `has`: the first child with that field (min id), then the stop mode as specified (the rule mode tests the field child and then only its children). `inside`: ancestor steps where the walked-from node is that ancestor's field-F child. |
| `nthChild` | `named_ix` / `named_n - named_ix + 1` against `An+B`. `ofRule`: the rank among the parent's named children matching `ofRule` (a window over `parent`). A capture-free `ofRule` is a per-batch set; otherwise it is evaluated on the sibling candidates with F's env. Then the node's own `ofRule` match commits its bindings. |
| `count` | relation join with no argmin, `COUNT` per `k`, bounds filter. Three-valued: count the unknown candidates as an interval. `as` binds a transformed variable. |
| `matches` (shared env) | inline the utility's lowering on F. A context-free utility (reads no captures, binds none the caller reads) is instead a **per-batch shared stage**: computed once over the nodes of its root kinds and keyed by utility digest across every clause of the batch. This replaces the per-file memo. |
| `matches` (parameterized `Call`) | build a frame holding only the params, unify the bound params with the caller's captures, run the body, and return the params the utility bound under the caller's names, with `unify_capture`'s min-range rule. |
| recursive utilities | they recurse only through relations, so the recursion is well-founded on the tree. Evaluate by depth level in a host loop: one statement per level per batch, never per node. |
| `constraints` | after the rule, for each constraint in map order, join the captured node with the constraint's lowering on that node (it may bind). Unbound and `$$$` captures pass. |
| `violation.each` | a host loop of exclusion rounds per batch. Round r re-runs the clause on the roots that produced a fresh range in round r−1, with an exclusion table; relations, `any` and `binding.occurrence` skip candidates whose binding of the enumerated name falls in an excluded range. A root stops when the name is unbound or repeats. |
| semantic leaves: `value`, `compare` (semantic modes), `layout`, `arguments`, `resolves`, `binds`, `parse`, `formatArgs`, `calls` | each is an **input-set terminal**. The virtual table receives F's distinct (file, node, read captures) tuples in one `xFilter` call via `sqlite3_vtab_in`. It evaluates them natively against per-batch fact state (scope tables, value decoders, parsed literals, line tables, call graph) and returns (row key, verdict, unknown, bindings or related nodes). `binds`/`resolves.binding` return related site/value/annotation/occurrence nodes, and their sub-rules lower as relations over them; first occurrence = min id. `compare` in `source`/`unquoted`/`node`/`tokens` modes is pure SQL over text and spans where exact. |

`select_when`, `require_each` and `budget` are SQL over per-file leaf rows and evidence rows (they were before D36; recover their SQL from `e049dfbd3`). They are implemented in `sql/select_when.rs` and `sql/crossfile.rs`, each on a per-thread in-memory connection whose tables are created once and cleared and refilled per call:
- `select_when`: leaf rows `(leaf, position, failed, partial, witnesses)`. The plan's tree shape drops out. A leaf that holds (a decision leaf with a witness) moves reading on to the next leaf in `Plan::leaves` order, and any other verdict reaches the root. One statement therefore computes each leaf's end, the first leaf that does not hold, the unknown row, the partial rows, and the witness budget as a running sum. `answer_leaves` pulls rows through a `Leaves` reader with exactly the native answer's calls, so lazy evaluation and its errors are unchanged.
- `require_each` / `budget`: each operand field is extracted natively at load time into `cf_value`. `eq`/`ne` operands use an injective canonical text that mirrors `same_text`, and missing fields are NULL compared with `IS`. Group and distinct keys use JSON text. The anti-join is a join plus `NOT IN`. The budget is a `GROUP BY` with `COUNT(DISTINCT …)`. `interrupted` is polled between statements; budget accounting is not emulated (§Budgets).

## What stays native and outside composition

The following stay native, at the boundary, and are never called per row from SQL:
- parsing and tape restore;
- terminal evaluation (patterns, regex, semantic facts);
- per-file fact construction;
- witness rendering after SQL returns `(clause, file, root, env)` rows: `transform`, syntax-error drop, suppressions, the witness cap, anchors, messages and ids.

## Budgets

With limits lifted, parity is exact. Native budgets are order- and memo-dependent: per-clause allowances, util steps, depth, and per-clause charging of file tables. Set evaluation cannot reproduce them. Before native leaves production, budgets are redefined:
- a per-(clause, batch) row budget and deadline enforced through SQLite's progress handler;
- a failing clause is retried per file to attribute the failure.

That redefinition is a product decision, recorded as a D-record. It is not an emulation.

## Execution

For each batch, the driver:
1. restores tapes and builds the `node` table state;
2. computes the shared stages: context-free utilities, stop sets and pattern terminals, keyed by digest;
3. runs one statement per clause, built from CTEs (`MATERIALIZED` where a stage is read twice);
4. runs the host loops for `each` rounds and recursion levels;
5. hands result rows to the native renderer.

Workers process batches in parallel, with the same thread count as native. Batch size is a tuning parameter.

The engine is selected at runtime (`engine: native|sql` in the evaluation options) so a single build can A/B and run differential tests. That switch is removed when native leaves production; native remains only as a `cfg(test)` oracle.

## Code layout

`native/src/code_rule_dsl/sql/`:
- `nodes.rs`: `node` virtual table;
- `plan.rs`: frame IR;
- `lower.rs`: `AstMatcher` → IR;
- `emit.rs`: IR → SQL;
- `terminals/`: one module per terminal family;
- `exec.rs`: batch driver and host loops;
- `tests/`.

## Gates

| Gate | Evidence |
|---|---|
| G0 baseline | Native whole-tree time, CPU and digest on this branch (`benchmark/data-layer`), stable across 3 runs. |
| G1 kernel | `kind`, `regex`, `pattern`, `all`/`any`/`not`, relations with every stop mode and `field`, non-recursive `matches`, `Call`, `nthChild`, `count`, `constraints`. A differential test generates random rules over fixture files in Python, TypeScript and PHP, and the two engines agree on ≥ 20,000 cases. |
| G2 data-layer parity | Semantic terminals, three-valued logic, `each`, recursion and `select_when`. Whole-tree output is byte-identical to native with limits lifted. |
| G3 speed | Whole-tree wall time ≤ 1.10 × native, alternating runs ×3, recorded with load average. |
| G4 production | Workspace and stateless paths on SQL; language conformance, pack and query-parity suites pass; budgets redefined (D-record); native only under `cfg(test)`; docs updated. |

## Known hard cases

Each gets a dedicated differential test:
- first-match binding across relations whose later terms read the bound capture;
- `any` with a later branch that would satisfy a downstream term;
- `not` over captures that are unbound at that point;
- three-valued withholding;
- zero-width siblings and Blade overlap scans;
- `has` neighbour including anonymous children;
- the field `has` rule-mode one-level quirk;
- the `each` stop condition;
- `$$$` unification over named nodes only.
