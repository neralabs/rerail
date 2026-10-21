# SQL join, CTE, and statistics experiments

These experiments compare SQL shapes on live engine frames and through complete workspaces.
The clearest improvement is a bounded, anchor-first join for the expensive sibling walks.
Additional CTE materialization and frame copies usually cost more than they save.
`ANALYZE` does not correct the virtual-input plans in these captures.

All alternatives are under `cfg(test)`.
The addon retains one SQL implementation and gains no runtime switches.
The statement hook is also absent from production builds.

## Workloads and checks

The experiments use the existing 135-policy Python workload, with its file lists, module contexts, batch size, and exclusions.
The 32-file corpus produces 33 findings.
The 138-file corpus produces 580 findings.
Each workspace uses four workers and two files per SQL batch.
These are the existing rule-pack benchmarks, rather than the unavailable original 818-clause catalogue.

Each fresh process runs native first, then the SQL prototype on a fresh workspace.
The driver compares complete findings, captures, failures, and blind-file results.
It also checks the stored native digest.
The 32-file digest is `16284c2e6cf80a95761f5b3e6510f30d3df32f69de72fc4a1685556b130a736c`.
The 138-file digest is `4ad2049fb003a6f5866efb44847b086a09a4e6b015bed17dc9aef77c3313669e`.

The host is an Apple M5 Max with variable concurrent load.
Both engines use the same diagnostic system allocator through `/tmp/librerail_jemalloc_check_shim.dylib`.
The benchmark uses the optimized `loop` test profile and `unbounded-work` feature.
CPU figures describe these test prototypes, not a production allocator or shipped addon.
The prototype includes first-use SQL finalization in apply, while the existing addon driver forces that step during setup.
Absolute times therefore remain specific to each driver.
The comparisons reverse variant order each round and report medians from three fresh processes per variant.
Wall times vary with host load and do not establish latency improvements.

## Live-query replay

The replay hook runs immediately before a generated statement consumes its live input frames.
Each alternative keeps the original projection, including duplicate rows, captures, parent keys, and ordinals.
The harness compares complete output multisets and records row-order changes separately.
A changed row order requires complete workspace validation before acceptance.

The targeted run keeps the largest observed input for each selected SQL text.
This avoids treating small initial recursive prefixes as representative of the expensive executions.
The final capture retains eleven snapshots: two sibling, three demand, three pattern, and three fact queries.
Every alternative starts with warm terminal caches and frame lookup indexes.
Execution CPU includes result collection and encoding, while preparation and copy/index/analysis costs remain separate.
Replay timings therefore describe query comparisons, not complete engine speedups.

Each sample records a fresh query plan, VM instructions, frame activity, and terminal filter counts.
The harness rejects excessive plans after approximately one second of thread CPU or 100 million VM instructions.
The progress callback checks these limits periodically, so a long native terminal call can exceed the CPU threshold.
Six fact-terminal alternatives reach this limit.
All completed alternatives preserve their output multisets, with zero mismatches and zero unexpected failures.

## Dependent walk shapes

The hot query starts with this logical relation:

```sql
FROM input_frame f
JOIN node fn ON fn.file = f.file AND fn.id = f.node
JOIN node c ON c.file = f.file
  AND c.parent = fn.wpar
  AND c.id <= fn.aprev
  AND c.kind IN (215)
```

Its default plan starts with global kind postings for `c`.
It then joins frames by file before checking the anchor and sibling bounds.
The experiment compares these alternatives:

| Shape | Purpose |
|---|---|
| `CROSS JOIN` | Keep the frame, anchor, and candidate in dependency order |
| Comma join with `WHERE` predicates | Check whether predicate placement changes the plan |
| Derived input with `LIMIT -1` | Prevent input-subquery flattening |
| Inline anchor CTE | Factor anchor metadata without requiring a copy |
| Materialized anchor CTE | Compute anchor metadata before candidate joins |
| CTE with explicit cross joins | Combine a CTE with fixed dependency order |
| Scalar anchor subqueries | Replace the anchor join with correlated point lookups |
| Correlated candidate `IN` | Enumerate sibling IDs inside a bounded subquery |
| Correlated candidate `EXISTS` | Check the relationship for each candidate |
| Unary-plus kind predicate | Prefer the parent access path over kind postings |
| Copied B-tree, with and without statistics | Compare ordinary indexes and measured cardinalities |

