# SQL composition implementation

This change implements the selected [composition improvements](sql-composition-opportunities.md).
SQLite retains rule composition and diagnostics.
The CPU path retains the streaming packed-node adapter.

## Forest containment

The forest adapter validates the selected edge graph before it publishes a relation.
It retains one preorder position per normalized identity and one interval per requested root.
The public relation still exposes `(__path, root, node)`.
SQL range joins produce requested rows for each consumer.
The adapter no longer stores every root and descendant pair.
Enumerating every pair still takes work proportional to the result size.

The intervals describe the selected graph, including disconnected subtrees and dangling endpoints.
Raw CST intervals cannot replace this validation.
Cycles and multiple distinct parents remain errors for a declared forest.
Arbitrary directed graphs retain cycle-safe recursive SQL.

A root query can select `__path,id,expand` in that order.
The two-column form retains its previous expansion behavior.
SQLite truth conversion determines expansion. False or null permits only the reflexive pair.
If any duplicate request permits expansion, the root expands.
A blocked root can remain an intermediate node for another root.
This declaration adds no extra producer dependency wave.

The four authored state clauses now use this forest relation.
Their source gates, selected edges, root expansion guards, and diagnostic identities remain unchanged.
The authoring generator emits the same declarations as the checked policies.

The state loop predicate fixes the join order for two small, correlated subtree probes.
`CROSS JOIN` keeps each demanded subtree before its identifier lookup.
Without that order, SQLite can scan all identifiers before testing interval membership.
The journal records this regression and the corrected VM work.

## Typed syntax intervals

Syntax sources expose `subtree_end`, the exclusive end of a node's raw CST preorder interval.
The `id` column supports indexed lower and upper bounds.
A consumer can use `child.id>=root.id AND child.id<root.subtree_end` within the same file.
This describes structural containment over the visible syntax rows.
If an excluded interior node disconnects the graph, selected-edge reachability still requires the forest adapter.

Syntax producers also accept root expansion guards.
Their request record contains file, node, and expansion as three `u32` scalars.
Count, scan, and scatter passes retain their existing broad dependency-wave shape.

## Demand boundaries

The candidate compiler retains a materialized span relation after the demanded-key join.
Costly source predicates consume that completed selection.
SQLite cannot flatten those predicates into a broader retained-property scan.
Structural properties retain their existing direct SQL access path.
Vocabulary and value postings continue to answer covered values without source decoding.

## Shared policy catalog

Candidate producers now share one definition catalog across the complete policy frontier for each grammar.
The previous 32-plan boundary no longer duplicates equivalent producers.
Definitions retain bound values, grammar, source scope, and conservative exactness metadata.
The authoritative matcher still owns captures and ordered matching semantics.

The compiler retains its physical cuts before shared expressions cause excessive preparation expansion.
Result queries contain at most 128 output arms.
Result queries share producer relations across packets.
This separation respects SQLite's compound-query limit without another producer catalog per output packet.
File batching and source invalidation retain their existing boundaries.

SQL generation indexes dependencies once for each definition.
Each result or preparation statement visits only its reachable definitions, in declaration order.
Temporary decode definitions borrow this catalog instead of copying every earlier SQL string.
Physical cuts remove their inline definition before later consumers are rendered.
This prevents catalog sharing from introducing repeated full-catalog copies and scans.

Diagnostic captures use `rerail.candidate-replay/v2` with a `queries` array.
The replay host accepts both versions and preserves result order across output packets.

## Candidate relation lifetimes

The [target ownership design](sql-materialization-shape.md) extends this implemented scheduler across inline producers and rule consumers.
The current table schedule alone does not provide that execution-wide sharing.

The compiler builds an execution schedule for each cached SQL template.
Physical table references create producer dependencies.
Retained-property reads and writes create additional ordering dependencies.
These dependencies preserve earlier reads, later fills, vocabulary completion, and pending-vocabulary cleanup.
The scheduler derives these references from generated SQL identifiers.
User expressions remain bound parameters.

The schedule prepares each producer before its first scheduled consumer.
It preserves output packet order and constructs each shared producer once.
After the last SQL consumer, it releases each temporary table and its index.
An ordering dependency does not extend storage lifetime without an actual table read.
An iterative traversal handles deep dependency graphs without recursive host calls.
This change retains batched SQL, retained decoded properties, and the streaming candidate adapter.
It adds no production execution switch.

