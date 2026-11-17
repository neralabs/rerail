# 0334. Work budgets and result limits never change results

Status: decided (2026-11-17); the replanning is built (packed batches, the label sweep, the wave's root walks, the
memory restart); `replan` as the default is what remains (Progress). After 0324 (the grid retired). The user, on Q19: "Allow a mode where we let the kernels run over. The goal
of stopping them is for a horribly misplanned kernel to not spend minutes or hours churning when replanning could
make it fast (we need planning to be very fast, so we will not be able to always do it right). We need to re-plan
over-long jobs, not just re-run them elsewhere. We still need to be able to do that. Help me think through the
correct design".

## Intro

The packed batch (the CPU JIT's and Metal's) gives each task a *workspace estimate*: the root's subtree (its file,
where the program repeats) × the program's words × 4,096, within [4,096, 2^26]. It sizes the task's states (pending
paths, captures, results). The ABI says so: "Workspace estimate only. Runtime task and path budgets are
independent." But the Metal kernel read that field as the task's work limit, so a task stopped (`WorkLimit`) at
its estimate. The CPU charges every task against `MAX_WORK_PER_TASK` (2^26) and every path against
`MAX_STEPS_PER_PATH` (2^20). So a small root with a long text test passed on the CPU and failed on the device
(Q19: the D416 clause).

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
   - `full`, the default: the CPU's limits. A task runs past its estimate, and the device's results are the
     CPU's. This is the mode the user asked for: kernels run over.
   - `replan`: the replanning below. (`estimate`, the old stop at the estimate, which changed results, is gone.)
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
   files planned a 471 MB Metal workspace, which the buffer pool refuses (`BackendCapacity`, Q22's 15 refusals;
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
- §1 to §5 of the decision (Q19's cause).
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
  buffer pool refuses it. Q22's 15 capacity refusals: 12 pass; the 3 left differ in results on Metal at any state
  size (Metal semantics, Q22). Test: `outgrown_states_restart_and_give_the_worst_cases_results`.
- **The label sweep** (`assign_resident`): under `replan` an evaluation's budget per node is its predicted steps
  per node times the slack, within [256, 2^26] a task. A task stopped below the ceiling reports its own status, and
  the host runs the sweep again with twice the budget (`Outcome::replanned`). Test:
  `a_misplanned_label_sweep_runs_again_and_finds_the_full_budgets_roots` (slow tier).
- **Another plan before the ceiling** (§5's choice): the wave's root walks on the CPU (`clause_roots`) are held to
  the evaluation's (clause × file) prediction times the slack. At an overrun, W's matcher at the candidates (its
  rate per subtree node, measured where it confirms roots, key `w:<program>`; else 4) is weighed against the
  transducer again with twice the budget, and the cheaper runs; W through `WaveRun::limited`, as at the ceiling.
  Test: `a_clause_over_its_budget_goes_to_w_where_w_costs_less_and_runs_again_where_it_does_not`.

Not yet:
1. **`replan` as the default.** Wanted first: the corpus run under `replan` gives `full`'s findings, and the cost
   store holds rates for the pack's programs.
2. **The wave's label passes on the CPU** (`labelled_sets`) run at the ceiling: only the root walks are budgeted.
   Measured on the fixtures, the root walks take at most about 2,000 steps an evaluation; the labels do the work.
3. **The Metal wave's root walks** (`roots_metal_split`) give no `limited`, so W's matcher is never chosen there.
4. The other plans of §5 (another anchor or driver, a prefilter, a split by subtree): only W and the doubled
   budget are weighed today.
5. A misprediction reprices the clause for the next run, not for the wave's chunks not yet started: the rates are
   read once a run.
