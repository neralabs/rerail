# Phase 2 / S7 one pass (wave 3)

Status: **steps 1–3 landed; the generic path beats the A3 path in both A/B pairs; `subst.rs` not yet deleted (step 4 is mechanical, listed below); G2.3 ≤ 90 s not met.** Branch `Moti-Velz/facts-engine-s7-onepass` (local), worktree `.worktrees/s7-onepass`. Runs: `~/rerail-bench/facts/phase-2/s7-onepass/runs/`.

**S7b (2026-10-23, § 7):** `subst.rs` and every `FACTS_A3_*` switch deleted; gate 25/25; full-corpus `pattern-check` 0/0 over 9,010 files; R14 (range probe) landed, 114.2 → 106.5 s CPU, parity 173/173. G2.3 ≤ 90 s still open: 16.5 s to go, residue in § 7.4. Branch `Moti-Velz/facts-engine-s7b-retire`, worktree `.worktrees/s7b-retire`. Runs: `~/rerail-bench/facts/phase-2/s7b-retire/runs/`.

## 1. Finding that changes the G2.3 picture

The decider's ledger says the A3→generic gap is "A3's dump groups ≈ 10 s vs one AST evaluation per input ≈ 61 s". That compares the wrong counters: A3's dump selection is booked as SQL stage time (`view:dump.select`), not `input_rows`. In the decider's own quiet pair (`s2-perfile/runs/20261022T221135-dq-a3-2` vs `…221213-dq-generic-2`):

| | A3 path | generic path (`FACTS_A3_OFF=1`) |
|---|--:|--:|
| `input_rows` | 8.0 s | 66.5 s |
| `view:dump.select` + `.rows` + `.ancestor` | 51.0 s | — |
| matcher inputs, true total | **59.0 s** | **66.5 s** |
| `nodes` / `descendants` / `ownership` | 6.9 / 5.8 / 3.1 s | 8.5 / 7.9 / 6.7 s |
| CPU | 134.9 s | 158.6 s |
So A3's dump groups cost ≈ 59 s, not 10 s; the step-2 target "≤ A3's ≈ 10 s" was never A3's number. The selection logic itself (the 20 KB `quality-conditional-1-selection` over all 28 M nodes) is the cost, and only a cheaper evaluation of it (not one pass vs many) removes it.

## 2. Gate rows

