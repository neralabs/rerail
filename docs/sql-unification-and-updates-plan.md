# Shared SQL execution and fact updates

## Objective

This implements follow-up item 4, shared SQL execution, and item 5, retained insert/delete maintenance.
The SQL session and corpus SQL path must share execution semantics and scheduling.
Fact insertions and deletions must maintain retained results without rerunning entire programs.
Native AST matching remains a producer of input facts.
The corpus controller retains its existing parsing, chunking, and bounded cross-file rounds.

## Implementation

1. Generalize the wave connection interface so an existing corpus connection can use the same frontier and folds.
   Preserve the product owner's storage and native source estimates.
2. Share SQL emission, lattice registration, top handling, and physical relation normalization.
   Route pure SQL execution through that kernel. Keep native component execution available.
   Remove the corpus flat-only limitation and verify flat/min/max, Boolean, aggregate, and custom-lattice behavior.
3. Retain each session program's checked SQL queries and dependency graph after execution.
   Add an atomic batch update API with inserted and deleted fact rows, using existing typed-value encoding.
4. Propagate positive insertions with per-atom delta variants.
   For positive recursive deletions, overdelete possible consequences, then rederive surviving support.
   Cycles must not preserve facts whose external support disappeared; alternate supports must survive.
5. Refresh affected nonmonotone strata when negation, aggregates, lattice replacement, or native tree walks require it.
   Preserve unrelated strata and independent programs. Propagate actual output differences downstream.
   Report delta work and refreshed strata so callers can distinguish these cases.
6. Roll back SQL changes, fact bindings, and update metadata together on failure or budget exhaustion.
   Preserve existing output tables and indexes during successful updates.

## Root causes and fixes

| Finding | Root cause | Implemented fix |
|---|---|---|
| Corpus SQL treated lattice values differently from the session. | Separate stratum loops and flat-only merges had diverged. | Both paths call the same frontier kernel, set fold, and lattice fold. |
| Rebinding facts discarded useful outputs. | The session retained object ownership but discarded checked query plans. | Retain plans, dependencies, row counts, tables, and indexes. Apply atomic fact batches. |
| A projection lost alternate support after a deletion. | A view did not retain its previous rows, and projection multiplicity could escape set semantics. | Materialize affected views once. Deduplicate projections unless the projection preserves a known key. |
| Recursive deletion could leave unsupported cycles. | Local support from another derived row does not prove external support. | Overdelete possible consequences, then rederive surviving support through positive delta waves. |
| Small edits scanned the retained closure. | Deletion tested each result row against the removal set. | Delete by primary keys from the small removal set. |
| Endpoint retraction used 6.95 million VM steps. | Rederivation enumerated many body matches for each removed head. | Use correlated existence probes. Stop at the first surviving derivation. |
| Physical normalization broke typed corpus rules. | Domain inference ran again after lowering had replaced symbolic constants with integers. | Preserve existing domains. Add only each physical sentinel's integer domain. |
| A true Boolean output serialized like an empty relation. | The CSV writer tested joined text instead of row count. | Emit the row terminator when a logical empty row exists. |
| Aggregate and lattice retractions need different maintenance. | Their current reducers do not retain invertible support state. | Refresh affected strata and propagate their actual differences. Report these refreshes. |

## Validation

- Compare corpus SQL outputs with native execution under identical cross-file bounds.
- Run min/max and registered custom joins through both SQL entry points.
- Compare incremental outputs with fresh execution after every edit in deterministic update sequences.
- Cover recursive cycles, alternate derivations, deletion of the last support, duplicate/no-op updates, and batched changes.
- Cover negation, global/grouped aggregates, lattice improvements and retractions, and chained programs.
- Enforce work bounds on small updates to large positive relations.
- Verify rollback, later session reuse, exact integers, both CLIs, and TypeScript declarations.
- Keep raw profiles outside the repository and record measured outcomes here.

## Results

Steps 4 and 5 are implemented.
The corpus planner retains its statistics and native dispatch.
SQL components use the session/product wave kernel, including min, max, flat, and registered native lattice operations.
The session exposes `applyChanges` through Rust, Node, and both CLI request paths.
AST matching remains native.

