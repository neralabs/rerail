# Lattice E4: tree closures with no rounds (L11)

Branch `Moti-Velz/facts-engine-lattice-trees` from integration `949428223`; code commits `45417e41a` (R38, R39, counters, mutants 71/73) and `6a666d0a7` (R39 leaves an empty initial level to R19, so it builds no index). Experiment E4 of `docs/facts/LATTICE.md`, invariant L11. Every number is copied from a run directory under `~/rerail-bench/facts/explore/lattice-e4/` (below, `E4/`). `[INFERENCE]` marks reasoning that was not measured.

## Verdicts

| Claim | Verdict | Deciding numbers | Runs |
|---|---|---|---|
| L11, operator part: "a closure over a forest is an interval probe over preorder ids, with no rounds" | **Holds, checked per content.** 23 of the 31 tree recursions take it: the 10 `descendants` walks (R38) and the 12 `ownership` walks plus `scoped_nodes` (R39). On the corpus none of the 23 falls back. | Rounds the tree family executes: **87,869 → 15,788** (−82%). The 15,788 that remain belong to the 8 recursions not taken; 12,978 of them are `scopes`. Exactness: 0 differences against `FACTS_REWRITES=0` over the 9,010 files. The algebraic fixpoint counts equal the reference's (ref50: 0 mismatches; gate-style stage-check: 11,667 stage counts compared, 0 mismatches). | `E4/runs/20261030T092043-before-counts`, `-094716-after2-counts`, `-094718-after2-unrewritten`, `-095141-stage-check-subset2` |
| L11, cost part: "this is the largest recursive cost in the rules (E0), and it is removed, not parallelised". This is also E4's question: does removing the tree fixpoints remove most of recursion's cost? | **Killed.** The rounds go, but the cost stays. | Physical counts of the 8 rules: 2,071,650,397 → 2,060,129,012 (**−0.6%**). Recursive share of the physical counts: 22.3% → 21.8%. Recursive share of the rows the stages emit: 56.6% → 56.6%. Thread time: corpus medians of 3 alternating runs, user 278.7 → 276.8 s, stage thread time 240.9 → 237.0 s. The runs span 233–262 s, so that difference is inside the noise. Only the *executed* algebraic rows fall (−27.9%), because a closure's fix rows count the same rows a second time (§ 4). | same, plus `E4/runs/*-time-{before,after}-{1,2,3}` |

Before the change, this run's totals equal E0's exactly (alg 156,676,167, phys 2,071,650,397; E0 § 3.1): the before side measures the same workload E0 did.

## 1. What was built

The rule texts are unchanged. Two physical rewrites in facts-a's SQL engine (`spikes/facts-a/src/sql/engine.rs`) and two physical counters (`spikes/facts-a/src/counts.rs`). Both rewrites are registered in `docs/facts/REWRITES.md` (facts-a table).

