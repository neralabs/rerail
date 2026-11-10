# Composed session retention and bulk frame publication

## Integration and baseline

Merged integration `5ea1d2f75` into this branch as `d65a24d47`.
The merge keeps integration's native default. Composed validation explicitly sets `RERAIL_COMPOSE=1`.
A default-mode pass cannot prove that the composed path works.

The merged baseline reproduces three failures:

- A session cache consumer cannot find `info` after a composed run.
- SQLite rejects the rule relations `back` and `BACK` as duplicate CTE names.
- Composed execution bypasses durable retention and produces no retained-strata report.

Tests run in separate processes. A panic cannot leave a session active for the next test.

## Resident output contract

The composed executor now retains typed output rows in the ordinary native relation store before decoding output text.
Both session maintenance and SQL consumers can read those resident facts.
No SQL table is required just to read an output back into memory.
Empty results replace earlier results with empty resident relations.

Logical relation names remain case-sensitive.
When SQLite would fold two names together, the compiler assigns distinct physical CTE names.
The rewrite traverses typed query blocks, aggregates, and correlated subqueries.
Output labels and fact identities keep their logical names.

## Durable composed reuse

The retained boundaries are requested outputs, including outputs requested for later consumers.
Each boundary records its grammar identity and the digests of its transitive input facts.
The engine admits unchanged boundaries together before execution.
It compiles the remaining rules into relational queries over current inputs and retained facts.
Ordinary graphs use one query; maintenance and reference limits can require shared publication boundaries.
When all requested outputs are unchanged, it decodes resident facts without executing that query.

Internal CTEs remain inside the query. The engine does not export them solely to persist every intermediate result.
A second program can reuse a shared retained output under the same identity and input provenance.
A changed key, changed rule, missing payload, or invalid admission causes evaluation again.

Native and tree leaves can read document state beyond their explicit demands.
The engine does not certify their outputs as unchanged without versioned provenance for that state.
They continue through normal execution. This restriction prevents stale results from incomplete cache keys.
No new cache map or `.cache`-specific execution path is introduced.

## Bulk persistence

Frame publication keeps the existing transaction and write-back queue.
Inserts use batches of 96 frames, with 866 parameters at most.
Deletions and usage updates use batches of 128 keys.
Their row-value membership queries probe the frame primary key.
Usage updates cover every partition of each demanded relation, as before.

The regression publishes, deletes, and touches 257 keys in an 8,192-frame space.
It also checks repeated publication of one key and replacement with a nullary relation.

| Measurement | Before | After |
|---|---:|---:|
| Executed statements | 782 | 18 |
| VM instructions for the changed implementation | — | 74,776 |

The gates allow at most 20 statements and 100,000 VM instructions.
All eight frame tests pass, including pending writes, deletion, restart, partition isolation, and collection.

The VM trace resets counters at statement start.
Without that reset, SQLite's cached statements also report untraced fixture setup.
The initial 410,030-instruction result included that earlier work.
The remaining full-space scan is the existing garbage-collection pass, once per completed publication.

## Validation results

The thirteen existing session/cache checks pass with composition explicitly enabled.
They cover edits, aggregates, negation, recursion, inlining, version changes, durable reuse, and sharing across programs.
The same thirteen checks pass in native mode.
All thirteen direct compiler tests pass. These include SQLite refusal before staged execution,
case-distinct inputs, one-query ordinary composition, and resident-object identity for quiet facts.
The three cache architecture checks pass. The positive fixture produces all 120 expected findings.
All eight storage tests pass.

The final engine test binary was built without incremental compilation after a stale-object linker failure.
No test ceiling was raised.

## Whole-job comparison

Both arms use the same final binary, four executor threads, private corpus copies, and fresh fact stores.
Each condition runs in a separate process over all 5,725 files.
CPU and wall time include startup, input processing, execution, and exit.
Cold means an empty fact store, not an empty operating-system page cache.

| Run | Composed CPU | Native CPU | Composed wall | Native wall | Composed SQL | Native SQL |
|---|---:|---:|---:|---:|---:|---:|
| cold | 18.514 s | 18.333 s | 9.967 s | 9.589 s | 81 | 396 |
| warm | 0.652 s | 0.718 s | 0.702 s | 0.815 s | 19 | 32 |
| warm-profile | 0.800 s | 0.873 s | 0.889 s | 1.364 s | 19 | 32 |
| edit | 0.662 s | 0.663 s | 0.803 s | 0.894 s | 10 | 30 |
| edit-warm | 0.672 s | 0.661 s | 0.730 s | 0.698 s | 19 | 32 |

All composed runs pass the unchanged 100-execution gate and produce the expected finding keys: none.
The positive fixture supplies the nonempty semantic check.
These are single observations on a shared machine; they do not establish a timing improvement.
SQL counts are core workload executions. Storage publication is measured separately above.

**At this revision, the integration default's native cold run fails the existing gate at 396 executions.**
The [native cold budget follow-up](native-cold-budget-2026-11-10.md) resolves this failure at 20 executions.
Its local phase accounts for 373. Warm and edited native runs pass at 30–32 executions.
The runner records that failure and exits unsuccessfully, even when asked to finish the comparison.
This separate integration limitation is not repaired by the composed-retention or frame-publication changes.
Integration's native default remains unchanged.

Reproduction commands, failures, traces, measurements, and hashes are in
[`session-publication`](../../benchmark/async-family-integration/session-publication/README.md).

## Bounded preparation

A maintained negation fixture exposed a separate preparation limit after output retention was repaired.
Its 38 CTEs occupy 44,239 bytes. Expanding the dependency references gives about 5.85 million reads.
This count describes SQL preparation expansion, not executed rows.
SQLite rejects the query after one table exceeds 65,535 references, despite materialization hints.

The physical planner counts typed relation reads, including correlated subqueries.
An estimate above 8,192 triggers a preparation check, not an automatic split.
If SQLite accepts the complete query, the executor keeps it fused.
If SQLite reports a reference or nesting limit, the planner cuts the graph into shared stages.
Each cut publishes relations used by the remaining graph in one query.
Those relations become ordinary resident inputs with statistics.
The compiled plan caches the preparation decision. Other SQL errors remain errors.

Automatic cuts at both 4,096 and 8,192 caused 154 executions on the async workload.
The unchanged whole-job gate caught both failures.
The earlier reconstruction from separate stages lost the scope of generated demand helper names.
A reconstruction of the uncut SQL gives 11,446 weighted references. SQLite prepares that query successfully.
Its query occupies 23,416 bytes. The uncut comparison passes at 81 job executions.

The planner permits at most 32 stages per composed run and reports each boundary in the profile.
Change admission and result evaluation share that budget.
Preparation has its own profile span; it is separate from SQL execution.

Known empty positive joins are removed before compilation.
Empty negation and aggregate inputs are not treated as empty results.
The executor requests retained change relations together before deciding which old facts it can reuse.
When a maintenance relation has no additions or deletions, it shares the old fact and its indexes.
A direct test requires the same resident object after publication, not merely equal rows.
Generated SQL is recorded before preparation so failures remain diagnosable.