All timing pairs ran under `bench-lock`, but at load 12–73 (other slices building and gating): **untrusted by the protocol**; the decider must re-run on a quiet machine. Parity was identical on every run.

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| G2.3 A/B pair 1 (binary `bin/facts-a-p3`) | generic ≤ A3 | A3 158.8 s, **generic 142.4 s** CPU; inputs 9.6 + dump-in-stages vs **30.9 s**; load 12–20 | generic < A3 (untrusted) | `20261022T234138-repo`, `20261022T234230-repo-generic` |
| G2.3 A/B pair 2 (`bin/facts-a-p4`, + closure keys, restricted passes, decoded outputs) | generic ≤ A3 | A3 207.2 s (`view:dump.select` 95.5 s), **generic 173.4 s**; load 53–73 | generic < A3 (untrusted) | `20261022T235059-repo`, `20261022T235214-repo-generic` |
| G2.3 ≤ 90 s | ≤ 90 s, 3 runs | not met; quiet re-run needed (estimate from pair 1 scaled to the decider's quiet ratio: ≈ 125 s) | **fail** | — |
| Parity | 173/173, 0 FP, 0 FN, 0 failures | 173/173, 0/0/0 on all four runs | pass | `parity.json` in each |
| `pattern-check`, physical vs leaf executor | 0 mismatches | **0 mismatches on 8.1 M rows** over the 200 heaviest files (sub200), 340 lowered inputs, 339 planned | pass (sub200) | `…20261022T2333*-pattern-check-sub200` (latest) |
| `pattern-check`, full corpus 9,010 files | 0 mismatches | **not completed**: the run (`bin/facts-a-p2`, reference on files ≤ 200 nodes) hit the 1 h cap with no report; shard it or skip the reference half | **open** | `…20261022T233157-pattern-check-full-ref200` (empty) |
| Gate on the wave-3 tree (before the integration merge) | 25/25, all mutants | **25/25 checks, 21/21 mutants**, 1,723 s at load 10–70 | pass (time untrusted) | `runs/*-gate` |
| After merging `d47f859c4` | parity, exactness | build green; `pattern-check` sub200 0/0; generic repo 173/173, 0/0/0 (gate not re-run) | pass (partial) | `…*-pattern-check-sub200-merged`, `…*-repo-generic-merged` |
| Memory (G2.5, generic) | per-file phase ≤ 300 MB end | end of per-file phase 121 / 108 MB; per-file peak 497 / 476 MB; overall 3.05 / 3.04 GB (A3 5.86 GB) | pass (end) | same pairs |

## 3. What changed

1. **Dump inputs lowered** (`facts/src/front/pattern.rs`): root-chain captures by relation searches (`any[REL{… $X …}, $_]` and `REL{… $X …}`) as `LEFT JOIN node x ON x.id = (first candidate in search order)`; stop rules (`stopBy: <rule>`, two-valued) as `Axis::{Descendants,Ancestors}Until`; `inside` with `field` (neighbour and end); `value` tests anywhere with their exact reach (all: earlier conjuncts not FALSE; any: not TRUE; relations: the anchor reached and no earlier candidate TRUE) and a string-literal guard; projections of capture kind/start/end/text/0-based line. `pub enum Axis`, `pub fn axis_pred`, `pub fn nodes`. Lowered inputs 65 → 340 of 622; every dump input but one (`serverplan-steps/raw_nodes`: a recursive utility, `step-user-callable`) lowers.
2. **Physical plan of lowered inputs** (`spikes/facts-a/src/lowered.rs`, new; R12/R13 in `REWRITES.md`): the algebra planned once per rule onto set-at-a-time operators over the file's nodes; axis `EXISTS` recognised by structural equality with `axis_pred`; predicates hash-consed per rule and memoised per file over the nodes evaluated so far (the union of a clause's selections evaluated once per node, generically); captures read from the facts; three-valued `value` tests; the `\N{` side condition per file (the corpus has none). `eval.rs` routes every planned input through it, else the leaf executor. `FACTS_REWRITES=0` disables it (unrewritten plan).
3. **Closure operator** (`sql/engine.rs::closure`): distinctness as `(carried id, node id)` u64 keys; edge lookups cached per `(carried probe values, node)`; `dst` interned once per edge row. Rows and order unchanged.
4. **Reference evaluator** (`facts/src/reference.rs`): `Evaluator::with_shared_content` scans lent sources by reference (filters, inner joins without `ON`, projections stream; `EXISTS`/scalar subqueries stop at the first row). Old constructors unchanged (S3 informed).
5. **`pattern-check`** is now three-way: leaf executor (truth) vs physical plan on every file vs reference on files ≤ `--max-nodes`.
6. `derived_check.rs`: A3's dump-group construction moved into the harness (`derived_check/a3dump.rs`, S5 agreed), so the engine no longer needs it.
7. Mutants 1–3 re-seeded in `lowered.rs` (stop rule dropped; last `OR` arm dropped; descendants from `id + 2`).

## 4. Interface changes

`pattern::{Axis, axis_pred, nodes}` public; `lowered::{Preds, Plan, FileState, rows, plan, lower_and_plan, rewrites}`; `eval::Input.phys`, `eval::Rule.preds`; `eval::project` `pub(crate)`; `reference::Evaluator::with_shared_content`; env `FACTS_REWRITES=0`.

## 5. Residue toward 90 s (pair 1, load 12–20; seconds)

| item | generic | lever |
|---|--:|---|
| matcher inputs | 30.9 | `quality-conditional-1`'s 20 members ≈ 13 s: per-member O(n) passes remain (ancestor capture per member); share the capture passes across members (same axis, same inner predicate); plan the 10 decoded-value/regex-projection inputs and the recursive-utility input |
| `nodes` (window + 3 text concatenations per row) | 10.0 | origin tracing: `kind‖':'‖start‖':'‖end` over one capture is injective in the node id → integer ids (makes R6/R8/R9/R11 fire) |
| `descendants` (closure over text ids) | 9.8 | same origin tracing → R9 interval probe; closure keys landed (not yet measured quietly) |
| `ownership` (bag fixpoint) | 8.1 | level order is read by consumers; needs an order-preserving interval form or the rule narrowed |
| `import_identifiers`, `<final>`, `direct_events` | 7.2, 6.7, 5.6 | rule-logic SQL over gather views (`Column::get` 13% of samples): S4's views shared per content (R11) |
| parse + extract | ≈ 10 | fixed |

