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
