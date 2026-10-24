# Chunk C: complete shared execution and the cold SQL budget

Status: specification. The replacement schedule and CPU improvement remain unproven.
The accepted baseline is `24998a39`, after chunks A and B.
The [count ledger](../fixtures/host/data-layer-port/profiling/sql-execution-chunk-c-20261024.json) records the frozen evidence and this delivery contract.

## Required result

**C must remove at least 2,150 actual engine submissions: 2,250 → at most 100.**
The target covers the unchanged complete guard, including cold compilation, both observations, both evaluation paths, and cleanup.
No remainder passes to an unspecified later chunk.
Smaller TDD commits support this delivery. They do not complete it.

C must also compute shared derived data once for compatible consumers.
Reuse of an empty table, a connection, or a query string does not establish this property.
CPU must improve on representative nonempty work, with no matched guard or memory regression.
An execution-count reduction alone does not complete C.

## What A and B leave

| Remaining engine work | Measured submissions | Required replacement |
| --- | ---: | --- |
| Connection and compiler setup | 244 | One cold infrastructure lifecycle and SQLite-owned schema inference |
| File publication | 36 | Bulk publication of required document versions, without duplicate imports |
| Catalog publication | 12 | One grammar catalog publication per backing store |
| Candidate preparation | 702 | Typed operation batches over shared owners and indexed facts |
| Relational evaluation | 1,091 | Shared derived producers, fused private pipelines, and batched emission |
| Cleanup | 165 | Complete owner lifetimes and bulk retirement |
| **Total** | **2,250** | **At most 100, including every replacement statement** |

These categories are disjoint. Opportunities inside them overlap and must not receive separate additive credit.
There are also 181 SQLite internal executions. They remain separately reported.

The trace exposes four repeated AST-store lifecycles and 68 compiler empty-schema probes.
Candidate storage still executes 380 population statements and 174 retirement DELETEs.
Relational execution still creates 489 objects, inserts into stages 310 times, and executes 39 ANALYZE statements.
It also executes 68 BEGIN/COMMIT pairs and 74 result or helper SELECTs.
These counts explain why empty-backing reuse does not complete shared execution.

## Target execution shape

The engine compiles one evaluation plan for the admitted rule set.
The plan spans candidate selection, structural demand, derived SQL, and result consumers.
It retains distinct logical evaluations, snapshots, rule budgets, and authorization scopes.
Transport packets and clause sessions do not define ownership or semantic completion.

The existing specialized scheduler becomes a controller over compiled operation descriptors and dependency arrays.
It advances affected ready operations, rather than creating a task for each node or rule instance.
Recursive inference remains inside declared SQL or existing native operations.
A generic graph scheduler and a native rule interpreter are unnecessary.

```mermaid
flowchart LR
    A[Eligible rules and pinned inputs] --> B[Intern producer bindings]
    B --> C[Compile dependencies and owner lifetimes]
    C --> D[Collect ready demand]
    D --> E[Batch SQL or structural operations]
    E --> F[Publish rows and completion]
    F --> D
    F --> G[Read shared owners and emit findings]
    G --> H[Retire after final demand and reader]
```

The existing [stage design](sql-execution-stages.md) and [producer contracts](shared-producer-contracts.md) supply the semantic model.
C delivers their missing runtime boundary, rather than another execution path beside the current one.

### Shared data and completion

A producer definition contains its versioned name, implementation, input slots, typed options, schema, multiplicity, ordering, and completion model.
Its binding contains the input bindings, options, generation, and semantic domain.
Complete definition comparisons establish equality. Digests only accelerate lookup and identify reports.
Conflicting definitions under one public name fail compilation with both origins and the differing fields.

The engine defines generic producers beside their implementations and registers them through `producers/`.
The pack defines reviewed derived producers under `packs/data-layer/candidates/producers/`.
Rules reference those definitions. Private stages keep rule-qualified identities.
The engine must not contain data-layer detector names or hardcoded linter algorithms.

