# Implementation spikes for incremental SQL preparation

Status: implemented experiments, with decisions recorded below.
These spikes implement and measure the [preparation architecture](sql-preparation-architecture.md) in dependency order.
Each spike ends with a keep, revise, or remove decision supported by evidence.

## Results: 2026-10-22

The [checked evidence](../fixtures/host/data-layer-port/profiling/sql-preparation-20261022.json) contains individual trials, source identities, and scope limits.
The [journal](data-layer-performance-journal.md) explains the results and remaining costs.

| Spike | Result and decision |
| --- | --- |
| S0 | Keep canonical generated-template identities, logical construction reports, checked workload selections, and candidate replay captures. |
| S1 | Keep unioned universal structural demand, streaming typed columns, fixed batched request tables, and read-only candidate SELECTs. Sparse child and span demands remain sparse. |
| S2 | Reject expanded posting retention: replay CPU increases 31% on the positive control and 25% across the three-rule plan batches. |
| S3 | Keep incremental predicate/term matches and deletion cascades. Bound regex compilation to the current predicate rather than each term. |
| S4 | Keep duplicate-expression removal inside a batch. Reject generic retention across batches: the natural three-rule replay increases CPU 16%, despite 38 reused constructions. |
| S5 | Keep invalidation and churn regressions. Do not add a production relation catalog, eviction, or retirement for the rejected caches. |

Cold process CPU decreases 22% on the positive control and 6% on quality.
These results miss the proposed 30% and 10% targets.
The experiments therefore do not establish the complete proposed architecture as a performance solution.
There is one engine path. The replay host compares storage layouts outside that path.
The requirements below remain the original experiment specification, including unpromoted proposals.

## Experiment rules

Each experiment uses a source change and a separately identified native binary.
The engine has one execution path and no runtime experiment switches.
An isolated benchmark can compare physical layouts outside the engine dependency graph.
Accepted changes become the normal path. Rejected changes are removed.

The two current controls are the twelve-file positive recording and `quality-five.json`.
The [current evidence](../fixtures/host/data-layer-port/profiling/value-access-20261021.json) pins their workloads, policies, binaries, build settings, and trial outputs.
The positive recording currently resides at `.rerail/profiles/batch-candidates-20261021/positive-two-recording.json`.
Spike S0 makes its reproduction independent of that local directory.

Every command uses the existing profiling controller with a deadline of at most 115 seconds.
A timeout is an incomplete experiment, not a usable timing result.
Builds and individual test groups use separate bounded commands.
The initial experiments do not require another full-repository run.

## Shared measurement contract

Run at least three fresh serial trials for each binary and workload.
Alternate binary order and retain the exact workload, policy, allocator, worker, and ingestion settings.
If trial variation exceeds 5%, add matched trials and report the variation.
Timing runs disable detailed tracing, SQL capture, census, and external sampling.
Separate diagnostic runs collect attribution and external bottom-up samples.

Report these totals for every comparison:

- Process user and system CPU, by compile, ingest, preparation, selection, matching, unchanged, and edit phases.
- Cold evaluation CPU and compile-plus-cold CPU, without excluding new maintenance work.
- Peak RSS and explicit cache ownership estimates, with host pressure recorded.
- Database, WAL, and temporary-file peak bytes, plus live retained rows and pages.
- Prepared rows, reused rows, removed rows, dependency frontiers, and batch chunks.
- Data statements, DDL statements, index builds, VM steps, sorts, and BLOB reads.
- Complete findings, multiplicity, failures, paths, and candidate conservativeness.

Statement counts distinguish temporary CTAS, retained inserts, completion markers, index DDL, and final SELECTs.
Metrics attribute a retained relation to all its consumers without counting its CPU twice.
Instrumentation records relation and dependency identities instead of generated SQL aliases alone.
The profiler adds observation only.

Each proposal estimates net savings from avoided construction, decoding, and indexing, minus additional maintenance and retained-access CPU.
The estimate uses measured duplication and reuse counts from S0.
The report compares that estimate with the actual complete-pipeline change.
Cold construction, new-plan preparation, and edited-file maintenance require separate estimates.
The combined target is not evidence that all current materialization cost can disappear.

New counters require focused regression coverage in the existing profiler tests.
Candidate row equality remains a gate for changes that preserve the existing anchor strategy.
If an experiment changes a conservative anchor strategy, report candidate differences and prove that no actual match disappears.

## S0: Establish repeated work and a reproducible baseline

Hypothesis: repeated intermediate construction crosses actual rule or preparation batch boundaries.
Existing CPU totals prove a cost, but they do not quantify this specific duplication.

