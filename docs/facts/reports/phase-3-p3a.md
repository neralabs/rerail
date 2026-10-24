# Phase 3, P3a: the cross-file memory model (M1 full maintenance compacted, M2 hybrid)

Status: **done, kill clause**. Neither model reaches G3a.1 and G3a.3 together. The floors and attribution of both are below, with the smallest set of relations that must be membership-maintained for exactness and the recommendation.

- Branch `Moti-Velz/facts-engine-p3a-memory`. M1 was built on the sub-branch `Moti-Velz/facts-engine-p3a-m1` (`f114a70d6`, by the M1 subagent) and merged here (`645d98976`).
- Runs are in `~/rerail-bench/facts/phase-3/p3a-memory/runs/<timestamp>-<label>/`. Specs are in `…/p3a-memory/specs/`.
- Every run uses 40 worktrees and the 5 running plans (`spec-replay-all5.json`: required-contracts, quality-project-taxonomy, state-flow-project-1, exported-families, unreachable-internal-symbols) on S3's replay (`seed-5eedb/events.jsonl`: 1,111 events, 1,000 single-file edits).
- Every run was single and unlocked. Counts come first; resident sizes and milliseconds are secondary. Three slices and a VM shared the machine (load average 8–65), so milliseconds are inflated, by about 2× at load 60.

## Decision table

Resident sizes are of the model's process. "Heap" is the bytes in use over every malloc zone, after returning free pages. M2 runs every plan evaluation in a forked worker; the workers' peak RSS is reported on its own line, as PHASE-3 § P3c counts workers in flight separately.

| Model | G3a.1: resident after load (target ≤ 250 MB) | G3a.2: resident at edit 1,000 (target ≤ 300 MB, no unbounded growth) | G3a.3: single-file edit p50 / p95 / p99 (target p95 ≤ 500 ms, p99 ≤ 2 s) | G3a.4: exactness (0 differences, 0 runtime refusals live) | Run dirs |
|---|---|---|---|---|---|
| M0, the current full maintenance (baseline) | 146M arranged records; footprint 26.6 GB, RSS 27.9 GB | S3 protocol: 252.8M records, 42.5 GB | S3 locked: 17 / 62 / 194 ms. This 300-event run, profiled: 30 / 77 / 209 ms | 0 differences (S3 wave 6) | `20261023-140025-baseline-300-prof`; S3 `protocol-all5-p1..3` |
| **M1**, full maintenance compacted (steps 1, 2 and 4 implemented; 3 needed no code; 5 not implemented; 6 and 7 measured) | **112.36M records; footprint 18.4 GB** (RSS 24.7 GB). **Fails, 74×** | 195.3M records; footprint 32.5 GB. Fails | **39 / 131 / 442 ms** (n = 1,050). **Passes** | **0 differences**: 21 checks, 26 worktree checks × 130 stage outputs, 300-event prefix; 0 refusals live at the end of the full replay | `…-164507-m1-final-full`, `…-154505-m1-s4-exact-stages-300` |
| **M2**, hybrid: exports resident, every plan from scratch on an affected edit, D1 skip | **heap 325 MB**, footprint 470 MB (393 MB at edit 100, once the allocator settles). Fails by 30 % with the exports in the process; 85 MB without them (§ 4) | **heap 347 MB, footprint 429 MB**. Flat: +22 MB over 1,000 edits, growing with the contents held (7,118 → 7,950). After the final 111 events (50 commits, 6 new worktrees; 15,784 contents held): heap 547 MB | **38.8 / 48.8 / 80.6 s** (n = 1,050; all five plans in one run; contended). Uncontended: dead code 32 s, exported-families 2.6 s per run. **Fails, about 100×** | **0 differences.** 300-event prefix: 31 checks, 36 worktree checks, each against the batch run **and** the generator's dataflow from scratch. Full replay: 23 checks, 44 worktree checks (batch). 0 refusals live at the end | `…-173208-m2-all5-full`, `…-145937-m2-all5-300-exact` |
| M2 workers in flight | — | peak RSS per worker 0.6 GB p50, 3.3 GB max (6.3 GB on the full replay, shared pages included) | — | — | same |

Reading: M1 keeps per-edit latency and exactness, but its floor is about 15 GB (§ 3). M2 is within 30 % of the memory budget and exact, but a dead-code edit costs a 32 s recompute.

