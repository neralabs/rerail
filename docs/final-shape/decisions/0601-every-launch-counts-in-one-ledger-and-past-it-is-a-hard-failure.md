# 0601. Every launch counts in one ledger, and past it is a hard failure

Status: accepted (2026-11-22). Changes 0540's ledger (where it lives, who charges it, what a refusal does) and list
step 13's plan-time bound; the budget itself (`KERNEL_LAUNCH_LIMIT`, 200) and 0540's reruns' budget are unchanged.

## Intro

The launch budget is a bound on how the engine's device code is written: an evaluation that needs more than 200
launches is code to restructure, not work to move elsewhere (user, 2026-11-22: "The launch budget is there to stop you
from writing stupid inefficient code ... Your making it fallback to the CPU entirely defeats the purpose").

Before this decision only the engine's commands (`rerail_metal::lane::Command`) charged the ledger. Every other
submitter had its own count, or none:

- the relational passes (`rerail_accel::gpu::join::Pass`: the device joins, `metal_rows`, accel's sorts, probes,
  tiles and column statistics) counted their own launches, and checked a process-wide guard
  (`shared::set_guard`) that the engine set to a closure opening a fresh evaluation, so it never refused;
- a join batch (`metal_join::Batch`) opened a new submission every 150 launches, so a stratum's launches had no bound;
- the executor's GPU lane checked the same guard before it ran;
- a refusal (`Error::BackendLaunchLimit`) was caught: a matcher wave's batch, a label run's steps and a wave's
  documents were split in halves until each half fit, and a refused stratum ran on the CPU.

## Decision

1. **One ledger, in `rerail-accel`** (`gpu::launch`): the evaluation (`LaunchScope`), its budget, the run's reruns'
   budget (0540) and the records. `rerail_metal::launch` re-exports it. Every submitter charges each launch before
   it is enqueued (`attempt`) and checks the evaluation at its commit (`check_submission`): `lane::Command`,
   `join::Pass` and the executor's GPU lane (`GpuBackend::run`). A UKL library call, which says its launches once
   it has enqueued them, charges them then (`charge_made`). No submitter keeps a count of its own as a bound; the
   process-wide guard is removed, and so is `metal_join`'s split at 150.
2. **The units of work are the evaluations**: a stratum's device work (`setfix::device::in_device`: its joins, every
   round of a recursive one), the executor lane's batch, a sort, a probe, a tile join, a column-statistics batch, a
   virtual table's chunk axis, a row scan. Each enters an evaluation, or joins the caller's. A stage run again after
   an overflow, a probe's or a lane batch's scatter again, is a rerun (0540).
3. **Past the budget is a hard failure.** The launch that would pass it, or a launch outside any evaluation, raises
   `rerail_accel::exec::OverBudget`: the ledger's account (the evaluation's launches by kernel) is printed and the
   payload panics. A catch on the waiting thread (`Job::wait`, `gpu::Running::finish_counted`) raises it again
   (`exec::resume_over_budget`); a catch on a worker, which has no caller to raise it to (a job's thread, a flight's
   end, a lane's host, upkeep, a stream, the corpus's workers), aborts the process (`exec::abort_over_budget`). So
   nothing turns it into an error, a CPU run or a smaller batch. A label run's plan that cannot fit is the same
   failure before it encodes (`launch::plan_past_budget`). `Error::BackendLaunchLimit` is removed, with every
   handler that split a batch (a matcher wave's tapes, a label run's steps, a wave's documents) or skipped a check
   on it (the parser executors' list of skipped runs).

## Measured

With every launch counted (m3, the setfix device tests): an 11-rule stratum charges 201 launches, 101 of them UKL
library scans and sorts, 40 dedup, 38 stage and 22 scan launches; a recursive stratum's rounds charge one stratum's
worth each. Each rule launches its own work, so a stratum's launches grow with its rules and its rounds.

At this decision's commit, six setfix device tests fail on it (m3), each with the account of its stratum:

| Test | Library | Dedup | Stage | Scan | Other |
|---|---|---|---|---|---|
| `a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts` | 100 | 40 | 38 | 22 | 1 |
| `a_recursive_stratum_on_the_device_is_the_cpus_fixpoint` | 95 | 68 | 28 | 20 | 63 (index, runs, sorts, copies) |
| `a_leveled_recursion_on_the_device_has_the_cpus_levels` | 62 | 56 | 22 | 14 | 46 |
| `text_functions_on_the_device_are_the_cpus` | 107 | 40 | 9 | 22 | 22 (row texts) |
| `text_tests_on_the_device_are_the_cpus` | 98 | 44 | 19 | 23 | 16 (row texts) |
| `parity::parity_combinators` | 57–67 | 44–56 | 18–34 | 12–14 | 50–60 |

They are the work below, not tests to change.

## The work this sets

- A stratum's launches independent of its rules: one launch a stage kind for all its rules, one scan, one dedup.
- A recursive stratum's rounds on the device, as a work queue's loop (as `label_phase`'s, 0540 step 7).

## Tests

- `rerail-accel`'s `gpu::launch::tests`: the budget, the reruns', and a launch past either or outside an evaluation
  raising `OverBudget`.
- `gpu::join::tests::a_pass_charges_the_evaluations_launch_budget`: passes share one evaluation's budget.
- `rerail-metal`'s launch tests: a command's launches charged by every dispatch form, and the launch past the budget
  raising `OverBudget` with nothing of the command run.
