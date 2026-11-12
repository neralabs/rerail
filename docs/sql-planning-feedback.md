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
The wave batch now has a signed branch for total positive joins with up to eight inputs.
SQL finds affected projected keys from inserted and deleted witnesses.
For each pivot input, the other inputs include the current rows and deleted rows.
This superset covers witnesses that disappear through simultaneous deletions.
An indexed virtual table probes both sources through their existing access paths.
A SQL union at this boundary can materialize the inputs and lose those access paths.
SQL then checks each affected key for a surviving witness in the current inputs.
Native storage patches the prior candidate root with those SQL results.
This preserves multiple witnesses for one head and supports self-joins.
Correlated scalar and negative rules can use the same batch when their head exposes every correlation key.
SQL finds affected groups from changes to the outer context and the scalar inputs.
It then removes prior heads for those keys and recomputes the complete current group.
This covers counts, nullable minima and maxima, and positive or negative existence tests.
Global, nested, ordered, and effectful scalar cases retain their full evaluation.
An eligible correlated scalar can also use an outer input from a positive CTE.

Positive acyclic CTE projections expand under set semantics before signed lowering.
Aliases and declared output columns retain their original meaning.
Recursive, multi-arm, aggregate, and effectful CTEs keep their SQL kernels.
Lookup-generated sources also keep their full branch because their implicit inputs need a complete change journal.

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
Learned forecasts remain advisory.
The signed branch uses a shared cost comparison for input sizes, changes, and indexed probe estimates.
It compares estimated sparse probe work against a scan, with headroom for support checks.
The batch caches each derived difference against the exact pair of preceding-wave roots.
External input statistics and signed journals stay fixed throughout the operation.
The ledger enforces the budget for the single SQL batch statement.

Each gate reports its input statistics, cost comparison, eligibility proof, and selected branch.
Batch totals report signed evaluations, full evaluations, reused results, affected heads, and native difference work.
Completed operation reports carry these gate statistics to the background writer.
Observation version 3 distinguishes fold, scalar, and preparation changes from the earlier batch measurements.

## Implementation order

1. Add exact statistics for composite join keys and probes on the changed keys.
2. Extend grouped maintenance to nested scalars and additional CTE shapes.
3. Use learned forecasts to compare complete candidate schedules and reserve credits before dispatch.
4. Add preparation, worker CPU, and native root construction to the cost model.
5. Reduce cold preparation costs and the full kernels for unsupported query shapes.
6. Measure complete sessions for each replay size.

Current statistics include input cardinalities, inserted and deleted row counts, distinct values for indexed leading columns, and probe estimates.
A derived statistic uses the same immutable roots as its signed difference.
An external statistic uses the source bound for the current operation.
The planner obtains these statistics without a full planning scan.

The replay matrix includes an unchanged run, a one-file edit, and a wide edit.
It covers explicit facts, snapshots, and file changes.
Capped runs include binding, query verification, and cleanup.
Capped and uncapped runs must return the same rows at every checkpoint.
One-file runs must measure CPU and VM work as well as submissions to detect a return to full scans.

## Signed batch checks

The debug comparison uses 1,024 groups, overlapping rule producers, duplicate witnesses, a derived self-join, and a recursive count.
The signed branch used 1,322,529 VM steps for a small edit.
The full branch used 3,036,650 VM steps.
This is a 56.4 percent reduction.
Both branches used six submissions for the edit and stayed within the session cap of 100.

The checks cover simultaneous insertions and deletions, duplicate projected keys, surviving alternative supports, failed execution, and a successful retry.
A separate process checks that the background writer stores the signed strategy and its gate statistics without a flush.
The shared projection proof also checks 4,096 combinations of old and new input sets.

The CPython comparison uses the repository's workload and call-analysis programs.
It checks program inputs and derived results after a file edit, an identical edit, and restoration of the original source.
Both sessions stay within 100 submissions, including verification reads.
Each file edit uses three submissions. The identical edit uses zero.

