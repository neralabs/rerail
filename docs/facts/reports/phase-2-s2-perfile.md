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

## Wave 2

Base: integration `caceb656f`, merged forward to `8960a5db7` (S6's leaf constructs; my `ast.rs`/`eval.rs` conflicts resolved to upstream, memo hunks re-applied). Machine load during every timing pair 17–71 (six agents): **all CPU numbers untrusted**; memory numbers are deterministic up to allocator noise. Frozen binaries in `~/rerail-bench/facts/phase-2/s2-perfile/bin/`: `facts-a-w2base` (= `caceb656f`), `facts-a-w2s1` (index lifetime), `facts-a-w2s2` (+ shared rules), `facts-a-w2s3` (+ G2.5 items), `facts-a-w2s3m` (merged with `8960a5db7`).

### Gate rows

| Id | Target | Measured | Pass/fail | Run dir (`~/rerail-bench/facts/phase-2/s2-perfile/runs/…`) |
|---|---|---|---|---|
| Overall peak, repository run, A3 path | ≤ 5.9 GB | 6,945 → 5,820 MB (step 1 pair); 6,925 → 5,844 MB (w2base vs w2s3) | **pass** | `20261022T203610-repo` / `…203656-repo`; `…213502-repo` / `…214937-repo` |
| Overall peak, generic path (`FACTS_A3_OFF=1`) | (≤ 5.9 GB) | 4,697 → 3,034 MB | pass | `…215036-repo-generic` / `…215146-repo-generic` |
| G2.5 compiled rules: one immutable object per process, ≤ 32 MB | ≤ 32 MB, shared | one `Arc<[Rule]>` (`Rule: Send + Sync`, static assertion in `eval.rs`), shared by every worker and `finish`; live heap 29–31 MB on the generic path (jemalloc `stats.allocated` delta around compile), 55–57 MB with A3's dump groups/kernels compiled in; RSS on `tiny` 90 / 256 MB (1 / 4 workers, w2s1) → 73 / 73 MB | **pass** (generic path; A3 path 57 MB, retires with `subst.rs`) | `results.json` `compiled_rules_heap_bytes` of `…215146-repo-generic`, `…215844-largest-w2s3m-A3_` |
| G2.5 per-worker peak, largest file (`--threads 1`, includes the rules) | ≤ 256 MB | generic path 607 → **230 MB** (merged tree); A3 path 637 → 494 MB (its kernels build `Box<[V]>` rows) | **pass** on the generic path; A3 path fails and is to be deleted | `…204756-largest-memtrace-w2s2-generic` (before), `…215844-largest-w2s3m-A3_`, `…215849-largest-w2s3m-Y=1` |
| G2.5 end of per-file phase, 4 workers | ≤ 300 MB | A3 157 MB, generic 119 MB (rules now resident once for the whole run) | **pass** | `…214937-repo`, `…215146-repo-generic` |
| Per-file phase peak, 4 workers | (wave 1: 1.46 / 1.31 GB) | A3 1,686 → 1,058 MB; generic 1,451 → 515 MB | improved | same pairs |
| G2.3 CPU, A3 path | ≤ 90 s | 175.7 → 163.1 s (load 17–71) | **fail**, untrusted | `…213502-repo` / `…214937-repo` |
| G2.3 CPU, generic path | ≤ 90 s; ≤ A3 to delete `subst.rs` | 212.6 → 207.3 s (load 34–68) | **fail**, untrusted; `subst.rs` not deleted | `…215036-repo-generic` / `…215146-repo-generic` |
| Parity | unchanged | 173/173, 0 FP, 0 FN, 0 failures on all four runs; gate on the merged tree: see resume notes | pass | `parity.json` in each run |

### Per-step table (largest file, `--threads 1`; peak RSS / live heap at the traced maximum)

| Step | What | Generic peak RSS | A3 peak RSS | Run dir |
|---|---|--:|--:|---|
| base (`w2s2`) | shared rules, index lifetime | 607 MB (heap max 559) | 637 MB | `…204756-largest-memtrace-w2s2-generic`, `…204745-largest-memtrace-w2s2` |
| 3a | syntax-spec tables and matcher inputs as compact fact columns (`Column::{U32,Sym,Const}`, span `Text` at 8 B/row with a lazily allocated read cache) | 465 MB | | `…210843-largest-memtrace-compact-generic` |
| 3b | hash indexes without stored keys (rows grouped by key hash, confirmed against a representative row; 4 B/row + ~32 B/key) | 369 MB | | `…211141-largest-memtrace-index-generic` |
| 3c | literal projections (`NULL AS x`, `'k' AS y`) as `Column::Const` | 289 MB | | `…211332-largest-memtrace-const-generic` |
| 3d | syntax `reachability` as interval membership (`Column::Interval`: per root `eligible[lo..]` over `(id, subtree_end)`, O(roots + nodes) bytes) | 263 MB | 500 MB | `…212127…`, interval runs `…2131*` |
| 3e | per-file memo: dense per-utility verdicts and `RelMemo` vectors as 2-bit cells (`trits.rs`); state_tripwires memo 71 → 17 MB | 227 MB | 496 MB | `…213403-largest-w2s3-A3_`, `…213411-largest-w2s3-X=1` |
| merged | `8960a5db7` + the above | 230 MB | 494 MB | `…215844-largest-w2s3m-A3_`, `…215849-largest-w2s3m-Y=1` |

Attribution method: `FACTS_MEMTRACE=1` now prints per stage the clause's stored-table and index bytes, RSS, jemalloc live heap and this thread's heap peak since the previous trace (`thread.peak`); per input set, per clause, and per rule the memo and input-cache bytes (`=2` lists every column ≥ 1 MB). The 530 MB of wave 1 was, in order: syntax-spec node tables as `Box<[V]>` rows (~200 MB retained, 341 MB transient), hash indexes with `Vec<Key>` keys (~100 MB), the per-file utility/relation memo (71 MB), materialised syntax `descendants` (75 MB), `NULL AS x` columns (~70 MB), matcher inputs (~50 MB per clause). Recursive-CTE batches and spill buffers were not material. Remaining at 230 MB: compiled rules 31, facts and source ~18, the clause's `nodes` stage (string ids built by the rule's SQL, 29 MB), clause-level indexes (≤ 35 MB), the generic closure's `Distinct` transient for `descendants` over the SQL-built edge relation (56–68 MB transient).

