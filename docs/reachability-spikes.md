# Reachability adapters

This document records historical experiments. The [final decisions](sql-shape-spikes.md#final-decisions) describe the current execution path.
The current branch removes experimental execution flags and alternate input adapters.
Historical commands require the corresponding older commit.

The experiment gives recursive SQL, a Rust scalar function, and a Rust virtual table the same request relation.
The adapters retain the existing linter SQL and return the same descendants relation.
The Rust and interval adapters remain experiments.
The engine now supports the batched recursive SQL stage described in [the integration report](batched-sql-performance.md).
The measurements below describe the earlier replay experiment.
The [native relation experiments](native-relations-spikes.md) add strict forest indexing, tape access, shared captures, and finite-set propagation.

## Shared contract

The request relation contains distinct `(path, root)` pairs.
The result relation contains distinct `(path, root, node)` pairs, including each root itself.
Diamonds produce one pair, and cycles terminate.
Traversal can pass through dangling edge endpoints.
Identical node IDs in different files remain separate.

The quality linter uses a conservative SQL producer for requested roots.
The conditional linter already has an explicit root CTE, which the experiment preserves.
This producer belongs to rule authoring. It is not a general optimizer that infers roots from arbitrary SQL.

Each immutable SQL scope assigns integer keys to `(path, node)` identities.
All experimental adapters use these keys and the same indexed edge relation.
The public `descendants` view maps results back to existing identities.
Consumers retain their original joins, filters, and diagnostics.

## Implementations

| Backend | Implementation |
| --- | --- |
| `baseline` | Original authored recursive SQL and materialization |
| `sql` | Recursive `UNION` over batches of requested roots |
| `scalar` | Rust `is_reachable(graph, root, node)` with a bounded memo |
| `vtab` | Rust virtual table with one `IN` scan per root batch |
| `vtab-direct` | A virtual relation that consumers constrain directly |
| `interval` | SQL tree intervals, followed by batched pair materialization |
| `interval-direct` | SQL interval joins without a materialized pair relation |

The materialized adapters use batches of 256 integer roots by default.
The batch size is configurable from 1 through 4096.
Even the largest root batch contains only 32 KiB of integer values before container overhead.
SQLite retains the shared pair relation and its forward and reverse indexes.
Batching limits traversal requests. It does not bound the total size of a retained closure.

Recursive SQL deduplicates `(root, node)` during recursion with `UNION`.
The recursive rows omit source paths because the integer keys already identify each file and node.
The roots enter through a temporary batch table, rather than an expanded SQL parameter list.

The Rust adapters share one generic directed-graph implementation.
It contains no linter names, syntax kinds, or detector logic.
It loads the graph once per SQL scope and supports forward and reverse traversal.
Its memo retains at most 250,000 node memberships across roots.
An unknown node does not create an empty cache entry.
Each graph has a work budget and a monotonic deadline.
An interrupted traversal does not publish a partial memo entry.

The virtual table supports root equality, target equality, and complete `IN` sets.
It rejects scans without either a root or a target constraint.
It prefers a consumer's equality binding over a broad request set.
A bound target with a root set selects reverse traversal and root membership.
Both `IN` sets select the smaller traversal seed set at execution.
Its composite key is `(graph, root, node)`, through a `WITHOUT ROWID` declaration.
SQLite documents both [batch `IN` processing](https://www.sqlite.org/c3ref/vtab_in.html) and [virtual-table planning](https://www.sqlite.org/vtab.html).

The interval experiment constructs actual DFS intervals in SQL.
It first checks distinct parent counts, then checks that traversal covers every node.
Diamonds and cycles select the recursive SQL fallback.
Only a verified forest uses `UNION ALL` during interval construction.
The prototype uses path strings and sorted enter/leave events. It does not obtain intervals from source-byte containment.

## Harness and boundaries

`reachability-capture.ts` runs the actual native extractor and captures its SQL inputs.
It first compares native findings with upstream expectations for oracle recordings.
Diagnostic packets contain at most 32 input rows and retain a real witness anchor.
Large captures remain local and do not enter version control.

`reachability_replay.py` recreates those input tables and executes the authored stages in order.
File scopes and repository scopes retain their original partition behavior.
Every backend uses the engine's regex crate and SQL regex behavior through the extension.
The harness compares complete SQL diagnostic rows, including spans and identities, against native findings.
It records nonrelational findings separately because reachability does not execute those clauses.

`reachability_closure.py` compares complete pair sets against the original recursive SQL on a real captured graph.
`reachability_probes.py` compares batched candidate-pair queries.
The SQL probe control retains its materialized closure and indexes for subsequent probes.
Both probe controls retain the common SQL key and edge preparation.
Rust serialization and graph construction are additional measured costs.

These measurements cover captured-input SQL execution, not complete native engine execution.
They exclude native extraction, compilation, workspace scheduling, and wire encoding.
The host uses Python's SQLite library for every adapter, including the original-SQL control.
They do not establish a speedup over earlier native-engine profiles.
The replay host uses SQLite 3.53.4. The engine's pinned SQLite source is 3.53.2.
Native integration must repeat the query-plan measurements on that bundled build.

The Rust bridge loads graph facts through JSON in this prototype.
A production adapter needs bounded graph-input batches and the engine's existing cancellation token and work counter.
The extension's own deadline and work budget do not provide that host integration.
Planner estimates are coarse, and the memo has a fixed membership limit.
The extension remains a local experiment, outside the engine dependency graph and policy function allowlist.

## Results

The [machine-readable evidence](../fixtures/host/data-layer-port/profiling/reachability-spikes.json) records comparisons, source hashes, timings, and profiler stacks.
Raw local artifacts are under `.rerail/profiles/reachability-2026-10-20/`.

The five-file production capture contains 1,026,452 source bytes.
The table records median CPU seconds from two fresh-database replays, except the single interval run.
All completed benchmark replays retain finding parity.
The timings exclude native extraction and compilation.

| Adapter | Statistics | CPU seconds | Peak RSS, decimal MB |
| --- | --- | ---: | ---: |
| Original SQL | Sampled | 9.25 | 957 |
| Batched recursive SQL | Sampled | 7.22 | 639 |
| Batched SQL, ordered pair inserts | Sampled | 7.18 | 637 |
| Batched Rust virtual table | Sampled | 6.75 | 669 |
| Batched recursive SQL | Full | 8.01 | 639 |
| Direct Rust virtual table | Full | 7.53 | 649 |
| Materialized SQL intervals, one run | Sampled | 8.18 | 657 |

The SQL change removes about 22% of replay CPU time and 318 MB of peak RSS relative to the original SQL control.
The virtual table adds a modest reduction. Two runs on a shared host do not establish a stable six-percent advantage.
Sorting SQL pair inserts does not materially change this comparison.
Full statistics repair the direct virtual table, but they add work to the SQL control too.
Shared key, edge, and request preparation still takes about 0.9 CPU seconds on the largest file in both adapters.
That preparation remains a useful SQL optimization target.

The scalar probe uses 20,000 candidate pairs and 128 function roots from the largest file.
It retains the same SQL key and edge preparation for both controls.
SQL closure construction and the first probe batch take about 21 ms after that common preparation.
Rust row retrieval, serialization, graph construction, and the first probe batch take about 76 ms.
Warm probe batches take roughly 11–12 ms in SQL and 8 ms through the Rust scalar function.
This is a useful point-lookup interface, with a graph-construction cost to amortize.
Bulk scalar enumeration needs 2,737,798,504 candidate pairs on the largest file, so the harness caps that shape.

The direct interval view exhausts the 200-million-step SQL budget.
Full statistics move the failure from `method_nodes` to `annotation_atoms` on the largest file.
The latter plan checks identifier candidates against interval membership instead of enumerating each small annotation subtree first.
The current SQL interval construction and access plans do not improve this workload.
This result does not measure intervals supplied directly by the extractor.

Three one-second external captures contain 2,490 active sampled stacks.
SQLite accounts for 2,469 of these stacks.
The largest leaves include VM execution, record comparisons, B-tree movement, and varint decoding.
These short capture windows do not estimate the exact Rust share across the whole run.

The corpus contains 200 recorded quality-linter calls, including supplemental probes.
It contains 134 positive SQL cases and 66 clean SQL cases.
One additional positive finding belongs to the unchanged nonrelational file-size rule.
Seven adapters pass 1,400 complete-row comparisons, including duplicate multiplicity.
Ordered SQL inserts add another 200 comparisons.
Full-statistics SQL and direct virtual-table runs add 400 more comparisons.
The largest-file SQL and Rust adapters also match all 421,698 requested pairs against the original 631,463-pair closure.
The materialized interval adapter passes that same pair-set comparison.

## Engine integration

Keep batched recursive SQL as the default implementation.
Keep the root request relation separate from traversal and from the public descendants relation.
This boundary permits a virtual-table implementation without changes to detector SQL.

An engine adapter can load requests in bounded batches, retain completed pairs, and share them across consumers within the same immutable scope.
Give internal tables explicit integer types and retain both traversal directions in the result indexes.
Keep graph construction and traversal under the engine's existing work and cancellation controls.
The experimental graph loader is an integration bridge, not a proposed policy-visible loading API.

Treat statistics as part of the physical plan.
The direct virtual table needs accurate estimates for surrounding SQL relations as well as its own access paths.
A selective statistics policy is a better next experiment than unconditional full analysis of every stage.
The measured incremental gain does not yet justify selecting the Rust extension as the engine default.

## Regressions found

The first integer mapping used `CREATE TABLE AS` for a computed row number.
SQLite gave that column no integer affinity, while the pair table declared `INTEGER` keys.
The mapping join then scanned possible targets for each method instead of using integer lookups.
Full `ANALYZE` did not repair the plan.
An explicit `INTEGER` declaration restored indexed access.
A selective-consumer regression test now enforces a small SQL work budget.

The first direct virtual-table plan treated a root batch like a single root.
It also walked every requested root when a consumer already bound the target.
The corrected plan distinguishes batch scans and uses reverse traversal for that access shape.
A regression test checks both the results and the number of traversals.

Sampled statistics still give the direct view a poor join order in `loop_names`.
The plan enumerates requested roots before the selective loop consumer.
Full `ANALYZE` changes the plan to scan loop candidates first and completes the workload.
The failed run records about 711,000 Rust work units before SQLite exhausts 200 million VM steps.
These counters use different units and do not form a speed ratio.

Repository scopes also exposed repeated textual IDs across files.
The key relation now includes the source path before assigning global integer identities.
A differential test checks file isolation for every adapter.

## Reproduce

Run each command through the existing controller with a new output directory:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output /tmp/reach-build -- \
  cargo build --manifest-path scripts/profiling/reachability/Cargo.toml \
  --release --locked --target-dir /tmp/reach-target
```

On macOS, set the extension path:

```sh
export RERAIL_REACH_EXTENSION=/tmp/reach-target/release/librerail_reachability_spike.dylib
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output /tmp/reach-tests -- python3 test/scripts/test_reachability.py
```

On Linux, use the `.so` artifact. This experiment was measured on macOS only.

Capture an all-shared quality policy and a recording:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output /tmp/reach-capture -- \
  node --import tsx scripts/profiling/reachability-capture.ts \
  --policy /path/to/all-shared/data-layer-serverplan-quality.json \
  --recording /path/to/quality-recording.json \
  --offset 0 --limit 20 --output /tmp/reach-inputs.json
```

Use `--production` for a production-file recording.
The existing sixth-batch generator creates the all-shared policy with `--quality-all-sql`.

Compare all adapters on a small corpus chunk:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output /tmp/reach-replay -- \
  python3 scripts/profiling/reachability_replay.py \
  --capture /tmp/reach-inputs.json --extension "$RERAIL_REACH_EXTENSION" \
  --backend all --batch-size 7 --limit 20 --output /tmp/reach-results.json
```

For large files, run one backend per command.
The scalar adapter rejects enumeration above five million candidate pairs by default.
Use the pair-probe harness to measure its intended lookup workload.
Add `--path` to select one file, `--repeat` for repeated measurements, or `--all-roots` for the unrestricted-root control.
Use `--ordered-pairs` for the SQL insertion-order control.
Use `--analysis-limit 0` to compare full statistics against the default sampled analysis.
Set `RERAIL_REACH_TRACE=1` to retain stage times and SQLite query plans.
For an external profile, add `--external sample --sample-module '*'` before the controller's command separator.
