# 0334. Work budgets and result limits never change results

Status: built (audited 2026-11-23). Decided (2026-11-17); the replanning is built (packed batches, the label sweep,
the wave's root walks, the memory restart); `replan` as the default is what remains (Progress). After 0324 (the grid
retired). The user, on Q49: "Allow a mode where we let the kernels run over. The goal of stopping them is for a
horribly misplanned kernel to not spend minutes or hours churning when replanning could make it fast (we need planning
to be very fast, so we will not be able to always do it right). We need to re-plan over-long jobs, not just re-run
them elsewhere. We still need to be able to do that. Help me think through the correct design".

## Intro

The packed batch (the CPU JIT's and Metal's) gives each task a *workspace estimate*: the root's subtree (its file,
where the program repeats) × the program's words × 4,096, within [4,096, 2^26]. It sizes the task's states (pending
paths, captures, results). The ABI says so: "Workspace estimate only. Runtime task and path budgets are
independent." But the Metal kernel read that field as the task's work limit, so a task stopped (`WorkLimit`) at
its estimate. The CPU charges every task against `MAX_WORK_PER_TASK` (2^26) and every path against
`MAX_STEPS_PER_PATH` (2^20). So a small root with a long text test passed on the CPU and failed on the device
(Q49: the D416 clause).

The fixtures that 0324 moved onto the packed JIT showed a second conflation: a *result limit* (a witness limit, at
most N results an evaluation) was applied as the evaluation's result capacity. Past the limit, a result could not be
reserved, and the task failed: `WorkLimit` on Metal, `BackendCapacity` on the CPU JIT. The CPU interpreter instead
finds every result and keeps the first N: "Limits select complete interpreter results. They cannot hide a later
operand or work failure in the original evaluation."

## Decision

1. **The estimate sizes; the budget stops.** `MetalTask` carries both: `workspace_work_estimate` (sizing only) and
   `work_limit`, the task's budget (`WorkBudget`). A path's and a value's steps are held to the budget and to
   `MAX_STEPS_PER_PATH`, as the CPU's (`step_limit` in the runtime).
2. **The budget is a mode** (`RERAIL_DEVICE_BUDGET`):
   - `full`: the CPU's limits. A task runs past its estimate, and the device's results are the CPU's. This is
     the mode the user asked for: kernels run over.
   - `replan`, the default since 2026-11-17 (below, Progress): the replanning below. (`estimate`, the old stop at
     the estimate, which changed results, is gone.)
   The resident label batches (`assign_resident`) take the same budget: under `full`, the evaluation's work per
   node is 2^26, which `resident_work` clamps to 2^26.
3. **A result limit selects; it sizes nothing.** `assign_workspace_with_result_limits` keeps the limits on the batch
   (`result_limits`) and no longer caps the result quota or arena. `PackedResultPage::compact` keeps each
   (file, program)'s first N results in the page's order (root, start, first capture, local ordinal). Both the
   CPU JIT and Metal compact through it. A result quota that runs out is now capacity (status 2), never a work
   limit.
4. **A rejected candidate leaves no result.** The kernel reserves a result before it applies the output
   directives. A candidate they reject (outcome 1) marks its slot (`taskOrdinal = NONE`), and the page skips it.
   Before, the slot kept whatever the reused workspace held.
5. **A failed task's early results are dropped, not refused.** A task that published results and then failed (a
   late operand failure) made validation refuse the whole batch (`BackendFailure`). Its results are now dropped
   with its evaluation, which reports the task's error, as the CPU's does.

## Over-long jobs are replanned

What the budget is for: a badly planned job must not churn for minutes when another plan would be fast. Planning
must stay fast (0122: planning costs no more than running), so some plans will be wrong; an over-long job is
replanned, not just rerun elsewhere.

