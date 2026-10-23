# Phase 3, P3b: scaling laws as the budget model

Branch `Moti-Velz/facts-engine-p3b-scaling`, from integration `1c5a84b7a`. Milestone commit: `907eac885`. Run directories are under `~/rerail-bench/facts/phase-3/p3b/` and `~/rerail-bench/facts/loop/runs/`. Every number is a count.

**Status: partial.** G3b.1 is delivered with a residue: the census and the classes are in place, and one runtime check (K1) is still owed. G3b.2 and G3b.3 are not started. § 5 lists exactly what is owed, in order.

## 1. What landed (`907eac885`)

| item | where |
|---|---|
| Operator cost bounds | `rerail_facts::phys` § "Cost bounds" |
| Plan class | same file |
| `EXPLAIN` class line | `physplan.rs` |
| Census command | `physplan.rs` |
| Lowered-input bound | `lowered.rs` |
| W1, the forest walk | `sql/engine.rs` |
| M1, a shrinking text | `sql/engine.rs` |
| M2, a counter going up | `sql/engine.rs` |
| Simple-path enumeration | `sql/engine.rs`, `phys.rs` |
| K1, keys checked per content | `phys.rs` |
| `bounds_unproven` count | `counts.rs`, `sql/engine.rs` |
| Mutants | 57 (`phys.rs`), 59 (`engine.rs`) |

What each item does:

- **Operator cost bounds.** These are `Mono`/`Bound`, sums of `n^a·d^b·log^c` terms, with `Card` (rows, keys, path-constant columns) and `Analyzer`. Every physical operator declares its work and output as a function of its inputs. The table is in the module comment. Unbounded shapes are named: a cross product, a join without a usable key, a join on columns constant over the content, a fixpoint without a decreasing measure, and simple-path enumeration.
- **Plan class.** The classes are `O(n)`, `O(n log n)`, `O(n·d)`, `O(n²)` and `unbounded`, with the operator that sets the class. `O(n·d)` and `O(n²)` absorb a log factor. `PlanCost::refusal` compares the class with a declaration.
- **`EXPLAIN` class line.** `facts-a explain --physical` prints `class: … (work …; set by …)` under each clause. Classes are computed once per rule, so the explain-physical stability check is unaffected.
- **Census command.** `facts-a explain --classes [--json F]` covers every rule file of the pack by default.
- **Lowered-input bound.** `lowered::Plan::cost`: `O(n)` per predicate (R13), and `O(n·d)` for subtree searches and regexes over node text. A leaf-executor input is `O(n·d)`.
- **W1.** `walk_bound` matches a bag recursion that probes one edge relation and then only does lookups. Before the recursion runs, `check_walk` verifies on the content that the walk graph is acyclic, that every node has at most one parent or at most one child, that every lookup finds at most one row, and, going down, that the initial keys are distinct. The bound is `n·d`. If the check fails, the recursion runs as written under the budget and the content is counted.
- **M1.** `shrinking_text` is checked per round in `Exec::recursive`: the round's longest text must be shorter than the previous round's.
- **M2.** `counter_up`: the counter's probe key is a K1 key.
- **Simple-path enumeration.** `extends_path`: a recursion that concatenates onto its own path value is classed unbounded with that reason.
- **K1.** When static key inference fails, a stored relation probed on plain values (not only on the path) has its probe columns taken as a key. These are listed in `PlanCost::checked_keys`. The path is declared constant per content, so a probe on `path` alone is named as a cross product within the content.
- **Mutants.** 57: every probe is taken as a key lookup; killed by `phys::tests::cost_*`. 59: W1 skips its acyclicity check; it has no test yet (see § 5).

**Unchanged behaviour.** sub200 parity is 90/90 with 0 failures in every clause, run cold with an empty `FACTS_MEMO_DIR` (memo 0/14,982). ref50 shows 0 mismatches. Run: `loop/runs/20261023T143952-t1`. T0: 12/12 mutants. `bounds_unproven` on sub200 is 176, all from `state-flow-1/transitive_sinks`: W1 is tried on that recursion and `call_graph` is not a forest, so it runs as before.

T1 caveat: `just facts-t1` serves sub200 from the digest memo even when engine files changed, because the S10 rule "T1 runs `--no-memo` when operator files changed" is not implemented. My parity runs set a fresh `FACTS_MEMO_DIR`.

## 2. The census (G3b.1)