Each binding has one row owner, coverage state, and required indexes within its pinned domain.
A keyed owner computes only missing requested coverage. Successful empty output completes its requested keys.
A domain owner publishes once after its complete input domain closes.
Later demand can extend keyed coverage without rebuilding covered keys.
Failure publishes neither partial rows nor successful completion.

Compatible consumers read the same populated owner.
They do not import its facts into separate clause tables or rebuild its indexes.
Two private stages with equal names remain separate unless their complete bindings establish shared work.
Different options, domains, source versions, or effective callback contexts prevent sharing.

### Physical storage and scope

A job-scoped SQL service owns the backing store and publication writer.
It serves compatible evaluation plans while each logical workspace retains separate membership and revision state.
The fixed guard keeps its current observations, API calls, and results. A harness rewrite cannot establish savings.
Compiled policies and workspaces retain handles to the same runtime service, with explicit logical evaluation leases.
Job closure releases rows and callbacks. An unbounded process-global row cache is not the storage model.
Cold service creation and final closure remain inside the captured job.

The four current stores cannot become one mutable snapshot merely because their schemas match.
Document versions remain immutable and file-qualified.
A membership relation selects the versions visible to each logical evaluation.
Same-path edits and simultaneous workspaces must retain independent answers.
Shared content cannot establish shared rule eligibility or result completion.

The positional FTS index retains the `ast-index-spike` representation and SQL access paths.
Version and membership metadata qualify the existing document identities.
C does not replace this index with a permanently resident Rust index.
SQLite stores typed producer rows and indexes, with spill under the existing resource policy.
JSON fields and generic property bags do not replace typed relation columns.

The first connection implementation uses one publication writer and the minimum required scoped readers.
Native operations still receive broad batches. SQL connection count does not determine native partition width.
Successful publications define reader barriers. Active cursors and BLOB handles close before incompatible maintenance.

Authored SQL first resolves against its isolated logical schema and current function allowlist.
Trusted lowering maps validated relation references to authorized owner bindings.
The mapping must preserve CTE shadowing and SQLite name resolution. Arbitrary identifier replacement is insufficient.
Authored SQL cannot access another binding through a physical name, generated alias, or forged CTE accessor label.
The existing protected-view exception does not provide this isolation.
Failure of this scope test blocks storage integration before any performance claim.

### Batches, pipelines, and materialization

Typed operations and dependencies enter the plan before SQL rendering.
Operations of the same family share requests qualified by binding, domain, and consumer.
The renderer does not construct complete SQL trees and concatenate them through UNION ALL afterward.
The rejected selector-packing implementation remains rejected.

Initial operation families cover kind and value selection, keyed property access, child-field access, and interval joins.
Request rows supply typed scalar options and owner references to existing SQL indexes.
The shared key relation retains `(binding, document_version, pre)` qualification.
Relations with bag semantics also retain occurrence multiplicity. A uniqueness constraint cannot silently turn them into sets.
Publication reads typed input cursors in bulk rather than issuing a statement for each file or row.
Input cursors expose immutable columns. They do not execute rule predicates or contain JSON row vectors.

Simple private transformations stay inside one SQL pipeline while their semantics permit fusion.
Shared producers and measured access paths receive explicit owners.
Dependency barriers, aggregates, windows, recursion, and required source predicates remain explicit.
Materialization occurs once for the complete binding or admitted coverage, not once for each consumer statement.
Repeated inline materialized CTEs cannot substitute for a shared owner across statements.

Pack extraction must identify concrete repeated derived computations from the trace and real rule definitions.
The acceptance example uses an actual nonempty derived relation with two independent consumers.
The repeated `nodes`, `functions`, `ownership`, and descendant-stage queries are discovery candidates, not automatic equivalence proofs.
The discovery report must explain every selected shared binding and every required separation.

Rule-bound findings share result queries where scopes, callbacks, and failure boundaries permit it.
Output tags retain rule, clause, file, witness, multiplicity, and existing ordering semantics.
Result packets cannot repeat producer computation.
An error in one consumer cannot silently remove another consumer's findings or failure attribution.

