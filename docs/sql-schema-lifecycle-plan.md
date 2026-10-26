# SQL schema lifecycle: implementation plan

Date: 2026-10-26.
Status: SC1–SC4 executor changes are implemented. Broad validation and corrected full resource acceptance are in progress.
The first full candidate failed acceptance; see the performance journal and rejected measurement receipt.

This plan expands Delivery 2 of the [cold CPU recovery plan](cold-cpu-recovery-plan.md).
The [spike results](cold-cpu-spike-results.md) and their receipts supply the evidence.
Candidate anchor selection and source publication remain separate deliveries in that recovery plan.

## Outcome and scope

The number of pending consumers must no longer determine the number of live SQL schemas.
SQL owners must reuse a bounded collection of family contexts.
Physical cuts must retain data only for an identified dependency.
Statistics work must follow the lifetime of those cuts.

The first production target is the complete 54-stage `quality-contracts-1` program.
The implementation must use generic SQL stages and existing producer contracts.
It must contain no branch that recognizes a data-layer rule ID.
After this target passes, the same executor covers state flow, authority sites, and legacy execution graphs.

The engine keeps SQL execution, recorded observations, relational clauses, and producer contracts.
Authored options, work allowances, source domains, and finding identities remain unchanged.
There are no production optimization flags or duplicate rule implementations.
No Linear ticket is part of this plan.

## Evidence and current behavior

The full diagnostic reaches 792 suspended consumers in four capacity partitions.
Its reported peak SQL admission size is 249,905,204 bytes.
The input allowance is 64 MiB. The SQL admission allowance is 256 MiB.

| Measured operation | Diagnostic self CPU, seconds |
| --- | ---: |
| Analysis calls | 679.109 |
| Relational materialization | 455.858 |
| Index work | 341.870 |
| Pool trimming | 190.128 |
| Consumer release | 132.883 |

These measurements include instrumentation and overlap other laboratory work.
They rank the work. They do not predict recoverable seconds in an uninstrumented run.
Span counts do not establish statement-submission counts.
Trimming and release include both Rust bookkeeping and SQLite operations.

The relevant implementation points are:

| Source | Current responsibility | Required change |
| --- | --- | --- |
| [batch.rs](../native/src/code_rule_dsl/relational/batch.rs) | Construct and advance SQL sessions before producer demand forms a complete cohort | Gather demand without a private SQL session per consumer |
| [relational.rs](../native/src/code_rule_dsl/relational.rs) | Load inputs, allocate aliases, construct stages, and read findings | Separate consumer state from reusable physical execution |
| [storage.rs](../native/src/code_rule_dsl/relational/storage.rs) | Retain tables, indexes, statistics, ownership, and pins | Own physical cuts and reusable contexts, with explicit lifetimes |
| [pool.rs](../native/src/code_rule_dsl/relational/pool.rs) | Allocate private names, configure leases, trim, and release | Reuse stable contexts without schema changes at each consumer transition |
| [database.rs](../native/src/code_rule_dsl/relational/database.rs) | Own SQLite and report resident storage | Report owner capacity at coarse boundaries |
| [compile/relational.rs](../native/src/code_rule_dsl/compile/relational.rs) | Check logical queries and construct the stage program | Preserve logical checks before physical grouping |
| [inference.rs](../native/src/code_rule_dsl/compile/relational/inference.rs) | Preserve SQLite output types | Supply contracts for fusion eligibility and retained cuts |
| [deferred.rs](../native/src/code_rule_dsl/relational/deferred.rs) | Expose deferred expressions through views | Preserve lazy expressions without a view for every consumer |
| [shared.rs](../native/src/code_rule_dsl/relational/shared.rs) | Compare complete producer definitions and source versions | Keep data identity separate from context reuse |

`create_stage` already reuses some declarations and indexes.
Private names and source-dependent physical identities still prevent broad reuse.
`Storage::analyze` repeats analysis for populated indexed stages.
SQLite then reloads statistics for the complete affected schema.

The metadata control shows the interaction between schema size and repeated analysis.
For 64 working consumers, analysis takes 17.209 ms with 128 retained tables.
It takes 102.277 ms with 1,024 retained tables.
The production diagnostic does not yet count the exact live schema objects at each admission boundary.

