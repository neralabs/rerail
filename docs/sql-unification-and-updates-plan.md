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
String aggregates, aggregate-dependent correlations, and nonmonotone rules with constant facts use the refresh fallback.

### Remaining work

- Recursive lattice and aggregate rules, unsupported walk blocks, and unsupported correlations still refresh affected strata.
  Eligible scalar aggregates, negation, lattices, and generated walks use retained maintenance.
- First-edit input indexing and projection migration require time and memory proportional to their rows.
- Native inputs use indexed overlays. Old SQL-result reads restore old membership from row deltas inside the batch savepoint.
  Neither path reconstructs a complete old input or multiplies old-read query variants.
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

The follow-up below implements Python document routing, bounded old SQL reads, and more retained rule cases.
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

## Routing, bounded old reads, and retained rule coverage

### Causes and implementation

Python call families expose document ids and node ids in different columns.
The shard adapter now identifies those columns explicitly, including optional keyword, owner, and reference nodes.
Point probes select one shard. Bounded node ranges traverse only the corresponding document interval.
Planner estimates use the maximum shard cost for routed columns, rather than the sum across all files.
The old estimate made a point probe look as expensive as a broad scan.
Negative sentinel values still probe every shard because each file can contain the sentinel.
Guest probes use host locations or host document ids because a guest id can identify a different document.

Old SQL reads previously distributed each changed atom over current and deleted alternatives.
A body with many changed atoms generated exponentially many query variants.
The update now temporarily restores old membership in retained SQL tables inside the batch savepoint.
It deletes inserted rows first, then restores deleted rows. Old reads use the existing indexes.
Before rederivation, it deletes the restored rows and reinserts the current additions.
This order also handles lattice keys whose old and new values cannot coexist under the unique key.
An error rolls back source changes, restored rows, outputs, and ownership together.
This path copies no complete old relation and creates no Cartesian product of query variants.

Computed aggregate correlations now retain expression indexes over outer context keys.
Both sides of an equality can contain expressions whose fields belong to that side.
SQLite ignored the context index when the comparison applied integer affinity from the changed key column.
Computed-key probes now remove that comparison affinity so the expression index remains usable.
Expressions that depend on another aggregate result still use the explicit refresh fallback.

Nonrecursive lattice outputs retain per-context contributions and candidate multiplicities.
An edit refolds only keys whose candidate support changes, then publishes actual output differences.
The same path supports flat, min, max, and registered deterministic lattice joins.
Duplicate supports survive independent deletions. Removing the last conflicting support can retract top to a concrete value.
Min/max and flat folds use indexed extrema. An opaque registered join can scan all candidates of an affected key.

Nonrecursive rules with generated tree walks retain their seed, recursive walk, and result relations.
The engine lowers those CTEs to positive SQL relations when the first affected edit prepares retained state.
It maps named native columns to positional delta columns and creates reusable probe indexes.
Later edits use overdelete/rederive. Reset and rollback include the retained walk relations.
A changed document can bind a native node before it binds a retained walk.
Maintenance orders the remaining joins by their connections to bound aliases, so those probes avoid unrelated walk rows.
AST matching remains a native producer.

### Coverage limits

Recursive lattice and aggregate rules retain the affected-stratum refresh fallback.
Walk blocks with scalar subqueries or negation also retain that fallback.
String aggregates and aggregate-dependent correlations remain separate extensions.
These changes do not remove first-edit support initialization, which remains proportional to the state it prepares.

### Regression results

The final combined parallel run passes 235 tests, with four ignored tests and the separate syntax census excluded.
The filters cover SQL execution, rule syntax, corpus scheduling, and the content store.
The run uses a separate shared corpus cache with collection enabled.
Eligible cases report no refreshed strata. Comparisons with fresh execution cover repeated insertions, replacements, deletions, and empty results.
Fault injection covers errors during old-row restoration and after lattice support mutation. Both cases restore state and permit a later retry.

The following counters come from native regression tests. They include SQL schema management and scratch-table cleanup.
The fact-scan column excludes `CREATE` and `DROP` statements because their counters include SQLite schema scans.
These scan counters describe SQLite table scans. The VM budget also covers virtual-table cursor work.

| Case | Retained size | Edit | SQL VM instructions | Scan steps outside schema changes |
| --- | ---: | --- | ---: | ---: |
| Fourteen changed SQL inputs in one join | 20,000 rows per input | Delete one source row | 38,823 | 0 |
| Computed aggregate correlation | 100,000 outer contexts | Delete one fact | 2,460 | 5 |
| Min, max, and flat lattice folds | 100,000 candidates at one key | Delete both extrema | 10,071 | 21 |
| Retained tree walk | 1,000 Python files | Delete one file | 123,648 | 124 |
| Retained tree walk | 1,000 Python files | Insert one file | 109,843 | 49 |

The computed-key edit reconsiders one context. Routing tests probe one shard at both 10 and 1,000 files, including bounded node ranges.
Sentinel lookups preserve matches across every file. The original batch adapter remains the oracle for real native fact rows and probes.
Additional checks cover zero-key lattices, duplicate supports, retraction from top, registered deterministic joins, and reset of retained walk state.
A separate regression changes two lattice inputs together and checks dependent results after both old values disappear.

The 234-test parallel suite took 188.81 seconds before the routing-estimate correction and 11.69 seconds after it on the shared host.
The final 235-test run, including the simultaneous lattice replacement regression, took 27.45 seconds.
Host load affects this timing. The probe counts, scan bounds, VM budgets, and output comparisons provide the repeatable checks.
The headless library and engine binary pass `cargo check --locked --lib --features headless --bin rerail-engine`.

## Completion criteria for execution work

The scope includes all five remaining items. Completion requires evidence for each item.

| Item | Implementation direction | Completion evidence |
| --- | --- | --- |
| First-edit preparation | Prepare mutable inputs, materialized projections, and retained support during binding or initial execution. | First edits over large inputs need no migration or support initialization and pass a bounded work budget. |
| Recursive lattices and aggregates | Maintain affected dependency regions, invalidate cyclic support, and rederive from surviving external support. | Cyclic insertion and deletion sequences match fresh execution without a whole-stratum refresh. |
| Remaining rule shapes | Retain ordered string reductions, dependent aggregate keys, and walk blocks with subqueries or negation. | Each shape has repeated-edit, empty-input, rollback, and fresh-execution comparisons without refresh fallback. |
| Registered lattice joins | Retain a balanced reduction tree over candidate support and combine summaries with the registered merge. | A sparse edit at a large key processes a logarithmic number of summaries, including retractions from top. |
| Temporary-table overhead | Reuse typed scratch tables and indexes across operations. | Repeated edits create no scratch schema objects, leave no scratch rows, and preserve pool consistency after rollback and reset. |

The implementation and final checks below provide evidence for each requirement. Follow-up work is listed separately from these completion criteria.

### Preparation, reusable scratch state, and registered joins

The first edit previously paid three deferred costs: constructing mutable input indexes, migrating projection views, and initializing support.
Bindings now construct mutable indexed sources. Initial execution materializes projections and initializes retained reducers and walks inside its savepoint.
First-edit reports contain no migrations or support initializations. The initial run reports these costs under `preparation`.
This changes when preparation occurs; it does not make the initial support joins free.

Eager preparation exposed a planner regression in the original CPython `type_of` workload.
The new projection tables lacked statistics. One support join consumed 136,715,295 SQL instructions and exhausted the 200-million-instruction run budget.
Analyzing the materialized projection tables before initializing support fixes the plan. The original budget remains unchanged.

Scratch tables previously caused repeated schema changes, index creation, and cleanup.
A connection-local pool now reuses schemas by exact column definitions and primary-key layout.
Successful operations empty their scratch rows. Pool metadata commits with the same savepoint as facts and results.
Failure preserves the previous pool; reset drops it. Capacity follows the largest operation so far and remains allocated until reset.

Registered joins previously refolded every candidate at a changed key.
Each key now has a persistent SQL AVL tree over distinct supported candidate values.
Nodes cache subtree joins. Insertions and deletions update search paths and rotations, using the registered merge function.
Duplicate witnesses change counts without changing the tree. Removing the last top candidate restores the remaining summary.
The existing lattice laws and agreement between aggregate and merge remain required.

Verified SQL regression counters at this stage:

| Case | Retained size | SQL instructions | Data scan steps |
| --- | ---: | ---: | ---: |
| First actual edit, projection and reducers | 100,000 input rows | 2,380 | 5 |
| Fourteen changed SQL inputs, sparse deletion | 20,000 rows per input | 7,313 | 0 |
| Computed aggregate key, sparse deletion | 100,000 contexts | 947 | 5 |
| Min/max/flat extrema deletion | 100,000 candidates at one key | 2,610 | 21 |
| Registered join, distinct candidate deletion | 100,000 candidates at one key | 2,262 | 0 |
| Registered join, remove top and insert another value | 100,000 candidates at one key | 2,990 | 7 |

The registered join tests require fewer than 300 merge calls per edit; the tested value changes use 26–65 calls.
Cached SQLite statement counters are reset before profiling, so the measurements exclude work from earlier preparation.
Scratch tests check unchanged schema versions, zero new tables after warmup, empty cached tables, rollback, retry, and reset.

### Strings, dependent correlations, and walks

String aggregates now retain physical input tuples and support counts, preserving original text conversion and ordering.
An update refolds only touched groups through the existing `rr_join` function.
This avoids the stratum fallback. Rebuilding a large output string still reads and sorts that group's supported tuples.

Aggregate-dependent correlations previously failed the eligibility check because an outer key contained a scalar subquery.
Reducer planning now collects nested scalar sites in dependency order.
Separate tables index each context's evaluated dependent key. Updates probe old keys, mark dirty contexts, and replace their dependency rows.
This handles changes to the earlier aggregate, the later aggregate's input, and both in one batch.
The 20,000-context test stays under 30,000 SQL instructions and reconsiders fewer than ten contexts per sparse edit.

