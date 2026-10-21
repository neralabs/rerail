# SQL workload profiling

The profiling mode runs a workload through the native engine and the SQL engine.
It compares complete findings, failures, and blind-file results.
It records every executed SQLite statement on SQL engine connections.
It also records native frame operations, including shared projections and recursive prefix operations.

## Run a workload

1. Build an addon with the benchmark feature:

   ```sh
   npm run build:native:unbounded-work
   ```

2. Create a workload configuration:

   ```json
   {
     "addon": "/absolute/path/to/rerail_engine.node",
     "capabilities": "/absolute/path/to/native/src/code_rule_dsl/capabilities.json",
     "corpus": "/absolute/path/to/corpus",
     "files": ["example.py"],
     "policies": ["/absolute/path/to/policy.json"],
     "language": "python",
     "threads": 4,
     "batchSize": 64,
     "timeoutMs": 600000
   }
   ```

   The configuration uses the same format as `benchmark/sql-set-investigation.mjs`.
   The addon worker count must match `threads`.
   Configuration paths resolve from the current working directory.
   File paths resolve from `corpus`.

3. Run the profiling command with a new output directory:

   ```sh
   npm run bench:sql-profile -- /absolute/path/workload.json /tmp/rerail-sql-profile
   ```

4. Open `/tmp/rerail-sql-profile/profile.html` in a browser.

5. Load `/tmp/rerail-sql-profile/trace.json` into Perfetto or Chrome tracing.

The command preserves SQL engine settings from the environment and records them in `workload.json`.
It disables the older SQL trace during this run to avoid duplicate instrumentation.
A result mismatch or workload failure produces a nonzero exit status.
A performance flag does not change the exit status.

## Read the outputs

| File | Contents |
|---|---|
| `profile.html` | Searchable query table, CPU ranking, flags, SQL, plans, and bytecode |
| `profile.json` | Complete query metrics, clause metrics, flags, thresholds, and workload provenance |
| `profile.md` | Query ranking and details for text review |
| `trace.json` | Execution timeline with query text, context, counters, and row observations |
| `raw/profile-*.jsonl` | Streamed execution events for each connection |
| `raw/profile-*.json` | Query catalogues with `EXPLAIN QUERY PLAN` and `EXPLAIN` bytecode |
| `workload.json` | Configuration, input hashes, addon hash, environment, and output comparison |
| `native.json`, `sql.json` | Workload results and engine reports |
| `native-outcomes.json`, `sql-outcomes.json` | Complete normalized results for the output comparison |

The HTML report sorts by CPU cost and filters by SQL text, clause, or flag.
A query row opens its metrics, flags, plans, and bytecode.
The timeline retains execution order on each worker, including repeated cached statements and loop checks.
Query identifiers connect the timeline to the report.
Clause contexts include the rule and clause names available from the engine report.
Session queries cover setup, input loading, transactions, and cleanup.

## Measurements

Each SQL execution records wall time, thread CPU time, returned rows, changed rows, and four SQLite counters.
These counters measure VM instructions, full-scan steps, sorts, and automatic-index rows.
Frame counters measure scans, lookups, cursor advances, column reads, inserts, index builds, and rows indexed.
The counters are differences for each execution, including cached statements.
The profiler does not reset counters that other diagnostics use.

Each native operation records its input frame sizes, output rows, clause context, and recursion depth.
Its exclusive time subtracts the SQL time inside that operation.
CPU rankings add SQL time and exclusive native-operation time.
Inclusive native-operation time overlaps its child SQL spans on the timeline.
Aggregate wall time sums worker spans and does not measure workload latency.
SQL spans exclude preparation, while parent operation spans include preparation.
Parsing and other pipeline phases outside these spans have no query attribution.

The profiler collects plans after execution, outside timed spans and SQLite callbacks.
It runs each explanation once per observed SQL text on each connection.
The explanation does not run the query body.
Parameters remain unbound and therefore have NULL values during explanation.
Different plans across connections remain visible in the report.
Native descriptors have no SQLite plan because the engine runs them directly.
Deferred explanations of setup statements can fail because their tables already exist.
The report retains these errors as `explain-unavailable` flags.

## Flags

Observed flags use execution counters or measured row counts.
Potential flags use query-plan shapes that need further investigation.
A flag identifies work to inspect and does not prove that the query needs a change.

| Flag | Evidence |
|---|---|
| `full-scan` | SQLite reports full-scan steps |
| `virtual-full-scan` | The plan selects virtual-table index zero |
| `runtime-sorts` | SQLite reports sort operations |
| `temporary-btree` | The plan uses a temporary B-tree |
| `correlated-subquery` | The plan contains a correlated subquery |
| `materialization` | The plan materializes an input |
| `automatic-index` | SQLite inserts rows into an automatic index |
| `frame-index-rebuilds` | A query builds frame lookup indexes at least 10 times |
| `high-vm-per-row` | At least 100,000 VM instructions and 1,000 instructions per returned or changed row |
| `empty-result-work` | At least 10,000 VM instructions with no returned or changed rows |
| `many-small-executions` | At least 100 executions with average CPU time below 1 ms |
| `row-expansion` | An operation publishes at least 1,000 rows and exceeds its largest input by at least 10 times |
| `repeated-frame-scans` | At least 100 frame scans and 100,000 cursor advances |
| `incomplete-operation` | An operation exits without completing |
| `explain-unavailable` | A query explanation fails or is missing |

The JSON report contains the thresholds and evidence for each flag.
The profiler does not infer disk spills, copied bytes, or actual subquery loop counts from plan text.

## Limits and direct capture

Profiling adds row callbacks, clocks, counter reads, and event serialization.
Profiled runtimes are diagnostic measurements and do not replace ordinary benchmark timings.
Native-operation time includes callback and event-writing overhead that occurs inside its span.
Frame counters cover frame virtual tables, but they do not count internal operations inside pattern or fact terminals.
Terminal work remains part of its SQL execution time.

Returned, changed, and published row counts describe different observations.
Their totals can count the same row in both an operator and its SQL child.
Shared frame rows do not imply copied cells.
Skipped statements do not run and therefore produce no execution event.
SQL that fails during preparation has no execution event.

The build excludes profiling from production addons.
For a custom workload runner, set `RERAIL_SQL_PROFILE_DIR` before the benchmark addon loads.
The directory receives raw catalogues and execution streams from SQL engine sessions.
A batch checkpoint flushes events and explains new SQL before the session can leave its worker.
The command requires a new directory to prevent stale traces from entering a report.
The full trace can be large because it preserves every repeated statement.