`facts-a explain --classes --json ~/rerail-bench/facts/phase-3/p3b/census/classes.json` gives 60 rules. 2 are refused at compile, as before: `imperative_private_functions` (rule key `calls`) and `internal_dead_code` (`file.module`). That leaves 558 clauses. The full per-clause table is `census/classes.md`.

| kind | O(n) | O(n·d) | O(n²) | unbounded |
|---|---|---|---|---|
| forbid ast | 3 | 377 | | |
| forbid regex | 113 | | | |
| budget | | 14 | | |
| relational, per file | | 28 | 3 | 15 |
| relational, repository | | 1 | | 4 |

Fixpoints: 62 in the 60 rules. 61 are bounded:

- 14 `ownership` fixpoints by W1;
- the counters;
- set recursions that copy columns or compute only from joined rows;
- `imports` by M1;
- `block_results` by M2.

The one left is `quality-contracts-1/call_paths`, a simple-path enumeration. It goes on Jeremy's list as a rule-shape item; the NARROWINGS `hierarchy` edit is the model.

19 clauses are still unbounded. Each is named in the census with its stage and operator:

| what is missing | clauses |
|---|---|
| a probe on `path` alone (`parameters: index nodes on (#0)`). This is a real cross product within the content; no key bounds it, and the rule's join is the issue. | state-signatures-1, state-flow-1, state-flow-project-1, step-contracts-2, step-contracts-5 |
| a range probe with no declared interval (R14 over stage tables). An I2 interval declaration on the stage would bound it. | direct-execution-lines, step-contracts-4 (and 3 O(n²) clauses: authority-sites, quality-contracts-3) |
| a correlated lookup whose total is bounded only per run (`keyed by the enclosing row` with an n² outer) | quality-contracts-2, step-contracts-1 |
| keys of stage outputs the K-rules do not reach (unions of inputs; `derived1`; `contexts`; `returns`) | step-contracts-3, state-projection-recovery-2, runtime-reference-summaries, required-contracts |
| fixpoint work above O(n²): derivations × probes of `receiver_edges` / `callable_edges` (n·n·n) | legacy-execution-graphs, step-contracts-6 |
| the counter fixpoint downstream of `imports` (initial n² rows) | exported-families |
| `executor_types`: a per-row recursion arm with a kind-only lookup | executor-provenance |
| reachability over a rule-declared forest, as roots × edges | quality-conditional-1 |
| simple-path enumeration | quality-contracts-1 |

## 3. Decisions taken with Main

- **Class declarations live on the facts side.** The overlay is `packs/data-layer/candidates/classes.json`, keyed `check/clause`; it is not written yet. There is no native allow-list change and no rule-file edit.
- **An unbounded clause is not refused.** It compiles with a warning in the census and in `EXPLAIN`, runs under the algebraic budget, and is reported as gate residue.
- **A failed per-content side condition (W1, M1) means the bound does not fire.** The recursion runs as written, under the budget, and is counted in `bounds_unproven`. It never gets a different answer and is never refused.
- **The exponent lane covers every plan.** An unbounded plan whose fitted exponent exceeds 2 is still reported.

## 4. Interface changes

- `phys::Node.meta: Meta`: binds, cols, reads, init, delta, measure, checked keys, paths, forest. It is not printed and not in the digest.
- `phys::PipelineItem.refs`.
- `phys::{Mono, Bound, Class, Card, Term, PlanCost, Analyzer}`.
- `counts::Phys::BoundsUnproven` and `Counts::bounds_unproven`. This is `#[serde(default)]`; `NPHYS` goes from 9 to 10.
- `sql::engine::{WalkBound, walk_bound, extends_path, shrinking_text, counter_up}` and `Exec::check_walk`.
- `lowered::Plan::cost`.
- `physplan::{clause_cost, input_cost, census}`.
- Integrator: plan texts are unchanged, but `engine.rs`'s behaviour around fixpoints changed (checks and counts only). Bump `memo::ENGINE_VERSION`.

## 5. Resume notes (owed, in order)

