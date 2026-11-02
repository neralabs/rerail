# T12 notes: dead code across files on `propagate` (Y1, Y3, E3 ported)

Implementation track T12. Branch `jeremybarnes/final-rerail-shape-facts-crossfile` from
`fa786cdb6`. Source **M** = `Moti-Velz/facts-engine@52b43e4ab` (read-only reference).

## Conventions

- Runs are under `~/rerail-bench/final-shape/crossfile/runs/` (below, `runs/`). Each holds
  `COMMIT.txt`, `load-start.txt` and `load-end.txt` (`sysctl -n vm.loadavg`), `stdout.txt`,
  `stderr.txt`, `results.json`, `events.json`. Driver: `~/rerail-bench/final-shape/crossfile/run.sh`.
- Every number comes from one run, on an 18-core Apple machine shared with other jobs; the
  1-minute load is given with each latency. Counts are deterministic (identical in every run).
- **M** = measured here; **[unverified]** = not run here.

## 1. What moved

| From M (`spikes/facts-b/src/`) | To (`native/src/code_rule_dsl/facts/`) | Kept | Left out |
|---|---|---|---|
| `lattice/{round,cols,maint}.rs` | `lattice/` | the operator, CSR, mapped columns, `min_level`/`set`/B/F maintenance, unit tests | — |
| `lattice/build.rs` | `lattice/build.rs` | `enumerate` | E1's column writer and timed runs |
| `lattice/e3.rs` | `deadcode/reach.rs` | `Base`, `Ext`, the rows step, `create`, `check`, `memory` | `set` (DRed, B/F) in the path, the driver |
| `y2/{dc,facts}.rs`, `y2/mod.rs` | `y2/` | `facts` whole; `dc`'s `succ`, scoping, roots, findings; `Dc` from scratch (oracle); the dump reader | `ef`, Y2's maintained reachability (`prepare`/`finish`/`pred`), Y2's drivers |
| `y3/{input,sym,wire,world,mod}.rs` | `y3/` | the symbol table, Y1 → `Refs` delta wiring, the layered world | M2 (`m2.rs`, facts-b `gen/`, `rerail_facts`), `exported-families`, `CrossFile`, `replay.rs` |
| `ygraph/**`, `lmap.rs` | already here (`9ffdb048`, `951514fe`) | equal at `52b43e4ab`; parity became a test (`f41fbb26`) | — |

Copies are verbatim (`74f5ea04`, `68a062f0`, `fc39d366`), adaptations one per commit; rows in
`provenance.md`. facts-b-only dependencies: `crate::mutant::on` → a test-only seam
(`lattice::mutant`); `rerail_facts::sql::value::{like, trim}` → copied into `y3/input.rs`;
`crate::replay::rss` → already `ygraph::rss`; footprint → `prof::footprint_mb`.

## 2. The seam (`facts::deadcode`)

Per worktree scope `wtNN`: the observer binds `{scope}#path:{path}` and `{scope}#tree:` (the
membership's set digest); five derivations follow, each recorded with its read set as
`derivation_input` rows:

| Output | Reads | Producer | Output digest |
|---|---|---|---|
| `#xref:` | `#tree:` | Y1 `apply`, edge delta (`wire::xref_step`) | `Refs` rows |
| `#symbols:` | `#tree:` | symbol table (`wire::symbols_step`) | table rows |
| `#deadcode.graph:` | xref, symbols | Y2 scope + inputs applied, CSR rows at the scope (`reach::rows`) | edges and roots, by state |
| `#deadcode.reached:` | graph | `min_level`: `recompute` (E1's fixpoint from the roots on the current CSR) or `maintained` (E3's levelled re-check) | reached states |
| `#deadcode.findings:` | xref, symbols, reached | Y2's `finding` at the scope's definitions and the readers of toggled annotated states | finding rows |

- Every digest is a set digest (SHA-256 per element by text, lanes summed), maintained by the
  step's delta: unchanged output ⇒ unchanged binding ⇒ early cutoff, exactly.
- Rows stay in the producers' layered state (Y3's `Top` over base + 8 cluster overlays, E3's
  `LWt` over the worktree-1 base); a step hands its delta to the next wave through the producer,
  as `Families` hands tapes to the core wave.
- The mode is chosen at run time (`reach::Mode`); both produce one digest (fast test, oracle test).

## 3. Exactness