The update tests compare maintained outputs with fresh execution after 60 deterministic graph-edit batches.
They also cover nonlinear recursion, simultaneous multi-input deletion, unsupported cycles, alternate support, and chained programs.
Other cases cover Boolean facts, exact integers, symbol input, projections, shortest paths, negation, and grouped/global aggregates.
Failures after fact mutation restore facts and outputs. Budget failures also restore projection ownership and allow later retries.
A work-budget regression retains 130,816 closure rows and limits each small positive edit to 30,000 SQL steps.

The Rust verification passes 57 tests: 32 session, 10 corpus/planner, 6 scheduler, 7 frontend, and 2 query-tree tests.
Two existing planner tests remain ignored.
TypeScript and boundary checks pass.
The structure checker retains 14 baseline failures and reports no findings in the new SQL files.
The built Node addon and both CLIs pass insert/delete batches.
The addon also passes rollback and retry checks.

### Measurements

These are medians of three optimized runs on the shared development host.
CPU time is more stable than wall time under concurrent load.
Counters include the driver's SQLite profile callbacks.
The first edit includes input migration; later edits reuse the mutable source.
Raw requests, SQL profiles, and output hashes remain under `/tmp/rerail-sql-evaluation/`.

| Workload | CPU | SQL VM steps |
|---|---:|---:|
| Initial closure, 512 nodes / 130,816 pairs | 254 ms | 9,375,658 |
| Initial closure, 1,024 nodes / 523,776 pairs | 1,148 ms | 36,988,842 |
| One disconnected insertion, 1,024 nodes, first edit | 0.85 ms | 1,180 |
| Another disconnected insertion, same session | 0.30 ms | 1,180 |
| Delete one disconnected edge | 0.56 ms | 15,692 |
| Extend chain by one edge; insert 1,024 paths | 2.63 ms | 100,411 |
| Retract that edge; delete 1,024 paths | 3.62 ms | 157,889 |
| Aggregate over 100,000 facts, initial execution | 7.96 ms | 1,067,750 |
| Aggregate, first one-row insertion plus input migration | 44.48 ms | 986,374 |
| Aggregate, subsequent one-row retraction | 10.11 ms | 986,363 |

Before the existence-probe fix, the same 1,024-path retraction used 6,945,450 VM steps.
The fix reduces that work approximately 44 times.
Both closure sizes retain their full tables throughout the edits.
The benchmark checks exact delta counts and final row-count/checksum oracles.
The aggregate benchmark checks results against a direct SQL reference.

Three CPython corpus workloads each ran three times through both SQL and native execution: 18 runs total.
Output hashes match for every pair, under identical cross-file bounds.
The planner regressions also preserve typed dependency summaries and native outputs.

### Follow-up: incremental aggregates and negation

The session now retains reducer state for eligible nonrecursive rules.
Their scalar subqueries read one relation and correlate through equality between inner and outer columns.
The positive body can contain joins. Multiple rules can support the same output row.
Completed derived relations can supply reducer inputs, so positive recursive producers remain compatible.
Lattice joins and unsupported subqueries retain the affected-stratum fallback.

| Root cause | Implemented fix |
|---|---|
| A one-row edit reran all correlated aggregate queries. | Retain group counts and sums. Apply signed fact deltas through SQL. |
| Removing one minimum could lose another identical support. | Retain value multiplicities under a group/value primary key. Probe the next extreme through that index. |
| A changed negated relation could force a scan of every candidate. | Retain matching counts. Reconsider indexed contexts only at zero crossings. |
| Several witnesses or rules can produce the same output. | Retain per-context contributions and head support counts. Publish only actual membership changes. |
| Simultaneous changes could double-count joined witnesses. | Keep complete positive context tuples. Union insertion variants and delete contexts by changed atom keys. |
| Signed sum arithmetic can silently become SQLite REAL values after overflow. | Reduce signed terms with a native SQL aggregate and a wider intermediate. Reject totals outside signed 64-bit range. |
| New state could survive a failed edit or outlive its program. | Initialize it inside the batch savepoint and register every table under program ownership. |

