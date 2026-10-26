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

## Complete-job execution accounting

The fixed diagnostic reference is **100 engine-submitted SQL statements for one complete cold job**.
Full-repository acceptance uses cold CPU, wall time, RSS, and complete findings with no engine failures.
Commit `c109a915` on the source branch defines this limit. The earlier profiler import omitted its enforcement.
Every worker and connection shares the allowance. File batches, rules, and output packets do not reset it.
Setup, publication, DDL, transactions, matching, and cleanup count. Each statement inside `execute_batch` counts separately.
SQLite internal statements remain separately counted. Diagnostic EXPLAIN and enclosing native timing spans do not count.
The user clarified this boundary after a single bulk FTS insert exposed internal document metadata writes.
Callback origin, native counters, and observed per-origin events must reconcile; unknown origins fail acceptance.

The capture includes started/completed counters and connection-open/close receipts.
Missing entire sessions, partial captures, incomplete operations, or failed parity prevent acceptance, even when observed counts are small.
Reports retain `execution-budget.json` and the first excess statement.
SQL capture and report commands return nonzero on failure after preserving the complete findings and evidence.
Historical captures without coverage receipts or execution origins remain useful diagnostics but cannot pass acceptance.

The mandatory CI guard uses unchanged positive and clean observations from the state-tripwire recording:

```sh
npm run test:sql-execution-budget -- .rerail/profiles/sql-budget/new-run
```

Use a fresh output directory and a profiling addon through `RERAIL_ENGINE_PATH`.
The [recovery workplan](sql-performance-recovery-workplan.md) defines the remaining steps and whole-repository acceptance requirements.

## Run a capture

Use an addon built with the `code-rule-profile` Cargo feature.
Set `RERAIL_ENGINE_PATH` to that addon.
Use the existing [bounded profiling workflow](code-rule-profiling.md) to build and select it.
Production builds exclude this instrumentation.

The [SQL shape experiments](sql-shape-spikes.md) record isolated changes, CPU results, parity checks, and reproduction commands.

## SQL page census

Ordinary CPU tracing does not scan SQL pages.
Use `--sql-working-set` with `scripts/profiling/run.py` to request per-stage object sizes and reserved-page counts.
This option requires `--trace on` and can be expensive over a growing corpus.
For a direct harness invocation, set `RERAIL_PROFILE_SQL_WORKING_SET=1` and `RERAIL_PROFILE=1`.
Keep the census out of CPU and wall-time controls.
The separate `--memory-census` option records workspace ownership at driver boundaries.

## Focus on one component

Set `RERAIL_SQL_PROFILE_SCOPE=relational` to capture relational statements while excluding candidate-index publication.
Use `ast-index` to select candidate preparation instead.
The filter matches the connection context, including the scope of each native operation.
A relational-only capture does not copy candidate databases or export candidate producer plans.
Normal execution still runs every required operator.

Filtered reports set `executionCoverage.fromSessionStart` to `false` and retain the selected context in `contextFilter`.
They cannot pass the complete-job accounting check, even when their statement count is below 100.
`filteredStatementNotifications` counts excluded trace notifications. It does not claim an execution-origin count.
Unset this variable for complete captures and for performance measurements.

## Candidate preparation captures

The candidate index also supports input-complete SQL captures.
The [preparation design](sql-preparation-architecture.md) and [spikes](sql-preparation-spikes.md) describe their purpose.
By default, SQL-profile capture writes `candidate-*.json` and matching SQLite snapshots beneath `sql/raw/`.
For large workload diagnosis, add `--sql-trace-only` to capture statements without these replay copies.
This mode retains SQL plans, counters, and statement coverage. It cannot supply standalone candidate replay databases.
The equivalent direct-run environment setting is `RERAIL_SQL_PROFILE_REPLAY=0`.
Use separate bounded replay captures when database contents are required.
Snapshots contain the selected source, packed facts, indexes, and retained property state before candidate execution.
The JSON files contain generated SQL, typed parameters, logical relation identities, and file identities.
They do not contain JSON AST rows.

