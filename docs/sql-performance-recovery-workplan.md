# SQL performance recovery workplan

Status: R0 is implemented and the real acceptance job fails at 24,159/100 executions.
Runtime recovery steps R1–R7 remain pending.
All work uses the current worktree. WP1 remains pending.
The [aggregate journal](data-layer-performance-journal.md#aggregate-optimization-checkpoint-2026-10-23) preserves the failing starting point.

## Required outcome

One complete cold job executes at most **100 SQL statements**, across all workers and connections.
This limit comes from `jeremy/sql-capture-demand-spikes`, commit `c109a915`.
The profiler import omitted that gate and its execution-coverage checks.
The producer-shape tests counted selected native passes. They did not enforce the SQL execution limit.

Each execution counts, including cached statements, publication, schema setup, indexes, transactions, matching, and cleanup.
An `execute_batch` call counts every SQL statement inside it.
A file, rule, input batch, result packet, connection, or worker never resets the allowance.
Diagnostic EXPLAIN and enclosing native timing spans do not consume the allowance.
The gate runs after complete execution. It never truncates findings at statement 101.

Counts alone cannot establish recovery.
Acceptance also requires complete findings, no execution failures, bounded memory, and CPU without a regression against matched references.
One large statement that repeats the same expensive work does not satisfy the CPU requirement.

## Measured starting point

| Metric | Earlier reference | Current checkpoint |
| --- | ---: | ---: |
| First-50 cold CPU | 125.804 s | 2,507.496 s |
| First-50 ingestion CPU | 73.350 s | 2,456.015 s |
| First-50 global apply CPU | 52.144 s | 51.145 s |
| First-50 reported peak RSS | 6.441 GiB | 6.143 GiB |
| All-60 cold CPU | 709.151 s, failed execution | 7,211.657 s, failed execution |
| All-60 candidate materialization/fill actions | Not comparable | 1,359,072 |

The current first-50 run passes repository parity.
The current all-60 run matches 24,993 findings but reports 34 state-tripwire relational failures.
The earlier all-60 reference also fails. Its timing cannot establish a clean performance ratio.
Candidate counters omit some publication, index, transaction, and cleanup statements. They are lower bounds on complete execution counts.

The implementation introduces three concrete sources of excess work:

- `workspace/evaluate.rs::prepare_candidates` collects every rule's plans before the later file filters.
- `anchors/sql/plans.rs` collects both branches of conditional queries before their runtime gates.
- SQL lowering repeats selectors and intermediate materializations across statements and input batches.

Index publication also executes SQL in loops over files, kinds, vocabulary terms, and overflow records.
Those calls participate in the same limit.
Persistent indexes and decoded rows remain useful, but their presence does not establish shared selector execution.

The restored gate records one positive and one clean observation through snapshot and workspace paths.
Their complete replay passes parity but executes 24,159 SQL statements on 136 connections.
The trace contains 11,544 CREATEs and 5,508 DROPs.
Execution 101 creates `syntax_21` during compilation schema validation.
This small case proves that session setup and relation lifetimes also need structural changes.

## R0: restore the acceptance boundary

Deliverables:

- Restore the fixed `JOB_SQL_EXECUTION_LIMIT = 100` acceptance gate.
- Record started, completed, and running statements from connection creation.
- Record connection-open and connection-close receipts outside individual trace files.
- Reject missing entire sessions, partial streams, mixed jobs, incomplete operations, and failed parity.
- Preserve the first statement that exceeds the allowance, plus setup, transaction, and matching totals.
- Make ordinary SQL capture and report commands return nonzero when acceptance fails.
- Add a mandatory CI workload with original positive and clean state-tripwire observations.

The initial CI workload is a small regression guard. It does not establish whole-repository acceptance.
Its findings must complete before the gate reports the excess executions.
The current engine remains an expected acceptance failure until recovery removes the excess work.
No skipped test, increased allowance, or optional production path converts that failure into a pass.

TDD ladder:

1. Accept 100 executions and reject 101.
2. Reject 101 executions distributed across workers and connections.
3. Count repeated cached executions and each statement inside `execute_batch`.
4. Reject low counts with missing traces, missing session receipts, or failed findings.
5. Preserve evidence and return nonzero from the ordinary report command.
6. Run the actual CI workload and record its count, parity, coverage, and first excess statement.

## R1: reconcile the complete execution inventory

This step precedes another optimization or full repository run.
Use one positive and one clean case, then the existing state and ten-rule selections.
Separate instrumentation runs from CPU controls.

Classify every execution as connection setup, catalog publication, file publication, candidate preparation, relational evaluation, or cleanup.
Attribute each execution to its producer, binding, source generation, file domain, and consumer when those identities exist.
Record the counts of input batches, rules, plans, and selected roots beside these totals.
Count actual executions rather than SQL strings, API calls, or static placements.

TDD ladder:

1. Reconcile trace totals against native started/completed counters and session receipts.
2. Preserve the total when one prepared statement runs repeatedly.
3. Preserve the total when identical work moves between connections or worker threads.
4. Add files and rules separately to expose each scaling dimension.
5. Confirm that an unrelated file does not increase the requested file's VM work.

Completion evidence contains an exact statement inventory and explicit unknown attribution.
It also identifies every source loop that cannot meet the complete-job allowance.
The inventory must reconcile before an improvement claim.

## R2: restore eligibility before candidate preparation

Retain the existing path, language, source-size, syntax, text, and rule-capability decisions.
Compute eligible `(plan, file, generation)` pairs before expensive SQL preparation.
Use the same admission logic for preparation and final evaluation.
Gather ready pairs in typed batch inputs. Do not issue one query per pair.
The primary sites are `workspace/evaluate.rs`, `anchors/sql/plans.rs`, and `anchors/sql/corpus.rs`.

Conditional consumers contribute demand only after their gate enables them.
The compiler can retain the complete plan inventory without executing every plan for every file.
Source publication must retain files required by cross-file analysis, even when local rules do not consume them.

TDD ladder:

1. Preserve findings for one admitted rule and file.
2. Add an excluded rule and prove that candidate execution does not increase.
3. Add excluded files and prove that the admitted rule's preparation does not increase.
4. Disable a conditional consumer and prove that it contributes no candidate demand.
5. Enable that consumer and preserve positive, clean, and cross-file results.
6. Preserve failure propagation, cancellation, edits, and source-generation isolation.

Compare the same compiled policy set and input corpus before and after the change.
Report removed plan/file execution and measured CPU separately.
Zero selected roots alone do not prove that preceding work was unnecessary.

## R3: batch publication across the cold job

The complete-job limit permits no SQL statement for each of 288 ingestion requests.
This constraint requires a real job boundary before SQL producer execution.
Use the [execution stages](sql-execution-stages.md) to separate source admission, publication, preparation, and consumption.
Preserve observable results and failure behavior for existing workspace requests.
Any required API boundary change needs an explicit compatibility test.

Publish language catalogs, file records, kind postings, value postings, and overflow data through typed batch relations.
Use SQL `INSERT ... SELECT` over those relations where it preserves the existing index layout.
Keep the ast-index-spike FTS layout and disk-backed SQL ownership.
Avoid JSON row payloads and unbounded native copies of decoded SQL facts.

Admit relational sessions only when their consumers can run.
Remove repeated schema construction on independent clause connections.
Include compiler validation connections in this change. Prepare-only validation must retain its authorizer and schema checks.
The primary sites are `compile/relational.rs`, `relational/database.rs`, and `relational/syntax.rs`.
Prove whether schema-only relation adapters can retain SQLite validation without repeated executed DDL.
Reject that approach if names, affinity, authorizer decisions, or validation failures change.
Use shared backing with consumer-qualified rows and bounded private scratch where semantics require it.
The 136-connection guard must demonstrate a smaller actual setup count, without omitted validation or weakened isolation.

TDD ladder:

1. Publish one file with the existing index contents and lookup results.
2. Publish many files with the same fixed statement families.
3. Preserve exact values, duplicate postings, missing values, overflow records, edits, and removals.
4. Preserve transaction rollback after interrupted publication.
5. Increase input batches without increasing the complete-job execution allowance.
6. Measure temporary bytes, retained bytes, peak memory, and publication CPU together.

Completion requires actual statement counts under the intended workload, including internal and helper SQL that the tracer observes.
If the required index layout conflicts with the limit, record the exact conflict before changing either requirement.
Do not redefine the counter to conceal the conflict.

## R4: establish shared producers and remove repeated construction

This step implements WP1 and the first useful execution slice from WP2.
Use the [named contracts](shared-producer-contracts.md) and the existing discovery and clash tools.
Names identify declared work. Complete bindings include inputs, options, generations, schema, and semantic domains.
Private stages retain rule-qualified identities.

Start with repeated source-span demand and selector work demonstrated in the captures.
Union ready eligible keys before construction.
Compute missing coverage once, including empty output, and publish rows with completion atomically.
Consumers read one physical owner across statements, clauses, and result packets.
They do not embed another materialized copy of the owner's definition.

TDD ladder:

1. Execute a two-rule, two-packet diamond over overlapping demand.
2. Prove one construction for shared selectors and admitted keys.
3. Preserve empty results, retries, cancellation, and different source generations.
4. Diagnose a public-name clash with both origins and the differing fields.
5. Exercise the same path with actual ported rules.
6. Repeat the captured producer with 53 host placements and prove one physical owner for compatible demand.

Keep the specialized dependency phases and current scheduler where they suffice.
A new generic graph scheduler is not a prerequisite.
Extend the same mechanism to properties and child fields only after the first slice passes.

## R5: bound SQL execution and materialization together

Lower compatible ready consumers into broad SQL operator families over binding-qualified inputs.
Group result emission without losing rule, clause, file, or witness identity.
Do not implement a new native rule interpreter.

Materialize a shared producer exactly once for its admitted execution domain.
Keep cheap single-use transformations inside their consumer.
Remove repeated temporary-table creation, index construction, and deletion around the same work.
Preserve type affinity, NULL behavior, multiplicity, recursive termination, and demanded-key access paths.

TDD ladder:

1. Preserve complete row multisets for each changed operator family.
2. Count setup, publication, preparation, matching, and cleanup in one cold job.
3. Increase files, roots, rules, and output packets independently without another call for each item.
4. Keep genuine dependency waves explicit and bounded.
5. Reject SQL that meets the count limit but increases VM work, CPU, or peak memory.

Completion requires at most 100 actual executions on the state and ten-rule workloads.
Static operator counts and three native producer passes are supporting diagnostics, not substitutes for this gate.

## R6: remove execution failures and attribute memory

Reduce the 34 full-repository state-tripwire failures to bounded reproductions.
The failures contain 12 regex-budget errors and 22 SQL interruptions.
Identify the work that consumes the shared allowance before changing its limit.
Preserve the existing allowance and complete diagnostic multisets during query-shape corrections.

Add ownership estimates for retained tape, scope, and preparation storage that the current census omits.
Reconcile ownership estimates with external allocator, footprint, and residency observations without adding overlapping totals.
Prove release after workspace closure and after repeated edits.
Do not infer a leak from RSS or allocation counts alone.

Completion requires no engine failures in the positive/clean corpus or the all-60 repository workload.
The unchanged and edit checks must also contain no retained failures.

## R7: accept the aggregate recovery

Run each small acceptance workload in a fresh process and capture directory.
Require complete parity, complete connection coverage, and at most 100 SQL executions.
Run matched CPU controls without tracing, census, SQL capture, or external sampling.
Use identical inputs, build settings, allocator, workers, and ingestion requests within each comparison.

The acceptance matrix covers one file, unrelated retained files, larger input batches, state, ten rules, first 50, and all 60.
The final requirement applies to the complete 9,021-file cold job, not each ingestion batch.
Run repository measurements only after the smaller gates pass.
Use the existing approval for full repository runs and addon builds. Other commands retain the 115-second deadline.

Require all 3,210 native corpus observations, all 40 edit transitions, and the appropriate native suite.
Resolve or explicitly retain the public performance-guard failure and six incomplete pack checks.
No full-suite pass follows from smaller passing groups.

Compare aggregate CPU and memory against fresh matched references.
The historical first-50 target is 125.804 CPU seconds, with the recorded memory-method qualifications.
The failed earlier all-60 run remains diagnostic evidence rather than a clean speed reference.
Record every experiment, rejected approach, source identity, and acceptance result in the journal.

## Completion order

R0 → R1 → R2 → R3 → R4 → R5 → R6 → R7.
Each step ends with a reviewable change, regression evidence, execution counts, CPU, memory, and a journal entry.
WP1 starts within R4 in this same worktree.
This plan creates no Linear tickets and adds no optional production execution path.
