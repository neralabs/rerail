# Phase 2, S2 per-file backend

Status: **in progress.** Branch `Moti-Velz/facts-engine-s2-perfile` (worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s2-perfile`). Raw runs: `~/rerail-bench/facts/phase-2/s2-perfile/runs/`.

Direction (decider re-steer, supersedes the brief's step 2 wording): no shape-recognised substitutions, no per-stage kernels, no text recognition after the parser. The deliverable is the generic physical layer and generic rewrites that make Spike A3's `subst.rs` unnecessary; `subst.rs` stays as a benchmark oracle until the generic path (`FACTS_A3_OFF=1`) holds or beats A3's 152.6 s with parity unchanged, then it and `FACTS_A3_*` are deleted. Differential: generic path vs the unrewritten path (`FACTS_A3_CHECK=1` runs both and panics on any difference in rows, order, values or anchors).

## 1. Setup

| Item | Value |
|---|---|
| Baseline | A3 median `~/rerail-bench/facts/spike-a/runs/20261022T142444-repo`: 152.6 s CPU, per-file phase peak 2.55 GB, overall 5.86 GB |
| Corpus | `~/rerail-bench/nera-data-layer-2b1504ad` (9,010 files) and the batch-6 recordings |
| Largest file | `impl/common/l2070_server_plan/steps/intent_versioning_extended.py` (764 KB, 148,502 nodes), copied to `~/rerail-bench/facts/phase-2/s2-perfile/largest/` with its relative path (scopes match) |
| Profiling subset | `~/rerail-bench/facts/phase-2/s2-perfile/sub200/`: files ranked 4–203 by evaluation time (58% of corpus evaluation time) |
| Build | `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=spikes/facts-a/target cargo build --release -p facts-a` |
| Runner | `FACTS_RUNS=~/rerail-bench/facts/phase-2/s2-perfile/runs spikes/facts-a/tools/run-repo.sh` (extended with `FACTS_RUNS`) |

## 2. Results

Filled at each milestone; see § 4.

## 3. How measured

- Per-worker peak on the largest file: `/usr/bin/time -l facts-a repo --threads 1 --root ~/rerail-bench/facts/phase-2/s2-perfile/largest --out-dir <dir>` (`maximum resident set size`; the process is one worker plus the main thread, rules compiled twice: worker and `finish`).
- Compiled-rules floor: the same on `~/rerail-bench/facts/phase-2/s2-perfile/tiny/` (one 2-line file in scope) with `--threads 1` and `--threads 4`.
- `FACTS_MEMTRACE=1`: per stored stage, rows, bytes held by its columns, current RSS; per compiled rule, RSS.

## 4. Steps and measurements

### Step 0: per-process regex cache (`4b14dfe7e`)

Finding: before any file, each worker held ~320 MB of compiled rules (state-tripwires 160 MB, quality 112 MB): every input query compiled the policy set's whole utility table, with its regexes, again (~1.7–2 MB per input; 25 dump inputs per clause). `src/rx.rs`: one compiled `Regex` per pattern per process (clones share the program), a per-thread handle for SQL `rx_*` (warm across files). Largest file per worker: 828 → 555 MB. Parity unchanged (`20261022T145930-step0-parity`: replay 1,253/1,259, 0 FP, 6 failures; repository 173/173, 0 FP, 0 FN, 38 failures).

### Step 1: columnar physical layer

- `Table` is columns (`sql/engine.rs::Column`): `Vals`, `Gather` (rows of a source by index, optional CTAS affinity applied on read), `Chain` (UNION ALL), `Text` (byte ranges of the file read on first use). A select's output column that reads a FROM item's column is a gather view sharing one row-index vector per item; `SELECT n.*` stores one `u32` per row, not ~40 values. ORDER BY / LIMIT / DISTINCT / UNION are selections over views. Bindings are `(table, row)` references, not row slices; group representatives and window bindings keep `(table, row)` pairs, not row copies.
- Semi-naive fixpoint over columnar batches: each level's rows are gathers of the arms' sources (a closure stores row indexes, not values); set semantics through per-column interned keys (`Distinct`), rows as `[u32; 4]` keys.
- Input tables built column-wise; node text is a `Text` column (span of the source, materialised on first read), so the O(nodes × depth) bytes of nested node texts are paid only for rows a stage reads.
- Inputs are dropped after the last clause that reads them (`Rule::last_use`).
- Utility table compiled once per policy set and shared by its queries (`Compiler::shared`); the utility and relation memos are shared per (file, policy set, utility table, valuedness) (`ast::FileMemo`), with a taint guard: a verdict whose evaluation refused or decoded a `\N{...}` escape is never memoised, so every query refuses exactly as its own evaluation would. This is the generic form of A3's "selection once per node": a dump's shared gate is a utility.
- Hash indexes over stored tables are built once per clause and shared by its statements (`Db::indexes`); per-statement indexes are keyed by (table, probe) without allocation.
- Lateral `json_each` tables are kept past their level only for grouped/window selects.

Measurements so far (single worker, untimed probes; timing runs in § 2 when the milestone lands):

| | step 0 | step 1 |
|---|--:|--:|
| compiled rules, 1 / 4 workers (tiny file) | 132 / 405 MB | 67 / 160 MB |
| largest file, A3 path, per-worker peak | 555 MB | 530 MB |
| largest file, generic path (`FACTS_A3_OFF=1`), per-worker peak | 814 MB | 513 MB |
| sub200, generic path, evaluation thread time | 102.5 s | 80.7 s |
| sub200, generic path, SQL stages | 79.9 s | 57.4 s |
| sub200, generic path, peak | 650 MB | 428 MB |

## 5. Design notes

See § 4 for the physical layer. Physical node set agreed with S1 (`facts/src/phys.rs`): `Pipeline{Scan, IndexNestedLoop, OrIndexProbe, LateralJsonEach}`, `SemiJoin/AntiJoin/Scalar`, `Group`, `Window`, `Sort`, `Union`, `Distinct`, `Fixpoint`, `IntervalProbe`, `MemoPredicate`.

## 6. Risks found

- Compiled rules were the largest per-worker cost before any file (320 MB); G2.5's 256 MB per worker could not pass with them.

## 7. Resume notes

- Commits: `4b14dfe7e` step 0.
- Next: commit step 1 after `FACTS_A3_CHECK` on corpus + recordings; timing runs; then the closure rewrite (fixpoint over a runtime-verified forest edge → interval membership, no materialised `descendants`), then delete `subst.rs`.
