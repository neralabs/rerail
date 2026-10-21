# SQL workload profiling

The [performance journal](data-layer-performance-journal.md) retains baseline measurements and experiment outcomes.

The relational executor can record every executed statement on its SQLite connections.
The capture includes setup, input insertion, stage materialization, indexes, result queries, transactions, and cleanup.
Compilation connections also appear in the capture.
Other SQLite connections, such as the content store, remain outside this capture.

This work comes from `jeremy/sql-capture-demand-spikes` at `5efcb830dcc2b75eb05f803ef9e7082f0e29d603`.
The statement tracer, report renderer, cost summaries, and report tests come from that branch.
The integration uses this branch's relational executor and bounded profiling controller.
The SQL matcher compiler and its optimization experiments remain on the source branch.

## Run a capture

Use an addon built with the `code-rule-profile` Cargo feature.
Set `RERAIL_ENGINE_PATH` to that addon.
Use the existing [bounded profiling workflow](code-rule-profiling.md) to build and select it.
Production builds exclude this instrumentation.

The [SQL shape experiments](sql-shape-spikes.md) record isolated changes, CPU results, parity checks, and reproduction commands.

Run the pinned five-file workload in a new output directory:

```sh
npm run profile:code-rule-dsl -- \
  --recording .rerail/profiles/batched-sql-integration/profile-five/selected-recording.json \
  --policies packs/data-layer/candidates/rules \
  --sql-profile --trace off --cold-only --deadline 115 \
  --allocator system-malloc-shim --output .rerail/profiles/sql-quality
```

Alternatively, supply `--source` and the pinned `quality-five.json` selection.
Omit `--cold-only` to capture unchanged and edit phases.
Add `--trace on` for the existing stage and producer spans.
External sampling remains available through the existing controller.
The command's deadline includes report generation.

For another workload driver, append its command after `--` and retain `--sql-profile`.
Without `parity.json`, the report marks the output comparison as unavailable.
Custom hosts can set `RERAIL_SQL_PROFILE_DIR` before the addon loads to capture raw events directly.

Open `sql/profile.html` inside the output directory.
The page ranks queries by exclusive CPU and supports filtering by SQL, context, and flags.
Select a query to inspect its counters, plans, and bytecode.

| Artifact | Contents |
| --- | --- |
| `sql/summary.md` | Five ranked investigation targets with measured costs and validation steps |
| `sql/profile.json` | Complete query metrics, structured contexts, flags, and provenance |
| `sql/profile.md` | Query ranking and detailed SQL for text review |
| `sql/profile.html` | Searchable report with plans and bytecode |
| `sql/trace.json` | Execution timeline for Perfetto or Chrome tracing |
| `sql/raw/profile-*.jsonl` | Every observed execution on each connection |
| `sql/raw/profile-*.json` | Query catalogues and deferred explanations |
| `sql/workload.json` | Workload and native hashes, policy digests, phase CPU, and parity status |
| `sql-report.log` | Report output or the reason report generation failed |

The usual `profile.json`, `parity.json`, stage traces, and external samples remain at the output root.
Their schemas and timelines remain separate from the SQL report.

Regenerate reports without executing the workload:

```sh
npm run profile:sql:summary -- .rerail/profiles/sql-quality/sql
```

This command preserves the raw capture and timeline.
It records the current report implementation hashes.

## Measurements and attribution

Each execution records wall time, thread CPU, returned rows, changed rows, and four SQLite counter differences.
The SQLite counters measure VM instructions, full-scan steps, sorts, and automatic-index rows.
Cached prepared statements retain separate execution events.
The tracer never resets counters that other diagnostics read.

Immutable syntax virtual tables record scans, indexed lookups, cursor advances, and column reads.
Owner index construction records timing when it occurs inside an observed SQL execution.
Normally, structural indexes already exist before SQL runs.
Their preparation remains visible in the existing stage trace.
The historical input adapter experiments no longer form part of the engine or profiling addon.

