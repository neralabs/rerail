# SQL shape experiments

The [performance journal](data-layer-performance-journal.md) records these decisions and the subsequent execution baseline.

The experiments isolate the four targets from the captured SQL report.
The preferred combination reduces cold CPU by 33% and edit CPU by 42% on the pinned workload.
The accepted changes now form the normal execution path. The engine has no experiment switches.
The historical measurements describe commit `b6934ddf`, before this promotion.

## Final decisions

| Proposal | Decision |
| --- | --- |
| Method identity index | Keep in the quality generator and generated policy |
| Return range index and bound | Keep in the quality generator and generated policy |
| Narrow projections | Apply during normal policy generation |
| Completed-output virtual table | Use in every build and remove SQL-copy publication |
| Forced join order | Reject and retain SQLite's join order |
| UNION ALL, materialized CTE, and unpivot roots | Reject and retain the original root query |
| Input row and capture adapters | Remove from the engine and profiling addon |
| Shared input kind index | Remove because the measurements show no established gain |
| Batch-size control | Remove and retain bounded 32-row batches |
| Stage-retention control | Remove and always release after the last consumer |
| Older authoring backends and broad shared scope | Remove their controls and retain the current three source domains |

The profiling feature now adds observation only. It does not select an execution algorithm.
The isolated historical SQL laboratory remains outside the engine's dependencies.
Old experimental commands require a checkout of the corresponding historical commit.

The permanent path passes the upstream corpus, production gates, and 177 edit transitions.
It also passes 400 native tests in each build and the five tests in the producer gate.
The ordinary build has no profiling feature. Its bounded build check disables LTO.
Three fresh runs record median CPU of 1,967 ms cold and 707 ms for an edit.
The [promotion evidence](../fixtures/host/data-layer-port/profiling/sql-shape-decisions.json) records those runs and the historical control.

## CPU results

Each result is the median of three fresh processes on the same machine and addon.
The workload contains five files and 1,026,452 source bytes at data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
CPU includes user and system time for each phase. Compilation is a separate phase.
These runs exclude SQL tracing and external sampling.

| Experiment | Cold CPU (ms) | Edit CPU (ms) | Peak RSS (MiB) |
| --- | ---: | ---: | ---: |
| Baseline | 2,966 | 1,180 | 657 |
| Method index | 2,822 | 1,011 | 621 |
| Forced owner-first joins | 2,842 | 1,048 | 656 |
| Method index and forced joins | 2,799 | 985 | 652 |
| Return range index and redundant bound | 2,943 | 1,120 | 647 |
| Narrow projections | 2,815 | 1,098 | 634 |
| Root selection with UNION ALL | 3,027 | 1,197 | 618 |
| Root selection with a materialized CTE | 3,131 | 1,296 | 642 |
| Root selection with one unpivot scan | 3,080 | 1,247 | 657 |
| Virtual publication | 2,412 | 989 | 446 |
| Preferred combination | **1,991** | **685** | **395** |

The preferred combination uses the method index, return bounds, narrow projections, and virtual publication.
It retains the original root selection and lets SQLite choose the join order.
A fourth baseline run records 2,996 ms cold CPU and 1,185 ms edit CPU.
This control agrees with the original baseline.

The [evidence file](../fixtures/host/data-layer-port/profiling/sql-shape-spikes.json) contains individual measurements, hashes, build settings, counters, and parity results.
The addon uses engine optimization level 1, dependency optimization level 3, thin LTO, and 256 codegen units.
All measurements use the same system allocator shim and four native workers.
These results describe this workload and build. They do not establish gains for every rule or repository.

## Join and index access

The `conditional_methods(__path,id)` index fixes the main `self_stores` access problem.
SQLite then joins ownership before the matching method, without a forced join order.
The diagnostic profile records 8.1 million VM instructions before this change and about 77,000 after it.
Native indexed lookups decrease from 366,302 to 483.
Forced `CROSS JOIN` controls establish the join-order effect, but the preferred combination uses the index alone.

The `integer_returns` query also scans too many return candidates.
The experiment adds an index on `returns(__path,__start,__end)` and an upper bound on `r.__start`.
The existing predicate already requires `r.__end<=m.__end`, and valid AST spans satisfy `start<=end`.
The additional bound therefore preserves the rule's predicate and gives SQLite a bounded index range.
VM instructions decrease from 8.76 million to about 74,000 in the isolated experiment.
The 273 sorts remain, so candidate access explains this gain.

