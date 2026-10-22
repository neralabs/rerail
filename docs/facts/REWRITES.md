# Rewrite registry (INVARIANTS I4)

Owner: S1. The logical side lives in `rerail-facts`; S2 implements the physical side.

Every rewrite takes the algebra (`ir::Rel`, or a resolved pattern query before it becomes a leaf), never SQL text or a name. A row lands here with:

- its side conditions, checked in code;
- its justification;
- its differential test;
- its mutant.

Mutants are `crate::mutant::on(n)` at the rewrite's site, behind the `mutants` feature. Run one with `FACTS_MUTANT=<n> cargo test -p rerail-facts --features mutants --lib`. The gate parses the `mutant` column, cell format `<n> <kind>`.

Numbers:

- 1–3: S2's interim A3 views;
- 4–6: S3's delta generator;
- 7: classifier;
- 8: planner audit;
- 11 upward: rewrites.

## Implemented

| id | rewrite | pattern | side conditions (checked) | justification | differential test | mutant |
|---|---|---|---|---|---|---|
| R1 | file-path gate | pattern query `ast.file: { path: C }` → rule `all: [{ filePath: C }, rule]` (`front/rule.rs::file_path_gate`) | `file` holds `path` only (module keys read other contents); the query sets `syntax` (without it native reports partial-source failures even for gated-out files) | `file.path` and `filePath` are the same `TextCondition::holds(file.path)` (`native language.rs:896-912`): unanchored `regex` must match, `notRegex` must not. It is constant over a file's nodes and binds nothing, so each root's verdict is the gate ∧ the rule | `classify::tests::rewrite_file_path_gate`; on the corpus the gate's `a-replay`/`a-repo` once the leaf executor runs the rewritten rules | 11 dropped side condition |
| R2 | deferred-column inlining | stage `deferred: { c: e }` → `Project` over the stage query: `c` replaced in place or appended, `e` rebased from stored to query positions (`front/rule.rs`, `rebase_cols`) | `e` reads stored columns only, with no subquery (`front::sql::lower_scalar` refuses one) | Deferred expressions are deterministic functions of the stored row, so computing them on read or on write gives the same relation; laziness is physical (S2) | `classify::tests::rewrite_deferred_reads_stored_columns`; parity through `a-replay`/`a-repo` (batch 6 uses `deferred` in 22 stages) | 12 wrong column map |
| R3 | file-locality cut | a repository clause's maximal file-local subtree (`classify::Loc::Local`) → `Scan(Exported e)`, with `e` defined per content by that subtree (`classify::Cut::rel`) | Every leaf is per content. The leaves are connected by path equalities (union-find over `ON`/`WHERE` equalities). No free outer reference. Every `Local` read is bound inside. Subqueries are bound to the row's content or constant. Aggregate, window, `DISTINCT` and set `UNION` keep a path column as key, partition or output | The union over contents of the per-content relation equals the relation over the union of the contents, because no operator in the subtree combines rows of two contents | `classify::tests::{repository_clause_reads_only_exports, rewrite_cut_requires_path_equality}`; the gate's `b-exactness` with real plans (S3) against S5's reference | 15 dropped side condition |
| R4 | export projection | an exported subtree → `Project(read columns)` per content, and on the repository side a re-widening `Project` (unread columns are `NULL` literals) | Demand analysis: a column is read when a parent expression, a join or filter predicate, a sort key, a group key, an aggregate argument, a `DISTINCT` prefix or a set `UNION` reads it, or a correlated subquery reads it through `depth ≥ 1` | Unread columns never reach an output or a predicate, so their values are irrelevant | `classify::tests::rewrite_projection_keeps_read_columns`; S3/S5 repository replay | 13 dropped side condition |
| R5 | export distinct | an exported subtree → `Distinct` per content when every reader is set-insensitive | Above the export, before any multiplicity-reading operator, there is a set boundary: `DISTINCT` (over the read prefix), set `UNION`, `EXISTS`/`IN`, `min`/`max`-only aggregates, `count(DISTINCT)`, reachability, or a set fixpoint. Only filter, project, join, `json_each` and `UNION ALL` lie in between. No `count`/`sum`/`group_concat`, window, `LIMIT` or scalar subquery | Set boundaries absorb multiplicity; the transparent operators commute with duplicate elimination | `classify::tests::rewrite_distinct_only_under_set_readers`; S3/S5 repository replay | 14 dropped side condition |

## Planned (not implemented; no mutant yet)

From the decider's list (re-steer). Each gets an R-number, side conditions from I2's fact metadata, a test and a mutant when it lands.

| rewrite | pattern | side condition it needs |
|---|---|---|
| view inlining | `Scan(Stage s)` read once → `s`'s relation in place | `s` read once, or by digest equality (CSE) |
| selection pushdown | `Filter(p)` over `Join`/`Union`/`Project` → into the input that binds `p`'s columns | `p` deterministic; not across a `LEFT JOIN`'s null-extended side |
| union-of-selections fusion | `Union all` of `Filter(pᵢ, Scan R)` → one scan with a per-row arm tag | arms over the same relation; arm order kept as row order |
| CSE by expression digest across the pack | equal `Rel` digests (Merkle over stage and input digests; `census` computes them) → one shared relation | equal digests and equal inputs |
| tree-reachability-to-interval | a recursive fixpoint over the tree edge (`child → parent`) → descendants = `(id, subtree_end]` | I2: preorder ids, the `subtree_end` functional dependency, the edge relation is the syntactic tree (or a filter of it closed under subtrees) |
| disjunctive-equality-to-union-of-lookups | `ON a = x OR b = y` → union of key lookups, deduplicated, in table order (A3's OR probes, generalised) | each disjunct an equality on a key of the probed relation; result order = table order |
| selection sharing by semijoin under subtree closure | a clause's gated node set ⋉ a shared all-nodes relation | the selection selects whole subtrees and the stage joins only along tree edges |
