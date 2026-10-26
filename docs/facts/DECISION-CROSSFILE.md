# Cross-file memory: decision table (exploration wave, 2026-10-23)

Same replay for every row: 40 worktrees from `~/tempo-qe/workspaces47`, S3's 1,111 events (300-event prefix where stated), the 5 running repository plans (dead code, `exported-families`, `required-contracts`, `quality-project-taxonomy`, `state-flow-project-1`), `facts-b greplay`. Target: ≤ 250 MB at load, ≤ 300 MB after 1,000 edits, p95 ≤ 500 ms, exact. Memory is footprint (RSS read low under this machine's swap compression). All agents' numbers; none re-verified, per the mode.

| Design | Memory: load / after edits | Per-edit p50 / p95 / p99 | Exact | Breaks | Verdict |
|---|---|---|---|---|---|
| **M0** today: full differential-dataflow maintenance | 28–30 GB / 42 GB (1,000) | 17 / 62 / 190 ms (locked) | yes | — | reference point |
| **M1** full maintenance compacted (P3a final: one presence, cross-plan stage sharing R37, interner sweep; compact records not built) | 18.4 GB / 32.5 GB (1,000) | 39 / 131 / 442 ms | yes, 0 diff | — | **fail**: floor ≈ 15 GB in row values alone |
| **M2** hybrid (P3a): membership + per-content exports + last results; every plan recomputed per edit unless D1 says its exports are unchanged | **325 MB heap / 470 MB footprint**, ≈ 85 MB if the store serves exports; flat (+22 MB over 1,000 edits) | 38.8 / 48.8 / 80.6 **s** (dead code 32 s, `exported-families` 2.6 s uncontended; the other three ≤ 10 ms) | yes, 0 diff on prefix and full replay | I8 for recomputed plans | **fail on latency only**; the only model within an order of magnitude of 250 MB |
| **X1** arrange only what operators need, column pruning | 16.0 GB / 17.0 GB (prefix) | 43 / 172 / 393 ms (load 40–80) | yes, 0 diff | nothing | **killed**: 123M records remain, all read by delta maintenance |
| **X2** u32 columns, one evicting interner | 10.2 GB / 10.7 GB (prefix) | 36 / 99 / 343 ms (unlocked) | yes, 0 diff | nothing (limits panic instead of refusing) | **killed**: 53 B/record is the floor; 630 MB of strings stay live because records hold them |
| X1 + X2 (arithmetic) | ≈ 6.7 GB / ≈ 7.5 GB | — | — | — | 25–30× over target |
| **X3** per clause maintain vs recompute by measurement | lowest mix 1.7 GB load, 5.8 GB peak | 38 / 43 / 43 **s** | yes | I8 for recomputed clauses | **killed**: dead code 37–71 s and `exported-families` 4–9 s per recompute; their inputs change on 88% of edits |
| **X4** maintain declared facts only, recompute all rule logic | 0.37–0.40 GB (+ ≈ 12 GB if `reference_edges` is maintained by today's generator) | rule logic only 7.3 / 8.5 / 8.9 s (adjusted); 92 s with the linker recomputed | yes, 0 diff | I3 (fact/rule boundary by stage label), I8 | **killed**: no plan passes the demand-scoping side conditions; cost ≈ +5 s per dead-code-sized rule |
| **X5** no dataflow: indexed tables, delta-scoped recompute | ≈ 0.9–1.6 GB (estimated; names alone 250–300 MB) | ≥ 7.6 / 7.6 / 10 s with the linker free | linker rule exact by derivation, not run | I8 (hand-derived delta rule) | **killed**: global fixpoints (`reachable`, `resolved_names`) and global windows can't be scoped |
| **X6** five rules rewritten on one shared declared-facts layer | 19.1 GB / 25.6 GB (1,000) on the dataflow; per-file engine 0.27–0.72 ms per rule per file, facts 0.5–1.4 ms per content | 23 / 73 / 226 ms (dataflow, unchanged) | yes, 0 diff; dead code 4,931 = 4,931 | I6/I7 (facts carry `__path`) | sharing saves 11–14% of records; **light rules alone fill 250 MB at 32–45 rules**; per-file cost linear in N (20 → 200) |

## What the wave established

1. **Full maintenance cannot reach the target by engineering.** The floor after every generic cut is ≈ 50 B × ≈ 120M records ≈ 6–7 GB for five plans, and the records are what delta maintenance reads. At 50 B/record the target admits ≈ 5M arranged records, i.e. two orders of magnitude fewer than these five rules need as written. Hundreds of rules make it worse linearly (X6: +19M records per dead-code-shaped rule, +20K per light rule; 32–45 light rules alone exhaust 250 MB).
2. **Recompute cannot reach the latency target as the rules are written.** Dead code and `exported-families` depend on global fixpoints and global windows; an edit changes their inputs 88% of the time and a recompute costs 5–70 s. No demand-scoping side condition holds for any of the five plans (X4, X5).
3. **The shared fact layer works and is cheap on the per-file side** (X6: exact, 0.3–0.7 ms per rule per file, facts once per content, linear in N), but it does not change the cross-file arithmetic: the membership-dependent state is the symbol table and the reference graph, 19M + 61M records, before any rule.
4. **P3a's finding closes the loop:** nothing beyond membership must be maintained for exactness; maintained state buys latency only. M2 holds 325 MB flat and is exact; what it lacks is the 500 ms, and 84% of each dead-code recompute is the linker. So the one structure worth maintaining is the reference graph, compactly, and the rules above it should be local queries.
5. **Exactness and latency were never the problem.** Every design that ran was exact, and maintained latency is 60–170 ms p95. The problem is the product of rule shape × maintenance granularity.

## Y1 + Y2 (built after the wave; the design the table pointed at)

| Piece | Memory | Per-edit p50 / p95 / p99 | Exact | Open |
|---|---|---|---|---|
| **Y1** maintained reference graph: linker as keyed tables + cached per-key queries recording their read sets; readers of changed keys re-run lowest level first; base → 8 cluster overlays → worktree deltas | base **72 MB**; overlays 193 MB (22–26 MB each); 40 deltas 32 MB at load, 50 MB at edit 1,000 (≈ 29 KB per diverging file); plus shared strings 89 MB and per-file inputs 288 MB. Heap 869 MB loaded / 956 MB at 1,000 | **0.73 / 2.4 / 3.9 ms** single-file; commits and bulk 45–205 ms, max 1.5 s | base = native's 145,431 edges; maintained = from-scratch at all 1,151 checks | frozen-layer compaction (est. 1.5–2×) not landed; query bodies hand-written per SQL stage (I8); two semantic deviations from the SQL linker with zero-count counters |
| **Y2** dead code and `exported-families` as local queries over the graph | dead code: a maintained reached set with levels, 12 B per state, **24 MB for 41 worktrees as base + delta** (182 MB as copies); `exported-families` 1–1.4 MB per worktree | dead code scoped **0.07 / 1.5 / 3.1 ms** (unscoped 150–310 ms); `exported-families` scoped 0.005 / 0.04 / 0.09 ms (unscoped 11 ms) | 4,931 = 4,931 and 0 = 0 on the oracle tree; 340 replay checks 0 differences; 800 random deletion steps 0 differences | the nine symbol-table inputs still come from the dataflow (not costed); per-definition traits per file; Y1 not wired in |

**Reading.** Together they replace 30–70 GB of arrangements with ≈ 0.9 GB today, of which the graph's own state (base + overlays + deltas) is ≈ 300 MB and the rest is strings and per-file input rows that the content store should serve; latency is milliseconds. Dead code's global reachability is the one property that needs maintained state, and it costs 24 MB across 41 worktrees when kept as a levelled reached set over the graph rather than as a dataflow fixpoint. Rule-shape item confirmed for Jeremy: 85% of the reached states are per-method dispatch searches; keyed by class they would drop to 135K states.

## Y3: the wired path (ship candidate)

One path, no differential dataflow: per-content exports → Y1 graph apply → maintained symbol table (same keyed-table design, replaces the nine `proj_*` inputs) → Y2 scoped updates → findings delta; the three light plans on M2 with the D1 skip. Exact over the full replay: 1,151 from-scratch checks, 0 differences; symbol table = dataflow on 1,098,978 rows. Perf run `20261025-210541-full-perf`.

| | Value |
|---|---|
| Heap, 40 worktrees | **1.94 GB at load, 2.09 GB at edit 1,000**: maintained state ≈ 950 MB (graph 72 + 193 + 32→54 MB; symbol tables 60 + 139 + 32→54; Y2 input indexes 47 + 167 + 92→143; reached sets 3 + 92 + 16→27), per-file inputs 253 MB and strings 285 MB (store-servable), ≈ 390 MB unattributed |
| Single-file edit p50 / p95 / p99 | **0.53 / 3.76 / 8.62 ms** (Y1 0.18/1.07, symbol table 0.11/0.67, Y2 0.10/2.19, M2 0.02/1.08) |
| 200-file bulk / real commit | 387 / 550 ms; 63 / 470 / 760 ms |
| New worktree | 11–13 ms (M2's three plans from scratch); ≈ 2.9 ms and 156 KB per diverging path |
| Floor | identical content 0.008 ms; content changed, nothing downstream 0.26 / 1.94 ms |
| Memory levers not taken | overlays re-add 2.29M reached states (62 MB) the base holds as 95K; Y2 keyed by position instead of DefId; three interners; frozen-layer arenas |

Against the targets: latency met by two orders of magnitude; memory 2.1 GB against 250 MB, with ≈ 540 MB store-servable and the levers above unapplied; against today's 42 GB, 20×.

## Conclusion for the decision

The target (250 MB for 40 worktrees, hundreds of rules, ≤ 500 ms) is unreachable by any engine design over these rules as authored. What it takes is a change on the rule side and a different maintenance granularity on the engine side, together:

- **Rule shape.** Repository rules written as global fixpoints over whole-repository symbol tables (`reachable`, `resolved_names`, windows partitioned by symbol across files) force either whole-repository state or whole-repository recompute. The same questions written against a maintained reference graph as *local* queries (is this definition referenced? by whom?) have bounded state and bounded delta. That is Jeremy's rule-authoring conversation, with X6's rewrites as the starting point and `DECLARED-FACTS.md` as the form.
- **Maintained core, bounded.** The only state that must be membership-maintained is the reference graph and the symbol table (X4, X6 agree). Today's generator holds them as 80M arrangement records (linker 61M, symbols 19M) because it maintains every intermediate of the linker's 79 stages. A dedicated maintained reference graph (compact adjacency, per-content summaries in, edges out, updated per changed module with the dependency rule X5 derived) is a few hundred MB for this corpus (X5's estimate: 0.9–1.6 GB with strings; ids instead of strings bring it under 1 GB) and is shareable across worktrees as base + delta. That is a design, not an optimisation, and it has not been built.
- **Per-worktree cost.** With rules as local queries over the maintained graph, per-worktree state is the worktree's delta of the graph plus its findings; the 40-worktree number becomes base + Σ deltas, which Spike B measured as proportional to divergence.

The next question is therefore not "which engine design" but "what does the reference graph as a maintained, compact, shared structure cost, and which of the 8 rules can be written as local queries over it". That is one experiment on the engine side (the graph) and one on the rule side (the rewrite), and it is the conversation to have with Jeremy before either is built.