## Target execution

The executor has a fixed sequence:

```text
logical checks and physical planning
             ↓
consumer descriptors and exact demand
             ↓
whole-batch producer execution
             ↓
reusable SQL family context
             ↓
findings or the next producer barrier
             ↓
release data pins and reuse the context
```

The existing stage program determines the next barrier.
The coordinator needs ordered family steps and readiness states.
It does not need a generic graph scheduler.

### Separate SQL scope from producer batching

A producer receives the complete demand batch.
Its output uses shared typed columns and indexed scope slices.
The SQL reader consumes completed output. It does not launch native work for each consumer or node.

Opaque authored SQL retains its original evaluation scope.
Adding a consumer column does not automatically preserve aggregates, windows, DISTINCT, limits, or correlated subqueries.
`partitionBy: file` describes the evaluation domain. It does not prove that arbitrary SQL distributes over files.

Initially, a reusable family statement runs over each original scope and reads that scope's complete rowsets.
This removes schema construction without a general SQL partition rewrite.
There are no per-node or per-field SQL queries.

When their contracts define the partition key, compiler-generated operators can combine scopes.
Such a batch keeps every request identity and source domain.
It does not turn distinct candidate scopes into one shared producer result.

The target therefore distinguishes three counts:

- Native producer computations: one per admitted whole-demand batch or bounded output wave.
- SQL preparations: one per selected family template and valid schema context.
- SQL executions: whole-rowset family executions, with the original scope retained unless partition safety is established.

Reducing submissions alone is not an acceptance gate.
Opaque SQL batching is not a prerequisite for removing schema overhead.

### Proposed runtime objects

These names describe responsibilities. Implementation can reuse existing structures that already provide them.

| Object | Owned state | Lifetime |
| --- | --- | --- |
| `FamilyProgram` | Ordered SQL families, producer barriers, schemas, field reads, and cut reasons | Compiled logical program |
| `ConsumerState` | Source domain, versions, options, budget, next family, and result destination | One logical consumer |
| `DemandBatch` | Complete correlated producer requests and required columns | One producer wave |
| `FamilyContext` | Stable aliases, functions, views, backing slots, and prepared statements | Reusable SQL owner context |
| `ExecutionBinding` | Immutable references to the active scope, input slices, and budget | One statement execution |
| `CutHandle` | Definition, exact data identity, completion state, and dependency pins | Through its final dependent read |

`ConsumerState` must not own a private SQLite schema or one connection per file.
`FamilyContext` must not establish data equality from a reusable table name.
The existing producer registry remains authoritative for equal definitions and domains.

### Stable storage and execution bindings

Required SQL cuts use stable backing tables with an internal binding key.
That key selects one complete producer definition, source version, and candidate domain.
Indexes begin with that key before the reader's lookup columns.
Different bindings can occupy one physical table without sharing their logical results.

Stable views expose only the rows selected by an engine-owned execution binding.
The binding is a small relation that remains fixed during statement execution.
Its joins use indexed binding keys.
It must not require a scalar callback for every data row.
Authored SQL cannot access other bindings or engine metadata through this mechanism.

At a producer barrier, the consumer retains handles to its live cut rows.
It releases the SQL context while native work completes.
The next execution rebinds those handles to the same logical input names.
The compiler directs required boundary output into its final backing or typed frame.
It must not create a scratch copy and then copy the complete result again for suspension.

Purely local scratch data leaves the context before another scope uses it.
Shared cut data follows dependency pins independently from context reuse.
No table name alone establishes that cached data belongs to the active scope.

## Delivery SC1: separate consumer admission from schema ownership

**Scope:** the admission/session split, stable family contexts, and the complete quality program on that path.
Initially retain its logical stages and necessary physical storage semantics.
Do not combine this step with a blanket ANALYZE removal.

### Implementation

1. Extract source checks, input discovery, and producer demand from `Session::new` into consumer preparation.
2. Retain the native input gate before body preparation.
3. Represent suspension with a descriptor, budget, next step, and data handles.
4. Form producer batches from those descriptors before binding private SQL state.
5. Allocate stable physical names from the program and context layout.
6. Reuse declarations, indexes, and empty backing across successive scopes.
7. Install common functions once per compatible context.
8. Bind the active source and budget without reinstalling functions at every consumer transition.
9. Keep that binding immutable until the statement and its cursors close.
10. Return the context after its result or producer request is complete.

