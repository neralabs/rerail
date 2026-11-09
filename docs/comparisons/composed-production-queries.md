# Composed production queries

The production compiler now builds a complete relational query before it executes that query.
For supported graphs, logical relations no longer cause separate publication, evaluation, or output-reading statements.
This change fixes the statement boundary. It does not establish performance parity with the two-query reference.

## Integration and workstream alignment

Merge `abf75bb79` includes integration hub `a27994c6a`.
Merge `b34b0376b` records executor tip `735bc4422`; its contents were already present, so this merge changes no files.
The merge preserves demanded aggregate lowering, native semantic locality, parser caching, planner sketches, and retained relation statistics.
The original SSC worktree and Tim's branch remain unchanged.

| Workstream | Reviewed work | Decision in this implementation |
|---|---|---|
| Integration hub | Prepared programs, process parser cache, planner sketches, phase checks | Use the existing compiler, statistics, source bindings, and preparation cache. |
| Executor | Frozen native sources, retained indexes, relational replay lab | Adopt existing native sources through their binding cells. Keep ownership and version changes intact. |
| Cross-file maintenance | Retained statistics and changes to input relations | Run existing load and release hooks around the complete query. Keep incremental support outside cold query composition. |
| Shared structural execution | Two generated reference statements and demanded Python facts | Preserve its batch semantic contracts. Generate SQL from actual RR rules; do not substitute reference SQL. |
| Parser | Cached parse results and grammar work | Keep the merged parser implementation. Do not introduce another parse cache. |
| Acceleration | Batch lanes, row programs, measured placement costs | Share immutable demand frames. Do not introduce another native join interpreter. |

During validation, the other workstreams advanced again.
Hub `7e85463a4` adds held-column index runs and incorporates changed-document input deltas.
Executor `ce80f6873` adds block row programs; `1f16e9a62` adds Metal scan, count, scatter, and tile joins.
Acceleration `9f8ffe4d4` adds measured lane prices.
These newer tips are survey evidence, not part of the tested merge snapshot.
The remaining structural-prefix work must use these executor contracts instead of creating a competing evaluator.

## Execution shape

Previously, `plan::run` entered setup and evaluation for each source chunk.
The native path still created tables for constants and outputs, read those outputs, and removed the tables on later runs.
The SQL fallback separately executed rule inserts and wave folds.

The new compiler traverses dependencies and appends CTE definitions.
Only the final executor steps the resulting statement.
All requested outputs share one tagged result stream:

```sql
WITH RECURSIVE
  structural_candidates(c0, c1, c2) AS MATERIALIZED (...),
  demanded_names(c0) AS NOT MATERIALIZED (...),
  demand_frame(handle) AS MATERIALIZED (
    SELECT rr_demand_frame(c0) FROM demanded_names
  ),
  qualified(c0, c1) AS MATERIALIZED (
    SELECT DISTINCT c0, sym_atom(c1)
    FROM rr_batch_python_qualified
    WHERE frame = (SELECT handle FROM demand_frame)
      AND targets = (SELECT json_group_array(sym_text(c0)) FROM target_names)
  ),
  result_a(c0) AS (...),
  result_b(c0, c1) AS (...)
SELECT 0 AS __output, c0, NULL FROM result_a
UNION ALL
SELECT 1 AS __output, c0, c1 FROM result_b;
```

The compiler uses the existing checked SQL tree and planner statistics.
Constants become query relations. Empty relations have typed, empty SELECTs.
Joins, anti-joins, aggregates, and ordinary linear closure remain SQL operations.
The closure uses recursive UNION for its visited set, without a program counter.

Single-use projections can inline. Shared relations and native results can materialize within the statement.
Native leaves receive complete demand vectors through statement-scoped handles.
Consumers share an immutable vector while retaining their separate target predicates.
The native Python producer keeps its existing semantic cache and conservative import screening.

The first version used SQLite's batched IN interface.
That interface still rebuilt a key set for each consumer.
The frame aggregate now collects and deduplicates each shared demand relation once.
Handles cannot escape the statement or access another nested execution's frame bank.

The result decoder reads symbols from the existing atom arena.
It does not execute a SELECT for each output relation.
Native exports keep their existing retained-source contract.

## Dependency and semantic repairs

The rule pruner removed dead rules but retained dead native producer metadata.
The file-local splitter followed those metadata edges and exported `async_reference` unnecessarily.
Pruning now removes dead native entries and runs again after simplification.
The generated result stream no longer exports the shared candidate relation.

