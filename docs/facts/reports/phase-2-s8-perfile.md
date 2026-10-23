# Phase 2 / S8 per-file (wave 4): levers 1 and 2 in the SQL engine

Status: **lever 1 (borrowed key cells, constant key columns) and the first half of lever 2 (text node ids kept as their parts, R16) landed; batch-6 CPU 104.7 → 93.7 s (mean of two quiet pairs), parity 173/173 on every run. G2.3 ≤ 90 s is not met: 3.7 s short.** The tree operators of lever 2 (interval `descendants`, one-pass `ownership`) and lever 3 were not reached. Branch `Moti-Velz/facts-engine-s8-perfile`, worktree `.worktrees/s8-perfile`. Runs: `~/rerail-bench/facts/phase-2/s8-perfile/runs/`; binaries in `…/bin/` (`facts-a-base` = integration `4387f42ad`, `facts-a-l1a` = lever 1, `facts-a-l2d` = lever 1 + R16 + flat ids + inline probe keys).

## 1. Gate rows

Timing runs: `spikes/facts-a/tools/run-repo.sh` under `bench-lock` (`/usr/bin/time -l`, 900 s cap, parity against `suite-b6.json`), started by `tools/ab.sh` only once the 1-minute load was below 6 (the 15-minute load was still 18–20: other slices were building). Each claim is two runs back to back, alternated with the base.

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| G2.3 | ≤ 90 s CPU, 3 runs, parity 173/173 | **92.7 / 94.8 s** (L2d; base 102.9 / 106.6 s in the same window); a third run was not taken before the budget ran out | **fail** (3.7 s short) | `20261023T053130-repo`, `…053301-repo` (L2d); `…053059-repo`, `…053229-repo` (base) |
| Parity | 173/173, 0 FP, 0 FN, 0 failures | 173/173, 0/0/0 on all 13 repo runs of every binary (base, L1a, L2a–L2d), including runs at load 125 | pass | `parity.json` in each `*-repo` run |
| Lever 1 A/B (L1a vs base) | lower CPU, parity unchanged | base 102.98 / 103.65 → L1a **98.8 / 98.78 s** (pair at load 2.8–4.8); second window 102.87 / 106.57 → 98.32 / 95.64 | pass | `…043420`, `…043522` vs `…043452`, `…043553`; `…053059`, `…053229` vs `…053159`, `…053331` |
| Lever 2a A/B (L2d vs L1a) | lower CPU, parity unchanged | L1a 98.32 / 95.64 → L2d **92.73 / 94.76 s** | pass | `…053159`, `…053331` vs `…053130`, `…053301` |
| Unit differential vs SQLite | rows equal | `sql::engine::tests::key_cells_match_sqlite`: 80 random table sets (a, b mixed types under INT/TEXT/REAL/NUMERIC, c node-id parts as symbols, spans, fact integers), 34 statements, vs SQLite 3 (rusqlite, bundled) | pass | `cargo test --release -p facts-a --features mutants --bin facts-a sql::engine` |
| Mutants 25 (R16), 26 (R17) | each killed by a unit test | `FACTS_MUTANT=25` and `=26`: `key_cells_match_sqlite` fails (round 1 resp. 15); mutant 0: all 3 engine tests pass; 23 still killed by `rewrite_range_probe_equals_scan` | pass | same command per mutant |
| `facts-gate` (integration's gate script) | green | **23/25**: every facts-a check passes (`a-replay` 1259/1259, `a-repo` 173/173 over 9,010 files, `a-subset`, `a3-check`, `a-reference` equal to the reference, `explain-physical`, `views-derived` 0 bag/0 nodes mismatches); failing: `rerail-facts-lib` and `classifier-leak`, both on `classify::tests::leak_{module_projection,reference_edges}_in_per_file_plan_is_rejected` in `facts/src/classify.rs`, a crate this branch does not touch (the failure is on integration `4387f42ad`, from the S3 merge). S5's `--mutants --only facts-a-unit,a-subset,a-unrewritten` is refused by integration's script (unknown checks) | facts-a part pass; gate red for a pre-existing reason | `~/rerail-bench/facts/phase-2/s5-gate/runs/20261023T053528-gate` (load 3.1, 1,250 s) |
| `pattern-check --shard` | 0 mismatches on ≥ 2 of 8 shards after lever 2 | **not run.** Lever 2a changes no lowered input (R12/R13 untouched); the SQL engine side is covered by parity and the SQLite differential | open | — |
| Memory (G2.5) | per-file end ≤ 300 MB | process max RSS 3.13 / 3.13 GB (L2d) vs 3.24 GB (base); the per-file-phase end figure was not extracted | partial | L2d runs above |

## 2. Per-lever A/B and where it moved (seconds; mean of the pair `…053229`/`…053301`, base vs L2d)

| stage | base | L2d | lever |
|---|--:|--:|---|
| `nodes` (all clauses) | 8.67 | 3.68 | R16 (no concatenation per row), window keys as cells, constant partition keys dropped |
| `ownership` | 7.16 | 5.94 | cells + R16 keys; still the bag fixpoint |
| `descendants` | 5.27 | 5.39 | unchanged within noise: the closure's per-row cost is now its `rows_seen` set and output, not keys |
| `direct_events` | 4.83 | 4.03 | IN lists and probes on cells |
| `<final>` | 4.50 | 4.51 | — |
| eval (thread total) | 88.3 | 76.4 | |
| CPU (process) | 106.6 | 94.8 | |

Regressions (base → L2d, mean of pairs): `quality-conditional-1` `ownership` 1.32 → 1.94, `returns` 0.24 → 0.42, `function_colors` 0.26 → 0.42, `node_ordinals` 0.40 → 0.54; `state-projection-recovery-2` `<final>` 2.04 → 2.43. Suspect: `quality-conditional-1`'s `nodes` joins `node_ordinals`, and its ids fall back to the lazy `Column::Tid` (parts read through gathers and chains per cell) instead of `TidFlat`, or its kind part is not a symbol. Not diagnosed (resume note 2).

Profile, sub200, 1 thread (`…035456-prof-sub200-base` vs `…050946-prof-sub200-l2d`): index builds 19.9% → 13.2% of `execute`, `Column::get` 19.5% → 2.2% (its work moved to `Column::cell`, 17.2%), `Index::find`+`get` 22% → `Index::group` 6.3%. Global index builds on sub200: 54,575 builds over 142 M rows, 9.19 s before (65 ns a row; the per-row `Vec<Key>` and `__path` hashing).

## 3. What changed (`spikes/facts-a/src/sql/engine.rs`)

1. **Borrowed cells** (`Column::cell` → `Cell`): a value read without cloning; source spans as `&str`, stored texts as `&Rc<str>`, symbols by id, node ids by parts. `cell_aff`/`cell_cmp_aff` are `apply_affinity`/`compare_affinity` with every unchanged case borrowed and every conversion the owned original (`maybe_numeric` is a necessary condition of `text_to_number`).
2. **Key cells** (`KCell`, `cell_key`): `key(compare_affinity(·))` on cells; one hash for every form of a text (`text_hash`: a text of node-id form hashes by its parts, a symbol's hashes memoised per thread). Used by `Index` (build, probe, group confirmation; no `Vec<Key>`), probes (`probe_key` → inline `KeyBuf` of ≤ 4 parts), the closure operator, and a new `Interner` behind `Distinct` (`UNION`, recursive `UNION`, `DISTINCT`) and `GROUP BY`.
3. **R17 constant key columns** (`Column::konst`): a key column that is one non-NULL value on every row (the partition's `__path`) is compared once per lookup instead of hashed per row; window partition/order keys on such a column of an inner-joined item are dropped. `FACTS_REWRITES=0` turns both off.
4. **Comparisons on cells**: `=`/`<`/…, `IN (…)`, `IS NULL` on columns and literals compare cells (`compare_cells`/`cmp_cells` = `value::compare`/`cmp`).
5. **R16 text node ids as parts**: an output column `a‖':'‖b‖':'‖c` of three plain columns (`tid_parts`) becomes `Column::TidFlat` (symbol + two `u32` offsets, 12 B a row) when every row fits, else the lazy `Column::Tid`; reads format the text only where the value itself is read; keys and comparisons use parts (equal ⇔ parts equal, since decimal offsets hold no `:`). Gathers compose into a lazy id's parts; a union of lazy ids is the id of the unions; CTAS affinity is the identity on ids (a text with `:` is never numeric). `FACTS_REWRITES=0` turns R16 off.
6. Window keys read as cells from the kept bindings; the sort compares cells.
7. Registry: R16 and R17 rows in `docs/facts/REWRITES.md` (facts-a table), mutants 25 and 26 documented in `mutant.rs` (numbers from S5Gate-3; 24/R15 went to S3CrossFile-4; next free 27/R18).

## 4. Interface changes

`sql::engine::{Cell, TidKind, KCell, KeyBuf}` (crate-private), `Column::{Tid, TidFlat}` (public enum variants), `Column::cell`, `Table::cell`. Env: `FACTS_REWRITES=0` now also disables R16 and R17. No change to `rel.rs`, `eval.rs`, `lowered.rs`, `repo.rs`, `facts/src/{rewrite,phys}.rs`.

## 5. Residue toward 90 s and what each part needs

Measured floor: **93.7 s** (L2d, mean of two). Per clause, L2d run `…053301-repo` (s): state-flow-1 20.7, quality-contracts-1 14.7, quality-conditional-1 13.7, legacy-execution-graphs 7.3, state-signatures-1 5.0, state-projection-recovery-2 4.0; categories: inputs 19.6, selection 5.4, tree views 11.3, Python-derived views 14.5, rule-logic SQL 28.2; parse 9.5, extract 1.0, finish 7.2.

| item | s | what it needs | estimate |
|---|--:|---|---|
| `descendants` (closure, 12 clauses) | 5.4 | R9 as an engine operator: when the edge table is a forest over distinct node ids (checked per file: each `dst` once, acyclic), drop the `rows_seen` set and emit per seed its subtree in the closure's order (level by level = preorder within each depth, so a depth-bucket pass over the preorder interval reproduces it exactly) | 3–4 s [INFERENCE] |
| `ownership` (bag fixpoint, 12 clauses) | 5.9 | one-pass walk: the recursive step is a key join on `edges`/`nodes` with three CASEs over the child kind; evaluate it per level with integer node ordinals instead of the generic select pipeline | 3–4 s [INFERENCE] |
| `quality-conditional-1` regression | ≈ 1.5 | find why its ids are slower (resume note 2) | 1.5 s |
| matcher inputs | 19.6 | lever 3 (shared capture passes across a clause's members) | 3–5 s (S7b) |
| `<final>`, `direct_events`, `return_kinds`, `ir_names` | 13.2 | IN lists of literals as a key set; R7 fusion of `direct_events`' 20 union arms over `calls` | 2–3 s [INFERENCE] |

Either tree operator alone closes the gap on paper; both give margin.

## 6. Pre-existing differences from SQLite found by the new differential (not changed; outside lever scope)

1. **Untyped columns.** The planner reads a column without declared type as having no affinity (an expression's); SQLite gives such a column BLOB affinity. A BLOB column compared with a TEXT column therefore converts in the engine (`2` = `'2'`) and not in SQLite. CTAS stage columns whose expression has no affinity are such columns. The test uses typed columns only.
2. **UNION representative.** For equal values of different classes across arms (`2` from an INT column, `2.0` from a REAL column) SQLite returns a different one than the engine's first seen. The test unions within one column.

Neither shows in the corpus (parity 173/173), but both are exactness gaps for the decider to rule on.

## 7. Gate

`just facts-gate` on the final tree (integration's script, load 3.1): 23/25, all facts-a checks pass; `rerail-facts-lib` and `classifier-leak` fail on two `classify::tests::leak_*` tests in `facts/src/classify.rs`, untouched here, so red on integration `4387f42ad` itself. Run `~/rerail-bench/facts/phase-2/s5-gate/runs/20261023T053528-gate`.

## 8. Resume notes (S8)

1. **Commits** on `Moti-Velz/facts-engine-s8-perfile` (local, not pushed): see `git log` (`s8:` prefix).
2. **First:** diagnose `quality-conditional-1` (`ownership`, `returns`, `function_colors`, `node_ordinals` slower after R16). Check whether `Column::tid` falls back to the lazy form there (its `nodes` reads the union of `syntax_rows_*` joined to `node_ordinals`), e.g. a temporary `eprintln!` at the two `return Column::Tid` sites of `Column::tid`.
3. **Then the tree operators** (§ 5), each as a registered rewrite with a mutant (27/R18 onward from S5Gate-3), measured with `tools/ab.sh base l2d <new>` (waits for load < 6).
4. Owed rows: a third G2.3 run; `pattern-check --shard k/8` on two shards; G2.5 memory from the L2d runs' `stderr.txt`; `just facts-gate --mutants --only facts-a-unit,a-subset,a-unrewritten` once S5's gate (with `facts-a-unit`) is on integration.
5. Tools: `~/rerail-bench/facts/phase-2/s8-perfile/tools/{ab.sh,sub.sh,prof.sh,residue.py,sample_incl.py,sample_callers.py}`.

## 9. S8b (wave 5): tree operators R18/R19, SQLite rulings — status at hand-off

Branch `Moti-Velz/facts-engine-s8b-tree`, worktree `.worktrees/s8b-tree`. Runs `~/rerail-bench/facts/phase-2/s8b-tree/runs/`, binaries `…/bin/facts-a-{base,s1,s2,s3,s4}` (base = integration `8e712ad65`). **G2.3 is not closed with margin; the gate was not run on this tree** (stopped at the request budget).

### 9.1 Gate rows

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| G2.3 | ≤ 90 s CPU, 3 runs, margin | quiet pair (load 1.8–2.9): base 89.79 / 90.63 → s1 89.04 / 89.62; noisy window (1-min load 3.9–4.8, 15-min 10–11, a sibling compiling): s1 94.80 / 93.54, s3 95.81 / 94.20, s4 94.45 / 93.90 | **fail / not measured quietly**: the three final quiet runs were not taken | `20261023T063105`…`063230` (pair 1); `20261023T0729{12,41}`, `0730{11,40}`, `0731{08,36}` |
| Parity | 173/173, 0/0/0 | 173/173, 0 FP/FN/failures on all 14 runs (base, s1, s2, s3, s4) | pass | `parity.json` per run |
| R18/R19 differentials, mutants 27/29 | killed | `rewrite_tree_closure_equals_closure` fails under `FACTS_MUTANT=27`, `rewrite_walk_equals_levels` under `=29`; mutant 0: all 5 engine tests pass; 25/26 still killed by `key_cells_match_sqlite` | pass | `cargo test --release -p facts-a --features mutants --bin facts-a sql::engine` |
| SQLite rulings 1, 2 (engine) | rows = SQLite 3.53.2 | `key_cells_match_sqlite` extended (untyped columns in `b`, 13 new statements); fails with the BLOB rule disabled (round 1, `CAST(a.v AS TEXT) = b.v`) and with first-seen / last-seen UNION representatives | pass | same command |
| `facts-gate --mutants` | green | **not run** | open | — |
| `pattern-check --shard 0/8, 1/8` | 0 mismatches | **not run** | open | — |

### 9.2 Per-step A/B (s, mean of each pair; process CPU in the noisy window is within noise)

| step | binary | change | stage effect (corpus, 4 threads) | CPU |
|---|---|---|---|---|
| 1 | s1 vs base | `Column::tid` takes the flat form for text kinds naming an existing symbol (`facts::lookup`, once per distinct text): `quality-conditional-1` is a repository clause whose spilled kinds are texts, so its ids fell back to lazy `Column::Tid` (diagnosed: 7 fallbacks on sub200, all in that clause) | sub200 1 thread, `quality-conditional-1` total 1.067 → 0.858 s (`ownership` 0.261 → 0.153, `returns` 0.063 → 0.032) | 90.21 → 89.33 (quiet pair) |
| 2 | s3 vs s1 | R18 tree closure (+ the closure's lookup cache keyed by node key, not node class: `5`/`5.0` key differently under TEXT) | `descendants` 5.37 → 4.22 s | 94.17 → 95.00 (noisy) |
| 3 | s3 | R19 reference walk | `ownership` 5.19 → 5.27 s: **no gain** (its cost is per-row probes and CASE conditions, which R19 keeps) | — |
| 4 | s4 vs s3 | rulings 1, 2 | neutral | 95.00 → 94.18 (noisy) |

### 9.3 What changed (`spikes/facts-a/src/sql/engine.rs`)

- R18 `Exec::tree_walk`, R19 `Exec::walk` + `Column::Pick` (gathers over a pick compose), both behind `PCte::Recursive::tree` (= planner `rewrites`, off under `FACTS_REWRITES=0`); registry rows R18/R19 in `REWRITES.md` (mutants 27, 29; `mutant.rs`); SQLite pin 3.53.2 recorded in `REWRITES.md`'s header.
- Ruling 1: planner tracks BLOB affinity (`ScopeItem::blob`, `PSelect/PQuery::out_blob`, `stored_blob`, `Planner::blob`); `blob_comparison_affinity` for comparisons, CASE operand, IN (SELECT).
- Ruling 2: `Exec::body` UNION keeps, per class, the right-most arm's first row (SQLite 3.53.2; Python's 3.51 keeps the last), `Distinct::slot`.

### 9.4 Resume notes (S8b)

1. **Decider's extended ruling (2026-10-23) is only partly done.** Engine still owes (c) bare columns in aggregates: exact under one min()/max() (any aggregates alongside; the engine's current `lone_extremum` requires it to be the only aggregate) or FD-determined, else runtime refusal when a bare column takes two distinct values (class + value) in a group; and (d) compound FROM-subquery/CTE/CTAS column affinity (left-most arm's, BLOB when `m` disagrees per `sqlite3SubqueryColTypeAndCollation`, Int→Real on read under REAL), each with a differential vs SQLite 3.53.2 and a mutant (odd numbers 31+). 3.53.2 facts: `SELECT a, min(k), count(*), b … GROUP BY a` reads the min row; `SELECT x FROM (SELECT v AS x FROM r UNION ALL SELECT k FROM nn)` (r.v REAL, nn.k NUMERIC 1) → 1.0.
2. Subagent `S8bTree.FactsTyping` was launched on the same worktree for (a)–(d) in `facts/src/reference.rs` and `facts/src/front/sql.rs` (commit prefix `s8b-facts:`, informs S3CrossFile-5); check its result/commit before building.
3. Then: `just facts-gate --mutants` (a-vs-reference may flag the reference/engine rulings until FactsTyping lands), three quiet `run-repo.sh` runs, `pattern-check --shard 0/8` and `1/8`.
4. Toward G2.3: R19 as built does not pay; the remaining measured costs are index builds (13% of sub200 samples; `Index::build`'s hash map is not pre-sized: rehash 11% of build time), the seeds select of `descendants` (`kind NOT IN (…)` compares symbol names per element), the `quality-contracts-1` reachability roots UNION (Distinct 43% of `reach`), and `ownership`'s per-row probes.
5. Tools: `~/rerail-bench/facts/phase-2/s8b-tree/tools/{ab.sh,prof.sh,sample_incl.py,sample_children.py,sample_callers.py}`.

## 10. S8c (wave 5): engine rulings (c), (d); the branch gated

Branch `Moti-Velz/facts-engine-s8b-tree` (worktree `.worktrees/s8b-tree`), on S8b-facts' `093977aa2` (reference and front-end typing for rulings (a)–(d)). Runs `~/rerail-bench/facts/phase-2/s8c/runs/`, binaries `…/s8c/bin/facts-a-{s8c,s4}` (`s4` = S8b's tip before S8c, for the A/B). `facts/src/front/sql.rs` and `facts/src/reference.rs` are unchanged (S3CrossFile-5's typing hash still holds).

### 10.1 Gate rows

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| `facts-gate --mutants` | green | **28/28 checks pass, 30/30 mutants killed** (946 s; 1-min load 71.8 at start, siblings compiling), commit `b758fa429` | pass | `~/rerail-bench/facts/phase-2/s5-gate/runs/20261023T083612-gate` |
| `a-vs-reference` | engine = reference on every compared stage | 0 mismatches on 11,667 stages (836 from 38 recorded calls, 10,831 from 288 subset files in 48 clauses); not compared: 1,924 per-file runs over `--max-nodes`, 182 reference refusals (syntax reachability and `source_text` in repository plans, as before), 3 runs over `--max-input-rows`. No stage was refused as `bare column` or FLEXNUM on either side | pass | same, `a-vs-reference/results.json` |
| `facts-a-unit`, `rerail-facts-lib` | pass | 12 facts-a unit tests, 30 lib tests | pass | same |
| Rulings (c), (d) differentials | engine = reference = SQLite 3.53.2 | `bare_columns_match_sqlite`: 150 random databases × 20 queries (3,000 cases, refusals and compared rows both occur); `compound_types_match_sqlite`: 150 × 27 (4,050, 6 of them as stages, CTAS read back). Each case: the engine's rows or refusal equal the reference's exactly (same refusal kind), and SQLite's rows whenever rows come back (one query compared with the reference only: § 10.3, 1) | pass | `cargo test --release -p facts-a --features mutants --bin facts-a sql::engine`: 7/7 |
| Mutants 31, 33 | killed | 31 (bare-column check dropped) fails only `bare_columns_match_sqlite`; 33 (arm walk stops at the left-most arm) fails only `compound_types_match_sqlite`; 22, 25, 26, 27, 29 each still fail their own test; mutant 0: 7/7 pass | pass | same command per `FACTS_MUTANT`; the gate's `mutants/31-…`, `mutants/33-…` |
| G2.3 | ≤ 90 s CPU, 3 runs | `run-repo.sh` × 3, 1-min load at start 5.95 / 5.49 / 5.15 (15-min 17.2 / 16.7 / 16.4: the machine was coming off the gate and siblings' builds): **91.70 / 92.07 / 92.28 s** CPU, wall 28.2 / 27.6 / 27.8 s | **fail on the target** (not a regression: interleaved A/B at load 3.8–4.0, `s4` 92.85 / 92.65 vs `s8c` 92.91 / 93.19, +0.3 s, within noise) | `20261023T085450-repo`, `…085518-repo`, `…085546-repo`; A/B `…085708`, `…085747`, `…085819`, `…085852` |
| Parity | 173/173, 0/0/0 | 173/173, 0 FP/FN/failures on all 7 runs | pass | `parity.json` per run |
| `pattern-check --shard 0/8`, `1/8` | 0 mismatches | shard 0: 1,127 files, 3,547,499 rows, 0 physical / 0 reference mismatches (reference on 116 files ≤ 200 nodes), 93 s wall, 356 s CPU; shard 1: 1,127 files, 3,003,329 rows, 0 / 0 (134 files), 107 s, 411 s | pass | `20261023T085931-pattern-check-full-ref200-shard0of8`, `…090104-…shard1of8` |

### 10.2 What changed (`spikes/facts-a/src/sql/engine.rs`, `rel.rs`)

- **Typing as the front end's.** The planner's per-column BLOB flag became `Ty` (none, BLOB, TEXT, NUMERIC, INTEGER, REAL, FLEXNUM: `front::sql::SAff`'s order), carried by scope items, CTEs and queries; each select records its output columns' `ArmCol` (type, `sqlite3ExprDataType`, whether a `CAST`). `cmp_ty` is the old BLOB comparison rule.
- **(d) Compound column types.** `query_ty` types each query's columns over its arms (`walk_arms` past arms of no type accumulating their data types, `settle`: TEXT → BLOB when a later arm may hold a number, numeric → BLOB when one may hold a text, numeric with a `CAST` left-most arm → FLEXNUM); `PQuery::out`'s affinities are these. A FROM subquery's or plain CTE's REAL columns read integers as reals (`real_cols`, `real_read` at the end of `Exec::query`). Recursive CTEs are typed over all arms column by column, the steps re-planned with the CTE's columns typed so far (`trial_steps`, whose plans are dropped); with a REAL column every arm's rows convert in the generic fixpoint and neither the closure nor R18/R19 runs (`PCte::Recursive::real`, `closure_applies`). A scalar subquery has its right-most arm's first column's type (`PQuery::first`); a compound in `IN (SELECT …)`, or a compound scalar operand, compares with one affinity on both sides (`cmp_ty_both`). Stages are planned by `plan_stage`, which refuses a FLEXNUM column with the front end's message.
- **(c) Bare columns.** `PSelect::extremum`: exactly one `min()`/`max()` (others alongside; calls of one function over one column are one, the reference's `same_call`); the representative is the row it took last, every row while it took none. Otherwise `PSelect::bare` lists the `(item, column)`s the select list, `HAVING` and `ORDER BY` read outside aggregates, through subqueries (`cols_read_at`, the reference's `ir::cols_read`); a row disagreeing with the group's previous one on any of them (`cells_identical`: storage class and value) fails the execution with `bare column: …` (as the reference refuses; the check runs before `HAVING`, as the reference's does).
- `sum()`/`total()` read a value as an integer only when it is one under numeric affinity (`'12abc'` adds 12.0, `'abc'` 0.0, both making the sum REAL), the reference's rule.
- Mutants 31, 33 in the gate's `MUTANTS` (family `facts-a-unit`), `mutant.rs`, REWRITES.md § Numbers.

### 10.3 Found while testing (reference = truth; the engine follows it; for the decider)

1. **Ties of the one extremum across join rows.** `SELECT a.p, b.v, min(a.k) FROM a, b WHERE b.p = a.p GROUP BY a.p`: when several join rows share the group's minimum (one `a` row matching several `b` rows), the reference and the engine take the first in text-order nested-loop order, SQLite 3.53.2 the first in its own join order ([INFERENCE] it loops over `b` outside or through an automatic index; 32 of 150 random databases differ). Example (raw inserted values): `a(k NUMERIC, v TEXT, p untyped)` = `(0.0,'2','p'), ('2.5','1.5','1'), (3.5,2,'1'), (1,2,1.0)`, `b(k TEXT, v REAL, p REAL)` = `(0.0,2.5,1), (1,'a',1.0), ('2','1.5',1), (2.0,2,NULL)`: reference and engine `{1.0, 2.5, 1; '1', 2.5, 2.5}`, SQLite `{1.0, 1.5, 1; '1', 1.5, 2.5}`. Single-table scans agree (rowid order is input order). Exact would need the ruling's lone extremum to refuse when its value ties between rows whose bare columns differ (as window ties are refused). That query is compared with the reference only in `bare_columns_match_sqlite`. No corpus stage hit a bare-column decision differently (a-vs-reference 0 mismatches; parity 173/173).
2. **Conservative refusals.** A select item that is the GROUP BY expression itself reads its input columns, so `SELECT k || ':' || v, count(*) … GROUP BY k || ':' || v` is refused when the parts differ inside one group (a NULL part makes every key NULL), where SQLite [INFERENCE: reads the select item from the group's key] is exact. `key_cells_match_sqlite` skips such refusals (its round 2 has one).
3. **Pre-existing engine/IR differences outside (a)–(d), not changed:** unary `+` keeps its operand's affinity in the engine (`Planner::expr` returns the operand) where the IR drops it (`Expr::Plus`); `CASE x WHEN …` takes one comparison affinity (the last `WHEN`'s) where the front end types each `WHEN`; an integer `sum()` overflow becomes REAL in the engine where the reference refuses (`integer overflow`, as SQLite). None shows on the compared stages.

### 10.4 Resume notes (S8c)

1. **Commits** on `Moti-Velz/facts-engine-s8b-tree` (local, not pushed): `2b11eba0d` (s8b: R18, R19, flat text-kind ids, rulings (a), (b)), `093977aa2` (s8b-facts: reference and front-end typing, (a)–(d)), `b758fa429` (s8c: engine (c), (d), sum, differentials, mutants 31/33), then this report (`s8c:`). The branch is gated (§ 10.1) and ready to merge.
2. **G2.3 is still open**: 91.7–92.3 s at load ≈ 5 (S8b's quietest pair at load 2–3: 89.0 / 89.6 s). S8c is neutral on CPU; the levers left are § 9.4 note 4's (pre-sized `Index::build`, `descendants`' seeds `NOT IN`, the `quality-contracts-1` roots UNION, `ownership`'s probes). Take the three G2.3 runs at load < 3.
3. **For the decider:** § 10.3, 1 (extremum ties across join rows) needs a ruling for the reference first; the engine change would be the same refusal in `select_rows`' grouped loop.
4. Tools: `~/rerail-bench/facts/phase-2/s8c/tools/ab.sh <label>…` (S8b's, run root `s8c`; waits for load < `MAXLOAD`, default 6).