Stage and publication operations record their inclusive cost and their exclusive cost after subtraction of child SQL.
CPU rankings add SQL cost and exclusive operation cost once.
Stage operations do not cross worker handoffs or producer suspension boundaries.
Contexts retain rule, clause, file where available, stage, and run phase.
A repository scope has a null file path.
The timeline links execution events to query identifiers in the report.

SQLite does not include CTAS output rows in its changed-row counter.
The report therefore treats CTAS output cardinality as unobserved.
It does not diagnose an empty materialized table from a zero counter.
Successful operations with unobserved rows remain distinct from incomplete operations.

## Plans and work limits

The profiler collects `EXPLAIN QUERY PLAN` and `EXPLAIN` bytecode outside SQLite callbacks and timed operations.
It collects explanations before last-use release removes temporary inputs.
For CTAS, it explains the SELECT body and records that body as `planSql`.
Parameters remain unbound and have NULL values during explanation.
Plans describe the schema and statistics at the checkpoint, which can differ from those at execution.

Explanation does not execute the query body or spend the rule's SQL instruction allowance.
The normal work budget, cancellation, and producer completion rules still govern actual evaluation.
Statements that fail during preparation have no execution event.
Deferred explanations of schema statements can fail because their objects already exist.
The report distinguishes these expected failures from unexpected missing plans.

The tracer owns a stable callback address and unregisters before its connection closes.
It flushes captures for failed sessions as well as successful sessions.
The report rejects missing catalogues, event-count mismatches, and recorded callback failures.
A missing profiling feature also causes the capture command to fail explicitly.
The controller clears inherited capture directories when capture is disabled.

## Interpretation

Flags identify investigation targets, not proven causes or guaranteed savings.
Observed flags use counters; potential flags use plan shapes.
Examples include full scans, runtime sorts, temporary B-trees, correlated subqueries, and many small executions.
Ranked actions use disjoint query groups and require both 10 ms and one percent of captured CPU.
Relational recommendations refer to this executor and the recorded authored stage.
The report retains compatibility with captures from the source branch.

The pinned data-layer recorder checks complete diagnostics and failures for this executor.
The report records that oracle result; it does not run the source branch's alternate SQL matcher executor.
Positive and clean behavior still require the corpus and production parity checks.

Profiling adds clocks, row callbacks, counter reads, serialization, and explanations.
Short statements can show substantial instrumentation overhead.
Captured spans do not account for all process CPU, including parsing and preparation outside operations.
Summed worker wall time is not workload latency.
Use an unprofiled matched run to assess an optimization.
The comparison tool rejects profiles with different SQL capture modes.

## Tests and recorded checks

Run the focused Rust tracer tests:

```sh
cargo test --manifest-path scripts/profiling/reachability/Cargo.toml \
  --no-default-features --locked code_rule_dsl::tests
```

The SQLite profiling lab imports the production tracer source directly.
It does not maintain another tracer implementation or compile the complete native engine test target.
CI runs these five tests and the JavaScript and Python profiling tests.
The tests cover cached counter differences, incomplete operations, worker moves, CTAS explanations, and explanation work isolation.
A scalar-function counter also checks that explanation never executes the query body.

The initial five-file capture records 28,145 execution events across 21 connections.
The unchanged-and-edit capture records 36,628 events across 26 connections.
Both captures pass diagnostic parity and have zero unexpected explanation failures or incomplete operations.
Their 230 expected schema explanation failures follow completed CREATE, ALTER, and DROP operations.
The unchanged phase uses cached results and executes no relational SQL.

The native addon also passes all 406 native tests, all 200 upstream corpus cases, and the producer execution-shape gate.
These checks validate the tools and execution behavior; captured timings do not establish a performance improvement.
Local captures and reports reside under `.rerail/profiles/sql-tools-port/`.