1. **K1's runtime check is not implemented.** The analyzer relies on `PlanCost::checked_keys`. Each relational clause needs them computed once at compile, stored on `RelPlan`, and verified in `rel::run` after each stage and its inputs are stored. A duplicate key over non-NULL values adds 1 to `bounds_unproven`. The verification is O(rows) per stage. Until it lands, K1-based classes are claims, not proofs.
2. **Mutant 59 needs a test, and both mutants need gate registration.** Write an `engine.rs` unit test that calls `walk_bound` and `check_walk` on a 3-cycle edge table and expects `Err`; mutant 59 must make it `Ok`. Register 57 (family `rerail-facts-lib`) and 59 (`facts-a-unit`) in `facts-gate.py` `MUTANTS`, and add REWRITES.md rows for W1, M1, M2 and K1 with their side conditions. Message P3cDaemon before touching the gate file.
3. **Class overlay.** Write `packs/data-layer/candidates/classes.json`. The census and `EXPLAIN` should print a warning for an unbounded clause without an overlay entry, and the gate should report the residue list.
4. **G3b.2 exponent lane.** `facts-a repo --file-counts PATH` should write JSONL rows `(path, nodes, depth, clause, counts, input pred_tests)`.
   - Strata: log2 node bins.
   - Estimator: OLS of log(p90 of work) against log(median nodes) per stratum. Fit algebraic and physical work separately.
   - Pass: the algebraic fit is within the class exponent plus the tolerance, and the physical fit is within the algebraic fit plus the tolerance. The physical check is what catches execution regressions, because algebraic counts are engine-independent.
   - Mutants to re-seed:
     - 53, the utility memo: skip the lookup in `ast.rs` `Eval::util`. The leaf executor's `steps` must then be reported as `pred_tests`.
     - 55, the OR-probe: execute an `OrIndexProbe` as a scan with the OR re-checked, in `engine.rs`.
     - 51, P3aMemory agreed to host it: `facts-b gen/lower.rs::var_readers` runs the old unmemoised walk under `FACTS_MUTANT=51`, plus one `construction` key in `gen/drive.rs`'s results JSON. Keep that edit to those lines only.
   - The lane must show each mutant detected on ≤ 200 files.
5. **G3b.3 algebraic budget.** Algebraic work is leaf, stage and fix rows plus match nodes plus input rows. The limit is `A · m(class)`, with `m(unbounded) = n²` and A chosen with ≥ 4× margin over the corpus maximum. Check it at stage and fixpoint-round boundaries: counts are monotone, so the verdict equals the reference's. The failure is `budget-exceeded: algebraic work …`. Memoise it through the memo `Entry` failures; P3c is rewriting `memo.rs`, and its `Entry` shape is unchanged. Restate the 8 native workspace allowance tests (`workspace/context_tests.rs`, lines 2100–2731) in algebraic units: clause ceiling, partial pass, shared work, a clause failing alone, verdict memo, nothing-to-match, scope table, php names.
6. **Parity runs** must keep using a fresh `FACTS_MEMO_DIR`, or T1 must gain `--no-memo` when operator files change.

---

# Continuation (P3bScaling-2), commit `e2d01569f`

**Status: partial.** These are done: K1, the census split into proven and assumed, the class overlay with its warnings, the algebraic budget mechanism (engine, reference, and the stage-check verdict comparison), the per-file counts, and the mutant sites with their tests. These are still owed: the gate registration and lanes, the REWRITES.md rows, and an exponent estimator that separates the mutants. § C5 lists them in order.

Runs:

- T0: `loop/runs/20261023T151249-t0`, 3/3 checks, 16/16 mutants.
- T1, cold with an empty `FACTS_MEMO_DIR`: `loop/runs/20261023T151344-t1`.
  - sub200: 90/90 findings, 0 failures, equal in every clause.
  - ref50: 0 mismatches. Budget verdicts: 650 compared, 0 mismatches.
  - b-exactness and b-generated: 0 differences in 101 checkpoints.

## C1. K1 checked per content; the census splits proven from assumed

- **The fallback class.** `phys::Analyzer::unchecked()` assumes no per-content check: no K1 key, and no W1/M1/M2 measure (`Meta::measure_checked`). `PlanCost.fallback` and `PlanCost.fallback_setter` record its result, and `PlanCost::proven()` is true when the fallback equals the class. The EXPLAIN class line names the fallback when the class is not proven. Test: `cost_unchecked_assumes_no_per_content_check`.
- **Where the class is stored.** `eval::Rule::compile` stores `Clause.class` and `Clause.fallback`. It also sets `RelPlan.checked_keys`, but only for clauses that are not proven: a proven class needs no key checked.
- **The runtime check.** `rel::run` checks each K1 key on the stored table right after the table is stored (inputs, syntax tables and stages) with `rel::key_holds`. It skips NULL keys and compares numeric texts and integral reals as numbers. Each failed key adds to `RunCounts.unproven`, which `Output::add_run` adds to `bounds_unproven`. The content's budget then uses the fallback class. A failed key never causes a refusal.
- **M2.** M2 is now a measure only when its probed relation is stored, so that K1 can check it. Its checked keys follow the same order as `measure`.
- **Tests and mutant 61.** `rel::tests::k1_checks_assumed_keys_on_stored_stages` covers the check. Mutant 61 makes `key_holds` always pass; both rel tests kill it.