Walk CTEs previously formed one maintenance stratum, which prevented their seed and output blocks from using retained reducers.
The compiler now lifts those blocks into separate dependency components and rewrites correlated fields inside subqueries.
The recursive traversal remains positive; supported aggregates and negation run in its seed or output components.
Validity columns preserve nullable threaded values without introducing SQL NULLs into retained integer fact sets.

Validation passes 243 combined tests, with four ignored tests and the separate syntax census excluded.
The filters cover SQL execution, rule syntax, corpus scheduling, and the content store.
The headless library and engine binary also pass the compiler check.
The string-group edit over 20,000 groups uses 1,541 SQL instructions and five data scan steps.
Walk comparisons cover negative guards, empty min results, null-to-value transitions, source edits, failure after partial maintenance, and retry.
Registered-join tests include 240 deterministic edits with composite keys, AVL balance checks, full-reduction comparisons, and removal of every node.

Items 1, 4, and 5 have implementation and regression evidence. Item 3 covers the supported scalar forms described above.
Arbitrary subqueries remain outside retained reducer eligibility. Item 2 remains active and prevents completion of the full goal.

### Remaining work: recursive lattices and aggregates

This requirement is still unfinished. Components without a proven independent partition still refresh an affected stratum.
Retaining only final support counts is insufficient: a cycle can preserve itself after external support disappears.
Widening and aggregate evaluation also depend on fixed-point progress, so an ordinary count-based work queue needs more state.