The older SQL tables also enforced non-null relation columns.
Without those tables, failed scalar conversions could enter CTE results as NULL values.
SQL lowering now rejects failed computed bindings and computed head values explicitly.
The regression tests include an invalid conversion whose value is later projected away.

## Boundaries that remain

This implementation executes one query per bounded source batch and one final cross-file query.
It does not yet accumulate narrow facts across all source batches for two whole-corpus queries.
The earlier audit's two-query target therefore remains open.
The separate 100-execution acceptance gate applies to the complete measured job, including setup.

Mutual recursion, nonlinear recursion, lattices, answer hooks, and other native operator kinds retain the existing executor.
Recursive walks that need nested recursive query scopes require another physical plan.
This change does not claim to remove every legacy scheduling path or qualify every catalog program.

Compiled SQL is cached within a job, keyed by the program, compiler version, and binding layout.
The cache does not persist compiled SQL between processes.
The existing prepared-program cache remains separate.
Profiling records compilation, SQLite preparation, query execution, EXPLAIN plans, and input cursor work separately.

## Validation and measured results

The artifacts under `benchmark/async-family-integration/composed/` contain the failing gates, final gates, profiles, SQL, and test results.
The baseline contains 3,849 executions and fails the hard budget.
The first composed attempt also fails, at 114 executions.
Its final assembly still used the old path because input lifecycle hooks excluded it from composition.
Loading those inputs before the query and releasing them afterward removes that exclusion.

The final run covers 5,725 files and 19,007,630 nodes in 72 source batches.

| Measurement | Previous retained-demand path | Composed path |
|---|---:|---:|
| Complete-job SQL executions | 3,849 | 80 |
| Relational evaluation queries | Per-relation/native scheduling | 72 local + 1 cross-file |
| SQL VM instructions | 336,517 | 4,965,288 |
| Intermediate rule-table publication | Present | Absent on this job |
| Shared candidate export | Absent after retention fix | Absent after metadata pruning |

Seven setup statements account for the remaining executions.
The local query has 21,777 SQL bytes; final assembly has 1,558 bytes.
The composed path therefore passes the statement gate but increases SQL VM work by 14.75 times.
The previous path performed most joins natively, so VM counts do not measure equal fractions of their total work.

Three unprofiled runs use the prepared-program store and disable corpus and cross-file memo shortcuts.
Their median corpus-run CPU is 13.221 seconds; median wall time is 17.894 seconds.
Median local evaluation CPU is 1.052 seconds, and median peak process footprint is 664 MB.
Each run executes 80 SQL statements.
These timings include source processing, batch construction, rule execution, and output writing.
They exclude rule-file flattening and initial path enumeration, which the harness performs before the corpus timer.

The earlier retained-demand series measured 8.510 CPU seconds and 7.332 wall seconds.
That series used another merged source snapshot and was not interleaved with this run.
It does not establish a causal speed ratio. No whole-job speedup is claimed.

The separate profile attributes 8.395 ms to local query compilation and 0.760 ms to cross-file query compilation.
Each compilation occurs once in the job.
SQLite preparation totals 4.842 ms; composed query execution, including preparation, totals about 885 ms.
Thus repeated preparation does not explain the remaining query cost.

Validation passes eight composition tests, one frame-lifetime test, four semantic-demand tests, 93 RR tests, and nine file-local tests.
Three pre-existing tests remain ignored.
The real edge jobs also pass the recorded Ruff comparison.

The edge comparison uses all 178 recorded Ruff findings.
The old independent reference misses four of those findings; that known discrepancy remains visible in its separate report.
Zero findings on the full corpus are not the only parity check.

## Remaining structural cost

The composed query still exposes a broad structural prefix to SQLite.
The profile records about 65,000 call-site probes and 50,000 text probes inside the query.
The attribute path also scans broad attribute facts before it tests the normalized method name.
This work explains why fewer statement executions do not imply fewer VM instructions.

The next physical plan needs a batch native structural prefix with a typed output relation.
It must combine execution-context selection, call-site projection, and normalized attribute-name filtering over the existing posting sources.
It must preserve file keys, nearest execution boundaries, and the exact candidate scope.
The executor's block row programs are the reusable implementation point.
SQL then composes narrow prefix results with demanded semantic facts and final findings.

This is a producer boundary, not another SQL interpreter or a whole-rule replacement.
Acceptance requires fewer cursor probes and lower whole-job CPU, with the current parity and execution gates still passing.
The broad shared-prefix scan and the missing two-query whole-corpus plan remain explicit gaps.
