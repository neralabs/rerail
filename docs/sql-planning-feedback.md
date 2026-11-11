# Cost-based SQL planning

## The replay cost

A one-file replay used 129 SQL submissions, 24.0 million VM steps, and 5.7 CPU seconds without a submission cap.
With a cap of 100, the same replay used 51 submissions, 172.4 million VM steps, and 13.0 CPU seconds.
The retained program had many stages and few credits remained. The planner selected full-wave batching.
The batch reran full queries for rules with changed inputs.
The sparse path used the signed changes to limit its work.
The number of stages describes the schedule. It gives no measure of how much data an edit changes.

## Shared planning inputs

The compiler supplies a graph with the rule reads, writes, and dependency gates.
It attaches proofs for uniqueness, monotonicity, scalar compatibility, and version reuse to this graph.
These proofs apply to both native and SQL execution.
The SQL planner also uses connection ownership, function purity, and the available access paths to determine which placements are legal.
SQL mode uses SQL for rule evaluation. Native code supplies facts, indexes, and retained storage.

The planner reads statistics from the immutable roots at each gate.
Initial statistics include input cardinalities, inserted and deleted rows, and the number of changed relations.
The next statistics describe which join keys changed, the distinct key count, and the fanout of probes on those keys.
Each statistic carries the input version, or the pair of versions for a delta.
The planner can reuse a statistic with matching versions.
It can share a retained summary across gates with matching input versions and maintenance cursors.

A virtual table can supply a cardinality estimate without a scan.
The planner records the source of each estimate. If the statistics are incomplete, the planner uses a wider cost range.
It obtains key statistics from indexes and bounded samples as needed.
A sparse replay must avoid a full-relation scan to collect planning statistics.

## Candidate plans and admission

The planner compares sparse phases, sparse SQL batches, and full-wave SQL batches at the same dependency gate.
A sparse SQL batch combines compatible signed transitions in one statement.
It preserves the cursors and immutable versions that each transition reads.
If the changed work approaches the cost to read the full input, a full-wave batch uses the existing whole-query kernels.
The current implementation has sparse phases and full-wave batches. Sparse SQL batching remains the next kernel.

Each candidate has a submission count or bound, estimated CPU and wall time, and a peak memory estimate.
The CPU estimate includes SQL preparation, join and aggregation work, transfer of result rows, and construction of native roots.
The SQL planner estimates preparation from statement size, CTE dependencies, and output arms.
It estimates evaluation from input rows, changed keys, fanout, and the size of temporary results.
Worker placement uses the shared scheduler and includes queue time, transfer costs, and the longest dependent path.

The planner removes candidates that exceed the available submission or memory budget.
It uses exact counts for known statements and conservative bounds for dynamic work.
An observed maximum can inform a bound. Only a bound justified by the plan can guarantee admission.
The planner then chooses the feasible schedule with the lowest estimated CPU for the operation.
If two schedules have similar CPU estimates, the planner prefers the shorter estimated wall time.

The session ledger remains the authority for all owner and worker submissions. It counts failures and recovery statements.
A session with a declared replay can account for later setup, verification, and cleanup before it admits early work.
A live session knows its remaining credits and the current operation.
Each recursive gate can revise estimates from the actual frontier and choose again from plans that can use its retained state.
The planner preserves version and cursor checks before it changes a kernel.

## Feedback in the shared database

The engine writes SQL reports to `sql_plan_observations` in the existing `RERAIL_COSTS` database.
The default path is `~/.cache/rerail/costs.sqlite`.
`RERAIL_COSTS=off` turns off collection.
Each row contains an operation report and the evaluation stages for its affected programs.

The report contains the strategy, the budget at the planning boundary, and a digest of the parsed program.
It also contains step counts, relation counts, input cardinalities, and the sizes of the inserted and deleted inputs.
The report stores no raw SQL, source text, file paths, row values, or error strings.
Operation totals include setup and publication within the operation, plus rollback on failure.
Evaluation totals cover the rule stage. A cold stage also includes retained support construction and final row-count publication.
Both totals report submissions, SQLite VM steps, owner CPU time, and wall time.
The ledger counts submissions and VM steps on the owner and workers.
Owner CPU excludes worker CPU. The existing scoreboard and profiler give worker CPU and preparation details.

The execution thread adds a completed report to a bounded memory queue.
The existing cost writer takes batches from this queue once a second and uses one transaction for SQL and other job samples.
If a write fails, the writer queues the SQL reports for retry.
If the queue fills, it discards the oldest reports and records the number it discards.
The command-line engine uses its existing final flush to write reports before exit.

The cost reader loads recent reports in the background on startup.
Each successful write also adds its reports to the current process model.
The model groups successful stages by program, strategy, cold or warm operation, cardinality ranges, edit size, and wave limit.
It uses records with the same machine key, thread count, build profile, and observation version.
It also groups records by detailed profiling and by the presence of time and VM limits.
After five matching stages, it supplies recent median costs and the largest observed submission count.
Failed operations stay in the database and do not supply successful cost estimates.

`explain().sql_planning` shows the input features, selected strategy, prior forecasts, actual measurements, and writer counters.
An alternative forecast can show the cost of a different retained strategy for a similar input.
The current retained history determines whether a different strategy is available at a warm gate.
The forecasts are advisory. The current placement rules select the strategy and the ledger enforces the budget.

## Implementation order

1. Add versioned key and probe statistics from immutable roots and their deltas.
2. Add sparse SQL batches for compatible signed transitions.
3. Compare the three candidates at each gate and reserve credits before dispatch.
4. Add preparation, worker CPU, and native root construction to the cost model.
5. Check output parity and the complete submission budget for cold and warm sessions.
6. Measure the forecasts and actual costs for each replay size.

The replay matrix includes an unchanged run, a one-file edit, and a wide edit.
It covers explicit facts, snapshots, and file changes.
Capped runs include binding, query verification, and cleanup.
Capped and uncapped runs must return the same rows at every checkpoint.
One-file runs must measure CPU and VM work as well as submissions to detect a return to full scans.
