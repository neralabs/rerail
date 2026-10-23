# Phase 2, S10: the short evaluation loop

Branch `Moti-Velz/facts-engine-s10-loop`, from integration `5ca57e471`, with `7287d9a86` (S3 wave 6) merged in. This slice implements `docs/facts/LOOP.md`. Every number below is a count, or a wall-clock measured on a loaded machine and marked as such. Run directories are under `~/rerail-bench/facts/loop/`.

## What landed

| item | where | commit |
|---|---|---|
| One build: `loop` profile into `target/loop` with the `mutants` features on; the gate builds `release` into `target/release` (`FACTS_GATE_TARGET` and its separate dir are dropped; output format unchanged) | `scripts/check/facts-gate.{sh,py}` (`TARGET`, `PROFILE`), `scripts/check/facts-loop.py` | `b74e8c12c` |
| T0 `just facts-t0`: the unit tests, then the mutants whose `mutant::on(n)` sites are in files changed since the merge-base | `facts-loop.py t0` | `b74e8c12c` |
| T1 `just facts-t1`: sub200 against `benchmark/facts/sub200-reference.json`, ref50 through stage-check, the 100-event replays when facts-b changed, the archetype bench | `facts-loop.py t1`, `benchmark/facts/{sub200-reference.json,ref50.list}` | `024b5fcec` |
| Counts: algebraic and physical, in `results.json`, in `facts-a explain --analyze FILE`, and against the reference | `spikes/facts-a/src/counts.rs`, engine/lowered/rel/eval hooks, `reference::evaluate_stages_counted` | `024b5fcec` |
| Bench `just facts-bench [archetype…] [--compare RUN]` | `benchmark/facts/archetypes.json`, `facts-a repo --clauses` | `024b5fcec` |
| Digest memo, consulted by `facts-a repo` (`--no-memo` turns it off) | `spikes/facts-a/src/memo.rs`, `repo.rs` | `024b5fcec` |
| Gate check `count-regression` | `facts-gate.py`, `REWRITES.md` § Count changes | `a105ba71b` |

## 1. One build

T0, T1 and the bench share one build: `cargo build --profile loop -p facts-a -p facts-b --features facts-a/mutants,facts-b/mutants`, plus `cargo test --profile loop -p rerail-facts -p facts-a -p facts-b --features …/mutants --no-run`. Both use one feature set, so nothing is built twice. The test binaries run directly, without cargo. The gate builds `release` with the same features into the tree's own `target/release`.

Rebuild after editing the body of `sql/engine.rs::execute` (a one-line change):

| build | facts-a bin | facts-a unit tests | load |
|---|---|---|---|
| before: the gate's release build in `target/facts-gate`, `CARGO_BUILD_JOBS=4` (thin LTO, `codegen-units=1`) | 4 m 02 s | 3 m 46 s | ~4 |
| after: `loop` profile | 5.7 s | 3.9 s | ~13 |

A cold `loop` build takes 2 m 07 s. A comment appended at the end of the file costs 0.9 s, because incremental compilation reuses everything.

With `FACTS_MUTANT` unset, the mutants feature changes no findings: three alternating 1-thread sub200 runs of each binary (pre-counts build, `~/rerail-bench/facts/loop/s10/ab-mutants`) give the same 90 findings and 0 failures. The CPU measurement did not settle:

- without the feature: 44.1, 44.7 and 45.0 s user;
- with it: 44.9, 46.7 and 51.4 s.

The 1-minute load rose from 3.6 to 6.4 during the series, and the feature runs came second in each pair. Only the first pair (+1.8%) ran at comparable load. **Not settled:** an ABBA series on a quiet machine is still owed (resume notes). The gate's release build already carried the feature before this slice.

## 2. T0 (≤ 60 s)