### Lifetimes, budgets, and GPU shape

Demand sources retain leases until they cannot request further coverage.
Conditional consumers request nothing before their gate enables them.
Skipped branches close their leases without universal preparation.
Retirement waits for demand-source closure, completed consumers, readers, and admitted continuations.

Compatible retirement actions share SQL maintenance statements at proven boundaries.
Retention until job completion is not the default solution to excessive DELETEs.
Spill or admission delay preserves a live owner's completed coverage without rebuilding it.
Owner rows, indexes, readers, statements, WAL pages, tapes, request buffers, and outputs all enter memory accounting.

Shared physical work retains explicit accounting for each logical rule budget.
Cache hits and consumer order must not change whether the same logical evaluation exceeds its allowance.
Source-text and regex callbacks retain the correct snapshot, options, cancellation state, and work counters.
The plan must define this attribution before it combines consumers in one statement.

Structural work retains broad count, scan, and scatter passes over typed requests.
Virtual tables read prepared data and indexes. Their callbacks cannot submit SQL, dispatch producers, or wait for device work.
Real dependencies create later waves. File boundaries, output packets, and SQL insertion pages do not create native waves.
The [GPU execution contract](gpu-producer-execution-contract.md) remains authoritative.

## Cold compilation and the complete budget

C includes the 68 remaining compiler probes. A warm process cache cannot hide their cold cost.
SQLite must retain authority over names, expression affinity, collation, and statement validation.
The deferred-CAST regression from B remains a required test.

Compilation must also avoid repeated lowering and SQL rendering for an identical producer definition.
Profiles must separate policy compilation, operator lowering, pattern compilation, SQLite preparation, SQL execution, and publication CPU.
An immutable compiled program can share its complete definition while each consumer retains separate mutable work counters.
Pattern reuse requires matching syntax, options, implementation revision, and limits. A textual match alone is insufficient.
The plan audit must identify repeated compiler work before another cache receives credit for a CPU improvement.

The compiler must prove a prepare-only path through SQLite's own resolver for schemas that need inference.
If the public API cannot expose the required result, the design needs an explicit SQLite integration decision.
A custom Rust type-inference parser is not an acceptable fallback.
Production EXPLAIN executions also count. Diagnostic exclusion cannot hide production schema inference.
Any remaining executed probes and their cleanup enter the same 100-statement budget.

The compiler must emit a complete cold statement schedule before promotion.
That schedule prices infrastructure, inferred schemas, owner schemas, indexes, catalog publication, and document publication.
It also prices requests, coverage maintenance, producer statements, statistics, transactions, result queries, and retirement.
Every statement inside `execute_batch` counts separately.

Component budgets must come from this executable schedule, not guessed replacement counts.
The current trace proves the 2,250 baseline. It does not prove a particular replacement schedule fits 100.
The feasibility audit is the first implementation step and cannot claim execution savings by itself.
If the complete schedule exceeds 100, the design still needs correction. The target stays fixed.
No setup moves before capture, and no per-connection or per-observation counter resets occur.
New non-SQL API work also appears in CPU, memory, and operation counters.

## Implementation order and TDD checkpoints

The following checkpoints form one delivery. They are not independent performance milestones.

| Order | Concrete work | Required red test and completion evidence |
| --- | --- | --- |
| C1 | Freeze B and export the complete proposed schedule | The unchanged guard fails at 2,250/100. The export prices every replacement statement and identifies unresolved costs. |
| C2 | Job storage, version membership, and scoped owner access | Two workspaces use the same path with different contents. Answers remain isolated, with one compatible infrastructure lifecycle and no cross-binding access. |
| C3 | Integrate named derived bindings and row completion | Two actual rules and two result packets read one nonempty derived owner. Coverage counters prove one successful build, including empty completion and later overlapping demand. |
| C4 | Compile operation-family batches and private pipelines | Permuted consumers preserve results, bound admission, and physical build counts. The plan removes per-clause imports and repeated inline owner builds. |
| C5 | Complete compilation, result, and lifetime integration | Cold CAST affinity, source callbacks, budgets, rollback, cancellation, retries, edits, and bulk retirement retain their contracts. All costs appear in the schedule. |
| C6 | Complete performance and repository acceptance | The unchanged guard removes at least 2,150 submissions. Ten-rule and repository jobs also pass the same complete-job 100 limit. |

