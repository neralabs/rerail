# Facts engine: architecture and invariants

Thesis: **SQL in, facts underneath, one algebra in between.** SQL (and the pattern DSL) is the only rule language. Facts are the only data. One logical algebra with SQLite's semantics is the only thing the engine reasons about. Speed comes from a columnar physical algebra, and from memoization and incremental maintenance that are properties of the algebra, not of individual rules. Nothing is handled by recognizing its text, its stage name, or its author.

This document is binding for Phase 2 onward. `PHASE-2.md` is read under it; where the two disagree, this one wins.

## Pipeline

```
rule (SQL stages | pattern DSL)
  → logical algebra, SQLite semantics (I1)
  → rewrites: general, justified, tested (I4)
  → dependency typing: per-content | membership-dependent (I6)
  → physical plan over columnar facts, compiled once per rule, EXPLAIN (I5)
  → execution: per-content memoized (I7) | membership-driven incremental, delta plans generated (I8)
  → findings; refusals named (I9); admission by gate (I10)
```

## Invariants

| Id | Invariant | Forbids | Enforced by |
|---|---|---|---|
| I1 | **Semantics are SQLite's**, pinned to the version native bundles (3.53.2, rusqlite 0.40.2). Bags and sets as written, affinity (including no-affinity columns and compound-subquery column typing), three-valued NULL, ordering only where the query specifies it. Where SQLite's own result depends on its scan order (an unproven window order, `LIMIT` ties, bare `GROUP BY` columns not determined by the key and not the lone min/max row, ordered aggregates), the construct is runtime-checked: a real tie is a named, retractable refusal, never a plan-dependent guess. The reference evaluator defines truth; every physical plan yields the same multiset and the same order where specified. | "Close enough" results; tie-breaking that depends on the plan; emulating SQLite's scan order | reference evaluator; random differentials against the bundled SQLite; the corpus differential; gate |
| I2 | **Facts are relations with declared structure.** Keys, functional dependencies (`id → subtree_end`, `child → parent`, field codes), the preorder total order of node ids and its interval property (descendants of `id` are exactly `(id, subtree_end]`), cardinality bounds. Derived relations (`parameters`, `calls`, …) are SQL views over the syntactic core, compiled like any rule. | Hand-written extractor code for anything a view can state; undeclared structure that a rewrite silently relies on | `SCHEMA.md` declarations; per-content structural checks in debug; views compiled by the same compiler |
| I3 | **After the parser, only algebra.** Matcher inputs are algebra over `node`/`child`/`field`. No component past `parse` holds SQL text, stage names, input names or rule ids for any purpose but diagnostics. | Shape-recognized substitutions; kernels selected by name; per-rule fast paths | type-level: rewrites and planners take `Plan`, never strings; review |
| I4 | **Rewrites are general and justified.** Each is `(pattern on the algebra, side conditions, justification, differential test, mutant)`, registered in `REWRITES.md`. A rewrite that needs a property of a relation takes it from I2's declarations. | A rewrite that fires on a name; a side condition that is assumed instead of checked | the registry; the gate's mutant per rewrite |
| I5 | **One physical algebra, columnar, planned once.** Operators: scan with pushed predicate, key lookup, hash join, merge join on ordered keys, interval probe, semi/anti join, group, window over a declared total order, semi-naive fixpoint, memoized per-node predicate. Values are borrowed from fact columns and the source slice; no row materialization of `n.*`. Plans are compiled once per rule at pack load, deterministic, printed by `EXPLAIN`, identical for every file; the plan audit rejects inner scans. | Per-statement planning at run time; `Box<[V]>` rows; a plan that differs between files | operator set in code; `EXPLAIN` stability check; plan audit in the gate |
| I6 | **Classification is a type, not a flag.** A plan is per-content iff every leaf is a per-content relation; otherwise it is membership-dependent. `partitionBy` is checked against the inferred type, never trusted. A per-content plan physically cannot reach a membership relation: its `FileFacts` handle has none. | A flag that marks a plan per-file; a per-file plan that reads `calls`, `reference_edges`, closure facts or another content's facts | dependency analysis in the compiler; the seeded-leak test |
| I7 | **Memoization by expression digest.** Every per-content sub-plan result is addressable by `(content hash, schema version, plan digest)`; the store keeps findings, exported relations and completion words under those keys. Nothing is recomputed for a known key anywhere in the fleet. | Caches keyed by path, worktree, or rule name; results that depend on evaluation order | the store's keys; the gate's cross-worktree replay |
| I8 | **Incrementality is generated.** Membership-dependent plans receive delta plans from one generic transformation: bilinear join expansion, count-based aggregates and antijoins, semi-naive fixpoint demand-driven from targets. Membership is a per-path base, cluster bases, and worktree deltas; per-worktree state is proportional to divergence. | Hand-written delta rules in production; per-worktree state proportional to the tree; `distinct` over `member ⋈ facts` | the generator; Spike B's expansions as its test vectors; the replay's incremental ≡ from-scratch check; per-worktree state measured against divergence |
| I9 | **Exact or refuse.** Anything outside I1–I8 is `Unsupported` with the construct named. No approximation, no fallback to another engine, no "simplified" semantics. | Approximate matching; silent native fallback; narrowed semantics presented as the rule | compile-time refusal; the oracle parity numbers reported with refusals counted |
| I10 | **Admission by gate.** No rewrite, operator, generator change or schema change lands without its differential test and mutant passing. The gate is one command and runs in ≤ 10 minutes. | Merges on parity numbers alone; "we'll add the test later" | `just facts-gate --mutants` on every merge |

## What this retires from Phase 1's code

| Phase 1 mechanism | Replaced by |
|---|---|
| A3's `subst.rs`: stage shapes recognized byte-for-byte, served by kernels named `nodes`/`edges`/`ownership`/`descendants` | I3 + I4: matcher inputs as algebra over `node`; view inlining, union-of-selections and projection pruning collapse the dump chain for any clause; tree-reachability-to-interval for any fixpoint over the tree edge; key-lookup joins for any equality on a declared key. `subst.rs` stays only as a benchmark oracle until the generic path holds or beats 152.6 s, then it is deleted |
| A3's relation memo (`inside`/`has` with `stopBy: end`) | I5's memoized per-node predicate operator, for any capture-free sub-rule |
| A2's OR-branch index probes as "a fixed rule" | I4 rewrite: disjunctive equality → union of key lookups |
| Spike B's hand-written delta expansions | I8's generator; the expansions are its test vectors |
| Spike A's per-clause "bind this stage to that fact relation" | I4 rewrite: selection sharing by semijoin, admitted only under the subtree-closure side condition (the selection selects whole subtrees and the stage joins only along tree edges); otherwise the stage runs over the subset, exactly |

## What is deliberately outside the engine

- Rule shape. If a rule insists on recomputing a relation per clause over a clause-specific subset in a way no side condition admits, the engine computes it exactly and the cost is the rule's. That is a rule-authoring conversation (Jeremy), not an engine patch.
- Semantics beyond SQLite's for the constructs the rules use. New constructs enter through the IR and the registry, not through a special case.