The first edit that can affect a relation initializes its eligible state against the old facts, before any source changes.
A dependency walk skips initialization for unrelated relations. The batch reports newly initialized relations explicitly.
This initialization can scan inputs and materialize positive body combinations.
Later edits reuse its indexes and update only affected reducer groups and contexts.
The batch budget covers initialization SQL. Native input migration remains outside the SQLite VM budget.

Ten new regressions cover duplicate extrema, empty/default aggregates, global aggregates, composite keys, constant filters, and negation zero crossings.
They also cover joined contexts, simultaneous changes, alternate rules, reset/rebinding, overflow, and rollback after reducer mutation.
A separate work budget proves that editing a small input does not initialize an unrelated 20,000-row aggregate.
Forty mixed edit batches compare maintained results with fresh execution.
A recursive reachability producer supplies actual tuple deltas to a later aggregate/negation program.
The fallback test uses an aggregate result as another aggregate's correlation key.
The work-bound test retains 100,000 distinct values in one group and 100,000 outer candidates.
Each warm edit passes a 60,000-step budget without a stratum refresh.

The following measurements are medians of three optimized runs with SQLite profiling enabled.
CPU timings come from the shared development host. VM counts provide the stronger scaling comparison.

| Workload | CPU | SQL VM steps |
|---|---:|---:|
| Previous 100,000-row aggregate: first edit and input migration | 44.48 ms | 986,374 |
| Retained aggregate: first edit, input migration, and support initialization | 77.45 ms | 2,892,413 |
| Previous aggregate: warm one-row retraction | 10.11 ms | 986,363 |
| Retained aggregate: warm one-row retraction | 1.16 ms | 3,684 |
| Combined reducers, 10,000 facts and candidates: warm retraction | 3.41 ms | 22,461 |
| Combined reducers, 100,000 facts and candidates: warm retraction | 3.30 ms | 22,461 |
| Combined reducers, 100,000 rows: initialize state and apply first edit | 376.60 ms | 32,027,762 |

The original aggregate benchmark now uses approximately 268 times fewer VM steps for a warm retraction.
The combined case runs global and grouped count, sum, min, max, plus negation.
It checks seven edit batches at each size, including duplicate extrema and negation transitions.
The 12 optimized trials pass output oracles and report no refreshed strata.
Raw requests, answers, and profiles remain in `/tmp/rerail-sql-evaluation/reducers-final-aggregate-profiles` and `reducers-final-stress-profiles`.
Two additional closure trials pass the existing positive-update oracles.
The full session run passes 41 tests. The additional recursive-producer regression passes with the ten reducer tests, for 42 distinct session tests.
The built Node addon and both CLIs pass aggregate/negation edit sequences. The addon also passes rollback and retry.
TypeScript and boundary checks pass. Structure checks retain the 14 baseline failures without findings in the new SQL files.

The main costs now occur during initialization and in wide positive joins with many retained combinations.
Reducer expressions currently retain separate state instead of sharing compatible indexes.
A changed global aggregate can still affect every outer context that reads it.
String aggregates, computed correlations, aggregate-dependent correlations, and nonmonotone rules with constant facts use the refresh fallback.

### Remaining work

- Lattice joins, tree walks, recursive aggregate rules, and unsupported correlations still refresh affected strata.
  The scalar aggregate and negation follow-up described next maintains eligible rules incrementally.
- First-edit input indexing and projection migration require time and memory proportional to their rows.
- Deletion reconstructs old inputs with a union of current and deleted rows. SQLite can build an automatic index over that input.
  The disconnected deletion above indexes 1,024 input rows even though its output delta has one row.
  An indexed overlay or disjoint old-input query variants could remove that cost.
- Overdelete/rederive can revisit many facts with alternate support. Retained derivation support could reduce this work.
- Snapshot/core facts still refresh by rebinding. Only explicit `bindRelation` inputs accept row patches.
- SQL corpus execution still creates control tables per component. The call workload remains slower than native execution.
  Shared scheduling removes semantic duplication; it does not remove every schema-management cost.
- SQL budgets cover SQL statements. Native input migration and atom interning need separate host-work limits if callers require them.
- Full column statistics and adaptive replanning remain separate improvements.

The API and operational details are in [SQL fact and rule execution](sql-executor.md).
