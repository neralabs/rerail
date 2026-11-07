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
- Old SQL-result reads use disjoint query variants. Several changed SQL atoms in one body can multiply those variants.
  Native inputs use indexed overlays; sparse native deletions no longer reconstruct or index the full input.
- Overdelete/rederive can revisit many facts with alternate support. Retained derivation support could reduce this work.
- `applyFileChanges` now feeds file patches into retained maintenance without a complete inventory.
  `applySnapshot` retains compatibility with complete replacement requests and must read those inventories.
  Queries without document constraints can still probe several document shards.
- SQL corpus execution still creates control tables per component. The call workload remains slower than native execution.
  Shared scheduling removes semantic duplication; it does not remove every schema-management cost.
- SQL budgets cover SQL statements. Native input migration and atom interning need separate host-work limits if callers require them.
- Full column statistics and adaptive replanning remain separate improvements.

The API and operational details are in [SQL fact and rule execution](sql-executor.md).


## Integration, indexed deletion reads, and native source edits

The next three follow-ups are implemented on the merged final-shape tree.

| Finding | Root cause | Change |
| --- | --- | --- |
| TypeScript could not compile the merged CLI. | An earlier merge kept newer callers but dropped their runtime APIs, storage support, and modules. | Restore the matching implementations and their integration tests. Keep the newer rollback protection. |
| Guest parse/splice/unparse returned no value documents. | The final-shape branch still had unfinished demand and guest-hiding changes. | Integrate committed decision 0058 and verify the previously failing test. |
| A one-row deletion indexed the whole old input. | The old-input `UNION ALL` view caused SQLite to materialize the input and build an automatic index. | Probe native current/deleted overlays; distribute reads of old SQL results over indexed query branches. |
| Editing a source file discarded SQL results. | Snapshot binding replaced every source and invalidated every execution. | Keep native document shards, rebuild changed files, and pass their fact deltas through the same atomic maintenance transaction. |
| Native probe results differed from a cold batch. | Native input consumers retain 32-bit row handles; a sharded adapter cannot place shard ids in higher bits. Range indexes also do not imply node-id columns. | Preserve dense row handles, use stable handles for lazy probes, and route by declared document metadata only. Compare scans, probes, and CPython analysis with the original adapter. |
| Rebinding repeated metadata work. | Every source binding recalculated the signature of every source. | Cache shard estimates and refresh the planner signature once per fact batch. |
| A source edit created empty delta tables for unchanged families. | The row-update transaction treated every rebound native family as a changed input. | Rebind the batch, but create tuple journals only for nonempty changes. |

### Validation and measurements

The deletion probe retains 523,776 paths over a 1,024-node chain, plus two disconnected edges.
A disconnected retraction deletes one path. The native overlay reduces that edit from 15,692 to 1,236 SQLite instructions.
Automatic-index rows fall from 1,024 to zero. A measured run used 0.44 ms CPU; host contention makes wall time unsuitable for comparison.
Retracting the chain's added terminal edge deletes 1,024 paths and uses 136,272 instructions, down from 157,889.
That run used 3.80 ms CPU. This edit must still process its output delta.

A regression enforces a 10,000-instruction budget for a warm one-edge deletion over 100,000 native input edges.
Other regressions cover simultaneous deletions, repeated atoms, downstream SQL programs, and scratch-module cleanup.
Native edit tests compare retained results with fresh execution after additions, replacements, removals, failed parses, and budget exhaustion.
They also verify shared literal facts, unchanged shard reuse, lazy containment, and result-index retention.

Repeated native file edits produce these optimized measurements. Each edit rebuilds one file and changes one `name` fact.
The workload retains a projected name relation and a global count.

| Snapshot | First edit: SQL instructions | Warm insertion: SQL instructions | Warm insertion: CPU |
| --- | ---: | ---: | ---: |
| 100 Python files | 14,262 | 4,599 | 3.09 ms |
| 1,000 Python files | 95,262 | 4,599 | 20.15 ms |

