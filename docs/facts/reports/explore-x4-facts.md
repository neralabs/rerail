# Exploration X4: design (d), maintain only declared facts, recompute all rule logic

Status: **killed** at the first number. The unscoped per-edit p95 is above 5 s, and the demand rewrite admits none of the 5 plans. Branch `Moti-Velz/facts-engine-x4-facts` (worktree `.worktrees/x4-facts`, base `eee7d6ac3`). Runs are under `~/rerail-bench/facts/phase-3/x4-facts/runs/`. Each run uses 40 worktrees, the 5 running plans (`specs/spec-replay-all5.json`: the w6 exports with this worktree's rule files) and S3's `seed-5eedb` replay. All runs were unlocked, with load average 21–80 on 18 cores, so counts come first.

## Decision-table row

| Design | Memory at load / after 1,000 edits | Per-edit latency (single-file) | Exactness | What it breaks | Kill point |
|---|---|---|---|---|---|
| (d) declared facts maintained, rule logic recomputed per edit | **Load:** RSS 231–281 MB, footprint 374–398 MB. Export blocks 149 MB, interner 113 MB, membership ≈ 9 MB, findings ≈ 11 MB. Not yet in that figure: the maintained `reference_edges` table (99,002 rows × 9 columns in worktree 1) and the state that would maintain it. **After 1,000 edits:** not measured (killed). After 60 edits: RSS 130 MB, footprint 383 MB. | **Rule logic only, frontier apart (60-event prefix, n = 60):** p50 / p95 / p99 = 18.7 / 21.8 / 22.9 s at load. Scaled to P3a's lighter load (factor 2.57 on exported-families): ≈ 7.3 / 8.5 / 8.9 s. **Split:** 43 unscoped, 0 scoped (no plan admits the demand rewrite), 17 skipped by D1 (0.04–0.07 ms). With the frontier recomputed too: p95 91.8 s. **On facts-a's columnar SQL engine** (worktree 1, reference_edges injected): 1,202 s for dead code plus exported-families; 1,197 s of it is `reach_edges` and `base_edges` | Prefix of 60 events, 7 checks, 8 worktree checks: **0 differences**, 0 runtime refusals. Cross-engine, worktree 1: facts-a's SQL engine over raw inputs plus the injected maintained `reference_edges` gives the same findings as `batch` over the IR plan over exports: dead code 2,570 = 2,570, exported-families 138 = 138 (matched on path and code) | **I3:** the fact/rule boundary is recognised from stage labels (`views/*`); production needs `DECLARED-FACTS.md`'s `facts` block. **I8:** rule logic gets no delta plans. Its only maintained fact, the linker, has no bounded-memory generated maintenance (under the generator it is the 61.4M-record, 42 % linker). I6, I7 and I9 are kept. Gate: nothing registered (the demand rewrite admits no plan) | **Unscoped per-edit p95 8.5 s, scaled (21.8 s measured) > 5 s.** The rule part alone exceeds the budget on both engines. It grows linearly with the cross-file plans (≈ 0.31 µs per rule row out; one dead-code-class plan adds ≈ 5 s per edit), and no plan scopes |

Commits: `751e8f0a9` (P3a's `batch`/`hreplay`, cherry-picked as the recompute engine), `0a6617742` (X4 harness), plus this report. Report: `docs/facts/reports/explore-x4-facts.md`.

## 1. What is maintained, enumerated

`facts-b dgraph --real specs/spec-replay-all5.json` (`runs/dgraph.json`) applies `gen/declared.rs` to the normalised graphs.

- **The frontier** is the set of fact nodes that a rule node or an output reads. Across the 5 plans it is exactly **one node**: `unreachable-internal-symbols/views/reference_edges` (n910). It has 9 columns: `__path, __start, __end, target_path, target_start, target_end, source_name, target_name, target_class`.
  - Behind it are 898 of dead code's 1,327 nodes: 77 linker stages over the `cgn_*` per-file summaries.
  - Dead code keeps 404 recomputed nodes in 20 stages.
  - No node is both maintained and recomputed.
- **`module_of` is not a separate fact the rules read.** Every rule stage computes its module from `__path` (`inventory`, already in the per-content export `exported:14`).
- **The other 4 plans read no membership fact other than the exports.** Their frontier is empty.
- **Per-content exports** stay resident as P3a's hash-consed blocks: 48,518 blocks and 9.39M rows at load.

So the declared layer for these 5 plans consists of two things: the exports (already per content) and `reference_edges` (one membership fact).

## 2. Demand rewrite: side conditions, and the verdict

Side conditions (`declared.rs` module doc; checked on the algebra, I3). An output is scoped by a key column when its rows with key k are a function of the leaf rows with key k only. The checker traces the column through each operator:

- **S1:** a `Map` keeps the key as a bare column. `Filter`, `Distinct` and `Affinity` keep it.
- **S2:** in a `Join` or `Semi` (anti included), the join keys equate the key with a key column of the other side, or the other side reads no leaf.
- **S3:** every input of a `Concat` is keyed.
- **S4:** the key is a `Group` grouping column (keyless fails), a `Window` partition column, or inside a `DistinctPrefix`.
- **S5:** in a `Fix`, init and step keep the key, with the variable assumed keyed at the same column.
- **S6:** leaves are `Exported` and frontier nodes.

Edit keys are the key values of the changed blocks' rows, old and new. The leaves are restricted to those keys across all members, and the output rows with those keys replace the stored ones.

**Verdict: 0 of 5 plans are scoped**, with or without the frontier as a leaf. The first failing operator for each plan and column:

| Plan | Failing operators |
|---|---|
| unreachable-internal-symbols | `declarations` (window partitioned by `id`, across files: S4) for path/start/end; `dead_traits` (join not on the key: S2) for `symbol`; `<final>` computes `category` (S1) |
| exported-families | `declarations` window (S4); `triple_violations` computes `issue`/`family` (S1) |
| state-flow-project-1 | `project_seeds`, `<final>` joins not on the key (S2) |
| required-contracts | `<final>` computes every column (S1) |
| quality-project-taxonomy | `<final>` join not on the key (S2), computed column (S1) |

The two heavy plans would also fail at their recursions if the window were narrowed. `reachable` joins `flow.node = reach_edges.source`, and `resolved_names` joins `i.target = n.fqname`; both move the key (S5). A demand from changed keys through those recursions is the transitive cone over the current relation. Computing it needs that relation, either resident (that is maintenance, not design (d)) or recomputed (no saving).

The three light plans fail too, but they cost ≤ 17 ms, so scoping is moot for them.

## 3. Measurements

### 3.1 Harness

- **Engine:** P3a's `gen/batch.rs` executor, cherry-picked. It evaluates the normalised plan graph over one worktree's members in a forked worker.
- **`batch::eval_with(…, first)`:** evaluates the frontier first and returns its seconds and counts separately. The outputs are the same relations.
- **`hreplay --maintained`:** reports the rule logic's milliseconds as the latency and the frontier's apart (`frontier_ms`, `…+frontier_from_scratch`).
- **What the frontier stands in for:** it is recomputed inside the worker, an oracle standing in for the maintained table. The latencies above are therefore a **lower bound** on design (d): they exclude the cost of maintaining `reference_edges`.
- **Exactness:** `--check batch` compares stored results with a from-scratch run of the same clause.

### 3.2 Runs

| Run | What | Result |
|---|---|---|
| `20261023-160046-x4-load40` | load, 40 worktrees, 6 workers, load 28 → 80 | Per-worktree rule recompute, all 5 plans: p50 / p95 / max **20.1 / 28.9 / 30.7 s** (scaled ×1/2.45: 8.2 / 11.8 / 12.5 s). Dead-code rule part p50 / p95 12.8 / 20.8 s (5.2 / 8.5 s scaled); its frontier 68.4 / 96.6 s (27.9 / 39.5 s scaled). Exported-families 6.4 / 9.4 s (2.6 / 3.8 s scaled; P3a measured 2.60 / 3.95 s). RSS 231 MB, footprint 374 MB |
| `20261023-170404-x4-prefix60` | 60-event prefix, `--check batch --check-every 10 --sweep 2`, 6 workers, load 21–45 | Per edit (rule only) p50 / p95 / p99 **18.7 / 21.8 / 22.9 s**. With the frontier from scratch: 76.6 / 91.8 / 97.9 s. Recompute ran on 43 edits; 17 were skipped by D1 (15 whitespace, 2 reverts). 0 differences, 0 refusals. RSS 281 → 130 MB, footprint 398 → 383 MB. Worker peak RSS: up to 3.4 GB per dead-code worker, counted apart |
| `dref-w1` (`facts-b dref`) | worktree 1 on `batch` | Dead code: frontier 88.6 s + rule 12.4 s. Exported-families 9.6 s. Outputs 2,570 / 138. `reference_edges`: 99,002 rows |
| `20261023-164223-x4-facta-w1` | facts-a's SQL engine (`repo`, `FACTS_X4_REFEDGES`), worktree 1, dead code and public_api_triples | Findings equal `batch`'s (above). Repository phase 1,202 s. Dead code 1,121.5 s, of which `reach_edges` 999 s and `base_edges` 120 s (rest 2.8 s). Exported-families 80.3 s, of which `base_edges` 78.6 s (rest 1.7 s) |

**Why facts-a's engine fails.** It plans joins in FROM order as index nested loops. On repository-sized inputs this is pathological in two stages: `base_edges` (a correlated `coalesce` scalar subquery under `OR`) and `reach_edges` (a 6-arm `UNION` with a `CROSS JOIN … ON` over `dead_traits × declarations`).

On the stages it does plan well, it is 2–5× faster than `batch` under load:

| Stage | facts-a | `batch` (40-worktree mean) |
|---|---|---|
| `reachable` | 0.72 s | 3.49 s |
| `imports` | 0.60 s | 1.62 s |
| `public_methods` | 0.40 s | 0.75 s |
| `dead_traits` | 0.14 s | 0.66 s |

[INFERENCE] A composite engine, with facts-a's columnar operators and `batch`'s hash-join plan for those two stages, would bring the 5 plans to roughly 2–4 s per edit at P3a's load. That composite was not run. It would not change the verdict at more than a few heavy plans (§ 4).

### 3.3 Memory (resident = facts + membership + findings)

These are P3a's M2 structures, unchanged: export blocks 148.9 MB (all narrow, 32-bit), interner 1.20M strings / 113 MB, membership 224,125 entries (≈ 9 MB), findings 108,614 rows (≈ 11 MB).

Not included:

- **The maintained `reference_edges` table.** Worktree 1 holds 99,002 × 9 values, ≈ 3.6 MB at 32 bits a value. Unshared for 40 worktrees that is ≤ 143 MB. [INFERENCE] Hash-consed per source path, as the exports are, it is a small fraction of that.
- **Whatever maintains it.** No bounded-memory maintenance of the linker exists. Under the generator it is 61.4M arranged records (≈ 12 GB, S3 § 15.2).

Design (d)'s memory is therefore within budget only if a linker maintenance under ≈ 100 MB exists. This exploration did not build one, because the latency kill came first.

## 4. What the recompute costs at 5 plans, and the extrapolation

Rule work per recompute, mean over the 40 worktrees, from `counts − frontier_counts` in `x4-load40`:

| Plan | rows out | fixpoint derivations | probes | index rows | exported rows read |
|---|---|---|---|---|---|
| unreachable-internal-symbols (rule part) | 15.55M | 4.76M | 5.41M | 1.82M | 0.44M |
| exported-families | 8.12M | 0.57M | 2.76M | 1.19M | 0.40M |
| required-contracts | 9.2K | 0 | 30 | 8.9K | 8.9K |
| state-flow-project-1, quality-project-taxonomy | 2 each | 0 | 0 | ≤ 1 | 0 |
| **all 5** | **23.7M** | **5.33M** | **8.17M** | **3.0M** | **0.85M** |
| for comparison, dead code's frontier (linker) | 35.2M | 13.6M | 14.8M | 18.0M | 3.4M |

Per-edit cost is Σ over affected plans of the plan's whole-worktree work. Measured: ≈ 0.31 µs per rule row out on `batch`, at P3a's load. [INFERENCE] ≈ 0.1 µs on facts-a's well-planned stages. Since no plan scopes, the cost does not depend on the edit beyond D1's skip (12 % of single-file edits on the full replay; P3a's offline count).

Extrapolation, keeping today's mix:

- The 60 rules carry 29 cross-file clauses. Of the 5 that compile, 2 are heavy (≈ 12M rule rows out each, on average).
- Each further dead-code-class plan adds ≈ 5 s per edit on `batch`, ≈ 1.5 s on a facts-a-class engine.
- At 200 rules: ≈ 97 cross-file clauses, ≈ 39 heavy, ≈ 465M rows per edit. That is ≈ 145 s on `batch`, ≈ 45 s on a facts-a-class engine.

Cross-plan CSE does not rescue it. The 6 stages shared by dead code and exported-families account for ≈ 3.7 of exported-families' 6.8 s under load. Removing them brings the scaled per-worktree p95 from 11.8 to ≈ 10.2 s.

## 5. Why it is killed

The kill criterion is the unscoped per-edit p95. Measured on the prefix it is 21.8 s (≈ 8.5 s at P3a's load). That is a lower bound, because the linker's maintenance is excluded. Every non-skipped edit is unscoped, because the side conditions fail in all 5 plans at cross-file windows, cross-key joins and key-moving recursions.

The design's cost is linear in the rule work of the cross-file plans, and the rule set grows. Even an ideal engine that met 5 s at 5 plans would pass it at a handful of heavy plans.

What would have to change for (d) to live:

- Rule logic that admits scoping, i.e. cross-file clauses whose findings are key-local. Dead code is a global reachability, so this is a rule-shape question (Jeremy), not an engine one.
- Or the heavy recursions (`reachable`, `resolved_names`, `hierarchy`) declared as maintained facts. That moves (d) toward full maintenance of exactly the 80 % of records the memory budget cannot hold.

## 6. Code (disposable)

**facts-b:**

- `gen/declared.rs`: `frontier`, `split`, `scope` (S1–S6), `dgraph` and `dref` commands.
- `gen/batch.rs`: `eval_with`, `eval_node`, `First`.
- `gen/hybrid.rs`: `--maintained`, `frontier_ms`, `frontier_counts`, `…+frontier_from_scratch` latencies.

**facts-a:**

- `rel.rs`: `FACTS_X4_REFEDGES=FILE` accepts a `references` clause and reads `reference_edges` from FILE. This is measurement only and is off unless the variable is set.
- The harness policy (`/tmp/x4-pol`) is the w3 dead-code policy without `raw_modules`. That input projects `file.module`, which facts-a refuses, and only the linker reads it.

**Checks run:**

- `cargo test --release -p facts-b`: 16/16.
- `cargo test --release -p facts-a --bin facts-a rel::`: 2/2.
- Neither T1 nor the gate was run.

**Resume notes:**

- `hreplay --maintained` with `--check batch` is the harness.
- Calibration: compare exported-families' p50 with P3a's 2.60 s under the same binary.