The implementation must retain causal iteration or dependency information, invalidate affected recursive support, and rederive from surviving support.
It must preserve the current cold SQL kernel's results and widening behavior.
Iteration-indexed incrementalization is described in the primary [DBSP paper](https://www.vldb.org/pvldb/vol16/p1601-budiu.pdf).
That is a design reference, not a claim that the current engine implements its algorithm.

Required checks include cyclic shortest paths, mutually recursive lattices, accepted recursive aggregates, alternate support, retraction from top,
simultaneous input changes, empty components, budget rollback, and retries. Compare every update with a fresh execution.
Large disconnected regions must show bounded sparse-edit work. A whole-stratum recomputation must remain visible as a fallback until it is removed.

### Recursive partition maintenance and aggregate scheduling

Recursive components can now use an invariant key proven from their checked query trees.
For each rule, every recursive input must share the head's partition, including inputs read inside scalar queries.
Equality proofs use each subquery's own scope and its ancestors. Sibling subqueries do not supply assumptions to each other.
The partition column can differ between relations. The engine prepares required indexes during the initial run.

An edit finds affected partitions through positive witnesses in old and new inputs, including aggregate and negation read sites.
Old SQL inputs temporarily restore their exact membership. Native overlays conservatively include current and deleted rows.
The engine removes only selected partitions, clears their widening counters, and rederives them from bottom.
The cold kernel keeps its step order, dependencies, and semi-naive delta variants.
Closed partitions cannot supply support to each other, so this removes unsupported cycles and preserves cold widening behavior.
Output differences compare only the old and new rows of affected partitions.

The first sparse test exceeded its 50,000-instruction budget because SQLite reordered a join and scanned unrelated rows.
Partition constraints now reach equivalent outer fields explicitly. Cross joins keep the small partition table first.
The revised test uses 1,347–3,352 instructions per edit across 20,000 independent groups and 100,000 weighted edges.
Its triggers reject any deletion or insertion outside the selected group.
The budget remains 50,000 instructions; it was not raised.

The work also exposed a compiler scheduling error in mutually recursive aggregate components.
The emitter removed all own-component reads from full rule steps, assuming delta variants would wake them.
Aggregates have no positive delta atom, so a count rule could stop after its initial evaluation.
Full aggregate queries now depend on their recursive inputs and run again when those inputs change.
A regression builds five item values through repeated recursive counts and requires the resulting max lattice to reach five.

Constant fact steps now retain immutable private tables with checked select queries.
Maintenance replays their known initial statements once to capture their rows; it does not parse SQL text.
Constant facts can therefore share retained support with aggregate and lattice rules.
Nonrecursive lattice outputs can also lift walk blocks with supported aggregates and negation.

Tests cover unsupported cyclic support, alternate paths, empty partitions, simultaneous native and upstream SQL changes,
rollback during old-input restoration, wave-limit failure, retry, nonleading partition columns, widening, and retraction from top.
Another test changes a global recursive count and checks results across partition keys, where the invariant proof must not apply.
The targeted SQL and compiler suite passes 88 tests.
The final combined run passes 257 tests, with four ignored tests and the separate syntax census excluded.
It includes SQL execution, compiler emission, front-end checks, rule syntax, corpus scheduling, and the content store.
The headless library and engine binary pass the compiler check.

This is progress toward item 2. It does not remove the fallback for unpartitioned recursive dependencies or recursive walks that cannot be lifted.
The next extension must retain causal dependencies or iteration state for those components. Renaming a stratum refresh does not satisfy this requirement.

### Versioned storage for iteration state

General recursive maintenance remains unfinished. The next algorithm needs retained state for each iteration, including reducer support.
Copying SQL tables or initializing support for every new iteration would scan accumulated state again.
The engine now has an internal storage backend that shares immutable index roots between operator versions.
SQL still evaluates rules and applies their deltas through writable temporary virtual tables.
At this storage milestone, the public session still used its existing tables.
The next section describes the recursive session integration.

Each relation has persistent indexes for complete rows, primary keys, row IDs, and individual columns.
Writes copy affected search paths. Active cursors pin their version before reading.
Canonical tree hashes identify unchanged subtrees, including versions built in different insertion orders.
A sparse comparison over 20,000 rows visits 94 tree nodes, including split traversal and emitted rows.
Ordered primary-key probes read minima and maxima lazily. Two probes over one 20,000-row group read fewer than 20 columns.
SQLite still checks additional filters before applying `LIMIT`.

Retained count, sum, min, and max operators can use this backend, including materialized computed correlation keys.
Forking an operator shares its result and support indexes. It rebuilds checked descriptors without initializing support again.
A 2,000-context regression applies insertions and retractions to independent forks and compares results with fresh SQL aggregates.
Each edit reconsiders fewer than three contexts. Saved support versions remain unchanged.
Scratch scopes reuse temporary tables between sequential operator updates while preserving the caller's input deltas.

Two integration failures had specific causes:

| Failure | Root cause | Fix |
| --- | --- | --- |
| Volatile state triggered the content store's durable-write guard. | Writable eponymous tables belonged to `main`. | Create writable virtual tables in `temp`. |
| `INSERT OR IGNORE` failed on duplicate keys. | The pinned rusqlite configuration helper omitted the required constraint-support flag. | Call `sqlite3_vtab_config` with the explicit flag and reject constraints before mutation. |

The constraint protocol follows [SQLite's virtual-table configuration API](https://www.sqlite.org/c3ref/c_vtab_constraint_support.html).
Native state uses an explicit Rust transaction guard to restore previous roots and remove tables created by a failed batch.
Ordinary SQL rollback alone does not restore this backend. Session integration must restore native roots too.

The combined suite passes 268 tests, with four ignored tests and the separate syntax census excluded.
It covers SQL execution, compiler emission, front-end checks, rule syntax, corpus scheduling, virtual-table bindings, and the content store.
The headless library and engine binary pass the compiler check.
These checks validate the storage and operator foundation; they do not validate an iteration-indexed recursive executor.

Next, retain iteration inputs, operator caches, published relations, frontiers, and widening counters.
Update each frame from input differences. Fork the preceding cache when execution needs another iteration.
Preserve the cold scheduler's gates and widening order, then publish only actual final differences.
Integrate native rollback and cleanup before enabling this path in sessions.
Compare cyclic and mutually recursive programs against fresh execution after every edit, including failures and retries.
Measure native index work as well as SQL instructions: SQL counters exclude native hashing, allocation, and index traversal.
That milestone left the visible stratum-refresh fallback for unpartitioned recursive lattices and aggregates.

### Retained recursive iterations

The session now prepares iteration state for supported recursive lattice and aggregate components.
This includes components with a proven partition key. They no longer rederive a whole affected partition.
Each frame retains its preceding inputs, rule-result caches, published rows, frontier rows, and widening counters.
SQL evaluates rule deltas and folds candidate values at changed keys.

Every recursive read uses the preceding frame's version. A cycle cannot supply support to its own initial frame.
Edits propagate through retained frames, including retractions, then publish actual output differences.
An initial comparison rejects any trace whose final rows differ from the cold SQL kernel.
Rules retain the kernel's original pending gates, step order, full queries, and semi-naive variants.
Constant seeds remain in stratum zero, including when the program also lifts a tree walk.

Inactive rule caches record net external changes. They evaluate their queries when the original gate becomes active.
Each trace also retains a closing frame with an empty final frontier.
Positive contexts can then be cleared by replacing native roots, without enumerating an obsolete frontier.
This clearing preserves scalar input indexes and changes still needed by those indexes.
Unused tail frames remain available for later edits and do not consume the wave limit.

Frames share fixed SQL table names and schemas. Scope activation changes their bound native roots.
Forking another frame shares persistent indexes and clones its descriptors, without SQL DDL or row copying.
The session coordinates native undo logs with SQL savepoints.
Failed batches restore changed roots, deleted tables, and tables created while extending the trace.
Cleanup also handles SQLite rolling back the SQL transaction before native rollback runs.
Writable modules use ordinary temporary virtual tables, without an implicit eponymous table after the schema disappears.
Rollback restores the original binding cell when SQLite revives an earlier table instance.

The tests cover shortest paths, alternate support, unsupported cycles, longer paths, negative cycles, and retraction from top.
They compare published values and widening counters with fresh execution after edits.
Other cases cover mutually recursive global counts, physical sentinel columns, registered joins, empty keys, and recursive walks with scalar and negative blocks.
Failures during publication and wave-limit failures restore the previous state and permit retries.
Forty deterministic mixed graph edits also match fresh execution.
Ordinary and recursive sums retain unused overflowing groups without failing preparation.
Reading an overflowing group fails atomically; correcting its inputs permits a retry.
These tests cover both signs, fresh-execution comparisons, and native-state rollback.

Sparse edits over 50,000 weighted edges and 10,000 disconnected regions have these measured costs:

| Edit | SQL instructions | Data scan steps | Native columns read | Version-diff nodes |
| --- | ---: | ---: | ---: | ---: |
| Delete an alternate path edge | 9,870 | 59 | 220 | 1,043 |
| Remove the region's root | 12,581 | 218 | 223 | 1,346 |
| Restore the root and edge | 17,397 | 284 | 529 | 1,600 |

The instruction limit is 50,000. Triggers reject publication changes outside the selected region.
Native column reads and version comparisons are counted separately because SQLite counters exclude native index work.
A recursive count with negation over 3,000 groups uses 26,939–36,573 instructions and 113–356 scan steps per edit.
That test extends the old iteration history, removes support, changes the negative input, then restores support.
It also rejects writes to unrelated count groups.

Integration exposed additional root causes:

| Finding | Root cause | Fix |
| --- | --- | --- |
| One retained CPython join consumed 119 million instructions. | Moving the delta first left an unrelated table before the next connected join. | Apply equality-based join ordering to retained context updates. |
| CPython preparation exceeded its existing 200-million-instruction limit. | Each frame repeated schema changes and SQL preparation. | Reuse fixed SQL slots and fork their native roots within separate scopes. |
| A sparse count edit exceeded 50,000 instructions when it needed another round. | A new cache inherited thousands of unrelated rows from the last nonempty frontier. | Retain a closing frame and clear provably empty positive contexts by roots. |
| An unused sum group failed an otherwise valid run. | Scalar preparation narrowed every group to 64 bits before a rule read it. | Retain exact 128-bit sums in two SQL integer columns; check the public range during context lookup. |
| Lifting an unrelated walk changed widening. | Dependency ordering moved constant seeds into the recursive stratum. | Preserve the original stratum-zero seed timing. |
| Recursive `split_dot` could not retain its operator. | The retained planner did not recognize the immutable integer domain. | Treat `rdl_int` as a positive source that never supplies an input delta. |
| A generated ancestor walk retained hundreds of thousands of intermediate rows. | Each seed carried every candidate function before testing the actual ancestor. | Move an independent candidate lookup to an existence check at the terminal ancestor. |
| Native preparation hashed every secondary-index path. | Secondary indexes computed Merkle hashes although comparisons use complete rows only. | Hash complete-row trees; retain secondary indexes without subtree hashes. |
| Cleanup hid an interrupted query behind a virtual-table drop error. | SQLite had already removed the temporary schema, leaving the module registered. | Check the actual schema and restore original module bindings during rollback. |

The ancestor rewrite checks that the candidate stays passive throughout the climb.
It preserves implicit column names, duplicate target rows, empty target sets, and distinct seed values.
Correlated candidates and candidates used during traversal are excluded from this rewrite.

The eligibility audit distinguishes public rule syntax from arbitrary checked SQL:

- Aggregate bodies with joins already lower to helper relations. Retained execution handles those helpers, including recursive bodies.
- Tested aggregate-dependent correlation expressions retain their dependency indexes. This is not a blanket guarantee for arbitrary SQL scalar subqueries.
- Generated nearest walks have positive traversal arms. Supported scalar queries and negation occur in their seeds or outputs.
- Dynamic nearest stop sets are rejected by the rule lowerer; they are not an accepted SQL maintenance shape awaiting retention.
- The standalone session binds all rule inputs as batch sources in atom mode. It does not emit the product planner's document-range optimization.
- Arbitrary checked CTEs and unsupported scalar shapes keep the existing explicit refresh or partition fallback.
  Unsupported retention alone does not fail a previously accepted program. Real evaluation errors and budget failures still propagate.

### Optimized measurements and final validation

The benchmark driver now includes recursive lattice edits at 1,000 and 10,000 disconnected regions.
Each case runs the same removal and restoration sequence twice, checks output deltas, and rejects unrelated publication writes.
The second cycle must create no scratch tables. Every edit must stay below 50,000 measured SQL instructions.
The reducer benchmarks also require no first-edit support initialization.

These are single runs of the optimized `profiling` build, with SQLite statement tracing enabled:

| Case | Initial execution CPU | Initial peak RSS | Edit CPU range | Edit SQL instructions |
| --- | ---: | ---: | ---: | ---: |
| 5,000 edges, 1,000 regions | 565 ms | 125.5 MiB | 3.45–4.47 ms | 9,754–17,397 |
| 50,000 edges, 10,000 regions | 6,754 ms | 1,123.1 MiB | 3.98–6.26 ms | 9,754–17,397 |
| Reducers over 10,000 rows | 51 ms | 26.5 MiB | 0.93–2.66 ms | 5,007–6,879 |
| Reducers over 100,000 rows | 500 ms | 130.9 MiB | 1.10–2.58 ms | 5,007–6,879 |

CPU time excludes other processes. Wall time varied under concurrent builds and is not used for these comparisons.
Peak RSS includes the process, retained session state, and SQL trace data. It is not a live-heap measurement.
An optimized standalone CLI run without statement tracing confirms the memory cost: 1,114.5 MiB maximum RSS.
Its full request takes 7.52 seconds CPU, including binding, initial execution, six edits, queries, and teardown.
The same final outputs and scratch-reuse checks pass, so tracing does not explain the large peak.
Recursive scratch allocations were `4, 7, 2` on the first cycle and `0, 0, 0` on the second.
The reducer case created 65 scratch tables on its first edit, then reused them for all six later edits.
New scratch shapes still allocate capacity. Repeated shapes reuse it until reset.

The final combined run passes 286 tests, with four ignored tests and the separate syntax census excluded.
The 100,000-edge cyclic path regression also passes its large fresh-execution, rollback, and retry comparisons.
That large run preceded the final sum-only change; the final combined run and 16 focused reducer tests cover that change.
Both ordinary and recursive unused-overflow examples now pass, including a standalone CLI check.
The headless engine and optimized benchmark probe build successfully.
The structural gate reports the same 15 failing metrics as `HEAD`, with no new findings.

The five acceptance criteria have implementation and regression evidence:

| Requirement | Evidence |
| --- | --- |
| First-edit preparation | Large-input edits report no migration or support initialization; reducer instruction counts stay unchanged at 10,000 and 100,000 rows. |
| Recursive lattices and aggregates | Cyclic edits match fresh values and widening counters, including unpartitioned dependencies, global counts, registered joins, and failure recovery. Supported components publish no refreshed strata. |
| Remaining rule shapes | Ordered strings, aggregate-dependent keys, and generated walks with scalar or negative blocks retain state through repeated edits, empty inputs, rollback, and fresh-execution comparisons. |
| Registered lattice joins | Large-key tests update balanced summary paths, preserve duplicate support, retract top, and restore state after errors. |
| Temporary-table overhead | Repeated edit shapes reuse scratch schemas. Extending recursive histories forks native roots within fixed SQL slots. Failure and reset checks cover cleanup. |

### Follow-up work

Initial recursive execution is the clearest remaining performance problem.
The session runs the cold kernel, then separately constructs and verifies a retained trace.
Dense transitions also build persistent indexes through individual writes and retain multiple iteration versions.
The 50,000-edge measurements show the resulting CPU and memory cost; sparse edits are much cheaper.

1. Capture retained state during cold execution, so the session does not evaluate another complete trace.
   Add bulk index construction for initial loads and dense transitions. Preserve canonical roots, stable cursors, and native undo.
   Validate values, gates, frontiers, and widening counters against the current cold oracle before removing the duplicate preparation pass.
   Repeat both the phase benchmarks and the untraced standalone memory measurement.
2. Extend the failure-behavior audit beyond sums, especially aggregate-dependent keys and arbitrary checked expressions.
   Unused sum groups now have regression coverage. Deferring an inactive rule query alone does not prove every initialization path safe.
3. Extend retention where arbitrary checked SQL needs it. Keep fallback visible until that shape has correctness and sparse-work evidence.
4. Reduce history traversal after changes stop propagating, while preserving gates, frontiers, counters, and failure behavior.
   Retain external input versions or revision journals so skipped frames can later consume their accumulated changes.
   Skipping frames without that accounting would leave stale scalar indexes.

These follow-ups are distinct from first-edit migration. Preparation, registered joins, string reductions, correlations, and scratch reuse now meet the stated checks.

### Shared profiling and initial recursive preparation

SQL extension sessions now use the existing query catalog, trace, EXPLAIN reporter, and logic CPU spans.
Headless builds support this capture without the workspace profiling feature.
The benchmark reuses the installed callback, so its counters cannot replace the shared trace.
Compiler search records describe SQL emission. Runtime records separately identify cold rules, retained frames, support initialization, and merge operations.
API error records retain the failure reason. Teardown explains the final atom writes before the connection closes.
Temporary old-input bindings receive plans before release. Diagnostic EXPLAIN does not consume the execution instruction allowance.

The initial capture attributed most recorded CPU to retained rule caches and merge/widen writes.
The cold SQL kernel was a much smaller cost on the disconnected-path example.
Native versions maintained a secondary tree for every column, including a redundant leading-column tree.
Wide context and contribution tables copied those trees during each insert, delete, and frame transition.
These native costs occurred inside SQL callbacks, so unchanged VM counts concealed their size.

The implementation now makes these changes:

- Leading-column probes use the complete-row tree and retain exact distinct counts.
- Known retained operator and support tables keep the secondary indexes their consumers require.
- Checked query predicates select recursive input indexes separately for each alias, including scalar subqueries.
- Context, correlation, and contribution access paths are prepared before the first edit.
- Secondary index construction sorts entries, then builds the canonical tree in one pass.
- Compound-key `IN` probes descend through existing prefixes and avoid a Cartesian product of candidate keys.
- SQLite rechecks tuple correlation and other predicates. Native candidate inspections appear in the shared counters.
- Forks, frozen readers, empty versions, and native rollback preserve the selected index configuration.

The compound-key path is necessary for sparse deletes after index pruning.
Choosing only the leading group column would otherwise scan a large group when the remaining key fields use `IN`.
A 20,000-row regression checks mixed affinity, nulls, tuple correlation, rollback, and bounded native candidate work.
Another regression compares bulk tree shape, hashes, and subsequent edits with individual canonical insertions.

Measurements use the optimized `profiling` build on macOS arm64.
Each cell is the median of three fresh processes with identical request JSON.
Shared profiling was disabled; the benchmark retained its lightweight SQLite counter callback.
Other compiler jobs ran concurrently, so CPU and RSS are more useful here than wall time.

| Independent regions / weighted edges | Initial CPU before | Initial CPU after | Peak RSS before | Peak RSS after |
| --- | ---: | ---: | ---: | ---: |
| 1,000 / 5,000 | 0.511 s | 0.321 s | 125 MiB | 87 MiB |
| 10,000 / 50,000 | 5.128 s | 3.750 s | 1,128 MiB | 706 MiB |

The larger case uses 27% less initial CPU and 37% less peak memory.
Initial SQL instructions remain exactly 10,579,437 and 104,701,629 at the two sizes.
Each six-edit sequence retains the same 9,754–17,397 instruction range.
The larger case takes 2.03–4.32 ms CPU per edit after the change.
Both sizes validate initial answers, every output delta, unaffected components, and repeated scratch reuse.

Separate final captures cover 4,494 and 4,518 SQL executions from session start through teardown.
Both have complete receipts, no running statements, no unexpected explanation failures, and no incomplete native operations.
Deferred CREATE explanations still report expected schema errors after their tables exist.
The diagnostic reporter preserves the complete-job 100-execution allowance and reports these jobs as over budget.
It does not import the benchmark's answer checks as a framework parity oracle or grant acceptance.

Requests, binaries, counters, and reports are under `/tmp/rerail-sql-evaluation/`:

- `sql-recursive-before-repeat` and `sql-recursive-after-repeat` contain the repeated measurements and validated answers.
- `sql-recursive-after-profile/job-23091/report/profile.html` contains the larger shared capture.
- The frozen baseline binary is `sql-profile-before-bench`, SHA-256 `b8a032c8e5dd373532bc5170d6e49f093a295e688b92e506fbd043be29a11fe0`.
- The final measured binary is `sql-recursive-final-bench`, SHA-256 `557a2e649dce57bf787045162dbe327e7d11d34342abc99d6d5e1767c40938f2`.

The baseline contains the profiling integration before index changes, based on `f36d9460d`.
The measured final binary also contains the teardown explanation fix.

Validation passed: 185 tests in the broad Rust selection, plus the separately run recursive count and 100,000-edge regressions.
The final isolated profiling test also checks native candidate work and complete teardown explanations.
All 15 report tests pass, and the library compiles with the workspace profiling feature.
The structure gate retains its 10 passing metrics and 15 existing failures, with no new findings.

The next performance changes remain substantial:

1. Capture retained state during cold execution and remove the second complete trace evaluation.
   Preserve wave gates, seeds, frontiers, widening counters, and failure behavior before removing the cold comparison.
2. Extend bulk construction to complete-row and row-id roots during initial loads and dense transitions.
   These paths still use individual persistent writes.
3. Reduce repeated helper statements across operators and frames.
   CPU improvements alone do not meet the shared complete-job execution allowance.
4. Add a SQL-session parity adapter to the existing acceptance report.
   Keep diagnostic reports distinct from accepted benchmark evidence until that adapter verifies complete outputs and failures.


### Single initial schedule, bulk roots, and helper statements

This work merges Final rerail shape at `9993e7e5f`, through merge commit `e830fafae`.
The changes below supersede the preceding initial-execution follow-ups for supported recursive programs.

The executor previously ran the cold kernel, then evaluated a second retained trace.
The new path constructs retained state and produces the initial answer through one global wave schedule.
A global schedule is necessary because dependency gates cross component boundaries.
Constants, pending frontiers, widening counters, and the closing empty frame retain their original timing.
Unsupported mixed programs keep the explicit cold fallback. They can still incur separate retained preparation.
The preparation report identifies the selected path through `initial_execution`.

Dense initial joins previously inserted persistent rows and indexes individually.
Bulk loaders now construct complete-row, primary-key, row-id, and required secondary trees from sorted entries.
Unchanged rows and branches retain their shared pointers. Row IDs and frozen cursors remain stable.
Initial join caches derive their results from retained heads, so they avoid another complete result query.
The initial dense path does not replace sparse maintenance on edits.

Small native lookups also copied version metadata and repeated tree traversal.
Frozen readers now share an immutable version pointer. Point probes return shared cells directly.
Large contiguous scans use a frozen iterator.
Scratch snapshots share roots without a scope change. Delta indexes initialize only when a lookup needs them.

Native journals compose set changes directly. Empty keys, folds, and unchanged caches avoid redundant SQL.
Candidate queries and wave statistics publish in batches.
A warm-walk regression exposed a missing index on a nonleading bound column.
The emitter now prepares that access path. The regression returns to zero full-scan steps.

An upstream edit can shorten a gate and activate an earlier frame that never initialized its operator cache.
Initial templates retain the operator roots for this case.
The frame preserves its external journals and reconciles all edits against the template before evaluation.
Nested native and SQL savepoints restore failed initial attempts without scanning existing sessions.

#### Measurements

The workload contains independent cyclic paths, a distance lattice, and six edits across repeated change shapes.
Both sizes validate initial answers, each output delta, unaffected regions, and scratch reuse.
`benchmark/sql-executor.py` generates these cases with `--only update-recursive-lattice --repeats 3`.
The runner is the `sql-executor-bench` example.
The optimized `profiling` build runs on macOS arm64.
Each value below is the median of three fresh processes. Other compiler jobs ran concurrently.
The baseline is `ddf330ef8`, with the preceding index and profiling changes already present.

| Regions / edges | Initial CPU before | Initial CPU after | Peak RSS before | Peak RSS after |
| --- | ---: | ---: | ---: | ---: |
| 1,000 / 5,000 | 0.419 s | 0.254 s | 86.6 MiB | 87.0 MiB |
| 10,000 / 50,000 | 5.180 s | 2.876 s | 700.6 MiB | 692.2 MiB |

The larger case uses 44% less initial CPU. Peak memory remains close to the baseline.
Retained wave versions still dominate memory. Bulk construction reduces write cost but does not remove that history.

| Counter | Before | After |
| --- | ---: | ---: |
| Initial SQL, 5,000 edges | 740 | 198 |
| Initial SQL, 50,000 edges | 764 | 198 |
| Initial VM instructions, 5,000 edges | 10,579,437 | 3,341,880 |
| Initial VM instructions, 50,000 edges | 104,701,629 | 32,375,880 |
| Statements per edit | 563–732 | 121–197 |
| VM instructions per edit | 9,754–17,397 | 4,351–9,177 |
| Complete shared capture, 5,000 edges | 4,494 | 1,173 |
| Complete shared capture, 50,000 edges | 4,518 | 1,173 |

The shared captures include session setup, initial execution, edits, queries, and teardown.
Both captures have complete receipts, no running statements, no unexpected explanation failures, and no incomplete native operations.
The larger capture contains 97 setup statements, 18 transaction statements, and 1,058 remaining statements.
The fixed complete-job allowance remains 100. Both captures still fail that allowance.
The diagnostic report also lacks a framework parity adapter, despite the separate benchmark answer checks.
These results demonstrate improvement. They do not grant framework performance acceptance.

The large request has SHA-256 `fc49371c11f55f9abc96d7a437426bff9055fba993eb7aa66457449043c7d5e8`.
The baseline binary has SHA-256 `557a2e649dce57bf787045162dbe327e7d11d34342abc99d6d5e1767c40938f2`.
The measured binary has SHA-256 `6e6f2b233adfb4b516c149c82bbfbd06ffcdd6e7d4189132349422da480bd5eb`.
The final source additionally checks that bulk expressions reject noninteger values, matching row insertion through the virtual table.

#### Validation

The final source passes 125 SQL-related Rust tests, with one child-process test ignored by design.
The focused storage, initial schedule, and profiling selection also passes all 22 tests.
Separate optimized runs pass the recursive count case and the 100,000-edge recursive lattice case.
All 15 report tests pass. The library compiles with `code-rule-profile` enabled.
The structure gate retains 10 passing metrics and 15 existing failures, with no new findings.

The independent cold oracle compares values, wave counts, positive frontiers, and widening counters.
A two-batch edit checks scalar journals before an upstream deletion activates an earlier frame.
Other regressions cover native savepoints, stable row IDs, shared roots, frozen cursors, and integer type errors.
Sparse work limits, failure recovery, projection views, and session reset also pass.

#### Remaining work

1. Fuse the context, contribution, head, and fold queries within each operator phase.
   The remaining fixed overhead repeats across rules and frames, even when deltas contain few rows.
   Preserve SQL expressions and native function semantics. Do not replace rule execution with an opaque native evaluator.
2. Reduce private schema and control setup. Setup alone consumes almost the complete allowance in the larger capture.
   Reuse fixed slots and publish control records in batches. Include cleanup in each complete measurement.
3. Add a SQL-session parity adapter to the shared acceptance report.
   The adapter must verify complete outputs, edit deltas, and failure behavior before acceptance.
4. Extend the single initial schedule to unsupported mixed shapes, with cold-oracle comparisons for each new shape.
   Keep the fallback visible until those checks pass.

The preceding cleanup removed its build targets and `/tmp/rerail-sql-*` artifacts at the user's request.
This includes the historical artifact paths above. The measurements and identities remain in this plan.


### Continuation: SQL phases and complete-state parity (2026-11-08)

The goal remains active. The complete-job allowance is still 100, and the current captures do not meet it.
The four remaining items above retain their original scope.

#### Implemented changes

The fold now uses one SQL statement with materialized CTEs for candidates, merged values, frontiers, and widening counters.
A tagged result stream returns all affected outputs.
Native code validates every patch before it publishes any root.
A failing expression or invalid later output cannot leave an earlier output changed.
Materialized expressions execute once even when several outputs use them.
Improvement-cache patches also use this mechanism.
SQL phases can return their own key sets alongside row patches.
Improvement queries now compute those keys directly, without separate key-table inserts.

Private frame inputs, outputs, frontiers, counters, and improvement caches now use read-only eponymous modules.
Their native owners can replace roots without CREATE or DROP statements.
Forks and rollback preserve the access mode, schema, binding cell, and row IDs.
Native index preparation also works for these read-only bindings.
Writable context caches and physical scratch tables still need further conversion.

Acyclic positive CTEs with arbitrary names now participate in the initial retained schedule.
The final SELECT must use DISTINCT. The new path excludes function calls, scalar subqueries, aggregates, and recursive CTEs.
Bag-sensitive and other unsupported shapes retain the existing fallback.
Cold-oracle checks compare initial rows, positive frontiers, widening counters, rollback, and sparse edits at two input sizes.

The shared report now accepts evidence from the SQL-session parity adapter.
The headless probe captures complete explicit facts, derived relations, and widening counters after every operation.
The independent oracle rebuilds from current facts through the original cold kernel in a separate, unprofiled process.
The adapter compares full states, reported output deltas, expected failures, rollback, recovery, and final queries.
Decimal strings preserve exact integer cells.

The bundle pins request bytes, runner bytes, both observations, and every raw capture file through SHA-256 digests.
Successful SQL receipts bind each complete-state checkpoint to its request and row count.
Missing checkpoints, altered files, incorrect deltas, and truncated queries prevent acceptance.
Snapshot, stored-fact, and file-edit requests need separate replay support. The current probe rejects them.
Verification reads have a separate inventory phase and remain part of the fixed allowance.

#### Current evidence

The new request includes an instruction-limit failure, a wave-limit failure, six successful edits, and nine complete-state checkpoints.
Its counts are not directly interchangeable with the preceding request, which lacked the failure probes and full-state reads.
The 5,000-edge baseline contains 1,190 submitted statements.
Fold fusion reduces that count to 954. Improvement fusion and private read-only roots reduce it further to 914.
The next change computes improvement keys inside the phase statement.
It reduces the two sizes to 847 and 843 statements.
Both final captures pass full-state parity and receipt coverage.
Both still fail the unchanged 100-execution allowance.

The final captures contain 90 setup statements each.
They contain 816 relational-evaluation statements and nine verification reads each.
Transaction and cleanup paths account for the remaining size-dependent difference.
These are unoptimized development captures. No new CPU or memory improvement claim follows from them.
The initial run now uses 141 statements, compared with 198 at the start.
Initial VM work increases from 3,341,880 to 3,460,361 instructions at 5,000 edges.
At 50,000 edges, it increases from 32,375,880 to 33,565,361 instructions.
The computed-key stream and CTE materialization trade extra VM work for fewer submissions and native helper roots.
Final unprofiled CPU and memory measurements remain necessary.

The larger capture attributes 499 edit statements to rule caches, 65 to recursive updates, and 58 to fused phases.
Another 62 statements cover API publication, control updates, and transaction work.
Setup includes 61 CREATE VIRTUAL TABLE statements, 19 CREATE TEMP TABLE statements, nine DROP statements, and one PRAGMA.
These counts identify the remaining fusion and storage targets.

The focused phase, storage, CTE, and recursive checks pass.
The broader SQL, virtual-table, and profiler selection passes 120 tests, with one child-process test ignored by design.
All 24 reporter and parity tests pass.
Both optimized scale regressions pass, including the 100,000-edge lattice case.
The library compiles with `code-rule-profile` enabled.
The structure comparison retains 10 passing metrics and 15 existing failures, with no new findings.
The complete profiling gate passes 38 Node tests and 82 Python tests.
That gate also exposed a stale vocabulary-test path from the merge.
The test now reads the retained D SQL baseline, with its comparison role stated explicitly.
Tests include native function evaluation counts, atomic publication, read-only binding rollback, and sparse CTE maintenance.

#### Completion audit and next implementation

| Requirement | Current state | Evidence still required |
| --- | --- | --- |
| Fuse context, contribution, head, and fold execution | Fold and improvement phases are fused. Context, contribution, and head queries remain separate. | Full operator or frame fusion, complete parity, bounded warm work, and captures within 100 executions. |
| Reduce private schema and control setup | Read-only frame roots avoid DDL. Remaining caches and scratch tables still incur setup. | Conversion of remaining private storage and batched control publication, including teardown measurements. |
| Add shared SQL-session parity | The adapter verifies explicit fact workloads, including failure and recovery. Other publication methods remain unsupported. | Broader replay support where required, plus final accepted captures after the engine meets its budget. |
| Extend initial execution for unsupported mixed shapes | Positive acyclic CTEs now retain state. Other shapes keep an explicit fallback. | Further checked shape families with independent cold comparisons and sparse-work evidence. |

The next planner change must combine context transitions, signed contributions, head counts, and folds within a shared SQL phase.
The computed-key phase API now supports sparse patches without helper statements for each table.
Independent operators can then share a frame statement while SQL preserves their preceding-wave inputs.
Scalar reducers and registered lattice functions must retain their existing semantics and failure behavior.
A native evaluator hidden behind one function call would not satisfy this requirement.
Inactive rules and unused scalar groups must not evaluate checked expressions earlier than the cold kernel.
Warm lookups must retain their prefix indexes and bounded candidate work.

Remaining writable caches can become read-only native bindings after their SQL writes become phase outputs.
Control-table creation must move out of repeated edit paths. Wave diagnostics can publish in batches.
All setup, diagnostics, verification, failure cleanup, and teardown statements must remain visible in the final capture.

The requests are reproducible through `benchmark/sql-executor.py --session-request REGIONS OUTPUT.json`.
The shared verifier runs through `scripts/profiling/sql-session.mjs VERIFIER REQUEST.json OUTPUT_DIRECTORY`.
A successful goal audit still requires complete accepted runs at both 1,000 and 10,000 regions.

### Completed continuation: shared SQL wave phases (2026-11-08)

This section supersedes the preceding completion audit. All four items from the remaining-work list are complete within the checked shape boundaries below.
The fixed complete-job allowance remains 100.

#### Changes and root causes

Each eligible wave now uses one SQL statement for contexts, contributions, support counts, improvements, frontiers, values, and widening counters.
Materialized CTEs retain SQL expression evaluation. Native code validates all replacement roots before publication.
Dense initial transitions rebuild complete roots through SQL result streams. Sparse edits keep keyed patches and persistent indexes.
No native function hides rule evaluation or AST matching.

The first cache fusion exposed an incorrect ordered-scan estimate.
An equality constraint on a nonleading key could select an ordered reader that scanned unrelated groups.
Ordered readers now require a leading equality prefix. Other constraints use selective indexes and sort their matches.
The sparse recursive regression no longer exceeds its 50,000-instruction limit.

Private contexts, contributions, heads, outputs, pending journals, and fact deltas now use read-only bindings where SQL writes are unnecessary.
The initial path creates public outputs and wave diagnostics first. It declares cold control tables only when the fallback needs them.
Output collision checks share one catalog read. Demand-module estimates share one statistics query.
Statistics tables persist across edits, and initial wave diagnostics publish in one batch.
Widening diagnostics share the final retained roots. Requested secondary indexes use sparse maintenance and survive rollback.
Public result writes still run through SQLite, including extension triggers.

Repeated preparation initially increased CPU for small edits, despite fewer SQL executions.
Phases now reuse prepared statements. Successful edits retain those plans only when every execution uses complete, checked fixed-probe phases.
Extra views, unsupported shapes, failures, and schema resets retain the full cache reset.
Cached statements reopen the current native roots. They do not cache rows or expression results.

Positive acyclic CTEs now accept multiple UNION or UNION ALL arms.
Their final consumer must use DISTINCT. Function calls, scalar queries, aggregates, recursion, and forward references retain their established fallback.
Cold-oracle tests cover single-arm, UNION, and UNION ALL cases at 32 and 512 regions.
They compare values, frontiers, widening counters, rollback, and bounded sparse edits.
Edits change recursive edges and delete or reinsert the CTE root input.
Scalar reducers and registered lattice joins retain their existing SQL execution paths and failure behavior.

#### Complete-session acceptance

Both cases contain two expected failures, six successful edits, nine complete-state checkpoints, and a final query.
The adapter checks all explicit facts, derived rows, widening counters, output deltas, rollback, and recovery against independent cold executions.
Both reports have complete receipt coverage, zero unexpected explanation failures, and zero incomplete operations.
Two deferred setup explanations fail as expected after their CREATE statements complete.

| Regions / edges | Previous checkpoint SQL | Final complete-job SQL | Initial SQL | SQL per successful edit |
| --- | ---: | ---: | ---: | ---: |
| 1,000 / 5,000 | 847 | **100 / 100** | 16 | 10 |
| 10,000 / 50,000 | 843 | **100 / 100** | 16 | 10 |

The complete inventory contains 39 phase statements, 24 transaction statements, nine verification reads, and eight result-delta writes.
It also contains seven statistics writes, four setup statements, four catalog/statistics reads, and two atom-flush statements.
Initial result publication, initial wave diagnostics, and the final query add one statement each.
All these statements count toward the allowance. There are no SQLite internal executions in these captures.

Successful edits use 4,538, 6,270, and 9,936 VM instructions across the three repeated change shapes.
Those values remain identical at both input sizes.
Initial VM work increases from 3,460,361 to 5,876,559 instructions at 5,000 edges.
At 50,000 edges, it increases from 33,565,361 to 58,724,559 instructions.
CTE materialization, intermediate grouping, and tagged result streams add SQL work while reducing submissions and native storage work.
The CPU measurements below establish the net result; statement counts alone do not establish a speed improvement.

#### CPU and memory

The comparison uses checkpoint `6e0a8735d` and the final engine, with identical benchmark requests and probe code.
The baseline probe receives only the new option to disable SQL trace callbacks.
`RERAIL_SQL_BENCH_UNPROFILED=1` disables those callbacks; both shared profiling environment variables are unset.
Every trial validates results, reported deltas, unrelated-region guards, and scratch reuse.
Each size uses three alternating baseline/final pairs in fresh processes.
These are unoptimized development builds with debug information and incremental compilation disabled.
The compiler is Homebrew Rust 1.97.1 on macOS arm64. Other worktrees ran jobs concurrently.
These values must not be compared directly with the earlier optimized timings.

| Regions / edges | Initial CPU before / after | Six edits CPU before / after | Peak RSS before / after |
| --- | ---: | ---: | ---: |
| 1,000 / 5,000 | 1.738 / 1.266 s | 57.7 / 48.2 ms | 89.5 / 85.6 MiB |
| 10,000 / 50,000 | 20.100 / 14.811 s | 61.6 / 60.6 ms | 687.0 / 638.9 MiB |

Initial CPU decreases by 26–27%. Peak RSS decreases by 4–7%.
The larger case's small-edit CPU remains close to baseline. The measurement does not establish a material large-case edit speedup.
The earlier version without statement reuse increased edit CPU by about 40%; that regression prompted the preparation changes above.

#### Validation and boundaries

All 129 SQL, virtual-table, demand, and profiling tests pass, with one child-process test ignored by design.
The selection includes the 100,000-edge lattice regression, recursive counts, six CTE shape/size combinations, and failure recovery.
Phase tests verify fresh roots on cached executions, one expression evaluation per execution, and atomic publication after invalid outputs.
Diagnostic alias tests verify shared roots, stable row IDs, index retention, and rollback.
The profiled library also type-checks with the repository's Rust 1.95 toolchain.
All 38 Node and 82 Python profiling tests pass.
The structure gate retains 10 passing metrics and 15 existing failures, with no added findings.

| Requested item | Outcome |
| --- | --- |
| Fuse context, contribution, head, and fold execution | Complete for checked positive, call-free caches; other shapes keep their SQL paths. |
| Reduce private schema and control work | Complete for this path; both full lifecycle captures meet the unchanged allowance. |
| Verify SQL-session parity through the shared report | Complete for explicit-fact sessions, including full states, deltas, expected failures, and recovery. |
| Extend the initial schedule to new mixed shapes | Complete for positive multi-arm CTEs, with cold-oracle and sparse-work checks at both sizes. |

Snapshot, stored-fact, and file-edit replay remain outside this parity adapter's accepted request format.
The adapter rejects those requests explicitly. Their executor paths retain separate regression coverage.
Cold fallback remains visible for unsupported query shapes.

Further performance work can reduce CTE materialization and grouping costs, extend fusion to scalar reducers and registered joins, and create execution-budget headroom.
The current acceptance cases use the complete 100-execution allowance.
These are follow-up improvements, rather than unfinished items in this continuation.

#### Evidence and cleanup

The final verifier has SHA-256 `9276e0f71ea52eb572e5c9a30b82b35b2103df8808d919f16d9d584eaeaaea1e`.
The two complete capture digests are `231f1b0cec4cbb41536874967b2f8d2744905c0abf2a77091f2d773de7d0d128` and `155279be1e51e1202e6e47d92474615e52494e2cd430a1fe24ca45b6f746adfb`.
The paired CPU baseline runner has SHA-256 `cc6759bcd84688d191e8044f6baaae5b9692fa8fe667a7e898c8ae804bb8710d`.
The final CPU runner has SHA-256 `b6c98a419636c2f9f0b25803f9d44646476d19a8edecfa682a16bfaa4d3acc25`.

The compact evidence archive is `/Users/jeremy/orca/artifacts/rerail-sql-wave-phases-20261108.tar.gz`.
It retains the final reports, raw receipts, parity bundles, paired CPU observations, requests, source patch, and validation logs.
The temporary comparison worktree, build targets, and intermediate capture directories are removed after archival.
Cleanup also removes stale Rerail prefilter test files from exited processes, while preserving active jobs and their outputs.

### CTE materialization costs (2026-11-08)

This change starts from `e727c6987` and preserves the fixed 100-execution allowance.
The initial phase forced temporary tables for projections and intermediate row sets, even with a single consumer.
Complete cache rebuilds also queried old rows through anti-joins and UNION operations, although their replacement keys covered those rows.

#### Implementation

`Phase::relation` retains its MATERIALIZED fence for checked expressions and native functions.
`Phase::view` permits SQLite to inline pure projections, set operations, and aggregates over frozen inputs.
Complete rebuilds use these views for head projections, result keys, candidate rows, and fold inputs.
They omit redundant DISTINCT operations on keys that are already unique.

A complete cache replacement covers every old result key. Old improvements are a subset of that result.
The planner therefore uses the new result directly and replaces the complete improvement relation.
It omits old-row anti-joins, UNION branches, and improvement-key packets that cannot affect the answer.
Partial updates retain keyed patches and their existing query shapes.

The first experiment also inlined sparse-update helpers and omitted empty old-row branches.
It reduced VM work but increased small-edit CPU and introduced two extra warm SQL query variants.
The final change restores the baseline warm queries. All ten warm phase queries match the baseline byte for byte.

#### SQL work and parity

Both sizes pass complete-state parity, expected failures, rollback, and recovery against independent cold executions.
Both final captures use 100/100 engine SQL executions, with complete receipts and no unexpected explanation failures.
The initial schedule still uses five phase statements and 16 SQL executions overall.

| Initial work | 5,000 edges: before / after | 50,000 edges: before / after |
| --- | ---: | ---: |
| VM instructions | 5,876,559 / 4,929,092 | 58,724,559 / 49,254,092 |
| Result-stream rows | 118,125 / 106,125 | 1,180,125 / 1,060,125 |
| Sorts | 60 / 39 | 60 / 39 |
| Automatic-index rows | 27,986 / 11,992 | 279,986 / 119,992 |

Initial VM work decreases by 16%, result-stream rows by 10%, sorts by 35%, and automatic-index rows by 57%.
Total captured VM work decreases by 14%, including verification reads and all other session work.
These counters identify reduced work. The CPU comparison uses separate runs with profiling disabled.

#### CPU, checks, and limits

The CPU comparison uses three alternating baseline/final pairs at each size, with fresh stores and identical requests.
Both development builds use Rust 1.95.0 with debug information and incremental compilation disabled.
Shared profiling and SQL trace callbacks are disabled. Other compiler and test jobs ran concurrently on this macOS arm64 host.
Every trial checks values, output deltas, guards against unrelated writes, and scratch reuse.

| Regions / edges | Initial CPU before / after | Six edits CPU before / after | Peak RSS before / after |
| --- | ---: | ---: | ---: |
| 1,000 / 5,000 | 1.326 / 1.231 s | 48.5 / 49.3 ms | 85.3 / 85.2 MiB |
| 10,000 / 50,000 | 13.468 / 11.919 s | 52.1 / 47.2 ms | 639.7 / 638.4 MiB |

Initial CPU decreases by 7–12%. Peak memory remains close to baseline.
Sparse-edit timings vary, despite identical warm SQL. These samples do not establish a separate warm-path speedup.
The comparison uses a new Rust 1.95 baseline, so its absolute CPU values differ from the preceding Rust 1.97 measurements.

All 129 SQL, virtual-table, demand, and profiling tests pass, with one child-process test ignored by design.
The checks include large recursive cases, direct CTE input changes, rollback, and cached expression evaluation counts.
The updated phase test shares an inline projection across two consumers and checks one native function evaluation per execution.
The library also compiles with `code-rule-profile` enabled on Rust 1.95.
The structure comparison retains 10 passing metrics and 15 existing failures, with no added findings.

Necessary materialization remains for checked expressions and shared evaluated values.
Grouping support counts and lattice values still requires separate aggregate work.
Further optimization can combine compatible aggregates, provided it preserves evaluation counts, error timing, and sparse access paths.
The complete-session allowance has no guaranteed headroom after this change.

Evidence is archived at `/Users/jeremy/orca/artifacts/rerail-cte-costs-20261108.tar.gz`.
The archive contains both baseline and final reports, raw receipts, full-state parity bundles, CPU trials, source patches, and check logs.
Task build and temporary directories are removed after archival.

### Compatible aggregate composition (2026-11-08)

This change starts from `61b5d995a`. The complete-job execution allowance remains 100.
The phase builder aggregated operator outputs that already contained one lattice value per key.
It also removed duplicate candidate rows before applying duplicate-insensitive builtin folds.
Flat folds maintained a distinct-value count alongside extrema, although extrema already detect conflicts.

#### Shared algebra and implementation

`logic::native::aggregation` defines the shared reduction algebra and grouping compatibility check.
Native flat, min, and max joins use this algebra. SQL planning uses its descriptors to select compatible compositions.
Compatibility requires the same reduction, including flat's top, and the same prefix keys.
The producer must supply at most one non-null value per key before merging previous values or applying widening.

The wave builder supplies this proof when one active operator contributes an already grouped result.
The fold then uses that result directly. With multiple operators, it retains the aggregate.
Builtin candidate unions use `UNION ALL`, because repeated values cannot change these folds.
Old and replacement branches from one operator cover disjoint keys.
Support-count queries retain their multiplicities and original grouping.

Flat SQL uses `CASE WHEN min(v)<>max(v) THEN top ELSE min(v) END`.
This preserves concrete values, conflicting values, top, and the null result for an empty aggregate.
The shared helper supplies both retained result queries and temporal folds.

Registered callbacks default to opaque. They retain their aggregation and duplicate-removal behavior.
The builtin proof is skipped during serialization and deserialization, so external JSON cannot claim it.
Trusted native lattice implementations can expose their algebra through the lattice trait.
This change adds no SQL statements or native rule-evaluation callbacks.

#### Captures and CPU results

All three workloads pass independent cold parity, failure rollback, recovery, and complete capture coverage.
The shortest-path sessions include nine complete state checks and six successful edits at each size.
The mixed workload has 256 regions, composite keys, duplicate supports, three contributing rules, and min/max/flat outputs.
Its five edits include two expected failures and three successful deletions, including removal of all support for one region.

| Initial work | 5,000 edges: before / after | 50,000 edges: before / after | Mixed lattices: before / after |
| --- | ---: | ---: | ---: |
| VM instructions | 4,929,092 / 4,741,001 | 49,254,092 / 47,374,001 | 3,088,884 / 3,028,470 |
| Sorts | 39 / 33 | 39 / 33 | 197 / 170 |
| Automatic-index rows | 11,992 / 11,992 | 119,992 / 119,992 | 5,865 / 5,865 |
| Result-stream rows | 106,125 / 106,125 | 1,060,125 / 1,060,125 | 59,517 / 59,517 |
| Complete-job SQL executions | 100 / 100 | 100 / 100 | 74 / 74 |

Shortest-path initial VM work decreases by 3.8%. Initial sorts decrease by 15% there and 14% in the mixed case.
The queries still publish the same support and result rows. They build the same automatic indexes.
Both shortest-path sessions retain five initial phase statements and 16 initial SQL executions.
The profile reports retain complete explanations, with two expected setup failures and no unexpected explanation failures.

The CPU comparison uses three alternating baseline/final pairs per size, with fresh stores and profiling disabled.
Both builds use Rust 1.95.0, development settings, no debug information, and no incremental compilation.
Other compiler and test jobs ran concurrently on this macOS arm64 host.
Every trial validates outputs, edit deltas, unrelated-region write guards, and scratch reuse.

| Regions | Initial CPU before / after | Six edits CPU before / after | Peak RSS before / after |
| --- | ---: | ---: | ---: |
| 1,000 | 1.113 / 1.129 s | 44.4 / 42.0 ms | 85.2 / 85.2 MiB |
| 10,000 | 12.440 / 12.220 s | 56.8 / 51.6 ms | 639.7 / 637.9 MiB |

Initial CPU remains close to baseline. These samples do not establish an initial speedup.
Edit medians are lower, but the small sample and concurrent work limit that conclusion.
The deterministic VM and sort reductions establish the work removed by this change.

All 139 selected Rust tests pass, with one child-process test ignored by design.
The checks cover native lattice joins, SQL folds, composite keys, duplicate support, top retractions, empty groups, and large recursive edits.
The compatibility tests reject different key widths, different algebras, different flat tops, and opaque operations.
They also check that external JSON cannot supply a builtin proof.
The library compiles with `code-rule-profile` enabled.
The structure check retains 10 passing metrics and 15 existing failures, with no added findings.

#### Rejected combination and remaining work

Support counts group complete tuples, while lattice values group a shorter key prefix.
One ordinary `GROUP BY` cannot directly produce both retained outputs at their required granularities.
A SQL experiment combined support counting with window functions for extrema and one representative row per key.
It returned the same support total and lattice checksum for 60,000 contributions across 10,000 keys.
It reduced sorts from two to one, but increased VM work from 2,830,083 to 5,220,105 instructions.
One profiled observation also increased query CPU from 44.6 to 70.0 ms. The implementation therefore retains separate groups.

Further work should propagate checked uniqueness through projections, then omit support grouping where every head has one producer.
Scalar count, sum, extrema, and ordered string reducers still keep separate retained summaries.
Combining them requires shared input, filter, key, null, overflow, and retraction semantics.
Opaque callbacks need an explicit trusted contract before any change to evaluation order or multiplicity.
Previous-wave merges and widening remain barriers to aggregate reuse.
Neither shortest-path session gains execution-budget headroom from this change.

The evidence archive is `/Users/jeremy/orca/artifacts/rerail-aggregation-20261108.tar.gz`.
It contains full parity bundles, raw receipts, reports, CPU trials, the window experiment, source changes, and check logs.
Task build and temporary directories are removed after archival.

### Executor-independent uniqueness and scalar summaries (2026-11-08)

This work first merged Final Rerail Shape at `220a5d4f5`, creating `b95bc145f`.
The branch advanced during implementation. A second merge incorporated `0f0049ec7` before the final builds and measurements.
The final comparison uses a clean baseline build of `0f0049ec7`, the same compiler, and the same requests.

#### Root causes and shared analysis

Full support builds grouped every head tuple, even when its projected columns determined one complete input witness.
Scalar sites independently scanned and summarized identical bags. Their min/max sites also retained duplicate value indexes.
The merged catalog eagerly rebuilt metadata after each run, adding 26 SQL submissions to the standard recursive session.
These costs require different decisions: logical proofs establish compatibility; the executor selects storage and publication methods.

`logic::analysis::keys` computes functional-dependency closure over symbolic columns, equality constraints, fixed values, and declared input keys.
It has no SQL types, connection, executor, or row statistics.
Logical sets supply complete-tuple keys. Lattices supply their declared key prefixes.
The program adapter propagates narrower keys through single-rule projections and joins to a fixed point.
It preserves the proof chain in `key_inferences`.
The bounded key search tries at most 64 projected columns; wider projections keep their complete key.

Unique logical output tuples do not imply unique witnesses. Hidden input fields can produce the same head more than once.
The checked-query adapter therefore proves witness uniqueness independently, using the same closure algorithm.
It preserves input key properties through private view names and wave bindings.
Full support queries use a constant support of one only for a single rule with this proof.
Multiple producers, unbound witness columns, and opaque operations retain their support grouping.
Signed old/new contributions still require reconciliation during sparse updates.

`logic::analysis::scalars` groups compatible count, sum, min, and max sites over the same non-null bag.
Compatibility checks source identity, local filters, grouping keys, outer lookup mapping, value expressions, and dependency barriers.
It accepts only known, nonthrowing intrinsic predicates and direct value projections.
It preserves multiplicity, zero defaults for count/sum, absent extrema, and exact sum state with deferred public range checks.
Every family member is checked, so count cannot connect two different value expressions.
Ordered strings, existence tests, nullable contracts, aggregate-dependent keys, computed values, and callbacks retain separate summaries with reasons.

The native planner computes and records these logical reports too. The SQL executor consumes the same proofs and summary layouts.
The shared analysis does not select a backend or claim that a native runtime already implements shared summary execution.
Immutable program and rule analyses use shared ownership. Wave frames share operator explanations rather than copying their JSON per frame.

#### SQL execution and explain

Compatible sites within a rule share one group summary and one value multiplicity index for extrema.
Sum state keeps signed 128-bit totals. Only a rule lookup checks the signed 64-bit result range.
A shared SQL phase computes signed changes, exact sums, changed values, extrema, and dirty keys against the old state.
Sharing summaries alone still required 112 submissions. Fusing updates and bulk publication reduced the scalar acceptance session to 98.
Extrema probe the ordered persistent index and the changed values. They do not scan untouched large groups.
Native scratch roots stage the result. Bulk SQL publishes ordinary SQLite summaries inside the existing fact-batch transaction.
Empty publications and unnecessary deletions are omitted using unique-key and containment guarantees.
Recursive caches retain native versions when their wave history requires them.
Native code publishes or validates rows; rule and reducer evaluation remains SQL.

`lower()` and `run()` return the logical `analysis`.
`explain()` returns inferred keys, witness proofs, scalar families, rejected combinations, retained choices, and recursive operator analyses.
It works without profiling and executes no SQL. A test checks unchanged SQL profiling totals around the call.
The existing native explain report includes the logical analysis and retained decision records.
SQL receipts carry full analysis contexts. The Markdown and HTML query views show them beside SQLite plans and bytecode.
`rr_analysis(execution, analysis)` exposes the same execution explanations as JSON for further SQL queries.

The catalog now refreshes on demand after runs, removals, resets, or successful fact edits invalidate it.
Ordinary fact evaluation does not rebuild unrelated metadata. Catalog queries still count their SQL submissions.
A test extends a recursive plan after reading `rr_analysis`, then checks that the next catalog read matches `explain()`.

#### Rejected storage experiment

The first fused update stored ordinary group summaries and value counts in persistent native roots.
It passed parity and used 96 SQL submissions, but transferring and retaining those rows added substantial cold cost.
This experiment preceded the final branch refresh.
Compared with compact SQLite state, native state increased initial returned rows from 138 to 30,138 at 10,000 groups.
Native column reads increased from 510,000 to 690,000 against that compact candidate.
The extra rows were summary storage transfers, rather than additional logical results.
Three unprofiled pairs against the original merged baseline measured initial CPU at 0.156 s before and 0.376 s after.
Peak RSS increased from 47.9 to 72.1 MiB.

The retained implementation keeps ordinary summaries in compact SQLite tables and uses native scratch only for sparse publication rows.
That restores the small initial result stream and removes the persistent native-root memory increase.
The executor-independent compatibility decision remains the same. Storage choice is a separate executor decision.
The rejected captures and CPU trials remain in the evidence archive.

#### Final captures

All final captures pass independent cold parity, failure rollback, recovery, and complete receipt coverage.
The recursive requests retain six successful edits and two expected failures, with nine complete state checks.
The scalar requests use three values per group, including duplicate values with different supporting IDs.
They cover a failed deletion and recovery, removal of both maximum-value supports, full state snapshots, deltas, and final queries.
Both sizes use exactly the same requests before and after the change.

| Workload | Complete-job SQL before / after | Initial VM before / after | Initial sorts before / after |
| --- | ---: | ---: | ---: |
| 1,000 recursive regions | 126 / 100 | 4,741,829 / 4,716,101 | 33 / 32 |
| 10,000 recursive regions | 126 / 100 | 47,374,829 / 47,124,101 | 33 / 32 |
| 1,000 scalar groups | 191 / 98 | 938,707 / 600,505 | 13 / 6 |
| 10,000 scalar groups | 191 / 98 | 9,344,707 / 5,973,505 | 13 / 6 |

Scalar initial VM work decreases by 36%, and sorts decrease by 54%.
Scalar edit submissions decrease from 86 to 32, including the expected failed batch.
Scalar edit VM work decreases from 3,157 to 1,848. Native column reads during initialization decrease from 720,000 to 510,000 at the larger size.
Recursive warm SQL work remains unchanged. Its initial support proof removes one sort.
The final query catalogs expose logical analyses on 15 recursive query entries and 39 scalar entries.

#### CPU results and limits

The refreshed comparison uses three alternating baseline/final pairs per workload and size, with fresh processes and stores.
Both development builds use Rust 1.95.0 without debug information or incremental compilation. Profiling callbacks are disabled.
Every trial checks outputs, output deltas, and triggers that reject unrelated-region writes.
The recursive trials also validate scratch reuse. Other compiler and test jobs ran on this shared macOS arm64 host.

| Workload | Initial CPU before / after | Edit CPU before / after | Peak RSS before / after |
| --- | ---: | ---: | ---: |
| 1,000 recursive regions | 1.216 / 1.226 s | 42.3 / 47.6 ms | 85.8 / 85.9 MiB |
| 10,000 recursive regions | 10.186 / 11.699 s | 44.7 / 48.7 ms | 638.3 / 638.9 MiB |
| 1,000 scalar groups | 21.3 / 17.2 ms | 2.69 / 2.47 ms | 25.3 / 25.7 MiB |
| 10,000 scalar groups | 189.4 / 151.0 ms | 2.59 / 2.52 ms | 48.2 / 47.7 MiB |

Scalar initialization is about 20% faster in this batch. Sparse-edit savings are small relative to timer and host variation.
Recursive CPU does not establish a speedup: the larger refreshed median increases by 15%, despite lower deterministic SQL work.
An earlier comparison measured a lower final recursive median. The refreshed large trials range from 9.55 to 12.66 seconds.
The profiling captures place most recursive CPU in native publication and initialization, rather than SQL grouping.
These observations do not isolate host variation, publication costs, and analysis overhead. Further recursive CPU work requires an isolated paired profile.

The 100-submission allowance is unchanged. Recursive sessions have no headroom; the scalar acceptance sessions have two submissions of headroom.
Longer edit sessions or extra catalog queries can still exceed it.
Sharing currently applies within a retained rule. Different rules, cold scalar queries, and member lookup probes still have additional combination opportunities.
Broader proofs for computed keys, disjoint producers, nullable values, and callbacks need explicit semantic contracts.
The shared analysis and explain records now provide the inputs for those extensions, including native consumers.

Validation includes the SQL executor, shared analyses, native joins, virtual tables, profiling, large recursive updates, and cache-backed corpus binding.
All 152 selected Rust tests pass, with one child-process test ignored by design. All 38 Node profiling tests pass.
The library compiles with `code-rule-profile` enabled.
The structure comparison against `0f0049ec7` retains 10 passing metrics and 15 existing failures, with no added findings.
The merged corpus-binding test is gated on its required `tape-cache` feature and passes with that feature enabled.
The 100,000-value test bounds sparse edits below 200 inspected rows and 50,000 VM instructions.
Other checks cover global empty summaries, duplicate support, overflow timing, triggers, rollback, and catalog freshness.

Evidence is archived at `/Users/jeremy/orca/artifacts/rerail-shared-analysis-20261108.tar.gz`.
It includes final and rejected captures, raw receipts, parity bundles, CPU trials, source changes, and validation logs.
Task build and temporary directories are removed after archival.

### Cold summaries, shared lookups, and native publication (2026-11-08)

Final Rerail Shape remains merged through `0f0049ec7`. The merge command reports that this branch is already up to date.
The comparison baseline is `0edfc6227`, which contains the shared uniqueness and scalar-summary work described above.
The baseline binaries were built from that clean tree before these changes.

#### Findings and root causes

Cold execution still evaluated each original scalar query before constructing retained summaries and contributions.
This repeated rule evaluation and scanned the same input bags through separate aggregate sites.
Retained families shared their state, but every member still used a correlated scalar subquery.
Head validity checks repeated those summary probes again.

Native publication also retained avoidable storage work. An empty patch constructed incoming roots, then inserted every row into another tree.
Bulk replacements rebuilt keys and indexes even when SQL returned the previous complete row set in canonical order.
Construction searched the primary map twice and sorted complete rows that unique primary prefixes had already ordered.
These are storage costs. SQL still computes all rule, reducer, and lattice results.

#### Implemented fixes and shared proofs

The executor-independent scalar analysis now reports a lookup proof for each family.
A complete grouping key selects at most one summary row, so one left join can supply every compatible member.
Missing rows retain zero count/sum defaults and absent extrema.
Raw sum words pass through the join. The public range check remains at each original expression use.
Computed and aggregate-dependent correlations retain separate probes and their existing dependency maintenance.
Sharing remains within a retained rule; different rules do not yet share their summaries or probes.

The same analysis provides `initial_evaluation`, which requires independent scalar inputs and pure outer expressions.
This function has no SQL types, database connection, executor state, or row statistics.
The SQL adapter adds schedule checks: upstream strata must be complete, and the output must not require lattice folding or opaque producers.
The ordinary scheduler initializes eligible retained support at that frontier and publishes its result through the existing wave fold.
It then retains that support for later edits. The original cold scalar evaluation pass is omitted for eligible rules.
Checked outer expressions, dependent reducers, ordered reducers, and unsupported query shapes retain the established cold path with reasons.
The independent verification oracle continues to use the original scalar queries.

Walk lowering can add checked rules after cold execution. The planner adds those descriptors when lowering changes the program.
It preserves already initialized summaries instead of rebuilding their rows.
Tests cover upstream positive strata, empty groups, duplicate witnesses, checked-expression fallback, and removal of individual supports.

Empty native publications now use validated bulk roots directly.
An exact ordered-row comparison reuses complete replacement roots without rebuilding keys or indexes.
Other replacements use one primary-map lookup per row and omit the redundant complete-row sort.
All paths preserve integer constraints, primary-key checks, stable row IDs, snapshots, and atomic multi-output publication.

`explain()` includes cold admission decisions, actual cold initialization, shared lookup counts, and native publication strategies.
Family lookup proofs and cold expression decisions include rejection reasons.
The native logical explain report receives the same family proofs. Native consumers can use the shared analysis functions.
SQL receipts include retained decisions and per-output publication strategies alongside the existing SQLite plans and counters.

#### Complete-session results

The final captures use the same requests, toolchain, and feature flags as the baseline.
Both recursive sizes cover six successful edits, two expected failures, recovery, complete snapshots, output deltas, and unrelated-write guards.
Both scalar sizes preserve duplicate values with distinct supports, extrema retractions, expected failure rollback, and recovery.
All four final sessions pass independent cold parity and complete receipt coverage.

| Workload | Complete-job SQL before / after | Initial VM before / after | Initial native column reads before / after |
| --- | ---: | ---: | ---: |
| 1,000 scalar groups | 98 / 98 | 600,505 / 405,501 | 51,000 / 13,000 |
| 10,000 scalar groups | 98 / 98 | 5,973,505 / 4,023,501 | 510,000 / 130,000 |
| 1,000 recursive regions | 100 / 100 | 4,716,101 / 4,716,101 | 233,000 / 233,000 |
| 10,000 recursive regions | 100 / 100 | 47,124,101 / 47,124,101 | 2,330,000 / 2,330,000 |

Scalar initial VM work decreases by 33%. Native column reads decrease by 75%.
Initial sorts remain six. The remaining sorts build summary and value indexes and fold the public results.
Scalar edit work decreases from 1,848 to 1,774 VM instructions, with native column reads decreasing from 67 to 60.
Recursive SQL work remains unchanged because its improvements are in native storage construction.
The final reports expose logical analyses on 38 scalar query entries and 15 recursive entries.
The execution allowance remains 100 for the complete job, including setup, helpers, transactions, failures, verification, and cleanup.

#### CPU observations and remaining costs

Three alternating unprofiled pairs use fresh processes and stores for every workload and size.
Both builds use Rust 1.95.0, development optimization settings, no debug information, and no incremental compilation.
Every trial checks outputs, deltas, unrelated-write guards, and scratch reuse where applicable.
Other work ran on this shared macOS arm64 host, so these observations are not isolated hardware benchmarks.

| Workload | Initial CPU before / after | Edit CPU before / after | Peak RSS before / after |
| --- | ---: | ---: | ---: |
| 1,000 scalar groups | 15.46 / 6.48 ms | 2.31 / 2.30 ms | 25.64 / 25.84 MiB |
| 10,000 scalar groups | 134.83 / 45.45 ms | 2.30 / 2.08 ms | 47.69 / 48.19 MiB |
| 1,000 recursive regions | 1.101 / 0.920 s | 46.67 / 43.49 ms | 85.97 / 86.14 MiB |
| 10,000 recursive regions | 11.550 / 9.818 s | 49.16 / 46.33 ms | 638.91 / 639.91 MiB |

The larger scalar initialization is 66% faster in this batch. The larger recursive initialization is 15% faster.
A separate profiled recursive capture places publication CPU at 8.10 seconds before and 6.91 seconds after.
Bulk delta storage decreases from 2.06 to 1.72 seconds in those captures.
The first empty-root-only candidate produced small CPU savings. Complete-root reuse and bulk construction changes produced the larger reduction.
Native publication remains the largest recursive cost, and retained history still uses about 640 MiB at the larger size.
Further work should reduce storage transfers and retained duplication across proven equivalent phase outputs.
Different rules and computed correlations still offer summary-sharing opportunities, but require additional proofs.
Longer jobs can exceed the unchanged SQL allowance. These changes do not create submission headroom for recursive sessions.

Evidence is archived at `/Users/jeremy/orca/artifacts/rerail-cold-summaries-20261108.tar.gz`.
It contains baseline, intermediate, and final captures, raw receipts, parity bundles, CPU trials, source changes, and validation logs.

Validation passes 161 selected Rust tests, including the scheduler and native storage tests, with one child-process test ignored by design.
All 38 profiling and report tests pass. The library also compiles with `code-rule-profile` enabled.
The structure comparison retains 10 passing metrics and 15 existing failures, with no new findings.
Task build and temporary directories are removed after evidence archival.