| First file edit in debug | Full VM steps | Signed VM steps | Full owner CPU | Signed owner CPU |
|---|---:|---:|---:|---:|
| Workload | 24,865,627 | 18,384,924 | 4.72 s | 2.81 s |
| Call analysis | 81,036,654 | 69,775,184 | 10.80 s | 8.45 s |

These CPU values come from one debug comparison with two configured workers.
The full control uses the same batch planner and disables signed reconciliation.
The comparison checks output parity and rejects an increase in VM work.

An earlier implementation used SQL unions for the current rows and deleted rows.
On the workload edit, that implementation used 133 million VM steps.
The full control used 25 million.
The indexed virtual-table view removed that materialization cost.
The CPython comparison now checks this regression.

The final debug checks passed 203 SQL executor tests, six admission tests, and two shared delta-proof tests.
The CPython comparison passed separately for both programs.
These measurements precede the fold, scalar, and preparation changes described below.
Learned forecasts remain advisory.

## Fold and preparation changes

Warm folds use the retained candidates from every producer, the preceding published rows, and the widening counters.
Equal inputs reuse the prior fold and its frontier.
Changed inputs supply signed roots to SQL, which selects the affected keys.
The original SQL merge and widening expressions run on those keys.
Native storage patches the prior output and constructs its frontier from immutable differences.
This preserves duplicate producer support, lattice values, and widening counts.
Cold folds and folds without a retained trajectory use their full evaluation.

The connection keeps the batch virtual tables and callbacks registered.
Each operation supplies its arena through a guard that clears the binding on success or failure.
The schemas have a fixed width, and the statement uses the connection's prepared statement cache.
The reset path keeps the runtime and removes every active operation binding.
Schema changes can still trigger SQLite's automatic reprepare.

Explain and background reports include the reused and signed fold counts, affected keys, scalar evaluations, preparation CPU, and automatic reprepares.
The fold and scalar reconciliation proofs live in the shared analysis module.
The CTE normalization lives in the shared logical planner.
The debug checks cover grouped counts, nullable minima, negative existence, positive CTEs, widening counters, rollback, retry, and warm cache reuse.

The expanded debug fixture includes 1,024 groups, duplicate witnesses, overlapping producers, a self-join, recursive counts, negative existence, and nullable minima.
For one small edit, the optimized execution used 93,849 VM steps.
The full execution used 3,243,184 VM steps.
Both used eight submissions within the same cap of 100.
The full control disables signed rule reconciliation, fold reuse, and signed fold reconciliation.
Both paths retain the connection runtime and prepared statement cache.
This comparison therefore measures evaluation changes rather than the separate benefit of statement caching.

The cache check applies three consecutive updates with stable schemas and requires zero automatic reprepares on the final update.
Preparation CPU measures explicit preparation or cache lookup.
An automatic reprepare occurs during statement execution, so its CPU remains part of execution CPU.
The runtime releases its operation binding after failed execution, and a later retry uses the same connection.

The updated debug suite passed 205 SQL executor tests, six admission tests, two shared delta-proof tests, and one shared CTE planner test.
Six SQL executor tests remain ignored in the normal suite.
The CPython comparison runs separately.

The updated CPython replay passed for both programs, including the identical edit and restoration of the original file.
The first edit produced these debug measurements with two configured workers:

| Program | Full VM steps | Optimized VM steps | Full owner CPU | Optimized owner CPU |
|---|---:|---:|---:|---:|
| Workload | 24,865,636 | 14,462,198 | 5.51 s | 2.32 s |
| Call analysis | 81,036,688 | 66,412,409 | 11.91 s | 8.25 s |

VM work fell 41.8 percent for workload and 18.0 percent for call analysis.
Each edit used three submissions. The identical edit used zero submissions and zero VM steps.
Both sessions stayed within 100 submissions, including result checks.
The checks compared all program inputs and derived relations at every checkpoint.
Owner CPU values come from one debug run on a shared machine and exclude worker CPU.
These results establish correctness and execution headroom for these sessions.
They do not establish performance acceptance for the complete product.

Remaining costs include cold preparation, schema-triggered automatic reprepares, global or nested scalar evaluation, and unsupported CTE kernels.
Composite statistics and learned ranking of complete schedules remain separate planner work.