The [shared-work tools](shared-producer-contracts.md#discovery-tools) find repeated generated definitions and diagnose proposed producer names.
They accept these captures without another engine run.
Declaration checks explain conflicting definitions and valid differences between bindings.
Their results support producer extraction, without automatic SQL equivalence claims.

Each request has normal and uncompacted descriptions.
The uncompacted description preserves the same operators before canonical producer sharing.
The engine executes only the normal description.
Artifact capture adds work and storage, so its timings cannot serve as performance controls.

Rank repeated logical construction from a diagnostic capture:

```sh
python3 scripts/profiling/preparation.py .rerail/profiles/sql-preparation/diagnostic \
  --output .rerail/profiles/sql-preparation/duplication.json
```

The report separates repeated construction within one batch from equivalent expressions over different file batches.
It estimates avoided construction CPU before maintenance and retained-access costs.
The existing report also includes `anchorPreparations` and `anchorStorage`.
These fields retain structural preparation CPU, reader counters, reserved pages, WAL bytes, and SQLite residency.

The small replay host compares temporary tables, retained postings, and retained shared node sets.
It uses the production SQLite functions and captured SQL.
Captures with equal file revisions and domains form one sequence, so the experiment measures reuse across actual plan batches.
Each sequence starts from its first snapshot and retains SQL state through its later queries.
The experiment checks exact candidate row parity for every query.
Its CPU totals cover candidate replay and cache construction, rather than complete application execution.

Build the [replay host](../scripts/profiling/preparation/README.md), then run each rotated trial through the bounded controller:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/sql-preparation/layout-trial-0 -- \
  scripts/profiling/preparation/target/release/rerail-sql-preparation-lab \
  .rerail/profiles/sql-preparation/diagnostic/sql/raw 0
```

Trials 1 and 2 rotate the layout order.
The experiment writes `retention-results-<trial>.json` beside its captures.
The host imports the production adapters, codec, storage settings, and CPU clock.
The layouts remain outside the engine execution path. They add no engine execution setting.
The native tests also invoke the same replay implementation.

The host also provides [composition kernels](../scripts/profiling/preparation/README.md#composition-kernels).
These compare tree intervals, selective demand, shared utility SQL, and bounded terminal columns.
The [design and decisions](sql-composition-opportunities.md) separate kernel evidence from production integration.

Preparation maintenance uses a fixed file-scope schema and bounded batched inserts.
SQL templates survive file changes, and structural coverage spans the complete requested plan frontier.
The public workspace API requires rule registration before evaluation.
Internal corpus tests cover later plan registration without adding public policy hot reload.

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

## Typed fact storage and input streaming

Relational AST producers use shared typed column batches. Capture text refers to source slices.
The SQLite authorizer records all columns read during validation, including nested predicates and wildcard reads.
The compiler removes unused projected columns without cloning the shared matcher tree.

The scheduler queues consumer descriptions. It restores each source when preparing an admitted session.
Completed projection batches enter SQLite through bounded INSERT statements. The input path retains only anchor metadata after insertion.
Existing cached cross-file evidence uses the same typed storage and borrowed SQL bindings.
JSON remains the public wire format and the representation for policies and JSON-semantic cross-file operations.

Use `--trace on` to record `input-stream` spans. The report exposes these spans as `native.inputStreams`.
Each record includes input rows, peak buffered rows, column batches, and retained projection rows.
The normal projection chunk contains at most 256 rows. SQL writes also enforce parameter and byte limits.
The small replay tests cover Unicode, NULL, duplicate rows, source release, and rollback after a failed batch.

Use `--memory-census` only for diagnostics. Queue census separates typed columns from JSON fields and shared source references.
Source references can overlap workspace storage. Do not add their byte counts to RSS.
Run timing controls with tracing, census, SQL capture, and external samplers off.
The journal records measured CPU and RSS independently from these storage counters.

External macOS samples record thread stacks over time.
The report separates known parked threads and marks read, write, and sync syscall frames.
I/O frames can execute or wait. Their percentages describe observed stacks, rather than CPU-time shares.
Use driver phase CPU for process totals and SQL thread CPU for operation rankings.

External `vmmap` summaries retain rounded physical footprints, malloc-region residency, swapped bytes, and allocator-zone totals.
Region and allocator totals overlap. They do not identify which Rust structures own allocations.
The profiler preserves the raw report beside the structured summary.

Selected native spans also record `memory.before` and `memory.after` when tracing is enabled.
These observations include the process RSS high-water mark and macOS allocator bytes in use and allocated.
The report preserves them in `native.memorySpans`. High-water marks are cumulative; do not add them across spans.
Allocator totals cover macOS system-malloc zones. They do not measure exclusive ownership by one rule or thread.
These totals do not cover allocations owned by a separate allocator, such as production jemalloc.
The suite driver also records the RSS high-water mark after each apply operation.
Use these boundaries to distinguish retained storage, temporary allocation, and allocator residency.

## Failure-only relation snapshots

Set `RERAIL_SQL_FAILURE_DIR` to a diagnostic directory before an engine run.
A failed relational stage or result query copies its main and temporary SQLite databases before owner cleanup.
Successful queries do no snapshot work and install no statement-tracing callback through this setting.
The JSON manifest records the rule, clause, failed query, database filenames, and capture limits.
Its budget record separates remaining SQL allowance from the outer clause's work limit, consumed polls, and observed exhaustion.
Failed-run CPU includes snapshot work. Such a run fails acceptance regardless of its timing.
SQLite backups do not serialize native virtual-table frames or external source callbacks.
A query that requires those producers also requires their original inputs for replay.