### Interface changes

- `sql::engine`: `PE::Lit(Lit)` with `Lit::{Null,Int,Real,Text(u32)}` (texts per statement, materialised once per execution) — plans hold no `V`; `Column::{Const,U32,Sym,Interval}` and `U32_NULL`; `Column::Text` is `(start,end)` cells + side `vals`; `Index` is a grouped hash index; `Db::release(name)`, `Db::bytes()`; `eval_scalar_rows` replaces per-row `eval_scalar`; `closure_applies(&PCte)` for S1's EXPLAIN.
- `rel::RelPlan::release` + `schedule_releases()` (tables and their clause-level indexes dropped after the last reader, kernels' reads included); `SyntaxSpec::tables` columnar.
- `repo`: rules compiled once (`Arc<[eval::Rule]>`), workers borrow them; `phases_ns.{compile,merge}` replace `worker_compile`/`merge_and_compile`; `compiled_rules_{rss,heap}_bytes`; `heap_in_use()`, `thread_peak()`/`thread_peak_reset()` (jemalloc; `tikv-jemalloc-{ctl,sys}` added to facts-a).
- `ast.rs` (S6's file, memo only): `MemoSet.util: Vec<Trits>`, `RelMemo` vectors are `Trits`; `query.utils` etc. `Arc` (S6 did the same upstream). `eval::Operand::Literal(Literal)` (S6's budget `where`) holds no `V`.

### Not done this wave (steps 4–6)

- Step 4 (`subst.rs` deletion): not started beyond measurement. Wave-1 per-stage A3-vs-generic gap (runs `…153525-repo` vs `…153804-repo-generic`): matcher inputs 7.6 vs 61.2 s (A3's one-pass dump groups vs one AST evaluation per input), `descendants` 9.2 vs 17.9 s, `ownership` 5.1 vs 11.8 s, `nodes` 9.7 vs 13.2 s; later stages ~1.3× slower over gather views. This wave's pair keeps the same shape (inputs 9.8 s A3 vs 60.9 s generic in `…214937-repo` / `…215146-repo-generic`). The deciding lever is a one-pass evaluation of a clause's union of input selections (S1's R7 `fuse_union` physical side); S1's R9/R11 do not fire on the pack's dump-built `nodes`/`edges` (their roots are not core `node` per S1).
- Step 5 (S4's views): S4Schema-2 is timing each view columnar vs the row interpreter (`facts-a-base`) and will send run dirs; nothing bound.
- Step 6 (memo keys behind S1's `Plan`): not started.

## 7. Resume notes

- Commits on `Moti-Velz/facts-engine-s2-perfile`: wave 1 `4b14dfe7e`, `0230e4ec3`, closure operator; wave 2 one WIP commit (`s2-perfile: wave 2 WIP …`, after the merge of integration `8960a5db7`) carrying everything in § Wave 2. Gate run on the merged tree: `~/rerail-bench/facts/phase-2/s2-perfile/runs/*-gate-w2s3m`.
- Messages owed: S5Gate-2 gets the replacement switch names when `subst.rs` goes (none yet). S6's ast.rs rewrite is merged; my memo hunks (MemoSet/RelMemo/`util()`/`verdict_at`/`*_end_memo`) are on top.
- Runner: `FACTS_RUNS=~/rerail-bench/facts/phase-2/s2-perfile/runs FACTS_BIN=<frozen copy> [FACTS_A3_OFF=1] spikes/facts-a/tools/run-repo.sh` (generic runs need `facts-a repo-compare` by hand for `parity.json`); `python3 ~/rerail-bench/facts/phase-2/s2-perfile/summ.py <run dirs>`. Per-worker peak: `/usr/bin/time -l facts-a repo --threads 1 --root ~/rerail-bench/facts/phase-2/s2-perfile/largest --out-dir <dir>`, with `FACTS_MEMTRACE=1|2` for attribution.
- Next, in order: (1) re-run the two A/B pairs on a quiet machine (CPU untrusted). (2) Step 4: physical one-pass union of input selections (generic counterpart of A3's dump groups: ~51 s of the generic path's gap), then the closure `Distinct` as `(carried-id, node-id)` u64 keys (halves the `descendants` transient), then per clause A3 vs generic; delete `subst.rs` + `FACTS_A3_*` when generic ≤ A3 with `a3-check` equal, re-seed S5's mutants 1–2. (3) Step 5 with S4's numbers. (4) Step 6 memo keys behind S1's `Plan`. (5) S1's audit (`f1edf7edc`) flags two inner scans (`block_results`, `function_symbols`): probe keys for depth-2 correlated predicates.