A statement-owned cursor must pin its input frame.
A context cannot switch scopes while that cursor remains active.
Native production remains a separate batch operation.
Repeated `xFilter` calls that expose completed slices must perform no producer computation.

Scope lookup must use actual ranges or reverse postings over shared columns.
It must not scan the complete frame behind an interface marked indexed.
Build that index once per frame and include its construction cost in measurements.

Use the current byte budgets for admission and cache retention.
Report active schema objects separately from pending descriptors.
Evict reusable contexts as complete units after their cursors close.

### Required observations

Carry these measurements with this delivery:

- Pending descriptors and active SQL contexts.
- Live tables, views, indexes, modules, and prepared statements.
- Declaration reuse, index reuse, and actual schema changes.
- Attempted analysis calls and actual ANALYZE executions.
- Metadata objects visited, data pins, eviction candidates, and actual drops.
- Resident SQL bytes and retained typed-frame bytes.

Collect object inventories at admission, producer completion, family completion, and owner closure.
Keep allocator enumeration outside the CPU run.
Keep host submissions separate from SQLite internal executions.

### TDD ladder

1. Prepare hundreds of suspended consumers without creating a schema for each consumer.
2. Run two scopes through one context and retain exact duplicate and NULL rows.
3. Close every cursor before rebinding the context.
4. Complete a large frame through indexed scope slices without scanning all rows for each scope.
5. Rebind source and budget while preserving prepared statements and source isolation.
6. Cancel one consumer without invalidating another consumer's frame.
7. Suspend two consumers at a producer barrier and preserve their distinct cut rows through context reuse.
8. Produce required boundary output once without a second full-row copy for suspension.
9. Run all original quality observations and complete repository parity.

**Completion:** pending-consumer growth no longer causes proportional schema growth.
The full quality program runs through reusable contexts with exact results.
Complete cold CPU improves without a peak-RSS regression.

## Delivery SC2: compile SQL families after logical checks

**Scope:** the generic physical family compiler and all eligible cuts in `quality-contracts-1`.
This delivery covers the complete clause, including its options and deferred fields.
A two-terminal-stage policy edit does not satisfy it.

### Implementation

1. Keep the existing checks for every authored query and option branch.
2. Preserve the 16-KiB authored-query limit.
3. Record dependencies, output contracts, field reads, source reads, and recursive properties in `FamilyProgram`.
4. Compute downstream column demand before producer construction.
5. Fold eligible single-use SQL intermediates into the consuming family.
6. Preserve shared cuts only for measured repeated reads in the same domain.
7. Keep explicit native readiness barriers and required recursive operator boundaries.
8. Respect the existing 1-MiB SQLite statement limit during composition.
9. Send terminal family results directly to finding consolidation.
10. Remove the corresponding declaration, population, index, analysis, and release operations.

Every retained SQL cut needs a recorded reason and its dependent readers.
Record unresolved semantic barriers explicitly in the cut inventory.
Do not silently classify every logical stage as a required physical cut.

Fusion must preserve SQLite affinity, values, and comparisons.
Even with equal column names, removing typed storage can change behavior.
Use the existing inference machinery to check candidate transformations.
If equivalent SQL semantics are not established, retain the typed boundary.
Do not substitute a blanket CAST rewrite or a native scalar callback per row.

Deferred expressions must remain lazy.
An unused source-text column must not trigger source reads during producer construction.
Named intermediate results must preserve their lexical SQL scope during composition.
The compiler must handle CTE shadowing and quoted identifiers through checked bindings.

### TDD ladder

1. Fuse a simple producer and its sole reader while preserving the complete row multiset.
2. Preserve typed comparisons for integers, text, blobs, NULLs, compounds, and windows.
3. Keep a deferred expression unevaluated until a reader demands its column.
4. Produce one shared result for equal definitions and domains, including complete empty results.
5. Separate overlapping but unequal domains.
6. Report conflicting producer definitions with both origins.
7. Preserve every authored option and unknown-option default.
8. Segment an internal statement at the existing limit without raising authoring allowances.
9. Run the 192 original quality observations, including 130 positive and 62 clean observations.
10. Run the broader corpus and both complete repository workloads.