Removing empty journals reduced the warm insertion from 46,057 to 4,599 instructions.
The first edit still initializes retained projection and reducer state, so its SQL work grows with the existing facts.
Warm SQL work is independent of file count here. Host CPU still grows because snapshot parsing, shard metadata, and shared-fact composition visit the documents.
These measurements include profiling overhead and are single runs on a shared host.

Final checks pass: 51 SQL tests, the guest parse/splice/unparse regression, and 312 selected TypeScript integration tests.
One platform-dependent integration test is skipped. TypeScript, the JavaScript build, and the layer-boundary check pass.
The headless engine also compiles. Both SQL CLIs execute a snapshot edit and return its updated retained result.

The benchmark driver accepts `snapshots` as well as explicit `changes`.
`benchmark/sql-executor.py --only update-native` exercises repeated edits over 100 and 1,000 source files.
The standalone SQL request and Node API expose the same snapshot operation.

### Remaining improvements

Editor patches, retained literal reference counts, and per-family document indexes are complete in the metadata follow-up.

1. Extend document routing to remaining producer schemas, such as the `py_*` families.
2. Bound old SQL-result branch multiplication, using a compact indexed representation for large bodies.
3. Extend retained support to the documented aggregate, lattice, and tree-walk fallback cases.
4. Add host-work limits for parsing, native journals, atom interning, and first-edit migrations.


## Remove snapshot-wide metadata work

The final-shape branch was merged at `ca76fe78f` before this change.
The first merge commit on this branch is `4a7998ff6`.
A subsequent merge includes `dde9f3dc4`, which adds the completed guest-hiding changes and the rule explorer.

The previous warm edit recreated every fact family's document vector and planner estimates.
It also rescanned all shared literal-type facts and copied unchanged document records into a new inventory.
Complete snapshot JSON parsing added another cost proportional to the repository size.

The native session now retains immutable balanced document indexes for each fact family.
Each node caches row counts, index availability, and lookup estimates for its subtree.
An edit replaces only its document entries and copies their search paths.
All untouched subtrees remain shared. SQL readers and rollback retain the previous roots.
Shared literal facts retain document reference counts, with ordered indexes on both literal value and type.
Their updates read only the literal facts in changed files.

`applyFileChanges` accepts `{upsert: [snapshot files], delete: [paths]}`.
It reads only those paths and sources. Complete `applySnapshot` requests use the same retained indexes.
They still scan the incoming and old inventories to determine additions, replacements, and omissions.
Both APIs use the existing atomic fact-change transaction and retained SQL scheduler.
Neither API lowers AST matching into SQL.

The response records `examined_files` and `metadata_nodes`.
The first counts supplied entries and deletions, or both inventories for complete snapshots.
The second counts rebuilt balanced-index nodes across all fact families and shared-fact indexes.
A regression compares 100 and 1,000 files and requires metadata growth to follow index depth.
Another regression compares mixed file patches with cold execution, including shared literals and both containment directions.
The persistent-index regression compares ordered lookups and retained historical versions with a standard ordered map.


### Metadata follow-up measurements

Each edit changes one file and one `name` fact. The workload retains a projected relation and a global count.
These optimized measurements include tracing overhead and come from one run on a shared host.

| Files | Warm file-patch insertion | Warm file-patch retraction | Metadata nodes rebuilt | Files examined per patch |
| ---: | ---: | ---: | ---: | ---: |
| 100 | 1.42 ms CPU | 1.28 ms CPU | 226 / 224 | 1 |
| 1,000 | 2.04 ms CPU | 1.50 ms CPU | 337 / 335 | 1 |
| 10,000 | 1.55 ms CPU | 1.43 ms CPU | 448 / 446 | 1 |

Warm insertion uses 4,254 SQLite instructions at every size. Warm retraction uses 4,265 at every size.
The previous 1,000-file complete-snapshot insertion used 20.15 ms CPU and 4,599 instructions.
The new complete-snapshot API uses the retained indexes too: its measured warm insertion is 2.42 ms at 1,000 files.
At 10,000 files it takes 13.93 ms because it still reads and compares complete inventories.
The file-patch API avoids that inventory cost.