The executor batches adjacent releases into one SQL call.
After an error, it releases remaining live tables and preserves the original error.
The next request can reuse the connection.
SQL profiling captures pending execution plans before their inputs disappear.
It visits new queries once and writes the complete report at the normal checkpoint.

Trace captures include `anchor-working-set` boundaries every 128 schedule operations and after result packets.
These diagnostic boundaries report live relations, table bytes, index bytes, reserved pages, free pages, and SQLite resident memory.
They also report current RSS, peak RSS, and available allocator statistics.
Each snapshot retains the largest 20 SQL objects and totals for all objects.
The summary treats memory measurements as gauges instead of summing them.
Snapshot queries inspect the whole SQL schema together and add diagnostic overhead.
Timing comparisons must disable tracing and SQL capture.

The standalone `lifetimes` replay binary compares captured inputs with the earlier eager layout.
Both layouts use production SQL, codecs, storage settings, and the production scheduler.
The eager comparison exists only in this diagnostic binary.
It writes exact candidate rows, operation CPU, and storage snapshots.
It also reports compiled statement bytes and SQLite allocation peaks.
The standalone process enables SQLite memory accounting before initialization.
This diagnostic accounting does not change production settings.
Allocator allocation counts describe managed address space, not resident physical memory.
The performance journal records the measured process results separately.

## Checks and measurements

Regression checks cover guarded roots, numeric and text identities, disconnected trees, arbitrary graph cycles, and forest storage growth.
Additional checks cover syntax bounds, source-predicate demand, and more than 500 policy outputs in one catalog.
Existing edit, restoration, removal, budget, and positive corpus checks remain required.

The [performance journal](data-layer-performance-journal.md#sql-composition-promotion-2026-10-22) records the measured results and their limitations.
Cold CPU medians decrease by 12.5% on the state snapshot and 25.8% on the smaller ten-rule snapshot.
Peak process RSS increases by 13.6% and 8.8%, respectively.
These small snapshots do not establish full-repository performance or lower memory use.
Workload commands retain the 115-second controller deadline.
The user approved longer optimized addon builds after two bounded build attempts expired.

## File-scoped kind positions

Kind selection first seeks the persistent `file_kind_counts` primary key `(fid,code)`.
Each existing count row also owns a delta-varint posting segment.
The generic `nera_postings` table expands that segment after file and kind selection.
The packed-record pass constructs positions alongside counts, without additional SQL rows.
Sparse files do not require decoded property coverage for this path.

The bare FTS index remains available.
Its vocabulary instance table accepts term constraints but cannot seek document constraints.
Joining requested files after that stream repeats corpus-wide work during ingestion.
The repository checkpoint exposes this cost; tiny source controls do not.

Store version 2 adds the posting column.
A transaction backfills positions from existing packed records through one ordered file cursor.
A failed backfill rolls back the column and version changes.
Normal edits replace counts and postings together; file removal cascades to both.
Regression checks cover unrelated-file growth, sparse coverage, malformed syntax, edits, removals, and upgrade rollback.

### Value lookup file fence

Exact value terms also need file-scoped seeks before posting expansion.
The value path orders terms, requested files, and `vidx` with explicit `CROSS JOIN` boundaries.
The existing `(lang,term,fid,kind)` primary key answers those probes.
The missing-value fallback likewise starts with requested files before `vcov`.

A regression test adds 200 unrelated files with the same exact term.
Before the correction, unchanged lookup results require 19 progress ticks instead of one.
Each tick represents 100 SQLite VM instructions.
The test enforces bounded work as unrelated files grow.

### Retained property read fences

Decoded property views also start from demanded keys before they seek retained rows.
Universal node reads start from requested files.
Keyed node, span, and child reads start from their selected `(fid,pre)` relation.
Explicit `CROSS JOIN` boundaries prevent a retained-table scan from replacing these probes.

The regression test retains an unrelated file with 2,000 statements after preparing one small requested file.
Before the correction, identical structural results require 806 progress ticks instead of six.
This test measures the complete lowered statement, including its physical materialization schedule.
An isolated hand-written CTE does not reproduce the planner behavior of that schedule.