The three small plans (required-contracts, state-flow-project-1, quality-project-taxonomy) meet both budgets under M2: they recompute in ≤ 10 ms, hold no maintained state, and D1 skips most edits for them. The two global plans (unreachable-internal-symbols, exported-families) meet neither budget in either model.

## 1. Baseline (M0): attribution

Method: S3 § 15.3's, via `scripts/p3a_attrib.py`. The run: `greplay --no-oracle --limit 300` with `FACTS_PROFILE=1`. At the end of the 300-event prefix the dataflow holds 171.1M arranged records.

| By plan | Records |
|---|---|
| unreachable-internal-symbols | 139.45M, of which the linker (`views/*`) 71.66M |
| exported-families | 31.63M |
| required-contracts | 0.026M |
| the other two plans | 8 |

| By role | Records |
|---|---|
| keyed arrangements `by` | 72.77M |
| fixpoint join arms `iter-join-l` | 29.33M |
| `distinct` presence in + out | 15.97M + 15.95M |
| fixpoint presence in + out | 7.71M + 7.28M |
| group out + in | 11.71M + 3.19M |
| semijoin presence in + out | 3.06M + 3.05M |

Interner: 3.30M strings, 629 MB. Exported-row cache: 670 MB.

## 2. M1: full maintenance, compacted

Each step was cumulative and measured on the 300-event prefix; each step's exactness ran on the from-scratch second dataflow.

| Step | Records at load / end of prefix | Footprint at load / end (MB) | Interner | Exactness |
|---|---|---|---|---|
| baseline | 145.9M / 171.1M | 26,580 / 32,627 | 3.29M / 600 MiB | — |
| 1. One presence arrangement: `presence_total` for DISTINCT, `semi_total` for semijoins, both on a totally ordered time | 131.4M / 153.0M (presence out 26.3M → 8.1M) | 26,147 / 29,557 | same | 0 (13 checks / 18 worktree checks) |
| 2. Stages shared across plans: R37, mutants 50/52/54. 9 exports shared between exported-families and dead code; merged graph 2,221 nodes, against 2,366 apart | 111.7M / 130.9M | 18,912 / 20,962 | same | 0; findings equal with sharing off (`FACTS_SHARE=0`) on 5 outputs × 46 worktrees |
| 3. Linker as shared base | no code change: `views/*` (54.0M) is read only by dead code, and step 2 already computes a stage once | | | |
| 4. One interner, swept by epoch: mutant 56 | 112.4M / 131.7M | 23,351 / 21,895 | 3.15M / 353 MiB | 0 (130 stage outputs) |
| 5. Compact records | not implemented. Blocker: a columnar layout in DD 0.25 hands operators row references, which changes every closure in `lower.rs`. Estimated saving −3.1 GB (24-byte row headers) | | | |
| 6. Trace compaction | traces compact. Loading the end memberships from scratch gives 129.4M against 131.7M replayed (1.8 %), so the growth is divergence, not merge slack | | | |
| 7. Exported rows not cached | −568 MB, but p95 rises 103 → 282 ms (retractions need the rows), so not adopted | | | |

The full replay (`…-164507-m1-final-full`), attributed at load (112.36M records):

| By plan | Records |
|---|---|
| dead code alone | 82.88M |
| nodes shared by exported-families and dead code | 21.20M |
| exported-families alone | 8.27M |

| By role | Records |
|---|---|
| `by` | 51.9M |
| fixpoint join arms | 24.8M |
| presence counts | 20.3M |
| group out | 6.0M |
| fixpoint presence out | 5.8M |
| group in | 2.7M |

The linker holds 54.0M of the total.

**M1's floor**: about 112M records at about 130 B each in rows alone (10.5 values per row, 1.33G values), which is about 15 GB. Meeting 250 MB would take 2.2 B per record. Even with every remaining item (narrow arrangements to the read columns, a columnar container, fewer keyed arrangements, a totally ordered fixpoint) the floor stays above 1 GB.

## 3. M2: the hybrid

### 3.1 What it is

`facts-b hreplay`, in `spikes/facts-b/src/gen/{batch,hybrid}.rs`.