## 6. Resume notes (S7)

- **Committed:** `4dcfc9ae0` (§ 3; gate 25/25, 21/21) and `53251bff4` (merge of `d47f859c4`; `Rule` keeps `preds` and S6's `report_count_gt`). Full-corpus `pattern-check` still owed.
- **Step 4 (delete `subst.rs`), exact list**: remove `mod subst` (main.rs), `subst.rs`; `rel.rs` `kernels`/`dump` fields, `schedule_releases` kernel reads, the kernel/check branches in `run`; `eval.rs` `plan_clause` call, `DumpNs`, `dump_rows`, `nearest_where`, `PENDING`, the `from_dump` block in `eval_file`; `physplan.rs` `kernel_name` and kernel printing; `ast.rs` the three `check_mode` blocks and the mutant-3 site (S6 agreed to the blocks; ask for the mutant site); `run-repo.sh` label → `FACTS_REWRITES`. Then message S5 (switch `FACTS_REWRITES=0`; `pattern-check --max-nodes 2000` as a gate check, pass = 0 physical and 0 reference mismatches; mutants 1–3 killed by default vs `FACTS_REWRITES=0` findings).
- Quiet-machine A/B (3 runs each) is the decider's.

## 7. S7b: `subst.rs` retired, full-corpus `pattern-check`, residue, R14

### 7.1 Gate rows

All timing runs used `bench-lock` at load 4.4–5.6 (under the protocol's limit of 8). Each claim has two runs, not three: the decider's protocol is two pairs back to back. Runs are under `~/rerail-bench/facts/phase-2/s7b-retire/runs/`.

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| `subst.rs` retired | no shape-recognised substitution, no `FACTS_A3_*` | `subst.rs`, `mod subst`, the `RelPlan.kernels`/`dump` fields, the kernel branches in `rel::run`, `eval::{DumpNs, dump_rows, nearest_where}`, the `from_dump` block, `physplan::kernel_name`, the three `FACTS_A3_CHECK` blocks in `ast.rs` and its mutant-3 site are deleted. `derived_check/a3dump.rs` stays because `views-derived` uses it. The differential switch is `FACTS_REWRITES=0` | pass | commit `299fb8942` |
| Gate after deletion | 25/25 | **25/25**, 1,344 s. Built in the worktree, because a new worktree has no earlier gate build for `--no-build` to reuse | pass | `20261023T010236-gate` |
| Parity (deletion) | 173/173, 0/0/0 | 173/173, 0 FP, 0 FN, 0 failures (both runs) | pass | `20261023T014003-repo`, `…014147-repo` |
| Mutants 1–3 against the subset differential | killed | **2 killed; 1 and 3 survived** (`a-reference`). Row-level `pattern-check` under the mutant does no better: on the subset 0 mismatches for both; on all 9,010 files 0 for mutant 1 and **1 row** for mutant 3. S7's 21/21 counted the old sites in `subst.rs` and `ast.rs` | fail → fixed below | `20261023T012511-gate-mutants-a`, `…012610-…-subset-mutant1`, `…012625-…-subset-mutant3`, `…012710-…-full-mutant1`, `…012852-…-full-mutant3` |
| Mutants 1–3 against unit tests | each killed | `lowered::tests::{stop_rule_bounds_the_walk, every_disjunct_selects, first_descendant_is_a_descendant}` (physical vs leaf on synthetic files, each asserting that the input plans): `FACTS_MUTANT=1/2/3` fail exactly their own test; 0 passes all | pass | commit `b63025e9c`; S5 adds `facts-a-unit` to the family |
| `pattern-check`, full corpus | 0 mismatches, 9,010 files | **0 physical and 0 reference mismatches; 25,427,160 rows; 9,010 files; 340 lowered inputs (339 planned)**. The reference ran on the 1,025 files of ≤ 200 nodes. 8 shards (`--shard k/8`), 98–134 s wall and 379–506 s CPU each at 4 threads | pass | `20261023T0119*`–`0133*-pattern-check-full-ref200-shard{0..7}of8` |
| Reference on files ≤ 2,000 nodes | (sizing) | one 1/32 shard hit the 900 s cap with no report. The reference evaluates correlated searches in O(nodes²), so the reference half stays at 200 nodes | — | `20261023T010418-pattern-check-shard0of32-ref2000` |
| R14 A/B pair (`bin/facts-a-retire` vs `bin/facts-a-r14`) | lower CPU, parity unchanged | base **114.6 / 113.8 s**, R14 **106.9 / 106.0 s** CPU (−7.7 s, −6.8%); parity 173/173, 0/0/0 on all four; wall 33.9/33.8 → 32.4/31.6 s; end of per-file phase 111–116 MB, peak per-file 515–545 MB, overall 3.04–3.09 GB | pass | base `20261023T014003-repo`, `…014147-repo`; R14 `…014039-repo`, `…014114-repo` |
| R14 vs unrewritten, full corpus | equal findings and failures | default (R12–R14) vs `FACTS_REWRITES=0`: findings 173 = 173, failures 0 = 0, multisets equal. Unrewritten CPU 171.9 s | pass | `20261023T020354-repo-unrewritten` |
| R14 unit differential | equal rows and order; mutant 23 killed | `sql::engine::tests::rewrite_range_probe_equals_scan`: 60 random table sets × 8 statements; it fails under `FACTS_MUTANT=23` | pass | commit `12930ad01` |
| Gate with R14 | 25/25 | **25/25**, 1,114 s | pass | `20261023T014510-gate-r14` |
| G2.3 ≤ 90 s | ≤ 90 s | **106.5 s** (2 runs) | **fail** | above |

### 7.2 What changed

1. **Deletion** (`299fb8942`), following the exact list in S7's resume notes. `tools/run-repo.sh` labels a `FACTS_REWRITES=0` run `repo-unrewritten`, writes `env FACTS_REWRITES=` into `command.txt`, and checks parity for every no-argument run. `pattern-check --shard k/n` (same convention as `derived-check`) keeps each run under 15 minutes.
2. **Unit tests for mutants 1–3** (`b63025e9c`), described in § 7.1.
3. **R14 range probe** (`12930ad01`; registry row in `REWRITES.md`, physical table). The planner (`probe_range`) takes comparison conjuncts `c OP e` on one column of a FROM item, where `e` reads earlier items or outer rows, and turns them into a range probe. At run time (`RangeIndex`), the item's rows are sorted by `compare_affinity(c, aff)` in `value::cmp` order inside each group of the item's equality index. The bounds are found by binary search, and the rows come back in table order: as a slice when the sort kept table order, otherwise sorted. The comparisons stay in the item's conjuncts and are re-checked. A NaN in the column disables the index; a NULL bound yields no rows. `FACTS_REWRITES=0` turns R14 off, so the gate's unrewritten differential covers it. EXPLAIN gains `Item::RangeProbe` (`facts/src/phys.rs`), printed as `range <rel> on (#k…) #c aff A bounds lo,hi`.
   - Where it fired, per stage (mean of the pair): `import_identifiers` 6.83 → 0.35 s, `<final>` 5.90 → 4.64 s, `contexts` 1.92 → 1.47 s, `parents` 0.84 → 0.61 s, `shape_branches` 0.50 → 0.33 s.
   - Why `import_identifiers` was quadratic: its `n.__start BETWEEN imp.__start AND imp.__end` against the file's identifiers was a full scan per import range.

### 7.3 Interface changes

- `sql::engine::{RangeProbe, plan_with}`, `PSelect.ranges`.
- `rerail_facts::phys::Item::RangeProbe`.
- `patcheck --shard`.
- Env: `FACTS_A3_CHECK`, `FACTS_A3_OFF` and `FACTS_A3_PLAN` are gone; `FACTS_REWRITES=0` also disables R14.

### 7.4 Residue toward 90 s

The numbers come from the R14 pair (mean of `…014039-repo` and `…014114-repo`): CPU 106.5 s, of which per-file eval 88.7 (matcher inputs 20.0), parse 9.8, extract 1.0, repository finish 6.4. The decider's quiet generic runs (`s7-onepass/runs/20261023T004913-dq-generic-1`, `…005030-dq-generic-2`, 118.4 / 119.3 s) have the same shape with `import_identifiers` at 6.7 s. Categories, computed by `tools/residue.py`:

- **selection chain:** `syntax_rows_*`, `nodes`, `node_ordinals`, `edges`;
- **tree views:** `descendants`, `ownership`;
- **Python-derived views:** S4's `TARGETS` plus `COMPOSITE`;
- **rule-logic SQL:** everything else, `<final>` included.

Per clause (seconds):

| clause | matcher inputs | selection chain | tree views | Python-derived views | rule-logic SQL | repository load | total |
|---|--:|--:|--:|--:|--:|--:|--:|
| state-flow-1 | 2.1 | 3.8 | 5.1 | 10.4 | 5.4 | 0.0 | 26.9 |
| quality-contracts-1 | 0.6 | 0.0 | 1.9 | 1.6 | 11.7 | 0.0 | 15.9 |
| quality-conditional-1 | 6.8 | 1.9 | 1.4 | 0.4 | 2.0 | 0.5 | 13.0 |
| legacy-execution-graphs | 0.9 | 2.2 | 1.7 | 0.6 | 4.5 | 0.0 | 9.8 |
| state-signatures-1 | 1.7 | 1.0 | 1.3 | 0.8 | 1.7 | 0.0 | 6.4 |
| state-projection-recovery-2 | 0.9 | 0.4 | 0.4 | 0.2 | 2.1 | 0.0 | 3.9 |
| quality-contracts-2 | 0.1 | 0.0 | 0.0 | 1.0 | 1.0 | 0.0 | 2.1 |
| runtime-reference-summaries | 0.2 | 0.5 | 0.0 | 0.7 | 0.2 | 0.0 | 1.6 |
| authority-sites | 0.7 | 0.0 | 0.0 | 0.1 | 0.7 | 0.0 | 1.5 |
| required-contracts | 1.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 1.1 |
| 43 other clauses | 5.0 | 0.3 | 0.3 | 1.4 | 1.5 | 0.0 | 8.6 |
| **all** | **20.0** | **10.2** | **12.1** | **17.2** | **30.8** | **0.5** | **90.8** |

Extraction (parse 9.8 + extract 1.0 s) is per file, not per clause.

**Operator profile.** `/usr/bin/sample`, R14 binary, 1 thread, the 200 heaviest files (`…014334-prof-sub200-r14`). These files carry most of the cost: `nodes` 6.9 of 9.0 s, `ownership` 6.1 of 7.1 s, `descendants` 4.9 of 5.5 s in the base profile `…011949-prof-sub200`. Shares of `eval_file` (inclusive):

| operator | share |
|---|--:|
| hash index builds (`Exec::index`, `Index::build` inlined) | 19.8% |
| `Column::get` (callers: `Exec::eval` 43%, index build 24%, `Index::find` 23%) | 19.6% |
| `Out::push` | 14.4% |
| `Exec::recursive` (bag fixpoints) | 13.9% |
| `Index::find` | 13.2% |
| `Exec::sub` | 10.8% |
| `lowered::rows` | 10.1% |
| `Exec::closure` | 4.7% |
| leaf `Input::rows` | 4.7% |

Tools: `~/rerail-bench/facts/phase-2/s7b-retire/tools/{residue,sample_incl,sample_callers}.py`.

Top 10 stages and the levers that apply:

| # | stage | s | where | lever |
|---|---|--:|---|---|
| 1 | `nodes` | 8.63 | 4 texts (6 state-*, 6 step-contracts, 2 others) | (d) integer node ids by origin tracing: `kind‖':'‖start‖':'‖end` over one capture is the node id if it is unique per file (a per-file check) and the id is only compared, never output. This removes three text concatenations per row and makes every later join on `id`/`parent`/`edge_parent` integer-keyed. (c) index builds on the text ids |
| 2 | `ownership` | 6.84 | 12 clauses, 1 text | (a) one preorder pass over the selection forest instead of a level-by-level bag fixpoint, keeping level order for its readers. Needs `edges` proven to be the selection's induced forest, so the same origin tracing first. (b) does not apply: the anchor is an anti-join on roots of the selection, so the view is not subtree-closed. A text-sharing bound of 4.0 s exists only if it did |
| 3 | `descendants` | 5.24 | 12 clauses, 4 texts (7 and 3 clauses share the two `WITH RECURSIVE walk` texts) | (a) R9 interval probe, which `rel::reach` already runs for `reachability` stages over syntax. It needs the same origin tracing to fire on the SQL form. The `expand` variant (3 clauses) also needs the filter carried |
| 4 | `direct_events` | 4.82 | state-flow-1 (≈ all), state-flow-project-1 | (c): 20 `UNION` arms, 12 of them over `calls` (most joined to `body_nodes`). A physical R7 (union of selections over one base) would join once. (b) bound ≈ 0: one clause carries the cost |
| 5 | `<final>` | 4.64 | state-projection-recovery-2 2.1, legacy-execution-graphs 1.9 | (c). R14 already took 1.3 s; legacy's `f.__start<=n.__start AND f.__end>=n.__end` is bounded on one side only (the other bound needs I2's `start ≤ end`) |
| 6 | `return_kinds` | 2.52 | state-flow-1 | (c); (b) bound ≈ 0 |
| 7 | `ir_names` | 2.33 | quality-contracts-1 | (a): it joins `descendants` (a Reach stage here) per node; (c) |
| 8 | `methods` | 1.64 | 4 clauses share a text | (b) ≤ 0.94 |
| 9 | `taints` | 1.50 | state-flow-1 | (c) recursive rule logic |
| 10 | `semantic_values` | 1.47 | state-flow-1 | (a) joins `descendants`; (c) |

Levers ranked by seconds at stake. "Recoverable" columns marked [INFERENCE] are estimates, not measured.

| rank | lever | seconds at stake | recoverable |
|---|---|--:|---|
| 1 | (c) columnar keys and borrowed values in the SQL engine: hash index build and probe without a `Vec<Key>` and an `Rc` clone per row, comparisons on borrowed values, fewer `Column::get` dispatches through `Gather`/`Chain` | index build 19.8% + probe 13.2% of eval, with `Column::get` inside both ≈ 29 s | 10–15 s [INFERENCE] |
| 2 | (a) + (d) origin tracing to integer ids, then R9's interval for `descendants` and a one-pass `ownership` | `nodes` 8.6 + `ownership` 6.8 + `descendants` 5.2 = 20.7 s, plus id-keyed joins downstream | 10–14 s [INFERENCE] |
| 3 | matcher inputs: share the per-member capture passes of a clause's dump inputs (S7 § 5; quality-conditional-1 has 20 members) | 20.0 s; quality-conditional-1 6.8 s | 3–5 s [INFERENCE] |
| 4 | (d) R14 range probe | — | **7.7 s, measured, landed** |
| 5 | (b) selection sharing by semijoin for byte-identical Python-derived stages | upper bound 2.7 s (methods 0.94, parameters 0.50, functions 0.30, import_identifiers 0.17, assignments 0.16, calls 0.13, …), and only where R11's subtree closure holds | ≤ 2.7 s |
| — | fixed or not per-file | parse 9.8 + extract 1.0; repository finish 6.4 (S3) | — |

Reaching 90 s needs 16.5 s more: lever 1 plus part of lever 2, or lever 2 plus lever 3. Lever (b), measured, is too small to matter.

**Why R14 was implemented first.** Lever (b) was the first candidate, and its measured bound is small. The profile then showed a quadratic join (`import_identifiers`, 6.5 s on two clauses) that a general rewrite with a clean side condition removes completely. Levers 1 and 2 are larger. Lever 2 needs origin tracing through text ids (the I2 proof plus a value-demand analysis) before R9 can fire. Lever 1 is an engine-wide change to value access.

## 8. Resume notes (S7b)

- **Committed** (branch `Moti-Velz/facts-engine-s7b-retire`, local, not pushed), with gate rows in § 7.1:
  - `299fb8942`: deletion;
  - `b63025e9c`: unit tests for mutants 1–3;
  - `12930ad01`: R14, mutant 23.
- **S5 informed** (`S5Gate-3`):
  - the switch is `FACTS_REWRITES=0`;
  - `a3-check`/`a-reference` become default vs `FACTS_REWRITES=0`;
  - mutants 1–3 live in `lowered.rs` (rows R12/R13) and the corpus does not kill 1 or 3;
  - the `facts-a-unit` check (`cargo test --release -p facts-a --features mutants --bin facts-a`) kills 1, 2, 3, 22 and 23;
  - the sizing of `--max-nodes`;
  - the stale `MUTANTS` site strings and the Justfile comment are S5's to update.
- **Next lever:** rank 1, columnar keys in `sql/engine.rs` `Index`/`Exec::eval`. Measure the same way: `bin/facts-a-r14` vs the new binary under `tools/run-repo.sh`, then `residue.py` on both, then the gate.