1. **Two numbers, never one.** The estimate (memory) and the budget (time) stay apart (done, §1). The budget sits
   below the ceiling (2^26 a task, 2^20 a path). The ceiling stays an error, the same on every backend (`WorkLimit`,
   then `WaveRun::limited`: W's matcher confirms).
2. **Steps, not seconds.** The budget counts the same steps on the CPU and the device, whatever the machine's load,
   so whether a job trips is reproducible (tests, the oracle). The cost model turns steps into time per backend:
   the budget then bounds a launch's time, and the GPU's watchdog.
3. **An overrun is a preemption, not an error.** A task over budget ends with a status of its own ("over budget").
   Its evaluation (program × file) is *unfinished*: none of its results are published, and it is not an error.
   The other evaluations of the launch finish. The ABI has the field for an evaluation's work across its lanes
   (`Evaluation.workUsed`, atomic, unused today): each lane adds its steps, and every lane of an evaluation over
   its budget stops at its next check.
4. **The budget is the plan's estimate times a slack** (k, e.g. 8). A well-planned job never trips it. The
   estimate is the planner's cost for that evaluation, not the workspace formula.
5. **On an overrun, replan only the unfinished evaluations.** The replanner gets the evaluation, the steps spent
   (at least the budget), where it stood (`status.programCounter`, the node, the queue's depth) and what the plan
   assumed. It records the misprediction in the cost store the planner reads, so later plans (the wave's unstarted
   chunks, the next run) start from it. Its choices, each costed against the evidence (the current plan costs more
   than the budget on this file):
   - another plan of the clause: another anchor or driver (a rarer kind first), a prefilter (the term index),
     labels first instead of a root walk, W's matcher (as `WaveRun::limited` does now), or the evaluation split by
     subtree (many smaller tasks, each bounded);
   - another backend, only as a plan whose cost wins, never as the fallback;
   - the same plan with the budget doubled, up to the ceiling, when nothing else looks cheaper. Doubling bounds the
     waste: the runs before the last cost at most as much as the last.
   Replanning covers few evaluations, so it stays cheap.
6. **Restart, never resume.** A device task's state (its pending paths in the workspace arenas) is large and tied
   to its lane, and another plan cannot use it. A restart with a doubled budget costs at most twice the final run.
7. **One mechanism on both backends.** The CPU charges the same steps, so a misplanned CPU job trips and is
   replanned the same way.
8. **Modes, and the test of them.** `full` (no budget below the ceiling, no replanning), and `replan` (this
   design), which replaces `estimate`. Every mode gives the same results; only time differs. One test plans a
   clause badly on purpose (a tiny estimate): it must trip, replan and give the CPU's results.

9. **Memory is mispredicted the same way, and handled the same way.** The workspace estimate (subtree × words ×
   4,096) was a step count, made large so that it rarely stopped a task. As the size of every task's states (up to
   65,536 a kind, times the lanes), it sizes for the worst case: three three-word programs over 128 three-line
   files planned a 471 MB Metal workspace, which the buffer pool refuses (`BackendCapacity`, Q64's 15 refusals;
   the CPU JIT allocates host memory and never refuses). Instead, the plan sizes a task's states from what it
   expects, and a task that outgrows them ends "unfinished" (as an overrun, §3): its evaluation restarts with
   twice the states, up to the ceiling. Same restart, same bound on the waste.

The user's answers (2026-11-17, the recommendations taken):
- The unit is the evaluation (program × file) where a task tripped; its misprediction reprices the clause for the
  wave's chunks not yet started.
- The budget counts steps, with a slack of 8 (`REPLAN_SLACK`).
- `full` stays the default until the replanning is the default (below).

## Progress

Done (2026-11-17):
- §1 to §5 of the decision (Q49's cause).
- **Packed batches** (the CPU JIT's and Metal's, `execute_planned`): `WorkBudget::Replan { slack, doublings }`
  (`RERAIL_DEVICE_BUDGET=replan`). A task stopped below the ceiling is `Error::OverBudget`, its evaluation
  unfinished; at the ceiling it stays `WorkLimit`. The CPU JIT holds tasks, paths and values to the same budget
  as the device. Each backend restarts its unfinished evaluations alone (`packed::restarting`), same programs and
  files, twice the budget, until they finish; the restart's results stand for theirs (`with_restarted`).
  Test: `a_misplanned_evaluation_is_restarted_and_gives_the_full_budgets_results` (CPU JIT and Metal).
- **The budget's basis is a prediction** (§4): an evaluation's predicted steps are its program's measured rate
  (steps per unit of size) times the task's size (its subtree's nodes, the file's where the program repeats,
  plus its bytes / 8 where the program tests text); without a rate, a prior (size × words × 2). At least 256
  (`MIN_PREDICTED`). The workspace formula is gone from the budget. Both backends count a task's steps the same
  way (the most steps of any of its paths, folded at no per-step cost), so rates from either apply to both.
- **Mispredictions in the cost store** (§5): each evaluation's steps, size and prediction, and whether it ran over
  (`rerail_accel::program_steps`), go into `costs.sqlite`'s `program_steps` table (sums, halved past 1,024
  evaluations), read back as rates at the next run. The overrun's steps count as its budget: a lower bound.