Implementation:

1. Add canonical graph identities and dependency classifications without changing generated SQL.
2. Record each logical relation's consumers, constructions, rows, bytes, and file domain.
3. Record physical lifetimes separately from logical identities.
4. Capture a checked selection for the positive recording from the pinned source snapshot.
5. Add an internal corpus sequence that registers new plans after an earlier preparation.
6. Preserve the current native binary and source fingerprint before subsequent changes.

The natural workloads must identify real cross-batch overlap before its savings enter an estimate.
A focused fixture also places one shared utility behind more than 32 distinct plan wrappers.
That fixture checks the batching boundary without treating duplicated synthetic plans as evidence of production speed.
New plan objects with equivalent definitions must retain equal logical identities.
The public workspace requires rule registration before evaluation.
These experiments add no public policy hot reload.

TDD ladder:

1. Check equal definitions with different aliases, parameter positions, and Arc addresses.
2. Check unequal regex flags, grammar identities, field directions, domains, and utility bodies.
3. Force a definition-hash collision and check exact-definition comparison.
4. Check approximate relations under negation and projected duplicate rows.
5. Check natural workload diagnostic parity with unchanged execution.

Exit criteria: complete baseline and a ranked list of repeated logical constructions with their domains and exclusive CPU.
Separate measured work from the estimated removable fraction.
If cross-batch duplication is small, prioritize preparation cost rather than assume retained intermediates solve it.

## S1: Separate preparation and batch decoded demands

Depends on S0.
Hypothesis: one preparation graph can remove repeated demand checks and source copies even when final results remain unchanged.

Implementation:

1. Collect structural, span, and child demands across all requested plans before the 32-plan split.
2. Compute missing-key unions in batched SQL frontiers.
3. Apply universal structural coverage only where a live demand already requires it.
4. Populate existing decoded tables and completion markers before their consumers execute.
5. Move final selection into an explicitly read-only phase.
6. Preserve physical dependency boundaries that prevent CTE expansion.

TDD ladder:

1. Check sparse structural demand, overlapping subsets, and universal coverage separately.
2. Check absent field children as cached NULL results.
3. Check cancellation between row insertion and completion publication, then retry.
4. Reject decoding calls and database writes during candidate selection with test instrumentation.
5. Check INTEGER affinity and both ordinal range constraints in the existing interval regression.
6. Replay positive and clean source cases, then run both timing controls.

Expected evidence: fewer property-fill statements and source copies, with unchanged decoded-node demand or a documented reduction.
A broad predecode that increases demanded nodes fails the sparse-demand contract.
Moving equal work outside the selection timer does not satisfy the CPU criterion.

## S2: Retain shared posting access across batches

Depends on S1.
Hypothesis: repeated posting expansion and indexing cost more than one retained typed relation and its maintenance.

Implementation:

1. Add file revisions, the relation catalog, retained node sets, and empty completion markers.
2. Admit the highest-cost repeated kind or value access paths identified by S0.
3. Populate missing relation/file pairs through batched SQL joins.
4. Create shared indexes once during schema initialization.
5. Join retained sets to request files without copying those sets into another indexed table.
6. Request only missing file/plan pairs when a partially prepared group receives another plan.

The physical comparison includes direct FTS/value access, current temporary expansion, and retained ordinal sets.
Use distinct binaries or an isolated layout benchmark for this comparison.
The accepted engine retains only the winning layout for each proven access shape.
Literal seeks do not require vocabulary match materialization.

TDD ladder:

1. Check one consumer, two consumers, and consumers across the 32-plan boundary.
2. Check a previously complete group after one new plan arrives.
3. Check an empty set twice and require zero reconstruction on the second preparation.
4. Delete a file, reuse its integer ID, and reject stale revision rows.
5. Change file filters and check reuse without cross-file leakage.
6. Replay natural controls and record total writes and CPU.

Expected evidence: each admitted relation/file/revision constructs once, with no per-batch index build.
The natural workload report must show reduced total CPU or justify rejection of posting retention.
An expanded index that adds more storage and cold CPU without measurable reuse is removed.

## S3: Maintain regex vocabulary matches incrementally

Depends on S1 and the catalog from S2. Posting retention is not required.
Hypothesis: repeated regex evaluation against unchanged vocabulary can disappear across rule batches and new-plan preparation.

Implementation:

1. Add predicate match rows and completion epochs by language.
2. Capture vocabulary additions and deletions during publication with batched SQL maintenance.
3. Evaluate existing predicates against new terms and new predicates against the current vocabulary.
4. Remove deleted terms and publish the completed epoch atomically.
5. Preserve direct literal seeks, coverage checks, and residue confirmation.

