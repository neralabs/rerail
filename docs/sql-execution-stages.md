# Compiled execution stages

Status: target architecture, before implementation.
This design implements the execution model behind [materialization ownership](sql-materialization-shape.md).
The selected [named producer contracts](shared-producer-contracts.md) simplify producer identity and define discovery and clash diagnostics.
The [implementation workplan](shared-producer-workplan.md) sequences runtime integration and its correctness and performance gates.
It does not change the current engine or claim new benchmark improvements.

## Decision

The compiler builds a general dependency graph and specializes it into a stage plan for each scheduled rule set.
The runtime executes that plan with a small frontier controller.
It does not need a general task scheduler, arbitrary task callbacks, work stealing, or a task per row.

Specialization follows operation families, schemas, coverage rules, and dependencies.
It never follows data-layer rule names or the current fifty-rule workload.
SQLite retains joins, filters, aggregation, windows, and relational recursion.
Native producers retain their declared algorithms and batched interfaces.

A fixed sequence of preparation, query, and output phases is insufficient.
Later candidate results can discover new property demand or activate another clause.
The compiled topology stays fixed, but demand and branch outcomes determine the executed frontiers.

| Concern | Planning decision | Runtime decision |
| --- | --- | --- |
| Dependencies | Producer graph, read edges, ordering edges, branch gates | Which gates resolve and which frontiers become ready |
| Sharing | Named contract bindings, compatible context, one materialization owner | Missing coverage within that owner |
| Execution | SQL templates or native operation families, schemas, required indexes | Batch sizes, input keys, capacity admission |
| Lifetime | Potential consumers and their demand sources | Completion, source closure, reader leases, retirement |
| Memory | Storage placement, partition legality, estimated workspace | Spill, admission delay, controlled capacity failure |
| Output | Rule bindings, ordering, set or bag semantics | Actual rows and diagnostics |

## Current components and the missing boundary

The candidate [lifetime scheduler](../native/src/code_rule_dsl/matcher/anchors/sql/lower/lifetime.rs) already produces a cached action list.
It infers dependencies from generated SQL and controls explicit tables within that list.
It cannot own repeated inline materializations or see every later clause consumer.

The clause [batch executor](../native/src/code_rule_dsl/relational/batch.rs) already gathers native demands into capacity-bounded waves.
However, each session advances through SQL stages until the next native demand.
Shared SQL work remains inside those separate sessions.

The target replaces these separate scheduling boundaries with one evaluation plan.
Existing SQL execution, native batching, cancellation, and output adapters remain implementation components.
The new controller coordinates their dependencies and ownership.

## Planning representation

The planning graph distinguishes producers, consumers, and demands.
A producer describes a computation. A consumer binds its output to another producer or rule.
A demand describes the required keys, complete partitions, or sealed domain.
Their identities remain separate from physical storage and transport packets.

Each producer declares:

- A versioned contract reference, typed options, and explicit input bindings.
- The contract's schema, multiplicity, ordering, and completion rule.
- Explicit input producers, semantic scope, context compatibility, and generation dependencies.
- Coverage form and the input-completion condition for that form.
- Representation: indexed access, pipeline, or owned materialization.
- Backend operation, required access paths, private workspace estimate, and legal capacity partitions.
- Potential consumers, conditional gates, and sources of future demand.

The compiler interns equal named bindings before it partitions SQL statements or output packets.
Full binding equality resolves hash collisions.
Contract definitions live beside their implementations. One registry references those definitions across rule consumers.
Authored SQL remains an opaque stage unless declared dependencies and context establish safe sharing.
The compiler does not infer equivalence from arbitrary SQL algebra or keyword searches.

Data dependencies determine when values exist.
Ordering edges also preserve mutable maintenance, callbacks, and required write ordering.
Storage references independently determine when rows remain live.
An ordering edge alone does not keep an unused table alive.

Recursion stays inside one declared algorithm stage.
The outer dependency graph is acyclic after recursive components become single producers.
Undeclared cycles fail planning rather than causing an endless demand loop.

## Compiled stage plan