Census: `~/rerail-bench/facts/phase-3/p3b/census2/classes.{md,json}`, format `rerail.facts-classes/v2`, with columns proven, fallback, checked_keys, declared and warning.

| class | clauses | proven | assumed |
|---|---|---|---|
| O(n) | 116 | 116 | 0 |
| O(n·d) | 420 | 420 | 0 |
| O(n²) | 3 | 1 | 2 |
| unbounded | 19 | — | — |

- The two assumed clauses are `execution_authority_bypass/authority-sites` and `serverplan_quality/quality-contracts-3`. Their fallback is unbounded.
- The W1 fixpoints sit in clauses that are already unbounded for other reasons, so W1 changes no class.
- Across the 9,010-file corpus, no K1 key failed. `bounds_unproven` is 446, all from W1 in `state-flow-1`.

## C2. The class overlay and its warnings

- **The overlay.** `packs/data-layer/candidates/classes.json` has format `rerail.facts-class-overlay/v1`. It holds 19 entries keyed `check/clause`, each with its class, stage, operator and reason (the reasons are the § 2 table).
- **Loading.** `physplan::overlay` reads it from `<rules>/../classes.json`, and `repo::compile_checks` sets `Clause.declared` for every compile path.
- **Warnings.** `physplan::warning` is printed under the class line in `EXPLAIN --physical` and in the census, with the count of stale entries. With the overlay in place: 0 warnings and 0 stale entries. Without it there are 19 warnings (`census2/classes.md` shows them before the overlay was written).

## C3. G3b.3, algebraic budgets

- **The rule.** `rerail_facts::budget` (`facts/src/budget.rs`). Work is `match_nodes + input_rows + leaf_rows + stage_rows + fix_rows`. The limit is `A·m(class)` at `n` nodes and depth `d` (`Facts::depth`), where `m(unbounded) = n²` and `A = 256`.
- **Choice of A.** The corpus maximum of work / m is 51, at one-node files where a clause's fixed rows dominate. From 100 nodes up it is below 1. Run: `phase-3/p3b/g3b3-corpus` (all 60 rules, 1,449,505 file-count rows, 0 budget failures).
- **Metering.** `Meter` checks against the fallback limit at stage boundaries (`rel::StageMeter::done`) and per fixpoint round:
  - engine: `Exec::spend_round` in `recursive` and `walk`; the walk restores its count when it gives up;
  - reference: `fixpoint` and `evaluate_stages_counted`.
- **The verdict.** At the end of the run, `budget::verdict` compares the work with the content's own limit: the fallback limit if `bounds_unproven > 0`. The message is `budget-exceeded: algebraic work over L`.
- **Clauses without stages** (forbid ast, budget) are checked after their inputs, and fail alone with their findings and rows dropped. Repository runs are unmetered: they are not per content, and the 50M fixpoint cap stays.
- **Memoisation.** Failures go into the memo `Entry` as before, and `budget::A` is now part of the digest.
- **Engine against reference.** stage-check runs the reference under the same meter, evaluating the stages plus the final query as `<final>`, and compares verdicts (`budget_compared`, `budget_mismatch_total`). On ref50: 650 compared, 0 mismatches. `a-vs-reference` and facts-loop do not fail on `budget_mismatch_total` yet (§ C5).
- **Tests.** `budget::tests` (2) and `rel::tests::algebraic_budget_uses_the_fallback_where_a_check_fails`. The 8 native workspace tests are **not yet restated** (§ C5).

## C4. G3b.2, inputs in place but the lane not passing yet

- **Per-file counts.** `facts-a repo --file-counts F` writes JSONL rows `{path, nodes, depth, clause, class, fallback, counts, pred_tests}` for evaluated clauses (run with `--no-memo`).
- **The leaf executor's work.** Its `steps` are now reported as `pred_tests`, in `Input::rows` and for forbid-ast clauses (in `input_counts`).
- **Mutant sites:**
  - 53: `ast.rs` `Eval::util` skips the memo lookup.
  - 55: `engine.rs` runs the OR probe as a scan, with the OR re-checked.
  - 51: `facts-b gen/lower.rs::var_readers` runs the old unmemoised walk, capped at 2^28 calls; the answers are unchanged. It records `(nodes, visits)` in `CONSTRUCTION`, and `drive.rs` adds the one `construction` key. P3aMemory agreed; it wrote nothing for 51.