## Projections and derived facts

The generator walks stage dependencies backward through SQLite bytecode.
It replaces unused output cells with NULL, while preserving column order, logical width, and row multiplicity.
Indexes and deferred expressions also contribute column demands.
The experiment preserves DISTINCT, grouping, and window semantics inside the original query.

The two `py_names` stages retain five active output columns instead of 70 and 53.
Their native column reads decrease from 4.70 million to 1.15 million.
Their aggregate diagnostic CPU decreases from 181 ms to 98 ms in the isolated experiment.
Total native column reads decrease from 19.10 million to 12.07 million.

This prototype does not remove physical columns or share derived tables between SQLite connections.
It establishes the value of column demand before a compiler change.
A production implementation needs demand propagation across all query alternatives and explicit cache identity for shared derived facts.
This experiment does not measure a shared derived-fact cache.

## Completed output publication

The optional virtual table reads the completed numeric output directly.
Forward and reverse CSR postings support root and node equality lookups, including batched IN requests.
SQLite retains joins, filters, affinity, NULL handling, collation, and result semantics.
The adapter performs no rule-specific inference and runs no producer inside a callback.

Diagnostic publication CPU decreases from 671 ms to 19 ms.
Some SQL consumers become more expensive, so the net cold CPU gain is smaller than the publication saving.
The existing count, scan, and scatter passes remain unchanged for each dependency wave.
The adapter provides a CPU reference over completed arrays. It does not implement a GPU backend.

The prototype guards retained arrays, both posting indexes, and scratch offsets with the existing 256 MiB output limit.
It retains the existing cancellation and work limits.
Its dense offsets depend on the maximum node ordinal, so sparse ordinals can waste memory.
The postings do not include the file identity, so SQLite filters candidates from other files.
Both layouts can increase cost for sparse selections or many files. The output guard bounds their allocation.
The module retains the source for SQLite reconnects. Live tables and cursors also retain the source.
DROP releases the module's source reference. The final table or cursor release then frees the completed output.

## Root selection

All three root experiments preserve the final DISTINCT over `(path,id)`.
The UNION ALL experiment changes finite seed branches only. It does not change recursive traversal on general graphs.
The materialized CTE reuses one narrow SQL table across those branches.
The unpivot experiment uses one metadata scan and `json_each(json_array(...))`.

Every version passes parity, but every version increases CPU in three repeated runs.
UNION ALL removes sorts and reduces VM instructions, without a CPU gain.
The materialized CTE and unpivot reduce native cursor advances but add more expensive intermediate work.
The preferred combination therefore retains the original root query.

## Parity and remaining work

The preferred combination passes 200 upstream cases: 135 positive cases and 65 clean cases.
It also passes 219 full-diagnostic and workspace comparisons, plus 177 edit transitions.
Additional differential tests cover file identity, reverse lookups, empty output, duplicate IN values, real values, text affinity, NULL, and collation.
The producer gate checks bounded execution waves and resident input reuse.

The next frontier remains ordinary SQL materialization and indexes.
The preferred diagnostic capture records about 1,037 ms in SQL materialization and 241 ms in ordinary index construction.
Root selection still costs about 120 ms.
These diagnostic measurements explain work distribution. They are separate from the untraced benchmark medians.
Two short macOS `sample` captures also show SQLite execution, record decoding, comparisons, and virtual-table column callbacks among the busiest functions.
These captures include stage tracing, whose writes also consume samples. They provide directional attribution rather than benchmark timings.
The next experiments need physical column pruning, shared derived facts, and narrower access to `scoped_nodes` and `scope_owners`.

## Reproduction

Generate the policy through the normal sixth-batch generator:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/dsl/data_layer_generate_batch_six.py \
  --source /path/to/pinned/nera-data-layer --repo /path/to/nera-data-layer-repo
```

Select the profiling addon through `RERAIL_ENGINE_PATH`.
Run a fresh process for each measurement:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/profiling/run.py \
  --recording .rerail/profiles/batched-sql-integration/profile-five/selected-recording.json \
  --policies packs/data-layer/candidates/rules \
  --trace off --deadline 115 --allocator system-malloc-shim \
  --output .rerail/profiles/sql-shape-current-run-1
```

For a diagnostic capture, add `--sql-profile --trace on --cold-only` and use a new output directory.
The same publication adapter runs in ordinary and profiling builds.
The publication regression compares completed output against an independent recursive SQL rule.
The old table adapter and runtime selection flags no longer exist.
