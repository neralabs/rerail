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