The two largest retained sibling snapshots contain 777 input rows and produce 1,196 output rows.
The following table sums each snapshot's median query CPU:

| Shape | Query CPU | VM instructions | Frame advances |
|---|---:|---:|---:|
| Default joins | 133.96 ms | 15,222,587 | 1,165,378 |
| Comma join | 133.27 ms | 15,222,587 | 1,165,378 |
| Derived input fence | 110.32 ms | 12,909,906 | 777 |
| Copied B-tree with `ANALYZE` | 43.01 ms | 4,706,702 | 0 |
| Correlated `EXISTS` | 134.28 ms | 18,687,831 | 777 |
| Correlated `IN` | 1.69 ms | 76,782 | 777 |
| Materialized anchor CTE with cross joins | 1.23 ms | 45,695 | 777 |
| Scalar anchor | 1.06 ms | 52,460 | 777 |
| Plain cross joins | 1.09 ms | 38,476 | 777 |
| Inline anchor CTE with cross joins | 1.04 ms | 38,476 | 777 |

The copied B-tree also costs 2.22 ms to create, index, and analyze.
It improves the starting cardinality estimate but still scans candidates by file and kind.
The derived fence reduces frame advances but retains substantial candidate work and an automatic index.
Frame advances alone therefore miss much of its remaining work.

The cross-join plan scans the input, probes the anchor, then uses candidate file, parent, ID range, and kind constraints together.
Its candidate plan is `f=p=i{k=`, compared with the default global `k=` path.
An inline CTE reaches the same plan and instruction count as plain cross joins.
Its small timing difference does not establish a separate improvement.
Materializing anchors adds work without improving that plan.

