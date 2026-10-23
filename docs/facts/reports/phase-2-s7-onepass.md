# Phase 2 / S7 one pass (wave 3)

Status: **steps 1–3 landed; the generic path beats the A3 path in both A/B pairs; `subst.rs` not yet deleted (step 4 is mechanical, listed below); G2.3 ≤ 90 s not met.** Branch `Moti-Velz/facts-engine-s7-onepass` (local), worktree `.worktrees/s7-onepass`. Runs: `~/rerail-bench/facts/phase-2/s7-onepass/runs/`.

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
| `pattern-check`, full corpus 9,010 files | 0 mismatches | running at hand-off with the reference on files ≤ 200 nodes (`bin/facts-a-p2`); result in the run dir | open | `…20261022T233157-pattern-check-full-ref200` |
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

## 6. Resume notes

- **Not committed at hand-off unless the gate run passes** (`runs/*-gate`); then commit `s7-onepass: lowered matcher inputs, physical plan, closure keys` with everything in § 3.
- **Step 4 (delete `subst.rs`), exact list**: remove `mod subst` (main.rs), `subst.rs`; `rel.rs` `kernels`/`dump` fields, `schedule_releases` kernel reads, the kernel/check branches in `run`; `eval.rs` `plan_clause` call, `DumpNs`, `dump_rows`, `nearest_where`, `PENDING`, the `from_dump` block in `eval_file`; `physplan.rs` `kernel_name` and kernel printing; `ast.rs` the three `check_mode` blocks and the mutant-3 site (S6 agreed to the blocks; ask for the mutant site); `run-repo.sh` label → `FACTS_REWRITES`. Then message S5 (switch `FACTS_REWRITES=0`; `pattern-check --max-nodes 2000` as a gate check, pass = 0 physical and 0 reference mismatches; mutants 1–3 killed by default vs `FACTS_REWRITES=0` findings).
- Quiet-machine A/B (3 runs each) is the decider's.