| Check | Result | Run |
|---|---|---|
| Full replay, 40 loads + 1,111 events, check every load and event (forked): rows = Y2 `succ`, transpose, roots, levels = E1's fixpoint on the maintained CSR = Y2 from scratch (states and levels), findings = Y2 from scratch, every output digest = from scratch; in the parent every bound name = the producer's digest | **maintained: 1,151 checks, 0 differences; 1,151 binding checks, 0** | `runs/20261102-093438-exactness-maintained-store` |
| Same, recompute | **1,151 / 0; 1,151 / 0** | `runs/20261102-094028-exactness-recompute-store` |
| Oracle tree `2b1504adc` (Y2's stage dump): lattice findings = dataflow `dc__findings` = Y2 from scratch | **4,931 = 4,931 = 4,931**, both modes, same findings digest, `check` clean | `runs/20261102-092827-oracle` |
| Y1 parity on the oracle tree | 145,431 = native's 145,431, 0 missing, 0 extra | `runs/20261102-093015-y1-parity` |
| Unit: `lattice` (12, incl. 1–8 thread determinism), mutants 75/77/79 killed, `y2::dc` (3), digest (1), seam (1: store and overlay, both modes, recompute at 1 and 3 threads) | pass | `cargo test --lib --features metal-tests facts::` |

## 4. Per-edit latency

Single-file edits (`edit`, `new_worktree_edit`), n = 1,050, ms, p50 / p95 / p99 / max.

| | maintained, store | maintained, overlay | recompute, store | recompute, overlay |
|---|---|---|---|---|
| Y1 apply | 0.182 / 1.058 / 2.024 / 6.67 | 0.172 / 1.058 / 1.985 / 6.30 | 0.196 / 1.064 / 2.255 / 6.16 | 0.175 / 0.961 / 1.813 / 5.98 |
| edge delta (`Refs`) | 0.019 / 0.279 / 0.707 / 2.87 | 0.018 / 0.283 / 0.694 / 2.96 | 0.020 / 0.280 / 0.719 / 2.79 | 0.017 / 0.260 / 0.671 / 2.57 |
| symbol table | 0.152 / 0.969 / 1.920 / 20.0 | 0.145 / 0.926 / 1.918 / 19.5 | 0.164 / 0.972 / 1.950 / 19.6 | 0.145 / 0.888 / 1.917 / 17.6 |
| Y2 scope | 0.009 / 0.158 / 0.280 / 0.71 | 0.008 / 0.160 / 0.279 / 0.67 | 0.010 / 0.169 / 0.300 / 2.12 | 0.008 / 0.152 / 0.260 / 0.64 |
| Y2 inputs applied | 0.026 / 0.251 / 0.499 / 2.03 | 0.025 / 0.239 / 0.466 / 1.88 | 0.029 / 0.243 / 0.486 / 2.25 | 0.025 / 0.216 / 0.440 / 1.89 |
| CSR rows | 0.037 / 0.681 / 1.546 / 4.17 | 0.034 / 0.668 / 1.663 / 4.25 | 0.038 / 0.695 / 1.647 / 4.25 | 0.033 / 0.628 / 1.471 / 3.70 |
| values | 0.000 / 0.004 / 0.013 / 0.51 | 0.000 / 0.005 / 0.013 / 0.52 | 3.106 / 3.669 / 4.104 / 5.05 | 2.829 / 3.112 / 3.331 / 3.60 |
| **scope + rows + values (E3's H7 sum)** | **0.050 / 0.852 / 1.664 / 4.46** | **0.047 / 0.806 / 1.698 / 4.53** | **3.183 / 4.190 / 5.081 / 9.00** | **2.882 / 3.612 / 4.339 / 6.41** |
| findings | 0.002 / 0.016 / 0.036 / 0.07 | 0.002 / 0.015 / 0.034 / 0.08 | 0.002 / 0.017 / 0.032 / 0.08 | 0.002 / 0.014 / 0.029 / 0.08 |
| digests | 0.001 / 0.013 / 0.051 / 0.38 | 0.001 / 0.012 / 0.047 / 0.38 | 0.002 / 0.015 / 0.051 / 0.40 | 0.001 / 0.013 / 0.052 / 0.36 |
| producers, summed | 0.474 / 3.138 / 6.516 / 22.5 | 0.463 / 3.198 / 6.408 / 21.9 | 3.561 / 6.457 / 9.347 / 24.7 | 3.196 / 5.603 / 8.409 / 23.1 |
| propagate + SQLite + observer | 0.220 / 0.435 / 8.329 / 10.2 | 0.184 / 0.302 / 8.880 / 10.3 | 0.269 / 0.479 / 8.820 / 10.4 | 0.196 / 0.290 / 9.453 / 11.7 |
| **push total** | **0.710 / 4.462 / 9.652 / 22.8** | **0.647 / 3.987 / 10.26 / 22.2** | **3.858 / 7.580 / 13.10 / 25.6** | **3.402 / 6.567 / 13.27 / 23.9** |
| 1-minute load (start → end) | 20.6 → 22.9 | 22.9 → 19.4 | 19.4 → 15.0 | 15.0 → 9.2 |
| run | `20261102-094712-workload-maintained-store` | `-094933-workload-maintained-overlay` | `-095022-workload-recompute-store` | `-095113-workload-recompute-overlay` |

Against M (same events, same inputs): E3's H7 sum (scope + rows + `min_level`) was 0.042 / 0.804
/ 1.788 / 20.5 and 0.039 / 0.754 / 1.677 / 18.1 ms (`lattice-e3/runs/20261030-093829-perf-w1-a`,
`-093922-perf-w1-b`, load 6.9 and 8.7–9.1). Here, maintained: 0.050 / 0.852 / 1.664 / 4.46 (store)
and 0.047 / 0.806 / 1.698 / 4.53 (overlay), at load 17–30. The max falls from 20.5 to 4.5 ms
because the scope no longer adds whole nodes for reached dispatch states (§6.3): recomputed ids
max 34,700 here against 63,733 in E3; p50/p95/p99 counts are E3's (24 / 808 / 3,386). Y3's
wired path was 0.53 / 3.76 / 8.62 ms total (`y3-wire/runs/20261025-210541-full-perf`, which
also ran `exported-families` and M2); the push here is 0.65–0.71 / 4.0–4.5 / 9.7–10.3 ms with
the derivation graph and its writes included.

Counts per single-file edit (p50 / p95 / p99 / max, every run): waves 3 / 4 / 4 / 4; re-runs
5 / 6 / 6 / 6; changed outputs 3 / 6 / 6 / 6; ids recomputed 24 / 808 / 3,386 / 34,700;
ids invalidated (maintained) 0 / 9 / 48 / 16,268, (recompute) 440,454 / 451,389 / 451,406 /
451,408; value rounds (maintained) 0 / 6 / 12 / 20, (recompute) 21 / 24 / 24 / 24; reachability
toggled 0 / 6 / 34 / 128.

### 4.1 Re-measured on a quiet machine (decider, 2026-11-02, `a08a3d296`)

Same workload, each run started at 1-minute load < 3 (`load-start.txt` 2.8–3.0, end 3.0–4.0). ms, p50 / p95 / p99 / max, n = 1,050.

| | maintained, store | maintained, overlay | recompute, store | recompute, overlay |
|---|---|---|---|---|
| scope + rows + values (E3's H7 sum) | 0.050 / **0.799** / 1.733 / 4.64 | 0.051 / 0.828 / 1.826 / 4.67 | 3.337 / 4.344 / 5.387 / 8.33 | 3.428 / 4.495 / 5.416 / 9.11 |
| values | 0.001 / 0.004 / 0.015 / 0.48 | 0.001 / 0.004 / 0.016 / 0.73 | 3.247 / 3.821 / 4.016 / 4.94 | 3.344 / 3.916 / 4.188 / 6.50 |
| Y1 apply | 0.178 / 1.064 / 1.932 / 6.93 | 0.178 / 1.078 / 2.100 / 6.45 | 0.201 / 1.096 / 2.156 / 6.83 | 0.206 / 1.119 / 2.156 / 7.05 |
| symbol table | 0.150 / 0.933 / 2.045 / 19.71 | 0.152 / 0.929 / 2.078 / 20.82 | 0.165 / 1.007 / 2.065 / 21.16 | 0.173 / 0.970 / 2.124 / 21.36 |
| propagate + SQLite + observer | 0.190 / 0.343 / 7.751 / 9.69 | 0.196 / 0.329 / 8.955 / 11.01 | 0.254 / 0.418 / 8.508 / 9.28 | 0.266 / 0.400 / 9.391 / 11.18 |
| **push total** | **0.665 / 4.308 / 9.218 / 23.6** | **0.686 / 4.155 / 9.907 / 23.55** | **3.994 / 8.302 / 12.869 / 29.14** | **4.146 / 7.974 / 13.763 / 29.10** |
| footprint end / peak (MB) | 4,043 / 5,264 | 4,004 / 5,261 | 3,991 / 5,239 | 3,959 / 5,249 |
| run | `runs/20261102-102505-quiet-workload-maintained-store` | `-102831-…-maintained-overlay` | `-102958-…-recompute-store` | `-103047-…-recompute-overlay` |

- Against E3 on facts-engine (0.804 / 0.754 ms p95, load 7–9): maintained 0.799–0.828 ms p95, the same within run-to-run spread. The port costs nothing on the lattice side; the derivation graph adds 0.33–0.42 ms p95 per edit.
- Recompute (03 §2's rule) against maintained, same events: values 3.8–3.9 vs 0.004 ms p95 (≈ 950×); whole push 8.0–8.3 vs 4.2–4.3 ms p95 (1.9×), because Y1 and the symbol table (≈ 2 ms p95 together) are paid in both modes.
- Exactness re-run at `a08a3d296` (after the clippy refactor of `reach`): full replay, check every load and event, maintained 1,151 checks / 0 differences, 1,151 binding checks / 0 (`runs/20261102-103159-final-exactness-maintained-store`); recompute the same (`-103721-final-exactness-recompute-store`).
- His slice is unaffected: S8 on this branch against `fa786cdb6` (both at load 2.6–3.1, passes 1–2): CPU cold 5.46–5.48 vs 5.36–5.40 s CPU, Metal cold 1.69–1.70 vs 1.59–1.61 s wall, edits 0.23–0.25 vs 0.21–0.26 s, keys 546/547 on both, footprints equal (`runs/20261102-102307-s8-a08a3d296`; baseline `~/rerail-bench/final-shape/baseline/20261102T084640-s8-fa786cdb6`). The ~2–5% is within the two runs' load difference [INFERENCE]; the slice path runs no code this branch added.

## 5. Footprint against the 500 MB budget

`runs/20261102-094712-workload-maintained-store` (`prof::footprint_mb`, MB): start 5; per-content
exports loaded (Y1 summaries and symbol blocks of every content in the 3.7 GB export) 1,706; Y3's
base layer and 8 overlays 2,661; worktree 1 and the lattice base 2,681; 40 worktrees loaded
3,109; end of the replay (46 worktrees) 4,038; lifetime peak 5,281. The exactness runs agree
(3,918 / 3,883 at the end). **8× the budget.** About a third is the per-content inputs (which
the content store should serve); the layered Y3 state and the lattice layers are the rest (E3
§6, Y3 §3(a)).

## 6. Contradictions and deviations

1. **03 §2, "No DRed and no level witness"**: `maintained` is E3's `min_level`, a level witness.
   It is the fast mode (values p95 0.004 ms). `recompute` follows 03 §2 and costs 3.1–3.7 ms p95
   per edit (440K ids, 21–24 rounds). Decision needed (O37).
2. **03 §2 with §14's waves, no level order**: the findings derivation reads both Y2's inputs and
   the reached set, so an edit that changes reachability runs it twice (re-runs 6 for 4 waves).
   Each run is correct for the bindings it read; the second is the counted spurious work (O38).
3. **Y2's scope without the reached set**: M's `prepare` added the reached dispatch states of a
   dirty node and whole nodes for the reached states of a changed definition's layer; here a
   dirty node stands for all its states and the layer for all its states. Exact (1,151 checks)
   and the reason the max is 4.5, not 20.5 ms.
4. **LATTICE.md L10** (two levels, base and worktree): unchanged from E3; Y3's state is layered
   three deep (base, cluster, worktree) and the lattice two (worktree-1 base, worktree): two
   layerings of one worktree (O36).
5. **xref/symbols read the tree name, not each path**: the producers take the observer's path
   delta (`Pending`), so `observe::recover` cannot recompute them after a crash (O39).

## 7. Not done

- Benchmarks at load below 8 (the machine was at 9–30); `recompute` with threads > 1 measured only
  for exactness in the fast test.
- Cluster overlays for the CSR, dispatch keyed by class, dense ids at the source (E3 §8).
- Feeding the families from this crate's own producers (tapes, `facts::core`): they read M's X6
  exports (O40).
- The facts daemon, M2, `exported-families`, Metal, the logic surface (non-goals).