**Maintained state, and nothing else:**
- Per-content exported relations as blocks, one per (export, path, content):
  - 32 bits per value when every value's word fits. All 90,086 blocks fit.
  - Hash-consed on their exact rows across exports, programs and paths.
  - Reference-counted by the worktrees holding the (path, content), and freed when none does.
- Per worktree: its membership (path → content) and each plan's last result (findings and refusals).

**Rule logic.** `batch::eval` runs, from scratch over a worktree's members, the same normalised plan graph that the generator lowers. Since R37 that is one graph for the 5 plans; a run evaluates only the outputs of the plans it runs, and a shared node once. It uses:
- the generator's per-record logic: `lower::{map_row, key_of, agg_logic, window_logic, distinct_prefix_logic, affinity}`;
- hash joins on the plan's keys, and semi/antijoins by key sets;
- per-key groups, windows and DISTINCT prefixes;
- a semi-naive set fixpoint over hash indexes;
- the generator's refusal records, attributed to plans by the node that raised them, as `lower::build` does.

An intermediate relation is dropped after its last consumer.

**Workers.** Every run happens in a forked child, as does every check. The child's relations, computed strings and numbers die with it. `--workers N` keeps N runs in flight; a child sees the state as of its fork, so runs for successive events are independent.

**D1** (REWRITES.md, facts-b table, mutant 60): an event skips a plan unless, at some changed path, a block that the plan's own outputs read differs before and after. It is exact for two reasons:
- a plan is a function of its leaves' multisets (I6, I7); order dependence is a runtime refusal, and the refusal is a function of the same multisets;
- blocks are hash-consed on their exact rows.

D1 has two tests in `hybrid::tests`, and `facts-b-unit` now runs `tests::`. T0 kills mutant 60.

**Checks.**
- `--check batch` compares the stored results with a fresh batch run.
- `--check engine` also compares them with the generator's dataflow over the members (base records only), so the executor is checked against the incremental engine's semantics.

### 3.2 Attribution

From `…-173208-m2-all5-full`. "Heap" is live bytes; RSS is not used because it drops below the live size when pages are compressed under memory pressure.

| Component | At load | At edit 1,000 | End of replay |
|---|---|---|---|
| export blocks (rows) | 149 MB (9.39M rows, 48,518 blocks) | 167 MB (10.52M rows) | 252 MB (15.98M rows) |
| export strings (interner) | 113 MB (1.20M strings) | 113 MB (1.22M) | 210 MB (2.07M) |
| compiled plans, export-file index | 62 MB | 62 MB | 62 MB |
| membership | 9 MB (224K entries) | 10 MB | 10 MB |
| results (findings + refusals) | 11 MB (108,614 rows) | 14 MB | 15 MB |
| contents held | 7,118 | 7,950 | 15,784 |
| **heap in use** | **325 MB** | **347 MB** | **547 MB** |

The interner is 1.2M strings because export rows carry names. Strings computed by an evaluation die with its worker; M0 holds 3.3M strings and M1 3.15M.

### 3.3 Per-edit cost

All five plans run in one evaluation per affected edit. D1 skipped 124 of 1,151 recomputes on the full replay.

The full replay used the graph's leaves for every plan, so it was conservative: it recomputed the small plans on every edit that touched the global ones. Per-plan leaves landed after it. On 60 events they skip state-flow-project-1 and quality-project-taxonomy on every event and required-contracts on 56 (`…-202032-m2-d1-plan-reads-light-ev60`, 12 engine checks, 0 differences). Offline, on the full replay's 1,050 single-file edits (`d1-skip-offline.json`):

| Plan | Edits D1 skips |
|---|---|
| state-flow-project-1, quality-project-taxonomy | 1,050 (all) |
| required-contracts | 984 |
| dead code, exported-families | 122 (110 whitespace edits, 11 reverts, 1 import) |

Work per affected edit (counts, mean per run of the 5 plans over 1,027 runs):
- 55.5M rows out
- 4.0M exported rows read
- 18.6M fixpoint derivations
- 863 rounds
- 20.8M index rows
- 22.5M probes

Time per run, one uncontended worktree, by exclusive stage:

| Plan | Total | Main stages |
|---|---|---|
| dead code | 34 s | linker `views/*` ≈ 25 s (`link_reach` 15.8, `name_sub` 3.1, `link_kid` 1.3, …); `reach_edges` 1.9; `reachable` 1.7; `imports` 0.7 |
| exported-families | 2.6 s | `traits` 0.8; `imports` 0.6; `public_methods` 0.3 |

