# Shared producer implementation workplan

Status: implementation plan. Runtime work has not started.
The [contract design](shared-producer-contracts.md), [execution stages](sql-execution-stages.md), and [ownership invariant](sql-materialization-shape.md) define the target.
WP1 will use this worktree after the [aggregate checkpoint](data-layer-performance-journal.md#aggregate-optimization-checkpoint-2026-10-23).
That checkpoint records a 19.93-fold first-50 cold CPU regression and failed full-repository 60-rule execution.
Source-corpus parity passes, but it does not replace the failed repository gate.

The first useful result is shared span preparation across actual rule consumers and result packets.
That result precedes a complete controller replacement or pack-library migration.
Each work item ends with a reviewable commit, correctness evidence, and a journal entry.
This document creates no Linear issues.

## Existing foundation

The discovery CLI and declaration checker already work, with 25 focused tests and CI coverage.
They identify proposed sharing and conflicting names, but the compiler does not export actual contract bindings yet.
The engine retains decoded properties, spans, and children in SQL.
The new work must eliminate repeated demand computation and intermediate construction around those retained facts.
Adding names to already shared rows alone does not establish a performance improvement.

The current candidate scheduler handles explicit table dependencies and local lifetimes.
The clause executor batches native demands, but clause databases still import compatible facts separately.
These existing components form the implementation base.

## Work items and dependencies

| Item | Deliverable | Depends on | Completion evidence |
| --- | --- | --- | --- |
| WP1 | Engine contracts, binding registry, and compiler export | Existing tools | Real compiler exports pass declaration checks and explain sharing or separation |
| WP2 | First shared span producer through actual execution | WP1 | Two rules and two packets reuse one owner, without repeated covered work |
| WP3 | Remaining structural contracts and demand preparation | WP2 | Properties and child lookups share correctly, including empty results |
| WP4 | Compiled frontiers, domain completion, and complete lifetimes | WP3 | Dynamic demand and conditional consumers preserve exactly-once publication |
| WP5 | Workspace SQL owners across clause connections | WP4 | Compatible clauses read shared backing without fact imports or repeated indexes |
| WP6 | Pack producer declarations and compiler resolution | WP1, then WP4 for integration | Validated pack references bind once, while private stages remain independent |
| WP7 | Reviewed derived-producer extraction | WP5, WP6 | Proven repeated work in real rules uses one implementation and physical owner |
| WP8 | Complete parity, profiling, and repository comparison | WP7 | Correct findings, measured execution counts, CPU, and memory with reproducible evidence |

The default implementation order is WP1 through WP8.
WP6 declaration work can proceed after WP1, but it cannot claim runtime completion before WP4 integration.
WP1 also includes an early connection-contract check for WP5, before the design depends on an unproven storage assumption.

## WP1: establish names and observable bindings

The engine registry lives under `native/src/code_rule_dsl/producers/`.
Contracts remain beside the SQL or Rust implementations they describe.
The first contracts cover node properties, source spans, and child fields in the existing decoder.

The registry accepts trusted engine definitions, compares complete declarations, and assigns compact contract and binding IDs.
Bindings include explicit input references, typed options, and relevant semantic domains.
The evaluation pins input generations. Existing workspace invalidation remains authoritative across edits.
This step introduces no general cache across evaluations.

The compiler preserves rule, clause, source location, and demand-source provenance before SQL rendering.
It exports actual `rerail.shared-work-plan/v1` manifests through diagnostic tooling.
Private stages retain rule-qualified names and never enter the public namespace implicitly.
The Python checker remains a diagnostic consumer, while the Rust registry owns runtime acceptance.

The acceptance ladder covers equal bindings, different options, incompatible schemas, different inputs, namespaces, and simulated hash collisions.
A conflicting public name fails compilation with both origins and the exact differences.
Different valid bindings under one contract name remain separate without a clash.
The exported manifest must pass the existing checker without manually supplied declarations.

An early storage check exercises one workspace writer and two scoped readers with the existing SQLite configuration and authorizer model.
It covers publication barriers, private scratch survival, reader closure, and forbidden backing-table access.
This check establishes the connection contract. It does not add a second production execution path.
Any incompatible assumption requires a design correction before WP5.

## WP2: deliver the first shared execution path

The initial vertical slice uses source spans because repeated demanded-span preparation already appears in captured workloads.
The compiler binds span requests from different consumers to one named owner.
Demand selectors remain separate computations unless their own definitions share safely.
Already materialized selectors contribute keys directly, without another rendering of their dependency graphs.
Per-rule file eligibility and conditional gates restrict the demand union before producer execution.
The checkpoint finds a language-wide plan/file frontier despite narrower final matcher eligibility.
A complete compiler inventory must not become an eager Cartesian execution frontier.
Batched request rows preserve eligible plan/file pairs without introducing a host query for each pair.

The executor unions ready span demand and computes missing coverage through batched SQL.
Completion records distinguish absent output from unfinished work.
Rows and completion publish atomically, while admitted coverage prevents overlapping attempts.
Existing SQL indexes and retained span backing remain the starting point.

The renderer replaces selected shared dependencies with owner references.
It rejects another embedded materialized copy of that owner in a consumer statement.
The required demanded-key fence remains before expensive source predicates.
Cheap single-use projections continue as pipelines.

This slice extends the existing action scheduler only as required for its explicit dependencies and consumer lifetimes.
It does not wait for a general controller rewrite.
The owner remains live across both result packets and every known later consumer in this slice.

The acceptance ladder starts with a two-rule, two-packet diamond and overlapping requested keys.
It then covers empty results, cancellation, retry, different file generations, and different scopes.
Actual ported-rule workloads must exercise the same path before completion.
An evaluation counter must prove that covered keys and shared selectors avoid repeated construction.
A disjoint-scope regression also proves that an eligible rule does not trigger another rule's universal preparation on unrelated files.
Disabled conditional consumers contribute no demand until their gate enables them.
Counters check these domains while query and kernel launches remain batched.
The first paired CPU and allocation comparison occurs here, rather than after WP7.

## WP3: extend structural sharing and simplify empty work

Node-property and child-field producers adopt the same contract and binding mechanism.
Different child fields remain different bindings under one public contract.
An absent child still completes its requested key.
No implementation infers completion merely from the presence of output rows.

Property fills use shared ready selectors and SQL coverage checks instead of repeated demand subqueries.
Independent consumers contribute known demand before capacity admission partitions their sessions.
Future demand waits for its actual dependencies.

Literal empty relations simplify without a dedicated owner or table.
This change preserves set and bag behavior, including duplicate elimination where SQL semantics require it.
Existing kind, value, and vocabulary indexes remain direct access paths.
This step does not restore the rejected expanded-posting or generic row caches.

Acceptance covers property-fill and result-consumer sharing, absent children, NULL values, and source-predicate fences.
File-batch changes must preserve bindings, findings, and successful build counts for unchanged required coverage.
Profiles must separate saved decoding from saved selector construction and reduced inline materialization.
They must also separate those gains from changes in SQLite cache capacity.
The aggregate checkpoint observes pager pressure and statement-journal writes with 1 MiB main and 512 KiB temporary cache targets.
Controlled replay must record cache hits, misses, writes, spills, CPU, and memory before attributing improvements to shared producers.
Cache capacity belongs to physical resource admission; it does not change a producer's semantic binding identity.
Publication experiments must preserve rollback guarantees while testing whether coalesced writes reduce journal traffic.

## WP4: compile the complete frontier controller

The compiler replaces SQL-token dependency inference with explicit producer, consumer, ordering, and storage edges.
It compiles the graph into indexed operation descriptors and adjacency arrays.
The runtime handles Discover, Build, Evaluate, and Emit operations through affected frontiers.
It does not allocate one scheduling task per node, key, or rule instance.

Keyed producers advance when their requested coverage completes.
Domain producers publish only after their exact input domain closes.
Recursive algorithms remain inside declared operations, rather than creating cycles in the outer scheduler.

Each potential consumer retains a demand-source lease until it cannot request further coverage.
Skipped branches close their leases without requesting data.
Retirement also waits for readers and admitted continuations.
Unconditional regions retain compiled last-use actions where possible.

Memory admission includes retained owners, private algorithm allocation, readers, and output buffering.
The controller spills backing storage or defers independent work without rebuilding a live owner.
Algorithm partitions require a semantic justification. Arbitrary file splits cannot change a global domain.

Acceptance covers later overlapping demand, conditional branches, sealed aggregates, maintenance hazards, and low-memory admission.
Output order, duplicate behavior, error attribution, and existing logical rule budgets remain unchanged.
Native operations receive broad typed frontiers suitable for count, scan, and scatter passes.
Profiling records frontier width, barriers, and operation counts to expose future GPU launch shape.

## WP5: share SQL backing across clauses

Reusable rows and owner indexes move into the workspace SQL backing where necessary.
One writer owns publication. Scoped clause readers consume registered bindings without importing the rows.
Private scratch and rule namespaces remain isolated.
The WP1 connection check expands into production integration tests.

Publication barriers close incompatible readers and BLOB handles before writes or schema retirement.
Eligible consumers then acquire the published snapshot under pinned logical generations.
Reader connections, page caches, prepared statements, scratch, and WAL retention participate in admission.

Each owner maintains the access paths required by its consumers once.
Two clauses cannot create identical fact copies or rebuild the same owner index.
Incompatible callbacks or effective execution context prevent sharing explicitly.
Physical work accounting records one build and its consumers, without silently changing logical budget semantics.

Acceptance uses two real clauses that currently import the same compatible facts.
Counters must show shared reads, zero duplicate imports, and one index construction for the owner.
Tests also cover allowlists, cancellation, edits, reader retirement, and failure attribution.
Candidate-only sharing does not complete this milestone.

## WP6: define pack libraries and resolve references

Candidate declarations live under `packs/data-layer/candidates/producers/`.
They describe SQL or existing engine operations in the pack namespace.
The loader validates declarations and references before the compiler creates bindings.
Shipped pack metadata must explicitly include producer artifacts before publication support becomes complete.

The compiler rejects conflicting definitions, missing dependencies, undeclared options, and dependency cycles.
Rule-local stage names stay private unless the rule explicitly references a shared contract.
Diagnostic exports retain pack source locations and every consuming rule and clause.

Acceptance covers two rules importing one declaration and two private stages with the same local name.
It also covers different valid options, version mismatches, namespace conflicts, and unavailable producer artifacts.
The contract checker and runtime registry must agree on accepted compiler exports.

## WP7: extract proven repeated derived work

The discovery report ranks candidates for semantic review, without automatically registering them.
Compiler provenance must identify actual consuming rules or clauses before extraction.
The first extraction requires two real consumers with matching computation, inputs, and completion semantics.

Repeated range and frontier selectors are initial review targets from the existing captures.
Generic operations remain engine templates. Data-layer-specific relations remain pack definitions.
Cheap indexed reads and constant expressions do not become retained caches merely because they repeat.

Each extraction records its old sites, new contract, semantic inputs, consumers, and completion unit.
It also records required columns, access paths, private workspace, and retirement conditions.
The change removes the previous duplicated implementation and uses one production path.

Acceptance compares exact intermediate rows where practical and all final findings, including duplicate behavior.
Profiles must show fewer physical builds and include storage, index, maintenance, and read costs.
A CPU regression beyond observed trial variation requires investigation and correction before promotion.
Name reuse counts alone do not justify an extraction.

## WP8: establish the complete result

The final gate replays the complete relevant positive and clean corpus through snapshot and workspace execution.
It includes edit transitions, cancellation, retry, and the existing native and public integration checks.
Typechecking, formatting, Clippy, boundaries, and structure checks remain required.

Repository measurements use the exact ported-rule selection from the chosen historical run.
The evidence records rule IDs rather than inferring the selection from the current pack size.
Source revision, file membership, policy digests, worker count, allocator, build settings, and binary hashes remain explicit.

The full repository evaluation retains repository-wide semantics and uses the earlier permission to finish with regular updates.
Short experiments remain bounded at 115 seconds.
Global analyses cannot use independent file shards merely to satisfy that deadline.
Build exceptions remain separate from workload timing.

The journal reports CPU, RSS, SQLite allocation peaks, materialized bytes, build counts, duplicate attempts, and output parity.
It separates compilation, execution, and diagnostic overhead.
The historical roughly 1,400-second observation cannot establish a CPU ratio unless its CPU evidence supports that comparison.

## Measurement gates

WP1 will start in this worktree after the [aggregate checkpoint](data-layer-performance-journal.md#aggregate-optimization-checkpoint-2026-10-23).
That checkpoint supersedes the small controls below as the broad execution reference.
Its final evidence records the addon, sources, complete corpus coverage, and repository CPU measurements.
WP1 remains pending during checkpoint validation.

The earlier installed baseline is addon `86846fe45e731c10b44beecb54cbfb35b7610c2eafb71f199ce3840969c07b6d`.
The [lifetime evidence](../fixtures/host/data-layer-port/profiling/sql-relation-lifetimes-20261023.json) retains its sources and trials.

| Existing snapshot | Median cold CPU | Median peak RSS | Diagnostic peak SQLite execution allocation |
| --- | ---: | ---: | ---: |
| State, one file | 2.597 s | 452.5 MiB | 68.5 MiB |
| Ten rules, two files | 13.106 s | 553.3 MiB | 98.6 MiB |

SQLite allocation peaks come from separate instrumented replays. They are not process RSS or extra memory to add to RSS.
The [discovery counts](../fixtures/host/data-layer-port/profiling/shared-producer-discovery-20261023.json) describe static placements, without runtime evaluation counts.

WP2, WP3, WP4, WP5, and each WP7 extraction require a matched small-workload comparison.
Three rotated observations per side form the initial timing set.
High variance or unexplained regressions justify additional observations.
Tracing, SQL capture, and allocation accounting remain off during CPU timing.
Separate diagnostic runs explain construction counts and memory peaks.

Every comparison includes planning, demand construction, writes, indexes, publication, reads, and cleanup.
Moving cost into another phase does not satisfy the performance gate.
The evidence must identify a reduced source of repeated work, even when host timing variation obscures a small CPU change.

## Completion boundary

The effort completes when named contracts drive actual shared execution across rules, packets, and compatible clauses.
The tools must explain every shared binding and conflicting name back to its source consumers.
Completion and lifetime counters must prove correct publication and retirement under edits, cancellation, and constrained memory.
Final evidence must establish parity and explain the measured CPU and memory changes.
Metadata registration or a successful compiler export alone does not complete the work.