The compiler groups maximal legal pipelines and emits a compact plan with indexed descriptors and adjacency arrays.
The plan retains producer identity, dependency ranges, consumer ranges, SQL bindings, and backend descriptors.
Runtime state uses indexed counters, bitsets, cursors, and frontier buffers.
There is no heap-allocated scheduling object for each row or requested key.

| Stage operation | Contract |
| --- | --- |
| Discover | Run a batched selector and add demand to declared downstream producers |
| Build | Compute missing coverage through a SQL template or a declared native operation |
| Evaluate | Run a ready relational pipeline against published inputs |
| Emit | Stream rule-bound results into the existing output adapter |

Publication and retirement are controller transitions around these operations.
Index construction belongs to the owning Build operation.
The plan permits operation fusion where no sharing, semantic fence, or memory boundary requires separate publication.
One stage can contain several private SQL statements or several native passes.

These operations describe execution contracts, not separate algorithms for each linter.
New rules compose existing operations through their compiled dependencies.
A new native algorithm requires a declared backend operation, rather than a custom scheduler.

## Frontier execution

The controller advances only affected successors and newly resolved gates.
It does not rescan every producer after each completed stage.
Stable plan order breaks scheduling ties and preserves reproducible diagnostics.

Each frontier uses this sequence:

1. Resolve completed gates and collect demand from every currently ready, compatible consumer.
2. Union demand by producer version and coverage form, without reconstructing existing selector results.
3. Subtract completed and already reserved coverage through batched SQL operations.
4. Admit legal batches using retained bytes, reader costs, and private workspace estimates.
5. Execute compatible SQL builds or broad native passes, then publish successful coverage.
6. Advance eligible consumers and close finished demand sources.
7. Retire owners with no remaining demand source, consumer, reader, or internal continuation.

Independent consumers can contribute demand before they acquire a clause session.
Session admission therefore cannot hide known demand behind an earlier capacity partition.
Unknown future demand waits for its actual dependencies.
The controller never delays a producer until all future consumers become ready.
Such a barrier can deadlock a consumer that discovers more demand for that producer.

The plan records whether each stage accepts complete key subsets or needs a sealed input domain.
For keyed producers, a consumer can run when its required coverage completes.
An aggregate or domain-dependent window waits for the declared domain to close.
A transport batch cannot close that domain or change its identity.

Native batches combine compatible operations over ready keys across files and rules.
SQL batches use demand relations and set operations, rather than repeated parameterized point queries.
Different schemas or semantics remain separate operations even within the same frontier.
Batching does not require one giant SQL statement for unrelated algorithms.

## Coverage, publication, and retirement

Runtime coverage has separate pending, reserved, and completed sets for each producer version.
The workspace writer owns reservations, which prevent duplicate admission before publication.
Rows, completion records, and required indexes become visible at one successful publication boundary.
An empty result still completes its input key.

Keyed coverage uses stable input identity and generation.
A complete independent partition can form one key.
Domain coverage includes membership and relevant input versions, then publishes only after sealing.
These are the two completion models in the named contract design.

Cancellation rolls back unpublished rows and releases their reservations.
Completed coverage remains reusable while the evaluation continues.
Retry can execute an unpublished attempt again. It cannot repeat a successful publication for unchanged coverage.

Every potential consumer holds a demand-source lease before execution starts.
A source closes only after it cannot request another key for that producer version.
A skipped branch closes its leases without creating demand.
Finished readers release their snapshot leases separately.

Retirement requires closed demand sources, completed consumers, no readers, and no admitted continuation that needs the rows.
The compiler emits static last-use actions for regions with unconditional consumers and fixed demand.
Dynamic regions use the same ownership rule through lease counters.
Capacity cohorts and packet boundaries cannot retire an owner with later planned consumers.

This model avoids one runtime object per potential file-and-rule pair.
The plan stores consumer groups and gates compactly.
SQL coverage relations record actual demanded keys and empty completion.

## SQL ownership and connections

Reusable SQL rows reside in one file-backed workspace database with one writer.
Clause connections keep their private namespace and scratch state.
Engine-created, read-only bindings expose the shared database through scoped views without row imports.
User SQL retains its existing allowlist and cannot attach databases or access the internal catalog.

