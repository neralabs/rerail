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

## Active completion goal: remaining execution work

The scope includes all five remaining items. Completion requires evidence for each item.

| Item | Implementation direction | Completion evidence |
| --- | --- | --- |
| First-edit preparation | Prepare mutable inputs, materialized projections, and retained support during binding or initial execution. | First edits over large inputs need no migration or support initialization and pass a bounded work budget. |
| Recursive lattices and aggregates | Maintain affected dependency regions, invalidate cyclic support, and rederive from surviving external support. | Cyclic insertion and deletion sequences match fresh execution without a whole-stratum refresh. |
| Remaining rule shapes | Retain ordered string reductions, dependent aggregate keys, and walk blocks with subqueries or negation. | Each shape has repeated-edit, empty-input, rollback, and fresh-execution comparisons without refresh fallback. |
| Registered lattice joins | Retain a balanced reduction tree over candidate support and combine summaries with the registered merge. | A sparse edit at a large key processes a logarithmic number of summaries, including retractions from top. |
| Temporary-table overhead | Reuse typed scratch tables and indexes across operations. | Repeated edits create no scratch schema objects, leave no scratch rows, and preserve pool consistency after rollback and reset. |

The previous implementation commit is verified progress. The five completion requirements remain active until their implementation and checks pass.

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
The public session continues to use its existing tables. This backend is not yet connected to recursive scheduling.

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
Ordinary SQL rollback alone does not restore this backend. Session transaction integration is still required.

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
Until that integration passes, unpartitioned recursive lattices and aggregates retain their visible stratum-refresh fallback.