- **Mutant 59.** Its test is `sql::engine::tests::w1_walk_check_refuses_cycles`; mutant 59 fails it.
- **File list.** `benchmark/facts/exponent.list`: 195 files, 15 per log2 node bin from 2^4 to 2^16, first by sha256 of the path. They are hard-linked at `phase-3/p3b/exponent-set`.
- **Prototype.** Runs are in `phase-3/p3b/exp-proto/m{0,53,55}`. For each clause, OLS of log(p90) against log(median) over log2 strata (at least 3 files per stratum, at least 4 strata):
  - algebraic against nodes: within class + 0.25 for every clause, worst margin −0.84;
  - physical against algebraic on the clean engine: up to 1.96 for leaf-executor forbid-ast clauses (their steps are O(n·d) against O(n) algebraic work) and 1.56 for `state-flow-1`. **So "physical ≤ algebraic + tol" fails on the clean engine.**
  - mutant 55 raises `state-flow-1` from 1.56 to 1.88 and total physical work from 1.53e9 to 2.80e9;
  - mutant 53 adds about 10% total physical work and no clear slope change on these files.
  - **The estimator as specified does not separate the mutants yet** (§ C5.2).

## C5. Resume notes (owed, in order)

1. **Gate wiring.** P3cDaemon is informed and is not touching `facts-gate.py`. Not done:
   - `MUTANTS` rows: 51 (facts-b, `exponent`), 53 and 55 (facts-a, `exponent`), 57 (rerail-facts, `rerail-facts-lib`), 59 (facts-a, `facts-a-unit`), 61 (facts-a, `facts-a-unit`). 57, 59 and 61 are verified killed by hand: `FACTS_MUTANT=n cargo test`.
   - lane `x` with checks `exponent` and `class-residue`. `class-residue` runs `explain --classes --json`, lists the unbounded clauses and their overlay entries, and never fails.
   - `run_check` and `run_mutant_check` branches.
   - `a-vs-reference` (`facts-gate.py:542`) and facts-loop's ref50 (`facts-loop.py:286`) should also require `budget_mismatch_total == 0`.
2. **Exponent estimator.** Physical work against algebraic work is superlinear by design for the leaf executor and for some joins. Fitting physical work against `m(class)(n,d)` per file (bound slope 1 + tol) is the principled alternative: the class bounds all operator work. Measure whether 53 and 55 exceed it on `exponent.list`. If they do not on 195 files, pick strata where the regressions bite: 53 needs utilities used repeatedly (deep files), 55 needs OR-probe clauses (state-flow-1 shows it). Register the tolerance in REWRITES.md. For 51, fit log(visits) against log(nodes) over `construction` from `greplay --plans none --real <required-contracts spec> --no-oracle --limit 1` (spec as in facts-loop `bench_one`); clean is exactly slope 1 (visits = nodes).
3. **REWRITES.md § Cost bounds.** Add a table whose header has no `rewrite` column (so the registry parser skips it; the mutants go in `MUTANTS`). Rows:
   - W1 (59; test `w1_walk_check_refuses_cycles`);
   - M1 (needs a unit test: a shrinking `substr` recursion has 0 `bounds_unproven`, a counter-bounded non-shrinking one has > 0);
   - M2 (key K1-checked, stored relations only);
   - the counter;
   - set recursion that copies columns;
   - K1 (61; `k1_checks_assumed_keys_on_stored_stages`);
   - the algebraic budget (A = 256, its tests).
4. **The 8 budget workspace tests in algebraic units on the facts path.** These are eval-level facts-a tests with `budget::set_test_a`. Planned mapping:
   - clause ceiling: each clause's verdict is the same alone and together;
   - partial pass: a verdict is unchanged with other clauses skipped (`out.skip`);
   - shared work: an input's counts are charged to every reader, so a clause's work does not depend on the cache;
   - a clause failing alone: the others keep their findings; the failure round-trips through `memo::Entry`;
   - verdict memo: a budget-failed clause leaves the next clause's input rows whole;
   - nothing to match: answers at A = 1;
   - scope table and php names: the syntax tables and the utility memo are never cut short, because the check runs only at stage and round boundaries.
5. **Decision for Main.** The facts path still carries `closure.rs`'s `maxVisitedNodes` emulation for repository budget closures. That is a repository-level native semantic, not a per-content kill switch, so I left it. Retiring it needs a decision.
6. The integrator should bump `memo::ENGINE_VERSION`: the budget and K1 behaviour changed, although the digest already includes A.