The C2 scope test also covers `count(*)`, nested CTEs, collations, NULL values, and bag multiplicity.
The C3 test covers distinct options and private names that must not share data.
The C4 test varies files, roots, rules, and result packets independently, with sufficient capacity and unchanged dependencies.
More rows can require more storage traffic. They cannot create one engine statement or native dispatch for each item.

The compiler and executor need direct ownership of dependency information.
SQL-token scans cannot provide the authoritative graph for C.
The existing candidate scheduler, clause suspension, producer registry, and storage-contract host provide implementation components.
Each component moves under the shared plan as its replacement passes tests.
No optional production execution flag preserves the old algorithm.

| Implementation boundary | Existing starting point | Required change |
| --- | --- | --- |
| Bindings and dependencies | `native/src/code_rule_dsl/producers/` | Dependent typed bindings, domain completion, consumer provenance, and shared owner IDs |
| Candidate plan | `native/src/code_rule_dsl/matcher/anchors/sql/lower/` | Explicit operation descriptors and request batches before rendering |
| SQL backing and publication | `native/src/code_rule_dsl/matcher/anchors/sql/store.rs` and `store/` | Version membership, shared runtime ownership, bulk publication, and reader barriers |
| Clause compilation | `native/src/code_rule_dsl/compile/relational.rs` | Resolved owner references, shared derived dependencies, and SQLite-owned schema metadata |
| Clause execution | `native/src/code_rule_dsl/relational/batch.rs` and `relational.rs` | Shared plan advancement, scoped consumers, budget attribution, and owner lifetime management |
| Pack definitions | `packs/data-layer/candidates/producers/` | Validated producer declarations and reviewed references from actual rules |
| Instrumentation and gates | `benchmark/sql-profile/` and `scripts/profiling/` | Complete schedule cost, actual build receipts, owner diagnostics, and full C acceptance |

## Evidence and acceptance

The profiler must report each binding's definition, inputs, generation, domain, consumers, requested keys, and completed coverage.
It must also report physical builds, rows, indexes, bytes, statements, reader leases, and retirement reasons.
Every submitted statement needs a job, phase, operation, and owner or explicit infrastructure identity.
Discovery and clash reports must distinguish valid separate bindings from conflicting public definitions.

Acceptance requires all of the following:

- The unchanged complete guard submits at most 100 engine statements, with complete coverage and at least 2,150 actual removals.
- The representative ten-rule job and complete repository job each obey the same 100-statement boundary.
- The complete available positive/clean corpus and workspace transitions retain findings and failure parity.
- The nonempty sharing tests show zero duplicate successful builds of the same binding coverage.
- Compatible clause consumers perform zero duplicate fact imports or owner index builds.
- Matched CPU controls show a repeatable improvement beyond control variation on representative nonempty work, with no guard regression.
- Matched peak memory and retained ownership do not regress. Low-memory tests preserve coverage or report explicit capacity failure.
- External bottom-up profiles and ownership measurements explain the remaining CPU and memory costs.

CPU controls use fresh processes, identical inputs, matched builds, allocator, workers, and no tracing or census.
A second rotated control series must reproduce the CPU improvement.
Detailed SQL, allocation, and external sampling runs remain separate from those controls.
The journal retains raw trials, source and binary hashes, rejected experiments, and the full remaining inventory.

Small commands retain the 115-second deadline. Existing authorization permits the optimized build and complete repository run to finish.
The broad run starts only after the bounded correctness, execution-shape, and CPU gates pass.
No Linear tickets follow from this specification without prior user approval.
