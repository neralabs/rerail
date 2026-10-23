# Phase 3, P3a: the cross-file memory model (M1 full maintenance compacted, M2 hybrid)

Status: **in progress** (first publication, 2026-10-23 15:40; numbers below are measured, rows marked *pending* are running). Branch `Moti-Velz/facts-engine-p3a-memory` (M2, baseline, scripts), sub-branch `Moti-Velz/facts-engine-p3a-m1` (M1, by the M1 subagent). Runs: `~/rerail-bench/facts/phase-3/p3a-memory/runs/<timestamp>-<label>/`. Every run: 40 worktrees, the 5 running plans (`specs/spec-replay-all5.json`: required-contracts, quality-project-taxonomy, state-flow-project-1, exported-families, unreachable-internal-symbols), S3's replay `seed-5eedb/events.jsonl`, single unlocked runs. Counts first; RSS/footprint and milliseconds are secondary (load 20–25 during these runs: three slices share the machine).

## Decision table

| Model | Resident after load (40 wt, 5 plans) | Resident after 300-event prefix | Per-edit p95 (single-file) | Exactness (prefix) | Run dirs |
|---|---|---|---|---|---|
| M0: current full maintenance (baseline) | 146M arranged records; RSS 27.9 GB, footprint 26.6 GB | 171.1M records; RSS 28.5 GB, footprint 32.6 GB | 77 ms (FACTS_PROFILE on; S3's locked protocol: 62 ms) | 0 differences (S3 wave 6) | `20261023-140025-baseline-300-prof` |
| M1 after steps 1–2 (one presence arrangement; stages shared across plans) | 111.7M records; footprint 18.9 GB | 130.9M records; footprint 21.0 GB | 58 ms (step 1) | 0 differences (13 checks / 18 worktree checks) | M1 notes: `…-144603-m1-s2-mem-300`, `…-145026-m1-s2-exact-300` |
| M2: hybrid (exports resident, everything else from scratch per affected edit, D1 skip) | 0 arranged records; resident state 149 MB export blocks + 113 MB interner + 9 MB membership + 11 MB results (+ ~140 MB compiled plans); RSS 77 MB under memory pressure (footprint *pending*) | *pending* | per affected edit = Σ plan runs: unreachable-internal-symbols 32–34 s, exported-families 2.4–2.6 s, the other three ≤ 10 ms; D1 skips 122 of 1,050 single-file edits for those two plans and every edit for state-flow-project-1 / quality-project-taxonomy, 984 for required-contracts | initial check 0 differences (batch and generator dataflow from scratch); prefix *pending* | `20261023-145937-m2-all5-300-exact` (running), smoke `…-141240-m2-smoke-all5`, `…-144859-m2-smoke-light-ev30` |

Reading so far: neither model reaches G3a.1 and G3a.3 together. M1 keeps the latency and is 75× over the memory budget after two of its seven steps. M2 is near the memory budget and 5–70× over the latency budget on the two global plans (dead code, exported families). See § Trade-off for the per-stage cost of moving between them.

## 1. Baseline (M0), attribution (S3 § 15.3's method; `scripts/p3a_attrib.py`)

`greplay --plans none --no-oracle --real specs/spec-replay-all5.json --events … --limit 300`, `FACTS_PROFILE=1`. At the end of the prefix: 171.1M arranged records.

| By plan | Records | | By role | Records |
|---|---|---|---|---|
| unreachable-internal-symbols | 139.45M (linker `views/*` 71.66M) | | keyed arrangements `by` | 72.77M |
| exported-families | 31.63M | | fixpoint join arms `iter-join-l` | 29.33M |
| required-contracts | 0.026M | | `distinct` presence in + out | 15.97M + 15.95M |
| the other two | 8 | | fixpoint presence in + out | 7.71M + 7.28M |
| | | | group out + in | 11.71M + 3.19M |
| | | | semijoin presence in + out | 3.06M + 3.05M |

Interner 3.30M strings / 629 MB; exported-row cache 670 MB.

## 2. M2: what was built

`facts-b hreplay` (`spikes/facts-b/src/gen/{batch,hybrid}.rs`):

- **Resident state**: per-content exported relations as blocks, one per (export, path, content), 32 bits per value when every value's word fits (all 48,518 blocks at load did). Blocks are hash-consed on their exact rows across exports, programs and paths, reference-counted by the worktrees holding the (path, content), and freed when none does. Per worktree: membership (path → content) and each plan's last result (findings + refusals).
- **Rule logic**: `batch::eval`, a set-at-a-time executor of the same normalised plan graph (`norm::Graph`) the generator lowers, from scratch over the worktree's members. Same per-record logic (`lower::{map_row, key_of, agg_logic, window_logic, distinct_prefix_logic, affinity}`), hash joins on the plan's keys, semi/antijoins by key sets, group/window/DISTINCT-prefix per key, semi-naive set fixpoint over hash indexes, the generator's refusal records and build-time refusals. Intermediate relations are dropped after their last consumer.
- **Workers in flight**: every plan run (and every check) runs in a forked child, so its relations, computed strings and numbers never reach the parent. The parent's resident size is the model's state; the child's peak is reported separately (PHASE-3 § P3c counts workers in flight separately). `--workers N` keeps N runs in flight; each child sees the state at its fork, so runs for successive events are independent.
- **D1** (REWRITES.md, facts-b table, mutant 60): an event skips a plan unless some export it reads has a different block before and after at a changed path. Exact because a plan is a function of its leaves' multisets (I6/I7; order dependence is a runtime refusal) and blocks are hash-consed on exact rows. Tests `hybrid::tests::{d1_blocks_are_hash_consed_on_their_rows, d1_affected_iff_a_read_block_differs}`; `facts-b-unit` now runs `tests::` (norm + hybrid); T0 kills mutant 60.
- **Exactness checks** (`--check batch|engine`): stored results against a from-scratch batch run of the same plans, and with `engine` also against the generator's dataflow over the members, base records only (S3's second dataflow), so the executor itself is checked against the incremental engine's semantics.