The first edit still initializes projection and aggregate support. Its instructions grow from 13,917 at 100 files to 904,917 at 10,000 files.
That is retained-rule initialization over the existing facts, not document metadata reconstruction.
Subsequent edits reuse that state.

Retraction support queries also need a bound node's document. Common native schemas now identify their document and location columns explicitly.
An indexed number column is not assumed to contain node ids. Routing uses the declared domains and preserves sentinel lookups.
Both fact scans and probes are compared with the original batch adapter.

The merge check exposed a separate guest-demand round-trip regression in the new quoted-annotation rules.
Printed rules already contain their demand helpers. The reader now preserves those explicit demands instead of generating another copy.
The dialect parity check normalizes legacy guest requests through the same lowering step.


### Final checks

- 57 SQL and persistent-index tests pass.
- After the collection fix, 226 rule-language, SQL/index, corpus, and content-store tests pass together in parallel, twice.
  Four report/write or performance tests are ignored, and the separate syntax census is excluded.
- 312 selected TypeScript integration tests pass, with one platform-dependent skip.
- TypeScript, the JavaScript build, and the layer-boundary check pass.
- The headless engine compiles. Both SQL CLIs accept file additions and deletions and return the updated retained result.


### Broader regression limits

The exploratory merged run finished with 778 passing tests, 13 failures, and 25 ignored tests.
Twelve failures require absent pack fixtures or grammars that this build does not include: HTML, Go, and Scala.
The other failure was `value_documents_are_memoised_with_the_files_that_asked` in the newly merged guest tests.
It reported a corpus-cache foreign-key failure during parallel runs. The test passed alone.
The parallel failure reproduced before the fix described below.
The missing fixtures and grammars still prevent a completely green repository-wide suite.

The final merged file-patch examples pass at 100, 1,000, and 10,000 files.
Their warm insertion uses 4,254 SQL instructions and rebuilds 226, 337, and 448 metadata nodes respectively.
The 10,000-file run measured 1.51 ms CPU for insertion and 1.33 ms for retraction.

### Concurrent corpus collection

The collector could delete a source between its lookup and publication of its memo results.
A run records its source manifest after local evaluation. Until then, newly stored or reused sources can appear unreferenced.
Another run's collector can advance the generation and delete those sources. Memo publication then fails its foreign-key check.
The generation check alone does not protect a run that spans multiple collection passes.

The store now uses a shared file lease while a corpus reads sources, parses files, and publishes memo rows, stats, and its manifest.
Standalone cached parses hold the same lease through tape publication.
Each collector tries an exclusive lease before it selects manifests or deletes rows. If readers are active, collection defers.
Readers can run together, including in separate processes. Closing a lease or exiting its process releases the lock.
The lock uses the database's canonical path, so symbolic aliases share it. Its sidecar file must remain in place.

A deferred pass leaves collection due. A later corpus run checks this flag even when its manifest has not changed.
The maintenance thread can also resume collection. Continuous overlapping analyses can delay collection until a gap between readers.
All processes that share the cache must use the lease protocol; an older collector cannot respect these leases.

Regression tests cover overlapping readers, memo publication, deferred collection, collection after readers finish, and warm-run resumption.
A child-process test also checks symbolic aliases and lock release after a process is killed.

The combined checks exposed a second lock issue: a subprocess can inherit an open lock descriptor before it executes its program.
Closing the parent's handle alone can leave the lock held briefly. Rapid collection passes then defer and leave a derived tape behind.
Leases now explicitly unlock on release. A regression test keeps a duplicate descriptor open to check this case.

Validation: `cargo check --locked --lib` passes. Both parallel runs pass 226 tests with four ignored tests.
Each run uses a separate `RERAIL_TAPE_CACHE` database; tests within that run share it with collection enabled.
This isolates validation from older engines running in other worktrees, which do not use the lease protocol.
The test filters are `content_store`, `code_rule_dsl::sql_executor`, `code_rule_dsl::logic::rr::tests`, and
`code_rule_dsl::logic::plan::corpus::tests`, with `rule_syntax_files_pass_check_rr` excluded.