**Completion:** each eligible stage loses its old physical lifecycle.
The cut inventory explains every remaining boundary.
Full resource measurements establish the aggregate effect.

## Delivery SC3: maintain statistics for the remaining physical cuts

**Scope:** statistics and indexes for the cuts left by SC2 and the unchanged published base relations.
The source kind/value indexes retain their current layout.

### Implementation

1. Classify remaining cuts by their exact reader access paths.
2. Retain indexes with measured repeated reads.
3. Use stored SQL statistics for discovery choices and native row estimates.
4. Maintain cut cardinalities from bulk publication results where those counts are already available.
5. Collect required cut statistics after a complete population batch.
6. Group statistics maintenance at the family or publication boundary.
7. Prepare readers after that boundary so they observe the intended statistics.
8. Remove automatic ANALYZE from stages that no longer exist or need no statistics refresh.

Equal row counts do not establish equal value distributions.
A previous `analyzed` row count is insufficient as a freshness policy.
A populated cut must not trigger a complete schema reload solely because it is populated.

During SC1, necessary legacy analyses can remain within the smaller context.
During SC3, each remaining refresh needs evidence that the reader plan benefits.
If a refresh reduces complete execution cost, retain that refresh.
This decision belongs to the physical plan, with no user configuration flag.

### TDD ladder

1. Read a populated cut twice without rebuilding its index or recollecting unchanged statistics.
2. Change source versions and preserve invalidation of data and statistics.
3. Change value distributions without assuming that equal cardinality means fresh statistics.
4. Exercise empty-to-nonempty and nonempty-to-empty transitions.
5. Check skewed joins and rare/common anchors against complete result rows.
6. Compare actual preparation, refresh, query, and release CPU together.

**Completion:** the remaining analysis work occurs at explicit population boundaries.
Removing analysis does not introduce a larger join cost or a correctness difference.

## Delivery SC4: bound data retention and cleanup

**Scope:** family contexts, shared cuts, typed frames, and their dependency pins.
Parser and source-graph retention remain visible but require their own ownership evidence before changes.

### Implementation

1. Keep schema identity separate from source-dependent data identity.
2. Pin cut data through its final dependent reader.
3. Record complete empty output as successful completion.
4. Clear private rows in reusable backing without changing its schema.
5. Retire shared rows after their final pin or source-version invalidation.
6. Perform capacity eviction at explicit owner boundaries.
7. Preserve prepared statements while their complete context remains valid.
8. When a context leaves the cache, remove its definitions, indexes, views, and modules together.

Pool trimming must distinguish cache lookup, registry traversal, row deletion, and SQLite DDL.
SC1 supplies those counters.
If repeated owner scans remain material, add owner postings and maintained byte totals.
Charge index maintenance and metadata retention to that comparison.
The current mixed trimming span does not justify a separate owner-index project by itself.

### TDD ladder

1. Complete repeated reads with one construction and one final retirement.
2. Preserve a pinned old source version during a later edit.
3. Roll back a failed producer without marking its output complete.
4. Retry without stale rows, names, or statistics.
5. Complete repeated edits with bounded live contexts and metadata.
6. Close the workspace and release its frames, cuts, cursors, and dependencies.

**Completion:** retained objects follow live dependencies and cache capacity, rather than historical consumer count.
CPU and ownership measurements include cleanup.
Current RSS, peak RSS, live allocation, and physical footprint remain separate observations.

## Budget, failure, and cancellation contract

SQL executes under the original consumer budget during the initial context-reuse delivery.
Batch producer work retains the existing per-consumer accounting.
No allowance increases and no per-node scalar budget callbacks are part of this design.

For a partition-safe SQL batch, one possible cap is the minimum remaining consumer allowance.
That cap is conservative and requires a measured retry bound before use.
If a batch exhausts its cap, discard its uncommitted results before subdividing complete consumer domains.
Each retry retains the original limits.
Do not split the files inside a repository-wide consumer.

The first delivery does not require this SQL retry mechanism.
It preserves opaque SQL scope and budget behavior through isolated family executions.
This avoids coupling schema reuse to a new generic failure scheduler.

