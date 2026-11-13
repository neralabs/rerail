# Batched recursive SQL

The engine supports a reachability stage with explicit SQL producers for roots and edges.
Data-layer quality rules use this stage for their descendants relation.
The conditional architecture rules retain their existing repository-wide root producer.
The primary quality rules request a conservative set of roots from their consumer families.

The engine normalizes `(path, identity)` keys once per stage.
SQLite evaluates recursive `UNION` over batches of 256 integer roots.
The result retains forward and reverse indexes across all batches.
Consumers read the existing `__path,root,node` columns through a mapping view.
The compact key column has explicit `INTEGER` affinity so SQLite can use its lookup index.

Root and edge producers follow ordinary SQL authorization and statement limits.
The graph includes isolated roots and dangling endpoints.
Null identities supply no root or edge.
Cycles terminate, and converging paths produce one result pair.
Identical identities on different paths remain separate.
The existing SQL work budget and cancellation handler cover all graph stages.
Batch size limits the traversal request, not the retained closure size.

The stage separates the logical relation from its physical implementation.
Another adapter could preserve these producer queries and result columns.
The engine currently executes this stage entirely through SQLite.

## Reproduction

Use `scripts/profiling/run.py` with `--deadline 115` for every measured command.
Set `RERAIL_ENGINE_PATH` to the exact binding under test.
The controller records the binding hash, input hashes, policy hashes, CPU, and peak RSS.
Use `--trace on` for phase attribution and `--external sample` for external bottom-up stacks.
The new phases are `sql-reach-prepare`, `sql-reach-walk`, and `sql-reach-index`.
The walk phase records root count, batch size, and batch count.

```json
{
  "name": "descendants",
  "reachability": {
    "roots": "SELECT __path,id FROM requested_nodes",
    "edges": "SELECT __path,src,dst FROM links"
  }
}
```

## Remaining costs

A five-file native trace attributes these cold CPU costs. Parent spans are excluded from this table.
The trace includes query-plan logging and external sampling; use uninstrumented runs to compare total CPU.

| Work | CPU seconds |
| --- | ---: |
| Ordinary SQL table materialization | 4.35 |
| Ordinary SQL indexes | 1.90 |
| Fact extraction | 2.17 |
| Reachability key, edge, and root preparation | 1.36 |
| Recursive reachability batches | 0.96 |
| Reachability reverse index and cleanup | 0.18 |
| Fact insertion | 0.50 |

These are the clearest next costs:

1. **Common SQL producers repeat across scopes.** The three quality groups retain separate file gates.
   Across those groups and the conditional scope, `nodes` and `ownership` each materialize 14 times.
   Their materialization costs total 1.17 and 1.25 CPU seconds, before their indexes.
   On the largest file, each quality group builds 67,774 node rows and repeats the same ownership walk.
   Share these producers within SQL while retaining the production domain gates.
2. **Facts are wide and remain materialized.** The three quality node tables copy 29.2 MB of node text for 1.03 MB of source.
   This count excludes other projected columns, derived tables, and indexes. It is not physical memory use.
   Several derived relations copy `n.*` or `f.*` again.
   Prune unused columns and release intermediates after their last consumer.
3. **Graph preparation exceeds traversal.** Sorting and deduplicating roots, assigning keys, and converting edges take 1.36 CPU seconds.
   The largest file contributes 0.82 seconds of preparation and 0.56 seconds of traversal over 158 batches.
   Preserve the shared SQL representation and reduce repeated sorting or supply compact identities earlier.
4. **Candidate selection and fact extraction allocate and repeat work.** The native counters attribute about 1.3 CPU seconds to initial anchor selection.
   One external sampling window is dominated by `FileAnchorIndex::select_complete`, `combine`, sorting, and budget polling.
   Another window shows witness capture maps, allocations, copies, and SQLite parameter binding.
   Reuse compatible candidate selections and projected facts within an immutable input scope.
   Fact insertion is already batched; its remaining copies and bindings still cost about 0.50 CPU seconds.

The SQL sampling window shows sorter allocation, record comparison and decoding, B-tree movement, and VM execution.
This agrees with the table construction and preparation costs above.
The three one-second windows do not estimate percentages for the whole workload.
A sampled statistics policy remains a planning risk from the earlier experiments, but this run does not prove a new production join-plan regression.

A diagnostic that aggregates all descendants exceeded the existing SQL work budget after adding its extra queries.
The narrower node statistics probe completed without failures.
Neither diagnostic is a production parity run or a timing benchmark.

## Native comparison and validation

The workload contains five files and 1,026,452 source bytes.
Both policies use the same compiled native binding, bundled SQLite 3.53.2, and system malloc shim.
The engine crate uses optimization level 1, thin LTO, and 256 code generation units for the bounded local build.
Dependencies use the release profile.

These are medians of two fresh processes per policy, with tracing and sampling disabled:

| Measurement | Previous SQL | Batched recursive SQL |
| --- | ---: | ---: |
| Cold process CPU | 15.76 s | 13.60 s |
| Peak process RSS | 1.36 GB | 1.07 GB |

Cold CPU falls about 14%, and peak RSS falls about 22%.
Wall times vary substantially on the shared host and do not establish a wall-time improvement.
The instrumented profile above is a separate run.
These numbers supersede replay-only comparisons for the integrated policy; build settings differ from older native experiments.

Validation passes:

- 200 upstream calls: 135 positive and 65 clean, with no finding, failure, or execution-path differences.
- 219 production scope probes and 57 edit, restore, and removal transitions.
- 277 native integration tests, including 17 graph tests and SQL interruption checks.
- 12 profiler tests, TypeScript checks, repository structure checks, Rust formatting, and Clippy with profiling enabled.
- Regeneration produces the same shipped quality policy.

[Machine-readable evidence](../fixtures/host/data-layer-port/profiling/batched-sql.json) records hashes, repeated measurements, phase totals, external stacks, and command durations.
Local raw traces and sampler output are under `.rerail/profiles/batched-sql-integration/`.
All commands complete or stop within 113 seconds.

To repeat the attributed profile with an exact binding:

```sh
RERAIL_ENGINE_PATH=/path/to/rerail.node python3 scripts/profiling/run.py \
  --deadline 115 --cold-only --trace on --query-plans \
  --external sample --sample-every 4 --sample-count 3 --sample-seconds 1 \
  --source /path/to/pinned/data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --policies packs/data-layer/candidates/rules \
  --output .rerail/profiles/batched-sql-new-run
```