SQLite explicitly preserves table order across `CROSS JOIN`, while ordinary inner and comma joins permit reordering.
This behavior is part of the [documented join-order contract](https://www.sqlite.org/optoverview.html#manual_control_of_query_plans_using_cross_join).

## Whole-workspace prototypes

The prototype rewrites only the selected emitted family.
It keeps the compiler's table effects and scheduling metadata.
Complete native output parity checks the resulting execution, including changed intermediate row order.
CPU includes the test hook and rewrite work.
The timed apply runs without replay, tracing, or `EXPLAIN`.

| Prototype | 32-file apply CPU | Change from control | 138-file apply CPU | Change from control |
|---|---:|---:|---:|---:|
| Control | 5031.1 ms | — | 27427.3 ms | — |
| Sibling cross joins | 4820.3 ms | −4.2% | 27090.8 ms | −1.2% |
| Inline sibling anchor CTE | 4823.4 ms | −4.1% | — | — |
| Materialized sibling anchor CTE | 4890.3 ms | −2.8% | — | — |
| Demand-key left join | 4990.5 ms | −0.8% | — | — |
| Materialized demand-key CTE | 5101.6 ms | +1.4% | — | — |
| Frame-first terminal cross joins | 4987.6 ms | −0.9% | — | — |
| Materialized terminal input CTE | 5275.2 ms | +4.9% | — | — |
| Sibling cross joins and demand-key left join | 4884.4 ms | −2.9% | 27321.5 ms | −0.4% |

All 36 fresh-process runs pass complete native output parity.
The sibling prototype changes 49 executed statements on 32 files and 281 on 138 files.
The combined prototype also changes 225 and 1,398 capture-demand statements, respectively.
The local join improvement is clear, but it removes only part of total workload CPU.
The small demand and terminal changes do not establish a reliable whole-workload improvement.

A separate sweep compares the sibling restriction with cross joins across other dependent candidate walks.
Its 32-file medians are 5121.1 ms for control, 4997.8 ms for siblings, and 4958.6 ms for broader walks.
The broader prototype changes another 1,913 statements and preserves complete native output parity in all nine runs.
These fresh controls differ from the earlier sweep, so results remain paired within each sweep.

On 138 files, a paired broader-walk sweep measures 27591.6 ms for control and 26838.2 ms for cross joins, a 2.7% reduction.
It changes 11,170 other candidate-walk statements and 281 sibling statements.
All six runs preserve the native digest and 580 findings.
Across the primary and broader sweeps, all 51 fresh-process runs preserve complete native output parity.
The observed reductions remain modest and do not close the engine's overall performance gap.

## Recommended compiler change

The preferred design emits one anchor-first shape for dependent candidate walks whose bounds require the anchor row.
The smallest useful form uses plain cross joins, with no extra CTE or copied frame.
It makes the required bounds available when the candidate cursor opens.
The prototype supports both the ordinary sibling source and its capture-guarded `UNION ALL` form.
The broader-walk prototype also covers existing anchor joins for parent, descendant, and other sibling walks.

The compiler must retain the existing parent keys and walk ordinals.
The completed workspace comparisons establish parity for both tested corpora, including the changed intermediate row order.
They do not establish the best join order for every possible rule or dataset.
Independent joins can still benefit from planner reordering.
The recommendation therefore concerns dependent walks, rather than every join emitted by the compiler.

The demand-key and terminal alternatives do not justify another runtime path.
A shared computed key column remains a possible compiler improvement, but the tested wide key CTE does not supply it cheaply.

## Demand keys

The demand experiments compare scalar key expressions, inline and materialized key CTEs, a left join, `EXCEPT`, and deduplication before the anti-join.
The largest retained input contains 23,738 rows.
Across all three retained demand snapshots, the original query takes 11.12 ms.
The left join takes 11.24 ms, despite reducing VM instructions from 921,099 to 729,828.
The materialized key CTE takes 15.19 ms, and deduplication before the anti-join takes 19.26 ms.
An `EXCEPT` form takes 12.60 ms and changes row order in one snapshot.

The smaller early-prefix sample favored the left join.
The larger inputs and complete workspace results do not support adopting it as a general fix.
The SQL still repeats canonical-key expressions, but an extra wide CTE is an expensive place to store those keys.
A future compiler experiment can carry canonical keys in shared frame columns without copying the entire input through SQLite.

## Terminal CTEs

The experiments keep materialization, remove the hint, request inlining, fix join order, or materialize the input instead.
The three hot pattern snapshots keep batched terminal calls under the tested inline plan.
This result does not establish that every pattern or capture-dependent terminal can use that plan.

Every hot fact snapshot restores excessive terminal filters after inlining or removing the materialization hint.
All six combinations reach the diagnostic execution limit.
Earlier smaller snapshots complete but run about 250 times slower in aggregate on their successful paired samples.
The complete-workspace input-materialization prototype is also slower, even with terminal batching intact.

The terminal result remains materialized until another producer contract guarantees one batched call.
SQLite describes materialization as a planning fence, while `NOT MATERIALIZED` does not prohibit materialization.
The [CTE documentation](https://www.sqlite.org/lang_with.html#materialization_hints) explains these distinctions.

## `ANALYZE` and statistics

The harness runs each statement below and records its CPU, resulting statistics, fresh query plan, and execution counters:

```sql
ANALYZE input_frame;
ANALYZE node;
ANALYZE main;
ANALYZE temp;
ANALYZE temp.shape_stats;
PRAGMA optimize=0x10002;
```

The named virtual-input commands succeed but create no `sqlite_stat1` rows for frames or nodes.
Schema-wide commands create statistics for ordinary demand and copied tables, but leave the tested virtual-input plans unchanged.
The corresponding VM counts also remain unchanged.
CPU differences with identical plans and counters do not establish an `ANALYZE` benefit.

Analyzing a copied B-tree input does improve the sibling plan relative to the unanalysed copy.
It remains much slower than the bounded virtual-table join.
For pattern snapshots, the analyzed copy takes 50.84 ms of query CPU plus 31.35 ms of preparation work.
The original virtual input takes 64.79 ms without that copy.
For fact snapshots, the analyzed copy takes 29.61 ms plus 29.70 ms of preparation, compared with 32.52 ms originally.
Copying frames to obtain statistics therefore does not repay its cost in these samples.

The current virtual-table estimates come from `xBestIndex`.
SQLite asks that callback for usable constraints, cost, and row estimates during query preparation.
The [virtual-table planning interface](https://www.sqlite.org/vtab.html#the_xbestindex_method) documents this contract.
`ANALYZE` supplies ordinary table and index statistics, as described in the [command documentation](https://www.sqlite.org/lang_analyze.html).
It does not replace the module's estimates in these experiments.

## Implementation and reproduction

- [query_shapes.rs](../native/src/code_rule_dsl/sql/spikes/query_shapes.rs) installs the test-only replay or prototype hook.
- [query_variants.rs](../native/src/code_rule_dsl/sql/spikes/query_variants.rs) defines the supported emitted shapes and SQL alternatives.
- [query_measure.rs](../native/src/code_rule_dsl/sql/spikes/query_measure.rs) measures queries, checks rows, and restores pre-existing statistics.
- [query_workload.rs](../native/src/code_rule_dsl/sql/spikes/query_workload.rs) runs the native and SQL workspace APIs.
- [sql-query-shapes.mjs](../benchmark/sql-query-shapes.mjs) summarizes replay snapshots and distinguishes budget rejections from mismatches.
- [sql-query-shape-sweep.mjs](../benchmark/sql-query-shape-sweep.mjs) alternates fresh prototype processes and records source and input hashes.

The normal SQL suite runs with:

```sh
cargo test --profile loop --features unbounded-work --manifest-path native/Cargo.toml code_rule_dsl::sql
```

The final suite passes 158 tests, including 500 generated differential comparisons, with zero differences and sixteen ignored measurement tests.
Clippy, the production build check, and the script tests pass.
The repository layout check retains its two existing failures in unchanged benchmark fixtures and policy-guard includes.

A live replay uses the existing benchmark workload schema, plus optional `shapeTargets`, `shapeSamples`, and `shapeRounds` fields:

```sh
RERAIL_SQL_SHAPE_CONFIG=/absolute/workload.json \
RERAIL_SQL_SHAPE_OUTPUT=/tmp/new-sql-shape-replay \
RERAIL_BENCH_WORKERS=4 \
cargo test --profile loop --features unbounded-work --manifest-path native/Cargo.toml \
  replay_sql_shapes_on_workload -- --ignored --nocapture --test-threads=1

node benchmark/sql-query-shapes.mjs /tmp/new-sql-shape-replay
```

The fresh-process sweep definition names `testBinary`, `sourceManifest`, `baseConfig`, `outputDirectory`, `rounds`, and `variants`.
Each variant has a name and one rewrite choice for each selected family.
The driver records a fresh native oracle in every process.
It reports CPU, wall time, output digests, rewrite counts, host load, binary hashes, and input hashes.

Artifacts are under `/tmp/rerail-sql-query-shapes-20261021`.
`replay-highwater/` contains the final targeted snapshots, raw plans, terminal counters, and summary.
`sweep-32/` and `sweep-138/` contain the primary prototype comparisons.
`sweep-broad-32/` contains the broader-walk comparison.
`sweep-broad-138/` checks the broader walk shape on the larger corpus.
Each comparison records its frozen binary and source manifest.
The `sweep-source/`, `highwater-source/`, and `broad-source/` directories preserve source for the frozen binaries.
Earlier `replay-32/` and `replay-hot/` captures are preliminary and do not supply the final replay table.