Under contention (the 300-event run), the linker is 84 % of dead code's recompute (`tradeoff-300.json`).

**M2's floors.**
- **Memory.** The in-process floor is the exports: blocks plus their strings, 262 MB at load. Add plans, membership and results, 82 MB: about 345 MB. With the exports served by the content store (P3c's capped page cache), the model's own state is about 85 MB.
- **Latency.** The latency floor is the from-scratch work of the affected plans: 55M rows out per dead-code edit. Meeting a 500 ms p95 needs about 65× the current row-at-a-time executor (≤ 10 ns per output row), or less work.

## 4. Trade-off and the smallest membership-maintained set

`scripts/p3a_tradeoff.py` joins each stage's arranged records (M0's profile) with its batch recompute time (M2's profile). Choosing a downward-closed set S of stages to maintain gives:

| Plan | M2 (S = ∅) | M2L (S = the membership-only cone: `module_of` and the linker views) | M1 (S = all) |
|---|---|---|---|
| unreachable-internal-symbols | 0 records maintained; recompute 100 % (32 s uncontended) | 71.7M records (M0; 54.0M after M1's steps), about 7–11 GB; recompute 16 % (≈ 5 s) | 139.5M records (M0), 22 GB; recompute 0 |
| exported-families | 0; 2.6 s | no membership-only cone: same as M2 | 31.6M records (M0), 5 GB |
| the other three | 0; ≤ 10 ms | — | ≤ 26K records |

- **Smallest set that must be membership-maintained for exactness: none beyond membership itself.** M2 is exact on the full replay with nothing maintained except membership, the content-keyed exports (which are per content, not per membership) and the last results.
  - The membership-dependent relations (`module_of`, the linker's `views/*`, and every stage that joins rows of different files) are exact when recomputed from the members.
  - Maintenance buys latency, not exactness.
- **Smallest set for the latency budget.** Maintaining the membership-only cone (`module_of` and the linker) is not enough: dead code still needs about 5 s per edit. Reaching 500 ms also needs `reach_edges` and `reachable` maintained, which is M1 for dead code (≥ 82.9M records). exported-families needs its own stages maintained (2.6 s; `traits`, `imports`).

## 5. Recommendation

Adopt **M2** as the cross-file memory model.

- It is the only model within an order of magnitude of 250 MB: 325 MB heap at load, and 85 MB once the exports move to the content store.
- It is exact (0 differences on 80 worktree checks, 36 of them against the generator's dataflow) and flat over 1,000 edits (+22 MB).
- Plans whose recompute fits the latency budget meet G3a.3 as they stand: three of the five here.

For the two global plans, G3a.3 is a design decision for the decider; neither model meets it. The options:

1. **Deliver global cross-file findings asynchronously.** P3c already pushes cross-file findings with the membership epoch. Coalesce edits per worktree, so one recompute serves a burst of edits, and set a separate budget in seconds.
2. **Cut the work.** 84 % of the dead-code recompute is the linker over the whole worktree. A faster set-at-a-time executor (≈ 65× for 500 ms) is unlikely from tuning alone. Two structural paths:
   - a narrower rule shape (the reference graph resolved per content plus a small global resolution relation, for Jeremy);
   - a demand-scoped linker: references of the edited file, plus references into the names it defines. This is a demand rewrite with side conditions; none of the generic ones I could state applies to the linker's recursion as written.
3. **Keep M1 for the two global plans.** Not viable: its floor is about 15 GB against a 1 GiB total (G3c.2).

## 6. Gate rows

| Id | Target | M1 | M2 | Run dir |
|---|---|---|---|---|
| G3a.1 | ≤ 250 MB after load | 18.4 GB footprint, 112.36M records: fail | 325 MB heap (470 MB footprint; 85 MB without exports): fail, near | `…-164507-m1-final-full`; `…-173208-m2-all5-full` |
| G3a.2 | ≤ 300 MB at edit 1,000, no unbounded growth | 32.5 GB, 195.3M records: fail | 347 MB heap (429 MB footprint), +22 MB over 1,000 edits: fail, flat | same |
| G3a.3 | p95 ≤ 500 ms, p99 ≤ 2 s | 131 / 442 ms: pass | 48.8 / 80.6 s: fail. The three small plans: ≤ 10 ms | same |
| G3a.4 | 0 differences; 0 runtime refusals live | 0; 0 live: pass | 0 (prefix: batch + engine; full: batch); 0 live: pass | `…-154505-m1-s4-exact-stages-300`; `…-145937-m2-all5-300-exact`, `…-173208-m2-all5-full` |

Runtime refusals counted: the 7 contents of the replay without retained source (S3 § 6) raise `no exports for …` when acquired, 35 times in total. None is live at the end.

## 7. Checks

- **T0** (`just facts-t0`) passed on every commit. On the merged tree: 3/3 checks, 20/20 mutants killed, including 50, 52, 54, 56 (M1) and 60 (D1) (`loop/runs/…t0`).
- **T1** was run by the M1 subagent on its branch (5/5). It was not rerun after the merge: the decider ruled to skip T1 and any re-measurement that does not change the row.

## 8. Interface changes

- **facts-b `hreplay`.**
  - Flags: `--real SPEC --events F [--limit N] [--check none|batch|engine] [--check-every N] [--sweep K] [--no-skip] [--workers N] [--worktrees N]`.
  - `results.json`: `plans[]` (runs, skipped), `runs[]` (per run label: ms, counts, stage profile), `attribution_loaded|end`, `rss[]` with `heap_in_use_mb`, `workers`, `exactness`.
- **`gen::batch::eval(plans, graph, src, files) -> Outcome`**: per-plan results, `Counts`, stage profile.
- **`gen::hybrid`**: `d1_affected`, the `Exports` block store.
- **`Real::{programs, program_rows}`**; `Val::{to_bits, from_bits}`. The `lower.rs` helpers listed above are `pub(super)`.
- **The gate's `facts-b-unit`** runs `tests::` (`norm::tests` and `hybrid::tests`), in `facts-gate.py` and `facts-loop.py`.
- **M1, merged here**:
  - `lower::{presence_total, semi_total, counts, presence_out, register_traces, walk_rows}`;
  - `Built.{groups, node_plans, op_groups}`;
  - `norm::{Plans, Graph.owners}`, recursion variables by height;
  - `Real::global`, export digests, `FACTS_SHARE=0`;
  - the interner's `sweep` / `live` / `free` list, `val::{sweep, syms_len}`;
  - `FACTS_LOAD_AFTER=1`, `FACTS_SWEEP=0`;
  - new keys in greplay's `results.json`.
- **Registry**: R37 (mutants 50, 52, 54), mutant 56, D1 (mutant 60).

## 9. Resume notes

- Commits on `Moti-Velz/facts-engine-p3a-memory`:
  - `8ec07ed74`: M2, D1, scripts.
  - `ad34d55d7`: the first report.
  - `645d98976`: the M1 merge, plus M2 on the shared graph.
  - `3e110b016`.
  - This report, plus the per-plan D1 leaves.
- Not pushed.
- Scripts:
  - `spikes/facts-b/scripts/p3a_attrib.py RUN…`: M0/M1 attribution.
  - `p3a_tradeoff.py GREPLAY_PROFILE_RUN HREPLAY_RUN`: the per-stage trade-off.
  - Offline D1 rates: `~/rerail-bench/facts/phase-3/p3a-memory/d1-skip-offline.json`.
- Binaries used: `…/p3a-memory/facts-b-m2a` (the 300-event run, pre-merge), `facts-b-m2b` (the full replay), and the M1 binaries in `…/p3a-memory/m1-bin/`.
- Open, in order:
  1. **P3c wiring.** The daemon's `CrossFile::apply` needs M2 as a library. `hybrid::run`'s closures should become a struct: `load(spec, memberships)`, `apply(worktree, [(path, old, new)]) -> per-plan findings delta`, with exports served by the content store. P3cDaemon asked; a facts-b lib target is the preferred shape.
  2. **Rerun the full replay** with per-plan D1 leaves (latency for the small plans; the global ones are unchanged) and engine checks every 50.
  3. **Interner eviction for M2's export strings.** M1's sweep applies: mark the strings held by blocks and results.
  4. **Exports in the content store** (mmap, compressed blocks); M2's own state is then about 85 MB.
  5. **The global plans' latency** (§ 5): the decider's choice between async delivery, cutting the work, and a budget change.