**R38: forest closure as interval enumeration** (`Exec::tree_walk`, after R18's checks).
- R18 already numbered the walked forest in depth-first preorder, so each seed's walk is an interval `[lo, hi)` of that numbering. R18 then still ran a level loop: one pass per depth, with a binary search per active seed per level.
- R38 replaces that loop with a counting sort. One pass over every interval counts the rows at each depth below their seed, and one placing pass writes `(seed, edge row)` into its depth's slot, stable in (seed, preorder).
- This is exactly the closure's level order. The closure emits each level by the previous level's row, then by edge row; within one seed that is preorder restricted to one depth (R18's argument). No round runs.
- Side conditions are R18's (s), (a)–(d), checked on every execution's rows. Where they fail, the closure runs and the execution is counted (`tree_fallbacks`).

**R39: bag walk over a forest in one pass** (`Exec::forest_walk` / `Exec::bindings`, before R19).
- Statically, the step must have W1's shape (`walk_bound`: item 1 probed by the walk key, later items reached by lookups). No conjunct, and no later item's probe, may read the recursion's row or hold a subquery. Each walk-key column must be an item's column or the same key column carried. Under these conditions a row's bindings are a function of its walk key.
- One breadth-first pass from the initial rows then finds each row's bindings once: item 1's group in table order, then each lookup's candidates, with the `ON` conjuncts checked. The projections are then evaluated row by row as R19's (source, row) pairs.
- Per execution, no group of item 1's index may be expanded twice (no shared child, no cycle, initial keys distinct), and the walk must stay under 50M rows. Otherwise R19 runs and the execution is counted.
- Breadth-first order from the initial rows, appending bindings in the join's candidate order, is the fixpoint's level order. So the rows and their order are R19's.

**Counters.**
- `rounds_run` (physical) counts the passes a fixpoint evaluator executes: the generic fixpoint, the closure and R19 count one per non-empty level; R38 and R39 count none.
- `tree_fallbacks` (physical) counts executions where a forest rewrite's per-content side condition failed.
- The algebraic `fix` stays the logical fixpoint's: R38 reports the widths per depth, R39 the level sizes, and R39 charges the algebraic budget per level as R19 does.
- So the reference evaluator (`stage-check`) and the budgets see the same numbers as before (§ 2), and L8/L11's "0 rounds" is a physical count.
- `memo::ENGINE_VERSION` 2 → 3 (an operator's counts changed while every plan text stayed the same).

**Why runtime-checked and not R9's static form.**
- R9 (`rerail_facts::rewrite::reach_to_interval`) needs the edge relation to be every row of one core `node` scan, with `src = parent`.
- The rules' `edges` stages are `SELECT __path, edge_parent AS src, id AS dst FROM nodes WHERE edge_parent IS NOT NULL`. Their `nodes` is the union of pattern-input captures, and the ids are R16 text ids.
- The child set is the matched nodes, and `edge_parent` is the matched node's direct syntax parent, matched or not. A walk therefore stops at an unmatched node, so `descendants(x)` is not the core interval of `x` restricted to matched nodes [INFERENCE from the stage texts, not measured].
- No I2 declaration makes these edges the core tree. A static proof would first need origin tracing through the pattern front end, the `UNION ALL`, the window and R16 (REWRITES.md § "What replaced subst.rs" already lists that tracing as not done).
- R38 needs none of it. It numbers whatever per-content forest the walk has, in that forest's own preorder.
- **The sub-forest case is therefore covered**, whether filtered or re-parented to a kept ancestor. Only the forest property is checked, per execution, never assumed.

**Mutants.** 71 (R38: level order dropped; each seed's rows in preorder as one level) and 73 (R39: a conjunct reading the recursion's row admitted). Both are registered in REWRITES.md's facts-a table (mutant column) and the Numbers list. The gate reads them from the registry, family `facts-a-unit` + `a-unrewritten`, so no edit to `scripts/check/facts-gate.py` was needed. Both are killed by facts-a's unit tests (T0, § 2).

## 2. Exactness

| Check | Result | Run |
|---|---|---|
| T0 | 3/3 checks; 10/10 mutants of the changed files killed (22, 23, 25, 26, 27, 29, 31, 33, **71, 73**) | `~/rerail-bench/facts/loop/runs/20261030T091949-t0` (before `45417e41a`), `20261030T094648-t0` (before `6a666d0a7`) |
| T1 | sub200 90/90 findings, 0 failures, equal to the reference in every clause; ref50 0 mismatches (2,837 stages, their algebraic counts included) | `loop/runs/20261030T092609-t1` (`45417e41a`, memo 0/14,982: every clause re-evaluated), `loop/runs/20261030T095121-t1` (`6a666d0a7`) |
| The gate's rewrite differential (`a-unrewritten`: findings and failures as multisets, default vs `FACTS_REWRITES=0`), run over the **full 9,010-file corpus** with the 8 recursive rules | 0 findings and 0 failures differ (default run: 0 findings, 1 failure, the compile refusal of `internal_dead_code`) | `E4/runs/20261030T094716-after2-counts` vs `-094718-after2-unrewritten` (and `-092044` vs `-092045` for `45417e41a`) |
| The gate's `a-vs-reference` command (batches 1–5 recordings + subset sample 0/4, ≤ 2,000 nodes, all 60 checks) | 0 mismatches; 836 recorded + 10,831 subset stages; 11,667 stage counts (rows and fixpoint rounds) compared, 0 count mismatches; 2,291 not compared (by reason in results.json) | `E4/runs/20261030T095141-stage-check-subset2` |
| Unit differentials | `rewrite_tree_closure_equals_closure` (R18 + R38: 400 random graph sets × 6 closures; rows, order and algebraic rounds equal the closure's; where R38 ran, `rounds_run` = 0); `rewrite_forest_walk_equals_levels` (R39: 300 × 5 recursions, incl. a `CASE` reading the recursion's row, a `WHERE` and an `ON` reading it, fallbacks counted) | T0 |

The full-corpus findings differential is weak on these 8 rules, because they report 0 findings on the corpus. The stage-level checks carry the exactness argument: stage-check and the unit tests compare rows, order and counts.

## 3. Counts per rewritten stage (batch-6 corpus, the E0 command)

All counts are from `E4/runs/20261030T094716-after2-counts/e4-tables.md` (`analyze.py` there; the before side is `-092043-before-counts`).
- *Rounds* are passes executed, before → after.
- Rows and logical fix rows are equal before and after for every stage.
- Phys is E0's sum: index_rows + range_rows + probes + rows_scanned + rows_joined + rows_out.
- Fallbacks on these stages: 0.

| clause / stage | # | rewrite | rows | rounds | phys before → after | rows_scanned before → after |
|---|---|---|---|---|---|---|
| state-flow-1 / descendants | 33 | R38 | 20,219,892 | 14,139 → 0 | 75,381,347 → 75,381,347 | 24,082,482 → 24,082,482 |
| state-signatures-1 / descendants | 29 | R38 | 8,398,110 | 1,758 → 0 | 28,827,818 → 28,827,818 | 9,306,482 → 9,306,482 |
| state-projection-recovery-2 / descendants | 41 | R38 | 1,464,236 | 529 → 0 | 5,377,259 → 5,377,259 | 1,723,301 → 1,723,301 |
| step-contracts-5 / descendants | 62 | R38 | 783,086 | 2,728 → 0 | 2,897,994 → 2,897,994 | 920,270 → 920,270 |
| step-contracts-1 / descendants | 50 | R38 | 558,576 | 2,066 → 0 | 2,111,124 → 2,111,124 | 667,425 → 667,425 |
| step-contracts-3 / descendants | 54 | R38 | 348,704 | 1,475 → 0 | 1,350,123 → 1,350,123 | 409,559 → 409,559 |
| step-contracts-2 / descendants | 53 | R38 | 282,769 | 1,562 → 0 | 1,099,655 → 1,099,655 | 345,606 → 345,606 |
| executor-provenance / descendants | 15 | R38 | 148,464 | 22 → 0 | 507,404 → 507,404 | 163,967 → 163,967 |
| step-contracts-6 / descendants | 65 | R38 | 2,096 | 18 → 0 | 7,744 → 7,744 | 2,460 → 2,460 |
| state-flow-project-1 / descendants | 43 | R38 (plan) | 0 | 0 → 0 | 0 → 0 | 0 → 0 |
| state-flow-1 / ownership | 32 | R39 | 3,862,590 | 15,103 → 0 | 65,661,429 → 61,798,839 | 30,896,992 → 27,034,402 |
| quality-contracts-1 / scoped_nodes | 19 | R39 | 2,407,265 | 14,180 → 0 | 31,667,002 → 29,259,737 | 12,041,445 → 9,634,180 |
| legacy-execution-graphs / ownership | 8 | R39 | 2,308,813 | 8,764 → 0 | 39,248,129 → 36,939,316 | 18,467,980 → 16,159,167 |
| quality-conditional-1 / ownership (repository run) | 23 | R39 | 1,419,253 | 21 → 0 | 24,100,271 → 22,681,018 | 11,317,984 → 9,898,731 |
| state-signatures-1 / ownership | 28 | R39 | 908,372 | 1,758 → 0 | 15,442,098 → 14,533,726 | 7,266,672 → 6,358,300 |
| state-projection-recovery-2 / ownership | 40 | R39 | 259,065 | 558 → 0 | 4,403,935 → 4,144,870 | 2,072,224 → 1,813,159 |
| step-contracts-5 / ownership | 61 | R39 | 137,184 | 2,728 → 0 | 2,331,318 → 2,194,134 | 1,096,409 → 959,225 |
| step-contracts-1 / ownership | 49 | R39 | 108,849 | 2,066 → 0 | 1,849,623 → 1,740,774 | 869,729 → 760,880 |
| step-contracts-2 / ownership | 52 | R39 | 62,837 | 1,562 → 0 | 1,067,419 → 1,004,582 | 501,633 → 438,796 |
| step-contracts-4 / ownership | 57 | R39 | 31,290 | 1,004 → 0 | 531,120 → 499,830 | 249,257 → 217,967 |
| executor-provenance / ownership | 14 | R39 | 15,503 | 22 → 0 | 263,548 → 248,045 | 124,020 → 108,517 |
| step-contracts-6 / ownership | 64 | R39 | 364 | 18 → 0 | 6,182 → 5,818 | 2,904 → 2,540 |
| state-flow-project-1 / ownership | 42 | R39 (plan) | 0 | 0 → 0 | 0 → 0 | 0 → 0 |

Totals for the two groups:

| group | stages | rows | rounds | phys | probes | thread time, corpus medians of 3 runs |
|---|---|---|---|---|---|---|
| R38 (`descendants` ×10) | 10 | 32,205,933 | 24,297 → 0 | 117,560,468 → 117,560,468 | 4,635,221 → 4,635,221 | `descendants` 5.27 → 5.17 s |
| R39 (`ownership` ×12, `scoped_nodes`) | 13 | 11,521,385 | 47,763 → 0 | 186,572,074 → 175,055,081 (−6.2%) | 41,234,019 → 41,234,019 | `ownership` 12.70 → 12.45 s, `scoped_nodes` 1.34 → 1.30 s |

Per-file single-thread A/B on sub200 (`serverplan_state_tripwires` only, secondary, different loads: `E4/runs/20261029T154423-profile-sft-sub200` vs `20261030T091538-sft-sub200-after`):

| stage | before | after |
|---|---|---|
| state-flow-1 `descendants` | 1,114.5 ms | 974.8 ms |
| state-flow-1 `ownership` | 2,823.4 ms | 2,363.4 ms |
| state-signatures-1 `ownership` | 812.4 ms | 646.6 ms |
| all stages of the rule | 19.88 s | 18.16 s |

What the counts show:

- **R38 changes no physical count.** R18's walk was already uncounted work: no lookup per walk row, only slices of depth buckets. The physical counts of `descendants` are the seed select and the final `SELECT __path, root, node FROM walk` (rows_scanned = rows_joined = rows_out ≈ rows + seeds). R38 removes the level loop's binary searches and the round structure, which the counters never saw.
- **R39 removes only the level tables' scans.** It cuts rows_scanned by exactly one row per derived row (state-flow-1 `ownership`: −3,862,590 = its rows). Probes are unchanged: 15,449,428 for 3,862,590 rows, ≈ 4 per row. [INFERENCE from the code] They split as two per row in the walk (the row's `edges` group, its child's `nodes` lookup, both by text-id key) and two per node from evaluating the initial select's `LEFT JOIN` twice (once more in W1's check).

## 4. The 8 rules before and after (H2's units, E0 § 3.1)

| measure | before | after | recursive stages before | share | after | share |
|---|---|---|---|---|---|---|
| alg = stage_rows + fix_rows (logical; equal by construction) | 156,676,167 | 156,676,167 | 109,086,111 | 69.6% | 109,086,111 | 69.6% |
| executed alg (stage rows + fix rows of executions that ran rounds) | 156,676,167 | 112,948,735 | 109,086,111 | 69.6% | 65,358,679 | 57.9% |
| stage rows emitted | 109,764,637 | 109,764,637 | 62,174,581 | 56.6% | 62,174,581 | 56.6% |
| phys (index, range, probes, scanned, joined, out) | 2,071,650,397 | 2,060,129,012 | 461,160,405 | 22.3% | 449,639,020 | 21.8% |
| tree family phys | 362,968,062 | 351,446,677 | | 17.5% of all | | 17.1% of all |
| rounds executed, tree family | 87,869 | 15,788 | | | | |
| `tree_fallbacks` (all closures and walks) | — | 1,032 | | | | |

How to read this table:

- **fix_rows double-counts.** A tree closure's fix rows are its emitted rows a second time (set closure: every admitted row is a stage row). E0's "69.6% of relational rows" therefore counts tree rows twice. After R38/R39 only 3,184,098 of the 46,911,530 fix rows are still derived round by round. That is the whole −27.9%. No physical count moves with it, because emitting a row costs the same with or without rounds.
- **Which fallbacks occur.** The 1,032 fallbacks are all outside the 23 taken recursions: `receiver_tokens` 430 and `reachable` 598 (call-graph and receiver closures that R18 already refused on those contents; they are now counted), and one each on `demanded`, `annotations`, `owners` and `user_callable`.
- **Where the rounds were.** R38 and R39 run no round, but rounds were never where the cost was [INFERENCE from §§ 3–5: the counts that do not move are the per-row ones].

## 5. Where the tree recursions' cost is now

Profile: samply on `serverplan_state_tripwires` over sub200, one thread, after the change (`E4/runs/20261030T091538-sft-sub200-after/profile-breakdown.txt`; 1 kHz samples; stage thread time in that run: `ownership` 3,407 ms, `descendants` 1,346 ms, all stages 19,132 ms).

`ownership` (R39):

| part | samples | share |
|---|---|---|
| `Exec::check_walk` (P3b's W1 bound check per content) | 1,684 | ≈ 49% of the stage |
| … of which re-evaluating the initial select (`select_rows`) | 408 | |
| … of which building `DKey` hash maps | 607 | |
| `Exec::pick` (the `CASE` projections) | 476 | |
| `Exec::bindings` (the per-row probes, mostly `probe_key` hashing) | 414 | |

W1 runs before every bag walk of its shape. R39's pass establishes a related fact on the reached part, but not W1's: W1 checks every row of the edge and lookup relations. So W1 stays.

`descendants` (R38): `Exec::closure` 1,185 samples. Of these:
- the seed select: 310;
- `tree_walk`: 812. Its self time and `Column::cell`/`cell_key`/hash inserts are the per-execution numbering of the walked forest: about three hash operations per edge row on R16 text-id keys.
- the gathers of the closure table: 51.

Readers (static, approximate scan of the rule texts: `E4/static/readers.txt`):
- `descendants` is read 39 times as `d.root = x AND d.node = y`: a membership test, i.e. interval containment on the walked forest's preorder. It is read 3 times by `node` only (the ancestors of a node) and once by `root`.
- `ownership` is read 16 times by `id` alone and 15 times by `(fn, id)`: a per-node attribute, the nearest function/class of a node.
- Today the engine materialises 20,219,892 `descendants` rows in state-flow-1 alone, and readers hash-index them on their probe columns, once per clause per column set.

[INFERENCE] The lever E0 attributed to "tree closures" is representation, not rounds:
1. Do not emit a closure's rows at all. Keep the forest numbering (`pre`, `end`, depth per node) as the stage's stored form, and let readers' `root = x AND node = y` become a containment test, `root = x` an interval scan and `node = y` an ancestor path.
2. Key the walked forest by the core node id when its origin is provable, so no text-id hashing remains per edge row or per derived row.

Neither is measured here.

## 6. The 31 tree recursions: taken and not taken

**Taken: 23.**
- R38: `descendants` ×10 (#15, 29, 33, 41, 43, 50, 53, 54, 62, 65).
- R39: `ownership` ×12 (#8, 14, 23, 28, 32, 40, 42, 49, 52, 57, 61, 64) and `scoped_nodes` (#19).
- On the corpus every execution of these 23 ran rewritten (`rounds_run` 0, `tree_fallbacks` 0). #42 and #43 (state-flow-project-1) have 0 rows under the default `entryPoint`, so they are taken by their plan but were never exercised.

**Not taken: 8.**

| # | stage / CTE | rounds on the corpus | why not |
|---|---|---|---|
| 1 | authority-sites `contexts` / `scopes` | 12,978 | its step projects `p.qualname ‖ … ‖ c.name` and `p.depth + c.is_function`: not reference forms, so not R19's shape and not R39's. A string path-fold (E0: no lattice) |
| 34, 44 | state-flow(-project)-1 `contexts` / `class_names` | 652, 0 | `p.name ‖ '.' ‖ c.name`: string path-fold, as #1 |
| 22 | quality-contracts-1 `scope_chain` / `chain` | 2,124 | `c.depth + 1` is arithmetic, so not a reference form. It is a forest walk; R39 could evaluate such projections from the parent's values, which is not done |
| 25 | quality-conditional-1 `descendants` / `demanded` | 17 | multi-source: all `descendant_roots` of a path share one carried tuple, so R18's (s) fails (1 fallback). This is `min_level` over a union of subtrees, not one interval per seed |
| 17 | executor-provenance `annotations` / `accepted` | 3 | walks upward (child → enclosing type node); a node with several children breaks R18's (a)/(b) on the content (1 fallback) |
| 56 | step-contracts-3 `payload_values` | 14 | five recursive arms: not the closure's shape |
| 60 | step-contracts-4 `mutable_values` | 0 | its step joins two items besides the recursion: not the closure's shape; 0 rows on the corpus |

## 7. Wall-clock (secondary)

Three alternating runs of the E0 command at 8 threads (`E4/runs/20261030T0926…-time-before-1` … `20261030T0942…-time-after-3`). Load is the 1-minute average from uptime before → after.

| run | wall s | user s | stage thread s | `descendants` s | `ownership` s | `scoped_nodes` s | load before → after |
|---|---|---|---|---|---|---|---|
| before-1 | 176.32 | 273.50 | 233.31 | 5.12 | 12.30 | 1.30 | 12.60 → 7.57 |
| after-1 | 178.51 | 276.80 | 237.01 | 5.15 | 12.15 | 1.30 | 7.57 → 9.65 |
| before-2 | 199.87 | 290.31 | 259.15 | 5.35 | 12.71 | 1.38 | 9.65 → 10.87 |
| after-2 | 177.80 | 274.64 | 237.00 | 5.17 | 12.45 | 1.29 | 10.87 → 6.33 |
| before-3 | 182.28 | 278.72 | 240.93 | 5.27 | 12.70 | 1.34 | 6.33 → 6.42 |
| after-3 | 194.01 | 296.42 | 262.30 | 5.96 | 14.81 | 1.57 | 6.42 → 9.95 |

Medians, before → after:
- user CPU: 278.72 → 276.80 s;
- stage thread time: 240.93 → 237.01 s;
- `descendants`: 5.27 → 5.17 s;
- `ownership`: 12.70 → 12.45 s;
- `scoped_nodes`: 1.34 → 1.30 s.

The spread within each side (233–262 s of stage thread time) is larger than any difference between the sides.

## 8. REWRITES.md and counts

- **New rows** (facts-a table): R38 and R39 with pattern, side conditions, justification, differential test and mutant (71, 73). The Numbers list now has "71, 73: R38, R39 (facts-a table, lattice E4; E4 takes odd numbers from 71)". LatticeE3Maint registered 75 and 77 in the same list and in the gate's MUTANTS, so expect a trivial merge.
- **Updated rows**: "What replaced `subst.rs`" for the `descendants` and `ownership` kernels.
- **Count changes on the gate subset** (`E4/runs/20261030T094729-subset-after2/count-diff.json`, against `benchmark/facts/subset-counts.json` at the merge-base): no clause grows; only rows_scanned shrinks, so no "## Count changes" row is needed. For the integrator's `subset-counts.json` refresh:

| clause | rows_scanned before → after | change |
|---|---|---|
| step-contracts-4 | 113,630 → 107,416 | −5.47% |
| step-contracts-6 | 7,001 → 6,681 | −4.57% |
| step-contracts-5 | 618,235 → 590,232 | −4.53% |
| quality-conditional-1 | 4,680,686 → 4,485,671 | −4.17% |
| legacy-execution-graphs | 8,347,392 → 8,011,932 | −4.02% |
| state-signatures-1 | 4,968,975 → 4,773,634 | −3.93% |
| step-contracts-2 | 435,150 → 423,443 | −2.69% |
| state-flow-1 | 24,756,039 → 24,132,493 | −2.52% |
| step-contracts-1 | 1,381,853 → 1,358,802 | −1.67% |
| quality-contracts-1 | 23,207,142 → 22,872,295 | −1.44% |
| state-projection-recovery-2 | 2,143,137 → 2,123,098 | −0.94% |

  The new physical keys `tree_fallbacks` and `rounds_run` have no baseline.
- **Bench consumers**: `scripts/check/facts-loop.py`'s `PHYSICAL` list for the bench does not name the new keys, so the bench prints neither. Not changed.

## 9. Contradictions with LATTICE.md

1. **L11's cost claim** ("the largest recursive cost … is removed, not parallelised") is wrong as stated. Removing the rounds removed 82% of the tree family's executed rounds and 0.6% of the 8 rules' physical work, with no measurable time change (§§ 4, 7). E0's "66.0 of 69.6 points" were logical rows in which every tree-closure row counts twice (stage row + fix row); E0 itself noted the 22.3% physical share.
2. **L11's mechanism** ("R9 extended to the rules' derived `edges` stages") is not how it can be done. The derived edges are not the core tree (§ 1). What works is the per-content forest numbering, checked on the rows, which needs no I2 origin proof.
3. **"R9 already does it for quality-contracts-1's `descendants`"** (E0 § 2.4, LATTICE.md Results table "interval probe, R9 form"). `rewrite::reach_to_interval` has no caller in facts-a or facts-b: a search over the worktree finds it only in `facts/src/rewrite.rs` and its test. quality-contracts-1's interval probe is the declared `reachability` stage with `syntax` (`spikes/facts-a/src/rel.rs::reach`, `Column::Interval`), not R9.
4. **L8's count check "tree-closure stages report 0 rounds"** cannot be on the algebraic `fix`. As written, the rules observe level order: it is the CTE's output order, and `ownership`'s readers read it. The logical rounds are part of the result's definition and of the reference's counts and budgets. The 0 must be on executed rounds (`rounds_run`).
5. **REWRITES.md's warning on `ownership`** (readers observe level order) did not need I1 ties. R39 keeps level order exactly.

## 10. Proposed amendments to LATTICE.md

1. **L11** becomes: "Tree closures run no rounds. A closure or bag walk over a per-content forest (FD child → parent, checked on the rows; derived sub-forests included) is evaluated from one preorder numbering of that forest: set closures as interval enumeration (R38), bag walks by one breadth-first pass (R39). Level order is kept, so readers that observe it are unaffected. Enforced by: REWRITES R38/R39 with per-content side conditions and a counted fallback (`tree_fallbacks`); the count check is `rounds_run = 0` on the taken stages."
2. **Strike "This is the largest recursive cost in the rules (E0) and it is removed"** and state the measurement: rounds −82% in the tree family, physical work −0.6% (8 rules), thread time unchanged within noise. Recursion's remaining cost:
   - materialising O(n·depth) rows, and readers' hash indexes on them;
   - text-id keyed probes (≈ 4 per `ownership` row);
   - the per-content forest numbering (hash operations per edge row);
   - P3b's W1 check (≈ half of `ownership`'s time on sub200).
3. **Add an experiment E5 (closure representation)**:
   - Store a tree closure as its forest numbering (`pre`, `end`, `depth` per walked node, per-seed intervals), read in place.
   - Rewrite readers' `d.root = x AND d.node = y` (39 occurrences) to containment, `d.root = x` to an interval scan and `d.node = y` to an ancestor path.
   - Serve `ownership`'s per-node attributes (31 point lookups by `id` or `(fn, id)`) from per-node arrays computed in the same pass.
   - Kill criterion: < 2× fewer physical counts on the tree family, or any reader whose plan needs the materialised rows.
   This is where L5/L6's "data does not move, regular shape" could apply to trees.
4. **L8**: name the two round counts. `fix_rounds`/`fix_rows` are algebraic (the logical fixpoint's, equal to the reference's, used by budgets); `rounds_run` is physical (passes executed).
5. **Results table**: correct "interval probe, R9 form" for quality-contracts-1 (the declared syntax `reachability` stage, not R9), and add E4's row.

## 11. Not done

- **No static R9 generalisation.** The origin tracing through pattern captures and R16 ids that a static proof needs was not attempted; § 1 argues why it would not apply to these stages.
- **Not taken:** the forest path-folds (#1, #34, #44; non-reference projections), arithmetic projections in a forest walk (#22), and multi-source closures (#25, `min_level`).
- **No representation change** (E5 above). No rule edits. No change to the repository engines (M2 / the generator). No change to P3b's W1 check.
- **Wall-clock**: only the 3 + 3 corpus runs; the machine was shared (load 6–13).
- **Not run:** T2 / `just facts-gate` (the integrator's).

## Reproduce

```
just facts-t0 && just facts-t1
cd ~/rerail-bench/facts/explore/lattice-e4
./run2.sh before-counts $PWD/before.bin 6 -- --file-counts @D/file-counts.jsonl   # before.bin: integration 949428223's target/loop/facts-a
./run2.sh after-counts  $PWD/after2.bin 6 -- --file-counts @D/file-counts.jsonl   # after2.bin: 6a666d0a7's
./run2.sh after-unrewritten $PWD/after2.bin 6 FACTS_REWRITES=0
python3 analyze.py runs/<before> runs/<after> runs/<unrewritten>                 # e4-tables.md, e4-counts.json
./timing.sh                                                                       # 3 alternating runs each, 8 threads
```