Each mutant site is found by grepping `mutant::on(<n>)` in the three crates. The mutant list and its families come from `facts-gate.py` (`MUTANTS` plus REWRITES.md's tables).

Each selected mutant runs the T0 form of its family's checks:

- the unit-test binary of its crate (`classifier-leak` and `planner-audit` are the whole lib binary);
- `b-generated` over the first 30 events with `--fail-fast` (mutants 4–6);
- `b-ties` (mutant 9).

Measured:

- Unit tests alone, every mutant against them: all 40 registered mutants are killed except 4, 5, 6 and 9, which are killed by the T0 replays. The 40-mutant T0 (`--base HEAD~40`) took 13 s.
- T0 on this branch (11 mutants in `lowered.rs` and `engine.rs`): 5–8 s with the incremental build, at loads 13–60.

## 3. T1 (≤ 2 min)

The checks:

- **sub200-parity** compares per clause with the committed reference (90 findings, 0 failures, 3 clauses with any). The reference is regenerated only by `just facts-t1 --rebase NOTE`, which appends to its `history`.
- **ref50-vs-reference** runs `stage-check` over 50 fixed gate-subset files of 400–2,000 nodes (the first 50 by sha256 of the path).
- The replays run when `spikes/facts-b/` changed.
- **bench** runs every archetype.

`20261023T112650-t1`, load 50, with every clause evaluated (the memo did not yet hold the new digests):

| check | wall | result |
|---|---|---|
| build | 15 s | ok |
| sub200-parity | 24 s | 90/90 findings, equal in every clause |
| ref50-vs-reference | 24 s | 2,837 stages, 0 mismatches |
| bench | 43 s | 5 archetypes |
| **total** | **58 s** | passed |

## 4. Counts

Definitions are in `spikes/facts-a/src/counts.rs`. Counting is observation only: no operator reads a count.

- **Algebraic:**
  - `leaf_rows`: rows of the input and syntax tables a relational clause reads;
  - `stage_rows`: rows each stage and the final query emit;
  - `fix_rounds`, `fix_rows`: rows admitted per round by top-level fixpoints (recursive CTEs outside expression subqueries; round 0 is the initial select, through the first empty round);
  - `match_nodes`, `match_roots`, `input_rows`: per pattern input read, the `node` rows the selection ranges over (matcher node visits), the nodes it keeps, and the rows it emits.
- **Physical (engine):** `index_builds`/`index_rows`, `range_builds`/`range_rows` (R14), `probes`, `rows_scanned`, `rows_joined`, `rows_out`, `subquery_runs`. They are counted in `Exec` cells and flushed per execution.
- **Physical (matcher):** `pred_tests` (nodes a memoised predicate was evaluated on) and `memo_hits` (nodes R13's memo answered). R13's memo is shared by a rule's inputs, so these are reported per input (`input_counts`), not per clause.

Where they appear:

- `results.json`: `counts` (totals), `rules[].clauses[].counts`, `clause_stage_counts`, `input_counts`.
- `facts-a explain --analyze FILE` prints per clause its counts, then per stage its counts and its physical plan, then the inputs. Two runs give byte-identical text.

**Identical between reference and engine.** The reference evaluator records rounds in `Evaluator::fixpoint` when `outer` is empty; the engine records them in `closure`, `tree_walk` (R18), `walk` (R19) and `recursive` when `outer` is `None`. Both count admitted rows round by round. `stage-check` compares the rows and fixpoint rounds of every stage it compares, and `pattern-check` compares matcher nodes, roots and rows three ways.

- ref50 (in T1): 2,837 stages, 0 count mismatches.
- sub200, runs with at most 2,000 input rows (`s10/sc-sub200-capped`, 813 s at 16 threads): 7,426 runs, 22,301 stages, 1.96 M rows compared, 0 count mismatches. 790 runs were not compared: the reference refused them, or they were over the input cap.
- sub200 uncapped (the reference's nested loops on the 200 heaviest files) ran for 35 minutes without finishing and was stopped.
- Matcher, leaf executor against physical plan: the R14 A/B below runs every matcher input on the leaf executor under `FACTS_REWRITES=0`. Every algebraic count, `match_*` included, equals the R12/R13 plan's on all four sub200 archetypes.
- `pattern-check` (the gate's `a-pattern`, gate subset, reference up to 200 nodes): 429,192 evaluations compared, leaf against physical on every file and against the reference where it runs, 0 count mismatches. At `--max-nodes 2000` on ref50 the reference side did not finish in 900 s.

**Deterministic across load.** The same counts in every run:

| sub200 run | threads | load |
|---|---|---|
| 8-thread run | 8 | ~10 |
| 3-thread run | 3 | ~10 |
| bench in T1 | 1 | 50 |
| bench, `ab-r14-on` | 1 | 46 |

The 8- and 3-thread runs agree for all 105 clauses, 454 stage rows and every input. The two bench runs give the same counts for every archetype; for example, `quality-contracts-1` has `probes=44487350` and `rows_scanned=100412145` in both.

## 5. Bench and the R14 A/B

Single thread on sub200, counts first. Wall-clock is not calibrated (load 46–60).

| archetype | clause | stage_rows | fix_rows | index_rows | probes | rows_scanned | wall |
|---|---|---|---|---|---|---|---|
| pattern-only | impl_direct_execution/direct-execution-lines | 237 | 0 | 118 | 546 | 593 | 6.9 s |
| relational-per-file | serverplan_state_tripwires/state-flow-1 | 28,495,525 | 16,466,533 | 47,205,152 | 44,457,523 | 96,219,375 | 56.4 s |
| closure-heavy | serverplan_quality/quality-contracts-1 | 15,552,163 | 2,116,870 | 38,870,064 | 44,487,350 | 100,412,145 | 31.8 s |
| selection-heavy | serverplan_quality/quality-conditional-1 | 1,407,863 | 267,994 | 3,737,269 | 1,490,267 | 5,701,878 | 15.1 s |

The fifth archetype, **repository** (`serverplan_target_architecture/required-contracts`), runs through `facts-b greplay` over 100 events with S3's w4 exports: 26 nodes, 160,713 records sent, 20,510 records arranged, 16.6 s.

**A/B: rewrites on (`s10/ab-r14-on`) against `FACTS_REWRITES=0` (`s10/ab-r14-off`).** Every algebraic count and every finding is identical. The physical counts that differ:

| archetype | without R14 (rewrites off) |
|---|---|
| closure-heavy | `range_rows` 11,419,016 → 0; `rows_scanned` +89.6% (100.4 M → 190.3 M); `rows_out` +10.4% |
| selection-heavy | `rows_scanned` +12.0%; `rows_out` +12.6%; `probes` −4.2% |
| relational-per-file | `range_rows` 205,809 → 0; `probes` +1.0%; `rows_out` +6.9% |
| pattern-only | `range_builds` 22 → 0; `probes` −32.6% |
| repository | no count differs |

## 6. Digest memo

The key is (content hash, plan digest, schema version) plus the path: the policies read `file.path`. The plan digest hashes:

- `ENGINE_VERSION`, the schema version and `FACTS_REWRITES`;
- the resolved rule without its clauses, its `define` clauses and the call scope;
- the resolved clause;
- the physical plan of each input it reads (`lowered::Plan::digest_text`);
- the physical EXPLAIN of each of its stages.

An entry stores the clause's findings, failures, per-file counts, repository-clause rows and budget rows. `eval::finish` always runs. The memo is off under `FACTS_MUTANT`, and the gate passes `--no-memo`.

| run | evaluated / memoised clause runs | wall | findings, failures, counts |
|---|---|---|---|
| sub200 cold | 14,982 / 0 | 22.5 s | equal to `--no-memo` |
| sub200 warm | 0 / 14,982 (200 files not parsed) | 3.5 s | equal |
| corpus (9,010 files) cold, 8 threads | 228,685 / 0 | 51.2 s (180 s CPU) | 173 findings, 0 failures |
| corpus warm | 0 / 228,685 (9,010 files not parsed) | 19.6 s (18.9 s CPU; the repository phase is most of it) | identical findings, failures, per-clause counts |

The corpus memo takes 77 MB (`s10/memo-corpus`). Wall-clock is at load 40–60.

**A change to one rewrite.** R14 temporarily stopped probing `<=` conjuncts; the change was reverted afterwards. Effect on sub200 (`s10/rw`):

- 9 of 105 clause digests changed.
- 1,586 per-file clause runs were re-evaluated and 13,396 memoised.
- Findings and counts equalled the `--no-memo` run of the same binary.

Caveat: a change to an operator that leaves every plan's text unchanged must bump `memo::ENGINE_VERSION`.

## 7. count-regression

The check runs in lane `a`, after `a-subset`. For each clause, every count of `a-subset` is compared with `benchmark/facts/subset-counts.json` as stored at the merge-base with `Moti-Velz/facts-engine` (`FACTS_COUNT_BASE`). A count that grows by more than 5% needs a row naming `check/clause` under REWRITES.md § Count changes.

The check writes `count-regression/subset-counts.json`, and the integrator commits it at the merge. No baseline exists at this merge-base yet, so the first run passes with "baseline written".

## The gate

`just facts-gate --mutants` at `a105ba71b` (`s5-gate/runs/20261023T113421-gate`, load 74): **passed, 29/29 checks, 41/41 mutants killed, 1,292 s.** That figure includes a cold `release` build of the new `target/release` (845 s at `CARGO_BUILD_JOBS=4`), so the checks and mutants took about 450 s.

- `a-repo`: 173/173 findings, 0 FP, 0 FN.
- `count-regression`: no baseline at the merge-base; the baseline was written, then committed as `benchmark/facts/subset-counts.json`.
- `a-pattern`: counts compared, 0 mismatches.

The 10-minute budget is met by the checks; the cold build is outside it and is paid once per tree. After an engine edit, the release rebuild still costs about 8 minutes (§ 1); that cost is T2's.

## Resume notes

- Gate output: `~/rerail-bench/facts/loop/s10/gate-mutants.txt`. The `count-regression` baseline is committed (`benchmark/facts/subset-counts.json`, from that run). Integrators refresh it at each merge.
- Owed: an ABBA series (mutants feature on/off, and the counts' own overhead) on a quiet machine. The counts add one `Cell` add per probe, scanned row and joined binding. Every measurement in this slice ran at load 13–74.
- Possible follow-ups:
  - `stage-check`'s `count_mismatch_total` could join `mismatch_total` in `a-vs-reference`; it is 0 everywhere measured.
  - The repository archetype reads S3's w4 exports and the event script, not sub200.
