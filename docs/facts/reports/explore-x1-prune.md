# Exploration X1: design (a), arrangement-demand pruning in the generator

Branch `Moti-Velz/facts-engine-x1-prune` at `eee7d6ac3`. Status: **killed at the first bad number.** After every cut, the 300-event resident is **20.4 GB RSS (17.0 GB footprint)**, against a kill line of 10 GB. The cuts are exact and cost no latency. They remove 28 % of the arranged records at load and 28 % after the prefix. That is far short of the ≈ 5× reduction the kill line needs.

## Decision-table row

| Design | Memory, load / after 1,000 edits | Per-edit p50 / p95 / p99 | Exactness | What it breaks | Kill point |
|---|---|---|---|---|---|
| (a) Arrangement-demand pruning in the generator. Cut 1: DISTINCT and GROUP outside fixpoints run as a total-order reduce that reads only its input arrangement, so there is no output arrangement and DISTINCT keeps one presence arrangement. Cut 2: column demand, where each arrangement keeps only the columns some consumer reads. Everything else already streamed. | **17.8 GB RSS / 16.0 GB footprint at load** (baseline: 29.3 GB unloaded machine, w6; 28.6 GB footprint same-session). **20.4 GB RSS / 17.0 GB footprint at the end of the 300-event prefix** (baseline footprint 28.9 GB). 1,000 edits: **not run**, killed at the prefix. Arranged records: 145.9M → **104.1M** at load and 171.1M → **122.8M** at the prefix. | **42.7 / 172 / 393 ms** (n = 298, prefix; machine load avg 40–80). Same-session baseline: 47.7 / 166 / 557 ms. No regression measurable. | Incremental ≡ from-scratch on the prefix: light plans 301 checks / 315 worktree checks, dead code 13 / 18. **0 differences, 0 runtime refusals.** Dead-code oracle parity 4,931 = 4,931 (0 missing, 0 extra). T1 vectors: r1+r3 0 differences; rt 280/280 refused. | Nothing in I3, I6, I8 or I9: both cuts are generic delta-maintenance changes keyed on the operator algebra, never on names. The gate (I10) would need a differential test and a mutant per cut; neither is registered. Cut 1 needs a totally ordered time, so it does not apply inside fixpoints. | **300-event resident 20.4 GB > 10 GB** after all cuts. The remaining cuts this design allows (the semijoin presence as one arrangement, the fixpoint's distinct output) are worth at most ≈ 10M records (≈ 1.7 GB). |

Commits: `b3f26ea98` (`x1-prune:` the two cuts behind `FACTS_X1_CUTS` and greplay's `arrangements_loaded`), plus this report. Runs are in `~/rerail-bench/facts/explore/x1-prune/runs/`.

## 1. What the generator arranged, and what the cuts removed

Before any cut, the lowering arranged only where an operator needed it: join sides, semijoin left sides, presence, group inputs, windows and fixpoint arms. Maps, filters, concats and join outputs already streamed. Two kinds of state were left that delta maintenance does not read:

1. **Output arrangements of reduces outside fixpoints.** DD's `reduce` keeps its output trace to compute corrections. With a totally ordered time (the outer epoch `u32`), the output at time t is `logic(input ≤ t) − logic(input < t)`. That can be read from the input arrangement alone. This is DD's `threshold_total`, generalised to values: `lower::reduce_total`. Both DISTINCT (`presence_stream`) and GROUP (`group`) use it, so the output streams. A consumer that needs the output arranged arranges it by its own key, as it already did. DISTINCT now keeps one presence arrangement, the counts.
2. **Columns no consumer reads.** `lower::demand` is a backward pass over the topological node order that computes the output columns each node's consumers read. Every arrangement then keeps only those columns: `keyed` narrows, and `joined`/`widen` put NULL back in the unread positions. The pass is conservative:
   - Whole-row consumers (outputs, DISTINCT, fixpoints, GROUP, windows, DISTINCT prefixes) read every column.
   - A map reads every column any of its expressions reads, so an unread expression still refuses at run time as before.
   - A semijoin's right side is read at its keys only.
   - An antijoin's left stream is masked the same way, so that hits cancel.

   Narrowing shrinks rows (more of them fit the 2-value inline `Row`). It also merges records that differ only in unread columns: `iter-join-l` fell from 24.4M to 10.1M.

Arranged records per class (`results.json` → `arrangements_loaded` / `arrangements`). Base load is w6's `mem-all5`; base at 300 events is this session's `base300`, the same binary with `FACTS_X1_CUTS=none`. Counts are deterministic.

| class | base load | cut 1 load | cuts 1+2 load | base 300 | cut 1 300 | cuts 1+2 300 |
|---|---|---|---|---|---|---|
| `by` (keyed node outputs: join/semi/window sides, fixpoint boundaries) | 64.51M | 64.51M | 60.27M | 72.77M | 72.77M | 68.15M |
| `iter-join-l` (fixpoint join arms) | 24.41M | 24.41M | 10.14M | 29.33M | 29.33M | 13.61M |
| `distinct:presence-in` | 12.59M | 12.59M | 12.59M | 15.97M | 15.97M | 15.97M |
| `distinct:presence-out` | 12.57M | **0** | **0** | 15.95M | **0** | **0** |
| `group-out` | 10.48M | **0** | **0** | 11.71M | **0** | **0** |
| `fix:iterate-distinct:presence-in` | 6.31M | 6.31M | 6.31M | 7.71M | 7.71M | 7.71M |
| `fix:iterate-distinct:presence-out` | 6.03M | 6.03M | 6.03M | 7.28M | 7.28M | 7.28M |
| `group-in` | 2.80M | 2.80M | 2.80M | 3.19M | 3.19M | 3.19M |
| `semi-pres:presence-in` (semijoin right sides) | 2.63M | 2.63M | 2.63M | 3.06M | 3.06M | 3.06M |
| `semi-pres:presence-out` | 2.63M | 2.63M | 2.63M | 3.05M | 3.05M | 3.05M |
| `iter-semi-l` | 0.94M | 0.94M | 0.68M | 1.09M | 1.09M | 0.81M |
| **total** | **145.91M** | **122.85M** | **104.08M** | **171.11M** | **143.44M** | **122.83M** |

Memory per cut (300-event prefix, all 5 plans, 40 worktrees). The machine was under heavy memory pressure: swap full, ≈ 69 GB in the compressor, load average 40–80. RSS undercounts compressed pages; the base run's RSS (21.3 GB) is below its footprint (28.9 GB). Footprint (`phys_footprint`) is therefore the comparable column. The unloaded w6 baseline was 29.3 GB RSS at load.

| run | footprint load / 300 | RSS load / 300 | peak footprint (`time -l`) | p50 / p95 / p99 (ms) | run dir |
|---|---|---|---|---|---|
| base (`FACTS_X1_CUTS=none`) | 28.6 / 28.9 GB | 22.1 / 21.3 GB | 33.8 GB | 47.7 / 165.7 / 557.1 | `20261023-173007-base300` |
| cut 1 (`total`) | 23.9 / 25.9 GB | 22.5 / 27.2 GB | 27.2 GB | 43.2 / 153.1 / 419.9 | `20261023-162813-total300` |
| cuts 1+2 (`all`) | **16.0 / 17.0 GB** | **17.8 / 20.4 GB** | 19.2 GB | 42.7 / 172.3 / 393.4 | `20261023-161101-all300` |

Bytes per arranged record at load with both cuts: (RSS − interner 600 MB − exported-row cache ≈ 613 MB − store) / 104.1M ≈ **167 B** (w6: 201 B). The interner (3.30M strings, 600 MB) and the row cache are unchanged by this design.

## 2. Which stages still dominate (profiled load, both cuts, `20261023-171836-prof-load-all`)

| Stage | base (w6) | both cuts | share |
|---|---|---|---|
| dead code `views/link_reach` | 12.41M | 10.55M | 10.1 % |
| `exported-families/public_methods` | 11.88M | 8.21M | 7.9 % |
| dead code `public_methods` (the same relation, computed again) | 11.88M | 8.21M | 7.9 % |
| dead code `reach_edges` | 14.50M | 7.97M | 7.7 % |
| `exported-families/traits` | 6.73M | 6.64M | 6.4 % |
| dead code `views/linked_edges` | 6.89M | 5.86M | 5.6 % |
| dead code `views/s_edge` | 6.51M | 5.67M | 5.4 % |
| dead code `reachable` | 17.29M | 4.40M | 4.2 % |
| dead code `views/name_sub` | 3.73M | 3.73M | 3.6 % |

By group, at load:

| Group | base | both cuts | share |
|---|---|---|---|
| Linker base (dead code `views/*`) | 61.4M | **50.5M** | 48.5 % |
| Dead code outside the linker | 56.2M | 31.4M | 30.2 % |
| `exported-families` | 28.3M | 22.2M | 21.3 % |
| Same-named stages in both plans (one copy: `public_methods`, `annotation_names`, `imports`, `declarations`, `parameters`, `hierarchy`, …) | 19.0M | 14.3M | — |

Pruning hit the fixpoints and their reduces hardest: `reachable` −75 %, `reach_edges` −45 %. It barely touched the linker: its views are joins and DISTINCTs whose columns are all read downstream. The linker is now half of everything that is left.

## 3. Why this is the kill and not a tuning gap

- Needed for 10 GB at 300 events: ≈ 52M records at 167 B/record, with the interner and row cache (≈ 1.3 GB) on top. Measured: 122.8M.
- The cuts this design allows but that were not built (the fixed records come from the 300-event column of the § 1 table):
  - **The semijoins' presence as one arrangement** (−3.1M). This needs a semijoin that reads per-key counts instead of an arranged presence: a custom bilinear operator.
  - **The fixpoint's `iterate-distinct` output** (−7.3M). Inside `iterate` the time is the partial order `Product<u32, u64>`. A reduce without an output trace there is not a known DD construction.

  Together they would bring the prefix to ≈ 112M records, about 19–20 GB.
- What remains is demanded state: join sides, presence counts, fixpoint arms over the linker, and the same relations computed by two plans. Removing it needs other designs: cross-plan CSE, the linker as shared base state, or columnar u32 records (X2Compact measures ≈ 52–59 B/record).

## 4. Exactness and checks (all with `FACTS_X1_CUTS=all`, binary `/tmp/x1/facts-b-c3` = `b3f26ea98`)

- Light plans (`spec-replay-light`), 300-event prefix, every event plus sweeps of 5: 301 checks, 315 worktree checks, **0 differences**, 0 runtime refusals (`20261023-162813-exact-light300`).
- Dead code (`spec-replay-dead-code`), 300-event prefix, every 25th event plus sweeps of 2: 13 checks, 18 worktree checks, **0 differences**, 0 runtime refusals. Worktree 1 has 2,570 findings, as in w6 (`20261023-164531-exact-dead300`).
- The from-scratch side is the same lowering, so it checks delta maintenance, not the lowering's semantics (w6 § 15.1). Semantics were checked separately:
  - Dead-code oracle parity: **4,931 = 4,931**, 0 missing, 0 extra, 0 live refusals (`20261023-171836-parity-dead-code`).
  - T1 vectors against Spike B's oracle: r1+r3 100 events, 101 checks / 180 worktree checks, 0 differences; rt 200 events, 280/280 expected refusals, 0 differences.
  - `cargo test --release -p facts-b` (generated ≡ reference on random tables): 14/14 under `FACTS_X1_CUTS=all`, `total` and `cols` separately.
- Not run: the full replay (killed first), the stage-level comparison (`FACTS_STAGE_OUTPUTS=1`), the gate.

## 5. Notes for whoever reuses the cuts

- Both cuts compose with the other designs: they reduce record count and width before compaction, CSE or columnar storage.
- Cut 1 counts runtime refusals per occurrence a little differently: the logic runs before and after each changed time. Live refusal records are unchanged.
- Column demand is per node, the union over consumers, so all arrangements of a node stay shared. With `FACTS_STAGE_OUTPUTS=1` every stage is an output, so it prunes less in that mode.
- Switches: `FACTS_X1_CUTS=none|total|cols|all` (default `all`).