Measured so far:

| Item | Value | Run |
|---|---|---|
| batch ≡ generator from scratch, worktree 1, all 5 plans | 0 differences (findings and refusals) | `…-141240-m2-smoke-all5` |
| light plans, 30 events, engine check every 5th + sweeps | 0 differences, 8 engine checks | `…-144859-m2-smoke-light-ev30` |
| from-scratch per worktree (p50 / p95 over 40) | unreachable-internal-symbols 32.4 / 43.0 s; exported-families 2.60 / 3.95 s; state-flow-project-1 6 / 10 ms; required-contracts 1.4 / 2.0 ms; quality-project-taxonomy 0.25 / 0.36 ms | `…-141240-m2-smoke-all5` |
| work per dead-code run (counts, mean over 40) | 50.8M rows out, 3.9M exported rows read, 18.3M fixpoint derivations, 860 rounds, 19.8M index rows, 20.3M probes | same |
| where dead code's 34 s go (one run, exclusive seconds by stage) | linker `views/*` ≈ 25 s (`link_reach` 15.8, `name_sub` 3.1, `link_kid` 1.3, `link_form` 0.8, …); `reach_edges` 1.9, `reachable` 1.7, `imports` 0.7, `public_methods` 0.35 | `…-144248-m2-prof-w1` |
| D1 skip rate on the full replay's 1,050 single-file edits (offline, export equality) | dead code and exported-families: 122 skipped (110 whitespace, 11 reverts, 1 import); required-contracts 984; state-flow-project-1 and quality-project-taxonomy 1,050 | `d1-skip-offline.json` |

## 3. Resume notes

- M2 300-event exactness run (`--check engine --check-every 10 --sweep 2 --workers 4`) running; next is the M2 full replay (`--check batch --check-every 50 --sweep 2 --workers 8`) for G3a.2–3.
- M1 subagent continues with steps 4–7 and its full-replay row.
- Trade-off table: `scripts/p3a_tradeoff.py <greplay FACTS_PROFILE run> <hreplay run>`.
