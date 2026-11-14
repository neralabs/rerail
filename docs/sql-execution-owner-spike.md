# Shared execution spike

The earlier experiments left shared fact ownership and most intermediate lifecycles untested.
This experiment changes those boundaries directly. The fixed complete-job limit remains 100 submissions.

## Execution contract

One explicit execution owns the file-backed positional index, source versions, and completed preparation.
Snapshot and workspace consumers keep separate file membership, revisions, permissions, and results.
File identity contains path, grammar, and source digest. Two versions of one path can coexist.
Removing a file changes that consumer's membership. It does not remove another consumer's pinned facts.
Failed publication cannot publish completion. Empty successful results do publish completion.
The execution retains bounded idle facts, with all eviction statements included in accounting.
Closing the last owner releases the backing database. There is no process-global result cache.

## Deliverables

1. Exercise one owner through both normal evaluation paths and multiple consumers.
   Require one schema build and one publication per source version.
   Cover different file sets, concurrent versions, removals, edits, rollback, and final release.
2. Batch indexed candidate construction by actual dependencies.
   Keep indexed access while separating logical producers from statement boundaries.
   Replace per-producer retirement with bounded cohort retirement.
   Compare full intermediate bags and count every replacement statement.
3. Execute flow producers in shared SQL scopes across multiple consumers.
   Test multiply consumed and recursive relations, rather than only private leaf stages.
   Retain physical indexes where query-plan and CPU evidence require them.
4. Integrate compiler inference batching and the selected storage path into the measured execution.
   A separate component report does not count as an integrated reduction.
5. Run the unchanged observations through the combined implementation.
   Record complete submission counts, CPU, memory, failures, and rejected shapes.

Use existing dependency passes and SQL evaluation. Do not add a native rule interpreter or an algorithm configuration flag.
Runtime commands retain the 115-second deadline. Optimized builds use the standing permission to finish.
