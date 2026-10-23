# Phase 3: memory model, scaling laws, store and daemon

Status: open 2026-10-23. Read under `INVARIANTS.md` and `LOOP.md`. Base: `Moti-Velz/facts-engine` at `c7f41a328` (Phase 2 closed; gate 29 checks, 41 mutants; loop T0 3 s, T1 ~1 min).

Three slices run in parallel. They touch disjoint code; their shared contract is the memo key and the count vocabulary already on integration.

## P3a: the cross-file memory model

Question: with the real repository plans, full incremental maintenance holds 146M arranged records at load (30 GB) and 350M after 1,000 edits (42–71 GB) for 40 worktrees. The target is 250 MB. Which model gets there, at what per-edit latency?

Two candidates, measured on the same replay (S3's 1,111 events, 40 worktrees, the 5 running plans, `facts-b greplay`):

- **M1, full maintenance, compacted.** Keep differential dataflow for every stage; prune: one presence arrangement instead of two; share identical stages across plans; the linker as shared base state; interned strings replaced by u32 ids; compact records (u32 columns, no per-record `Vec`); compaction of traces behind the frontier; eviction of exported rows no tag holds.
- **M2, hybrid.** Maintain incrementally only the shared, compact, membership-dependent relations: per-content exports, `module_of`, the reference graph (`reference_edges` as a base table maintained by the generator). Rule logic (the stages of each repository clause) is recomputed per edit on the columnar per-file engine from those relations, scoped by the delta where the plan allows (demand from the changed keys, as a registered rewrite with side conditions) and from scratch otherwise. Per-worktree state = membership + the maintained relations' deltas + findings.

Both must be exact: incremental ≡ from-scratch on the replay (the second-dataflow check for M1; the from-scratch run of the same clause for M2), 0 differences, runtime refusals counted.

Gates:

| Id | Gate | Target |
|---|---|---|
| G3a.1 | Resident after loading 40 worktrees with the 5 plans | ≤ 250 MB; report both models and the attribution |
| G3a.2 | Resident after 1,000 edits | ≤ 300 MB, no unbounded growth |
| G3a.3 | Per-edit latency | p95 ≤ 500 ms, p99 ≤ 2 s (M2's recompute is allowed to cost more than M1's 62 ms) |
| G3a.4 | Exactness | 0 differences on the replay; runtime refusals = 0 with the narrowed policies |

Kill: if neither model reaches G3a.1, report the floor of each with its attribution and the smallest set of relations that must be membership-maintained for exactness; the decider chooses the budget or the design change.

## P3b: scaling laws as the budget model

Question: replace per-file budgets (`maxVisitedNodes`, native's work allowances, closure step limits) with cost bounds that are functions of input size, enforced at compile time and checked empirically.

- **Operator bounds in the IR:** each physical operator declares its cost as a function of its inputs' declared cardinalities (scan O(n), key lookup O(1), interval probe O(out), hash join O(n+m+out), semi-naive fixpoint O(derivations), memoised predicate O(nodes), sort O(n log n)); unbounded shapes (a join without a key, a cross product, a fixpoint without a decreasing measure) are named.
- **Plan class:** composed per plan from the operators and I2's cardinality bounds (nodes, children, fields; declared keys and FDs): O(nodes), O(nodes · depth), O(n²), unbounded. Printed by `EXPLAIN`; a per-content plan declares (or the compiler infers) its class, and an unbounded class is a compile-time refusal unless the rule declares `class: unbounded` explicitly.
- **Empirical exponent check:** a gate lane fits the measured algebraic counts (S10's `counts`) against file size (nodes) on size-stratified files and fails a plan whose fitted exponent exceeds its class by more than the tolerance stated in the registry; it would have caught the quadratic utility memo (1,616 s), the quadratic `parameters` join (149 s) and the exponential DAG walk (3 h) on 200 files.
- **Budgets in algebraic units:** the remaining kill switch is a limit on algebraic work per content (not time, not native steps), identical in the reference and the engine, so a budget verdict is a fact about `(content, plan digest)` and memoisable; native's step emulation retires from the facts path. Native budget semantics stay in native.

Gates:

| Id | Gate | Target |
|---|---|---|
| G3b.1 | Every plan of the 60 rules has a class | printed by `EXPLAIN`; refusals named with the operator |
| G3b.2 | Exponent check | passes on the current engine for every per-content plan; detects each of the three historical regressions when re-seeded as mutants |
| G3b.3 | Algebraic budgets | the 8 budget workspace tests restated in algebraic units; budget verdicts memoised per content; parity on batch 6 unchanged |

## P3c: store and daemon

Question: the production path. One daemon, per-worktree state = membership + deltas + findings, hash-first edit protocol, memo in the content store, findings as epoch-stamped deltas.

- **Store:** S10's memo key `(content hash, plan digest, schema version[, path])` moves into the content store (`native/src/code_rule_dsl/content_store`), with completion words; path-dependent work split from content-pure work so identical content at two paths is evaluated once for the content-pure part.
- **Daemon:** one process for all worktrees; membership per worktree (path → hash) with the per-path base and cluster bases from Spike B; an edit arrives as `(worktree, path, hash, bytes?)` over a Unix socket (bytes only when the hash is unknown); Git ODB for bulk sync; per-file findings returned synchronously; cross-file findings pushed with the worktree's membership epoch; results delivered as deltas (introduced / removed).
- **Memory:** resident = store page cache (capped) + membership + the cross-file model's state (from P3a, whichever wins) + workers in flight; eviction of facts no worktree holds.

Gates:

| Id | Gate | Target |
|---|---|---|
| G3c.1 | End to end on the memory47 harness, 40 worktrees, the 60 rules | per-file findings ≤ 100 ms p95 after an edit; new worktree ≤ 1 s; unchanged re-apply 0 work |
| G3c.2 | Resident, 40 worktrees, steady state | < 1 GiB total (with P3a's model) |
| G3c.3 | Exactness | findings equal native's on every worktree of the harness, per-file; cross-file equal to the from-scratch run |

## Rules

- Each slice: own nested worktree under the integration worktree, branch `Moti-Velz/facts-engine-p3<x>`, report `docs/facts/reports/phase-3-<slice>.md`, `LOOP.md` tiers (T0 before commits, T1 at milestones, no full gate, counts not wall-clock), commits prefixed `p3a:`/`p3b:`/`p3c:`, never push.
- Ownership: P3a owns `spikes/facts-b/src/*` and the cross-file half of `facts/src/classify.rs`; P3b owns `facts/src/{ir,phys,rewrite}.rs`, `spikes/facts-a/src/counts.rs` and the gate's new lane; P3c owns the store, a new daemon crate, membership tooling, `conformance/` harness wiring. Shared: the memo key (fixed), the count vocabulary (fixed), `REWRITES.md` (append only, mutant numbers: P3a even from 50, P3b odd from 51, P3c from 100).
- Decider merges with the gate, refreshes the counts baseline, bumps `memo::ENGINE_VERSION` when operator files changed.
