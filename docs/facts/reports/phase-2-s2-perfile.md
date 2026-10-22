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

All timing runs of this session ran at a 1-minute load of 12–43 (other slices building): **untrusted** by the Phase 1 protocol (> 8).

| Id | Target | Measured | Pass/fail | Run dir (`~/rerail-bench/facts/phase-2/s2-perfile/runs/…`) |
|---|---|---|---|---|
| G2.3 CPU, A3 path, step 1 | ≤ 90 s | 165.7 s median (164.8 / 165.7 / 170.2), wall 51.8 s (A3 baseline 152.6 s was at load 3–5) | **fail** (untrusted) | `20261022T153525-repo`, `…153617-repo`, `…153709-repo` |
| G2.3 CPU, generic path (`FACTS_A3_OFF=1`), step 1 | ≤ 90 s; ≤ 152.6 s to delete `subst.rs` | 212.6 s median (203.3 / 212.6 / 215.2) | **fail** (untrusted) | `…153804-repo-generic`, `…153914-repo-generic`, `…154015-repo-generic` |
| G2.3 per file p50, 10 heaviest rules | ≤ 10 ms | not computed | open | |
| Parity after every change | unchanged | replay 1,253/1,259, 0 FP, 2 FN, 6 failures; repository 173/173, 0 FP, 0 FN, 38 failures; all misses `quality-contracts-1`; identical on both paths | **pass** | `20261022T145930-step0-parity`, `…151000-step1a-check`, `…153235-step1-check`, `…154344-step2-closure-check`, the six timing runs |
| `FACTS_A3_CHECK=1` differential, 9,010 files + 1,259 calls | no difference | passes at step 1a, step 1, closure | **pass** | same check dirs |
| G2.5 end of per-file phase | ≤ 300 MB | 81 MB (A3 path), 77 MB (generic) | **pass** | step-1 timing runs |
| G2.5 per-worker peak, largest file | ≤ 256 MB | 828 → 530 MB (A3 path), 814 → 513 MB (generic) | **fail** | § 3 command |
| Per-file phase peak, 4 workers | (A3 2.55 GB) | 1.46 GB (A3 path), 1.31 GB (generic) | improved | step-1 timing runs |
| Overall peak | (A3 5.86 GB) | 6.93 GB (A3 path), 4.70 GB (generic): `finish` holds clause-level indexes | regression on A3 path | step-1 timing runs |

Per step, generic path on sub200 (one worker; load 9–35, untrusted):

| step | eval (s) | SQL stages (s) | sub200 peak | largest-file peak | compiled rules 1 / 4 workers |
|---|--:|--:|--:|--:|--:|
| step 0 (`facts-a-step0`) | 102.5 | 79.9 | 650 MB | 814 MB | 132 / 405 MB (320 / 1,312 before step 0) |
| step 1 (`0230e4ec3`) | 80.7 | 57.4 | 428 MB | 513 MB | 67 / 160 MB |
| closure operator | `descendants` 11.7 → 10.2 s in an A/B at load 26–35 | | 433 MB | 559 MB | same |

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

- Compiled rules were the largest per-worker cost before any file (320 MB). Now 67 MB for one worker, still per worker (rules hold `Rc`s), not one `Arc` per process as the decider asked (≤ 32 MB): `Query`, `Names`, SQL plans and `V` literals must become `Send + Sync` first.
- The generic path is still ~50 s CPU above the A3 path. Residue (generic probe `20261022T152003-step1b-off-probe`, before the shared memo): inputs 48.6 s (dump selections), tree-rebuilding SQL 33 s (`descendants` 14.2, `nodes` 9.9, `ownership` 9.3), rule-logic SQL ~105 s. Profile (sub200, `sample`): `Column::get` 13%, AST matcher 11%, expression interpreter 10%, hashing/index 8%, allocation ~10%.
- The clause-level index cache raises the repository partition's peak in `finish` (A3 path 5.86 → 6.93 GB): indexes over large repository tables live until the clause ends. Needs a per-table last-reader drop before the next timing.
- Every timing run of this session ran at load 12–43; all CPU numbers need a re-run on an idle machine.

## 7. Resume notes

- Commits on `Moti-Velz/facts-engine-s2-perfile`: `4b14dfe7e` step 0 (regex cache, `FACTS_MEMTRACE`, `run-repo.sh` `FACTS_RUNS`/`FACTS_BIN`); `0230e4ec3` step 1 (columnar layer, shared utility table and memo, index cache, input lifetimes); the closure-operator commit after it (`git log`). Not yet merged: `Moti-Velz/facts-engine` (`2b111331b`: S1's move of `sql/{parse,value}.rs`, `policy.rs`), S4's `ast.rs` annotation arm (`275b016d1`, `Eval::parse` only).
- Runner: `FACTS_RUNS=~/rerail-bench/facts/phase-2/s2-perfile/runs FACTS_BIN=<frozen copy> [FACTS_A3_OFF=1] spikes/facts-a/tools/run-repo.sh`; `python3 ~/rerail-bench/facts/phase-2/s2-perfile/summ.py <run dirs>` summarises CPU, peaks, parity, load.
- Next, in order: (1) merge integration; 3 + 3 timing runs on an idle machine. (2) Drop clause-level indexes after a table's last reader (fixes the `finish` peak). (3) Interval membership for closures over a runtime-verified forest (no materialised `descendants`; rows ordered by (level, seed, preorder) to stay exact) — the G2.5 per-worker lever together with the dump inputs (~60 MB per clause on the largest file). (4) Compiled rules `Send + Sync`, one `Arc` per process. (5) Lower S1's `Plan` (`rerail_facts::compile`) onto this executor; emit `phys::Node` per statement (node set agreed with S1). (6) Delete `subst.rs` / `FACTS_A3_*` once generic ≤ 152.6 s. S5's mutants 1–2 sit in `subst.rs` (re-seed in `gather`/the closure operator when it goes); mutant 3 (`has_end_memo`) is unchanged in place.
