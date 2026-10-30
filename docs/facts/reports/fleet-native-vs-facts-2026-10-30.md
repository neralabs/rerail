# Native vs facts daemon: 40 worktrees, the data-layer rules (2026-10-30)

Controlled comparison of the native engine and the facts daemon on the same 40 checkouts of `nera-data-layer` (`~/rerail-bench/memory47/workspaces/01…40`, 4,643–4,831 per-file findings each), the same rules, the same 20 edits per worktree, one engine at a time on an otherwise idle machine.

Run: `~/rerail-bench/facts/compare/runs/20261030T112616-fleet40-p4`. Drivers: `benchmark/facts/fleet-ab/{run-fleet-ab.sh,run-native58.sh,analyse.py}`.

## Setup

| | Native | Facts daemon |
|---|---|---|
| Build | `rerail_engine.darwin-arm64.node`, release, sha256 `fbbaeeab16eb…` | `facts-a`, release, sha256 `86c0abdc5d00…` |
| Source | `Moti-Velz/facts-engine` @ `54a7dd9a4` (both) | |
| Rules | 58 (the 60 minus the 2 the daemon refuses, below); a 60-rule native run is reported separately | 58 (558 clauses: 529 per-file, 29 cross-file) |
| Shape | one process per worktree (its production shape), 4 at a time, `--store off` | one daemon for all 40 worktrees, `--threads 4`, one content store |
| Cross-file | evaluated in the scan and on every edit | `--cross scratch`: one from-scratch repository pass per change, on a worker thread |
| Machine | Apple M5 Max, 18 cores, 128 GB | same |

Not compared: `imperative_private_functions` and `internal_dead_code`. The daemon refuses both by name at compile (`calls` rule key; projection `file.module`). On native they produce 774,208 of the 970,313 findings on the 40 worktrees (80%).

## Exactness

| Check | Result |
|---|---|
| Per-file findings, scan, 40 worktrees (`compare-findings`, 188,979 findings) | **identical on 40/40** |
| Per-file findings after the 20 edits | **identical on 40/40** |
| Cross-file findings (29 clauses, 7,126 findings) | **counts equal on 40/40**; content not compared (the harness keeps push counts only) |
| Total findings | native 196,105 = facts 196,105 |

One harness bug was fixed on the way: `compare-findings` stripped one `data-layer-` prefix from native rule ids, so `data-layer-data-layer-compat-aliases` never matched the daemon's `compat_aliases` (one false "facts-only" finding per worktree). Fixed in `benchmark/workspaces/facts.ts`.

## Results (58 rules, both engines)

| | Native | Facts daemon | Ratio |
|---|---|---|---|
| **Cold scan, CPU, 40 worktrees** | 11,407 s (scan phases) | 2,369 s (whole process, per-file + cross-file) | **4.8× less CPU** |
| Cold scan, one worktree, nothing cached | 220 s wall p50 (245 p95), 282 s CPU p50 | 31.6 s wall (worktree 01) | **7.0×** |
| Each further worktree | 220 s wall p50 (same work again) | 2.5 s p50, 9.6 s p95 (only contents not seen before) | **≈ 90×** |
| Cold scan, 40 worktrees, wall | ≈ 2,235 s (Σ scan walls / 4) [INFERENCE: the fleet wall, 4,237 s, also holds the edits] | 2,111 s, of which per-file 131 s and cross-file 1,961 s (40 passes, serial on one thread) | ≈ 1× |
| **Edit, per-file findings** | 7.8 s p50, 14.6 s p95 (all rules incl. cross-file) | 1.3 ms p50; evaluated edits (180) 8.4 ms p50, 2.27 s p95; memo-served (620) 1.1 ms p50 | **≈ 900× evaluated** (≈ 6,000× with memo hits) |
| — largest file (694 KB) | 14.3 s p50 | 2.24 s p50 (evaluated) | 6× |
| **Edit, cross-file findings** | in the 7.8 s | 47.3 s p50, 59.6 s p95 (from-scratch stand-in) | **6× slower** |
| Memory, steady, per worktree | 5.0 GiB p50 (peak 20.1 GiB) | — | |
| **Memory, steady, 40 worktrees** | ≈ 199 GiB (Σ 40 processes) [INFERENCE: if all 40 are resident at once] | 1.88 GiB (one daemon); 0.74 GiB per-file only | **≈ 100×** |

## What the numbers mean

- **Per-file work is where facts wins today.** It evaluates each distinct file content once for the whole fleet (content-addressed memo), its per-file engine is faster, and an edit costs one file. Same findings as native on every worktree.
- **Cross-file is where it loses today.** The daemon's cross-file side is still the from-scratch stand-in: about 47 s per change, against native's whole edit at 7.8 s. That cancels the per-file gain on cold wall time (2,111 s vs ≈ 2,235 s) and makes cross-file findings slower after an edit.
- **The maintained cross-file path exists but is not wired into the daemon.** On the 1,111-event replay over the same 40 worktrees, Y3 (dead code, exported-families and three smaller plans) measured 0.5 ms p50 / 3.8 ms p95 per edit and 2.1 GB, 0 differences against from scratch (`reports/explore-y3-wire.md`); E3's lattice version of dead code 0.75–0.80 ms p95 (`reports/lattice-e3-maint.md`). Those are separate measurements on a replay, not this harness, and cover 5 of the 8 repository clauses.
- **Coverage gap:** the 2 refused rules are 80% of native's findings. They need the `calls` rule key and `file.module` in the daemon before facts covers the full rule set.

## Caveats

- Wall-clock is secondary: native's processes use about 1.3 cores each (CPU/wall), so 4 of them load the machine at about 5–7; 1-minute load during the 58-rule run had p50 6.8, max 10.5 (`native58/load-samples.txt`). CPU totals are the robust comparison.
- Native ran with `--store off`: with its content store on, the harness's edit phase fails in native (`stored results for evicted _internal/pure_lazy.py are unavailable; apply it again`). Store off vs on, scan of worktree 40: 260 s vs 274 s (single runs).
- Edits: the same appended comment goes to the same four files (minimum, median, p95, maximum Python file by size) in every worktree, so after the first worktree most facts edits are memo hits (620 of 800). Native processes do not share work across worktrees, by design; the evaluated-edit row is the like-for-like per-edit cost.
- The 60-rule native run (same session): fleet CPU 32,670 s, edit p50 20.8 s, steady 6.1 GiB per worktree. Its worktrees 21–32 overlapped other work on the machine (scan +14% wall and CPU against the rest); it is reported for the cost of the two refused rules only.
- Single runs; no repetition.

## Reproduce

```
cargo build --release -p facts-a
npx napi build --platform --release --manifest-path native/Cargo.toml --output-dir target-addon/addon
benchmark/facts/fleet-ab/run-fleet-ab.sh <run>        # facts full, facts per-file + edits, native 60 rules
# rules58: packs/data-layer/candidates/rules without data-layer-{imperative-private-functions,internal-dead-code}.json, in <run>/rules58
benchmark/facts/fleet-ab/run-native58.sh <run>        # native 58 rules
python3 benchmark/facts/fleet-ab/analyse.py <run>
```