Each publication wave ends active shared readers before incompatible writes or schema changes.
The writer commits rows and coverage, then eligible readers acquire the published snapshot.
Logical source generations remain pinned for the evaluation.
Reader transactions and incremental BLOB readers close before the next incompatible barrier.

Private stage state can survive across barriers without an open shared-reader transaction.
Connection count, page caches, private scratch, WAL retention, and prepared statements count toward admission.
The shared backing does not imply an unbounded connection pool.

Pure, compatible work can share one physical build while preserving existing per-consumer logical accounting.
Different callbacks, cancellation behavior, or effective limits can prevent sharing.
An incompatible stage retains its own producer identity and existing rule attribution.
Budget semantics do not change implicitly through reuse.

The implementation first proves this connection contract with a writer and two scoped readers.
That proof covers barriers, authorizer restrictions, callbacks, cancellation, and disk-backed storage.
Cross-clause sharing remains incomplete until those checks pass.

## Memory and output shape

Admission includes retained owners and the next operation's private execution peak.
The controller defers independent work or spills backing storage instead of rebuilding live materializations.
Private inline CTE allocation remains part of the operation's measured cost.
SQLite spill settings alone do not establish a hard bound on those allocations.

The compiler splits large pipelines at explicit semantic or measured memory boundaries.
It does not turn every CTE into a table.
The existing 68.5 MiB and 98.6 MiB execution peaks are baseline measurements, not expected savings.

Final pipelines can stream directly to compact result adapters.
Transport packets bind to owners and preserve rule order without repeating computation.
Required output buffering participates in admission and uses file-backed storage when bounded RAM cannot preserve the order.

## GPU-compatible boundaries

The frontier carries typed columns, offsets, stable IDs, validity bits, and selection domains.
Native producers consume these batches through broad count, scan, and scatter passes.
Their internal passes can synchronize without creating scheduler tasks per node or rule.
The outer controller submits one operation family over a broad compatible frontier.

The design minimizes frontier barriers through legal pipeline fusion.
It cannot promise a fixed number of GPU launches across arbitrary dependency chains or recursive algorithms.
Recursion and newly discovered demand can require additional rounds.
Launch count, frontier width, transfer bytes, and synchronization count remain explicit profiling metrics.

## Implementation and acceptance

| Step | Reviewable result | Required evidence |
| --- | --- | --- |
| 1 | Typed producer graph and compiled plan | Existing candidate actions remain equivalent, including maintenance ordering |
| 2 | Shared candidate owners and storage bindings | Diamond dependency across two rules and packets builds once |
| 3 | Unified demand frontier and coverage ledger | Overlap, empty results, conditional consumers, and later waves never duplicate completed work |
| 4 | Workspace writer and scoped clause readers | Two clauses share facts without imports, with isolation and cancellation preserved |
| 5 | Admission, retirement, and output integration | Low memory changes admission or spill, never identity, semantics, or successful build counts |

Steps extend one production path. They do not introduce optional engine strategies.
The existing positive and clean corpus, edit parity, and native checks remain acceptance requirements.
Matched CPU measurements include graph planning, demand union, writes, indexes, publication, reads, and retirement.

The TDD ladder expands through these cases:

1. A fixed DAG preserves output order, read dependencies, maintenance hazards, and early storage release.
2. A diamond across rules and packets produces one successful build for each shared coverage key.
3. A later selector requests overlapping keys after an earlier consumer finishes, without retirement or repeated completed work.
4. A conditional branch closes unused demand sources, while an empty completed result satisfies an active consumer.
5. A sealed aggregate rejects partial-domain publication, while independent keyed work progresses without a global barrier.
6. Cancellation releases unpublished reservations, preserves completed coverage, and reports failures under the correct rule.
7. Two scoped readers share workspace rows under low memory without imports, unauthorized access, or live-owner reconstruction.
8. Different rule compositions and capacity partitions preserve semantics and use the same operation families.

The profiler records producer builds, attempted duplicate writes, covered empty keys, reader leases, retained bytes, and per-operation allocation peaks.
It separates planning CPU, controller CPU, SQLite CPU, native CPU, and publication cost.
The placement audit remains supporting evidence, not a runtime build counter.
Performance acceptance requires lower repeated computation and allocation peaks without a material regression in complete evaluation CPU.