- **Memory** (§9): a task's states are sized from its size (4 a unit, at least 512), not from the step count. A
  task whose states run out below the worst case is `Error::StatesOutgrown`, and its evaluation restarts through
  the same mechanism with twice its states (per task, `StateGrowth`). Metal's workspace halves its lanes where the
  buffer pool refuses it. Q64's 15 capacity refusals: 12 pass; the 3 left differ in results on Metal at any state
  size (Metal semantics, Q64). Test: `outgrown_states_restart_and_give_the_worst_cases_results`.
- **The label sweep** (`assign_resident`): under `replan` an evaluation's budget per node is its predicted steps
  per node times the slack, within [256, 2^26] a task. A task stopped below the ceiling reports its own status, and
  the host runs the sweep again with twice the budget (`Outcome::replanned`). Test:
  `a_misplanned_label_sweep_runs_again_and_finds_the_full_budgets_roots` (slow tier).
- **Another plan before the ceiling** (§5's choice): the wave's root walks on the CPU (`clause_roots`) are held to
  the evaluation's (clause × file) prediction times the slack. At an overrun, W's matcher at the candidates (its
  rate per subtree node, measured where it confirms roots, key `w:<program>`; else 4) is weighed against the
  transducer again with twice the budget, and the cheaper runs; W through `WaveRun::limited`, as at the ceiling.
  Test: `a_clause_over_its_budget_goes_to_w_where_w_costs_less_and_runs_again_where_it_does_not`.

- **Mispredictions reprice at once** (§5, "the wave's chunks not yet started"): every observation folds into the
  in-memory sums the predictions read (`program_steps::observe`), so the next evaluation of the program, in the
  same run, is priced from it. The store's sums are read at a run's start (`install_sums`) and written at its end.
  Test: `an_observation_reprices_its_program_at_once`.
- **The wave's label passes on the CPU** (`labelled_sets`): a pass on the interpreter (where its micro walk is not
  one) is held to its prediction like a root walk (`LoweredMatcher::evaluate_budgeted`); a recursive group's sweep
  (`sweep_with`) to the group's prediction. At an overrun W for the clauses the labels feed is weighed against the
  pass or sweep again with twice the budget. A label set given to W is not stored: the clause that owns it, and
  every clause that reads it (`feeds`), are W's on that tape (`WaveRun::limited`).
- **The Metal wave** (`roots_metal_split`, the label passes, sweeps and root walks): a task over its budget, or at
  the ceiling, marks its evaluation (`Evaluation.outcome`: 1 over, 2 at the ceiling) instead of failing the run;
  the host copies each dispatch's evaluations out (`Runner::report`) and, per unfinished (pass, file), weighs W for
  the clauses it feeds (`LoweredWave::label_w_cost`) against the run again with twice the budget
  (`replanned`). W's go to `Outcome::unfinished` and from there to `WaveRun::limited`; at the ceiling always W.
  The overruns are observed for the cost store. Without a W (a caller with none) an overrun runs again and the
  ceiling fails, as before. Test:
  `on_metal_an_unfinished_pass_goes_to_w_where_w_costs_less_and_runs_again_where_it_does_not`.