TDD ladder:

1. Check equivalent predicates across independently compiled rules.
2. Check a new predicate with no matches and repeat its preparation.
3. Add one term, remove its last occurrence, and reintroduce it.
4. Delete one occurrence while another file retains the term.
5. Cross the residue threshold and check unchanged-file candidate validity.
6. Cancel maintenance, retry, and check epoch and finding parity.

Expected evidence: regex calls follow new predicate/term pairs rather than repeated occurrence or batch counts.
Report initial vocabulary CPU and edit-maintenance CPU separately.
If the controls contain little repeated vocabulary work, report that limit instead of claiming a broad gain.

## S4: Retain proven shared utility relations

Depends on S2. S3 can proceed independently after its prerequisites.
Hypothesis: shared file-local utilities avoid repeated set operations and temporary indexing across rules.

Implementation:

1. Select one expensive repeated utility from S0's natural workload report.
2. Compile its typed graph into a retained node relation.
3. Reuse its unchanged child relations after a utility or rule definition changes.
4. Keep pattern choices, scoped negation, and unclassified operations request-scoped.
5. Apply consumer-based admission across the complete request graph.

TDD ladder:

1. Check context-free and context-sensitive invocations of the same utility.
2. Check `EXCEPT`, nested negation, NULL, and duplicate projections against the existing SQL output.
3. Change the utility body and check invalidation of its dependents only.
4. Change the caller's scope and reject reuse with an incompatible universe.
5. Repeat the natural cold and new-plan workloads, including all retained writes.

Expected evidence: fewer utility constructions and sorts, with unchanged diagnostics and valid candidate supersets.
If no expensive shared file-local utility exists in the controls, add a pinned natural selection from the ported rules.
Do not hardcode a rule name or generated stage name into engine admission.

## S5: Bound maintenance and assess the combined design

Depends on the accepted parts of S1 through S4.
Hypothesis: retained relations improve complete execution without unbounded storage or expensive edit invalidation.

Implementation:

1. Add batched retirement for definitions that no active plan references.
2. Measure deletion plans and add maintenance indexes only where necessary.
3. Establish fixed retention limits from measured row sizes and workload growth.
4. Exercise scratch preparation when a relation exceeds the retention limit.
5. Record database freelist reuse and WAL checkpoints under repeated edits.
6. Run the combined implementation through the complete acceptance matrix.

Unused pages can remain allocated in the database file.
The requirement is bounded reuse after churn, not file shrinkage after every deletion.
Routine edits must not require VACUUM.
WAL size and checkpoint CPU form part of the maintenance report.

The acceptance matrix includes cold runs, unchanged runs, new equivalent plans, new distinct rules, file edits, deletion, filters, and grammar changes.
It also includes failed preparation, cancellation, retry, empty results, and retention-limit exhaustion.
A churn sequence performs at least 100 edits and periodic predicate replacement in one retained SQL store.
Split sequence construction into bounded commands only if the complete state remains in the same live workspace.

Acceptance targets:

- At least 30% lower positive-control cold CPU than 2.504 seconds, approximately 1.753 seconds.
- At least 10% lower quality-control cold CPU than 3.192 seconds, approximately 2.872 seconds.
- No repeat preparation on an unchanged request.
- No more than 5% CPU regression on another matched control without an explicit reviewed tradeoff.
- No more than 5% peak RSS regression, with retained SQL bytes and allocator evidence reported separately.
- Bounded live rows and file growth after churn, with no stale revisions or unreachable definitions.
- No per-file or per-rule host query loop and no candidate-selection writes.
- Exact diagnostic parity, with zero missed true matches and unchanged failure behavior.

The targets are experiment gates, not measured improvements.
If the combined result misses a gate, record the shortfall and the remaining cost distribution.
Do not report a microbenchmark improvement as completion of the full optimization.

## Evidence and promotion

Each spike writes raw artifacts beneath `.rerail/profiles/sql-preparation/`.
A checked-in evidence file records source and native hashes, commands, individual trials, medians, counters, parity, and limitations.
The report separates natural workload results from synthetic correctness and scaling fixtures.
The [performance journal](data-layer-performance-journal.md) records each completed decision and links its evidence.

Focused tests precede each implementation change.
Accepted changes then receive the native DSL/workspace suites, public API suites, and recorded positive and clean corpus replay.
Repository typechecking, structure checks, formatting, and Clippy remain promotion gates.
External bottom-up samples compare the current baseline with the accepted combination after tracing is disabled.

S0 through S5 have explicit results above.
Future retention work needs new workload evidence before promotion.