Cancellation must close cursors before context rebinding or reuse.
Failed or cancelled work must not publish a complete cut.
Tests must place a failing consumer beside successful consumers and compare both findings and failures.

## Delivery order and review units

| Delivery | Dependency | Reviewable result |
| --- | --- | --- |
| SC1 | Existing spike evidence | Complete quality clause on reusable contexts, with admission and lifecycle counters |
| SC2 | SC1 | Generic SQL family compiler and a complete cut inventory for the quality clause |
| SC3 | SC2 | Evidence-based indexes and refresh boundaries for every remaining cut |
| SC4 | SC1 counters, SC2 lifetimes | Bounded retained state and measured cleanup |

Each delivery can contain several TDD commits.
The implementation delivery must cover its complete scope before performance acceptance.
An empty abstraction, a reduced submission count, or one faster helper is insufficient.

After SC2 and SC3 pass, apply the same path to `state-flow-1`, `authority-sites`, and `legacy-execution-graphs`.
Do not fork their rule logic or introduce a general SQL partition rewriter.
Keep the existing batched recursive SQL and whole-demand producer contracts.

## Resource gate and journal protocol

Before the first production delivery, freeze a matching uninstrumented control artifact and policies.
Record the final import-membership correction in that identity.
The earlier frozen references precede that correction.

For each complete delivery:

1. Run the focused semantic and lifecycle ladder.
2. Run the affected original positive and clean observations.
3. Run broader native, shape, static, and profiling checks.
4. Run all 50 ported rules over all 9,021 files.
5. Run all 60 ported rules over the same complete repository.
6. Keep tracing, SQL capture, allocator census, and external sampling off during those timings.
7. Record compile CPU separately from cold CPU, wall time, and OS peak RSS.
8. Preserve native, policy, workload, driver, and source identities.
9. Record complete expected and actual findings and failures.
10. Journal the result and retained cut inventory before the next delivery.

The complete repository runs can finish under the existing authorization.
Normal diagnostic commands retain the two-minute bound.
Acceptance runs must not overlap other work from this worktree.

The previous passing references are 672.782 cold CPU seconds for 50 rules and 2,540.634 seconds for 60 rules.
They remain qualified by their frozen identities.
The first-50 recovery target remains 125.804 CPU seconds.
There is no equivalent clean pre-SQL all-60 reference in the evidence.

Each delivery must reduce complete cold CPU without a peak-RSS regression against its matching control.
The final recovery gate remains broader than this schema plan.
Candidate selection, publication, and source ownership still contribute to the complete result.
Statement counts explain execution. They do not substitute for these resource and parity gates.


## Implemented shape and acceptance status

Stable SQL layouts and exact data bindings now have separate identities.
Consumer descriptors retain data pins, options, budgets, and source domains.
Ordered family frontiers refresh published cuts before dependent readers prepare.
Whole native demand batches retain the existing count, scan, and scatter boundaries.
The executor adds no generic graph scheduler or SQL partition rewrite.

Imported input views use an outer one-row binding gate.
Their backing index covers `(binding,__path,__start,__end)`.
Indexed stage cuts preserve their original binding predicate and authored lookup columns.
This distinction follows the input and stage contracts, with no rule-specific dispatch or configuration flag.

SQL programs share immutable queries and schemas across compatible layouts.
The family compiler fuses eligible cuts and records reasons for retained cuts.
The default quality clause retains 39 physical stages from 54 logical stages, with 15 fused stages.
Deferred fields, authored options, typed comparisons, and original work allowances remain covered by parity tests.

Statistics refresh only dirty physical relations at coherent family boundaries.
Unchanged cardinality does not suppress distribution invalidation.
Targeted ANALYZE retains SQLite's metadata reload; it avoids repeated analysis of unchanged data within the bounded schema.
Completed data and idle layouts have bounded caches.
Final readers release exact binding pins, and cleanup groups ready-row deletions by backing relation.

The first full candidate matched findings but failed resource and error acceptance.
The journal records that rejected result and the subsequent input-planning correction.
Full corrected 50- and 60-rule runs must pass before this plan is accepted.
The broader cold CPU recovery target includes candidate selection and source ownership, which this delivery does not claim to solve.