- **The evaluation split by root** (§5): a root walk over its budget keeps the roots that finished before the one
  that ran over (`Program::execute_roots_split`, `Partial`), and a rerun takes only the rest, within the doubled
  budget less the steps kept. Not for a tree-sitter query, whose outputs are reduced across roots. Test:
  `an_evaluation_over_its_budget_keeps_its_finished_roots`.
- **The prefilter's plan** (§5): at a root walk's overrun the program's micro walk, where it is one with text tests,
  is weighed with W and the transducer again (`predict_walk`: the interpreter's prior without the text, plus one
  pass of the text layer a slot). The walk reads every text test as bits, from the term index's row where the tape
  has it (`R[tid]`, `Micro::slot`), never a test per node: Q49's shape (a long string's regex at every node). A walk
  that outgrows its bound leaves the rest to the transducer and W, by cost.

- **`replan` is the default** (2026-11-17; `RERAIL_DEVICE_BUDGET=full` for the CPU's limits). The check, in
  place of a corpus run (a run from this checkout stops under strict SQL, work queue Q21, whatever the budget):
  every lowered Python clause of the pack (491), in waves, over corpus0's files, with `full` and with `replan`
  from an empty cost store; a clause left to the transducer finds `full`'s roots, one given to W has W's matcher
  find them at its candidates (`replan_finds_the_full_budgets_roots_on_the_pack`, CPU and Metal). On 10 files:
  4,910 evaluations a backend, all `full`'s roots, no rerun and nothing given to W (the priors never tripped). On
  50 files, Metal, the first 192 clauses: the same. The tests that trip on purpose cover the rest (above).
  The switch showed one Metal bug, hidden under `full`: a task whose steps ran out in its output directives (a
  Strip's regex) reported capacity, not a work stop (`apply_output` passed the regex's code 3, "work", where its own
  3 is "an arena full"). The host took it for outgrown states and doubled them to the cap, then failed with
  `BackendCapacity` (`inputs_transform_pair_cursors_resume_and_survive_history_compaction`). `apply_output` now
  has a code of its own for a work stop (5, status 4), which restarts with twice the steps.

Not yet:
1. **Another anchor or driver** (§5) is not a replan choice: it moves to the planner (2026-11-17). Prior work
   (ast-index-spike): anchor selection is a planning decision from statistics, settled there (the rarest mandatory
   concrete variable, 112 s to 33 s, D S049-S050 L3024-3112; 1.05 anchor rows per answer, D S151 L6640-6686, DH
   L2578-2597; a value or exclusion seed scored against the whole pattern's rarest bound, DH L5074-5143 S205, DH
   L5466-5509 S209; leaf-up against root-down priced by the walk's work from per-edge counts, not by kind counts or
   a fitted ratio, DH L8691-8730 S277), and choosing a plan by running and retrying was built and removed (D S069
   L4110-4129: it spends the budget on the queries with least headroom, makes latency nondeterministic and cannot
   batch). Decision 0026 took the same rule ("climb versus descent is a planning choice: anchor on the rarest
   kind"), but the wave's lowering never did: a clause's root walk starts at every node of its root's kinds
   (`Domain::of`, every node where the root has none), and each label pass at every node of its domain; the
   prefilter uses the rarest requirement (`Statistics::choose`) only to gate files. Measured
   (`anchor_rows_per_answer`, 50 of corpus0's files, every file active, the 491 Python clauses): 7,753,599
   candidate roots for 4,566 roots (1,698 a root, against the spike's 1.05), 34.8 M root-walk steps, and 206.6 M
   label-pass rows (1,522 a node); the worst clauses start at every node (SIM118, RUF005: 135,743 candidates, one
   root). The replanning's part stays as built: an overrun's steps reprice the program, so the next plan starts
   from them. **Decided and built on the CPU as 0365** (a walk starts within its anchor: its program's tests as a
   formula, 0036's mechanism; root walks at 1.02 starts a root on 10 files). The label-pass rows above were domain
   rows: most of those passes are 0036's formulas, which follow their members and do not walk from each.
2. The micro walk's prediction is a prior (`predict_walk`): the walk counts no steps the cost store could learn
   from.

(Work queue numbers as on the hub: written as Q19 and Q22 on oscar, the hub's Q49 and Q64.)
