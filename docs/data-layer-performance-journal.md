# Data-layer performance journal

This journal records measurements, explanations, and implementation decisions for the data-layer ports.
Each entry identifies its implementation and evidence. Historical results remain intact after a new baseline.
Experiments stay in isolated worktrees or standalone harnesses until a keep-or-reject decision.
The engine retains one chosen execution path. Profiling adds observation and does not select another algorithm.

## Current baseline: 2026-10-21, execution `7da151b8`

This entry establishes the comparison point after removal of the experimental execution paths.
The [baseline evidence](../fixtures/host/data-layer-port/profiling/execution-baseline-7da151b8.json) contains all 15 timing runs, diagnostic summaries, and external samples.
The evidence includes policy, source, binary, allocator, and measurement-code hashes.

All times in this table are **process CPU milliseconds**, including user and system time.
Each value is the median of five fresh processes. Compilation is separate from cold evaluation.

| Pinned workload | Files / source bytes | Compile | Cold | Unchanged | Edit |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quality | 5 / 1,026,452 | 88.2 | **1,940.1** | 1.96 | **710.1** |
| State | 7 / 1,123,712 | 298.3 | **12,512.3** | 1.61 | **1,137.8** |
| Dispatch | 1 / 136,810 | 46.8 | **982.7** | 1.43 | **972.5** |

Quality cold CPU ranges from 1,893.6 to 2,011.0 ms.
State ranges from 12,277.8 to 12,719.3 ms. Dispatch ranges from 954.9 to 1,005.1 ms.
Median process peaks are 444 MiB, 1,223 MiB, and 390 MiB, respectively.
These peaks include compilation and workspace storage. They do not identify particular heap allocations.

The quality result agrees with the promotion measurements of 1,967 ms cold and 707 ms per edit.
This entry records a new measurement series, with no implementation change or new claimed speedup.

### Method and limits

The implementation is `7da151b8e55dccbce883f100a9f941a09583b927`.
All source selections use data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
The harness checks source hashes before evaluation and compares cold and incremental findings.
All 15 timing runs pass. No timing run is discarded.
Every measured command has a 115-second deadline.

The reference addon has SHA-256 `8a1f573e09b7837fabbf07073dbf7064b26ff298a7b104f0394acd2e8cac310f`.
Its source hashes match the committed implementation and promotion evidence.
It uses engine optimization level 1, dependency optimization level 3, thin LTO, and 256 codegen units.
The build includes `code-rule-profile`, with tracing, SQL capture, and external sampling disabled for timing runs.
The allocator is the existing system-malloc shim. The evidence records its library hash.
The runtime uses Node 26.8.1 and four native workers on an Apple M5 Max with 18 logical CPUs.

This is a reference build, not a benchmark of the standard optimization-level-3 production build.
The ordinary correctness build disables LTO and does not enter this timing series.
Cold means the first workspace evaluation in a fresh process. The procedure does not flush OS caches.
The workloads run sequentially, with alternating order between rounds, on a shared host.
Small differences within the observed ranges do not establish an optimization gain.

The three performance selections expect zero findings.
Positive coverage remains separate from timing: the quality policy and addon match the [promotion proof](../fixtures/host/data-layer-port/profiling/sql-shape-decisions.json).
That proof contains 200 upstream cases, including 135 positives, plus 219 production comparisons and 177 edit transitions.
This baseline reuses that proof. It does not claim another corpus replay.

### Diagnostic checkpoint

Three additional captures record SQL statements, plans, bytecode, counters, and native spans.
These captures pass parity and contain no incomplete executions.
They remain outside the timing medians.

| Cold diagnostic span CPU (ms) | Quality | State | Dispatch |
| --- | ---: | ---: | ---: |
| SQL materialization | 1,096 | 5,495 | 675 |
| Ordinary SQL indexes | 251 | 2,410 | 158 |
| Completed numeric publication | 20 | — | — |

These columns describe separate instrumented runs. They do not form percentages of the untraced timing medians.
The evidence retains the five most expensive SQL statements for each workload, including complete SQL and counters.
The local HTML reports provide the complete captures:

- Quality: `.rerail/profiles/rebaseline-7da151b8-20261021/quality/diagnostic/sql/profile.html`
- State: `.rerail/profiles/rebaseline-7da151b8-20261021/state/diagnostic/sql/profile.html`
- Dispatch: `.rerail/profiles/rebaseline-7da151b8-20261021/dispatch/diagnostic/sql/profile.html`

Quality's remaining targets are `scoped_nodes`, `scope_owners`, root collection, and ordinary SQL indexes.
Its capture records about 131 ms in `scoped_nodes` and 124 ms in `scope_owners`.
The main root query costs about 113 ms.

State is now the largest cold workload in this baseline.
Its leading SQL statements construct descendant relations, their indexes, syntax nodes, and ownership.
Those paths still reconstruct facts through the older evidence and SQL representation.
The quality implementation's shared syntax and completed numeric publication do not automatically cover those rules.

Three one-second external samples cover cold intervals in the state workload.
The first window also overlaps the compile boundary by about 14 ms.
The first two windows show matcher selection and allocation. The third shows SQLite execution, comparisons, and memory copies.
These windows expose both preparation and SQL costs. They do not estimate whole-run CPU proportions.
The evidence retains the top self costs and bottom-up caller chains.

### Reproduce this baseline

Use the implementation, source pin, build configuration, and allocator from this entry.
Select the matching addon through `RERAIL_ENGINE_PATH`.
Run each measurement in a fresh process and a new output directory:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/profiling/run.py \
  --trace off --deadline 115 \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --policies packs/data-layer/candidates/rules \
  --allocator system-malloc-shim \
  --output .rerail/profiles/new-baseline/quality/round-1
```

Use `state-seven.json` or `dispatch-one.json` for the other workloads.
Repeat each workload five times without overlap.
The evidence records the exact order, phase CPU values, ranges, and commands.
For diagnostic captures, add `--sql-profile --trace on --cold-only` and use separate output directories.
For external state samples, use `--external sample --sample-every 2 --sample-count 3 --sample-seconds 1`.
Select the native module through `--sample-module` if the addon has another filename.

## Experiment ledger

### 2026-10-20: shared SQL and reachability

**Decision:** keep SQL as the semantic engine and reuse shared facts before comparing execution backends.
The earlier specialized Rust evaluator did not provide a fair comparison against equivalently shared SQL preparation.
Its results do not establish a general native-engine speedup.

The reachability experiments compare recursive SQL, a Rust scalar function, and a Rust virtual table on the same requests.
The implementation at `d97e1239` keeps batched recursive SQL for general directed graphs.
Recursive UNION retains cycle termination and deduplicates converging paths.
UNION ALL is valid only with a proved unique-path invariant.
Scalar probes do not replace a broad batch interface.

Evidence: [shared SQL profile](shared-sql-performance.md), [reachability experiments](reachability-spikes.md), and [batched SQL implementation](batched-sql-performance.md).
The detailed reports retain the original build and workload boundaries.
Their results do not form a cumulative speedup series with later builds.

### 2026-10-20: generic forest traversal and storage

**Decision:** use native structural primitives where their graph contract is explicit, while SQL retains rule decisions.
The implementation at `f52817fc` adds strict forest traversal, deferred source text, and release after the last consumer.
General directed graphs retain cycle-safe SQL.
The IR-name propagation experiment is slower and does not become a native engine path.

The first budget model charges native visits too heavily and treats derived output as new AST input.
The corrected model retains limits and cancellation without those duplicate charges.
This result requires both positive corpus coverage and the larger pinned workload.

Evidence: [native relation experiments](native-relations-spikes.md), [implementation report](native-relations-implementation.md), and [measurements](../fixtures/host/data-layer-port/profiling/native-relations-implementation.json).

### 2026-10-20: shared syntax preparation

**Decision:** remove repeated preparation before adding more execution backends.
The implementation at `c4bd76bf` shares immutable syntax facts and uses direct parser identities and subtree boundaries.
SQL no longer reconstructs those facts for the quality groups.
This change addresses the expensive preparation that limited earlier native-access experiments.

Evidence: [requirement](shared-syntax-requirement.md), [implementation](shared-syntax-implementation.md), and [controlled measurements](../fixtures/host/data-layer-port/profiling/shared-syntax-metrics.json).

### 2026-10-21: broad producer passes

**Decision:** batch independent structural producers above their SQL consumers.
The implementation at `eec751d0` uses count, scan, and scatter passes for each dependency wave.
Resident packed input avoids repeated uploads or preparation between waves.
Tests keep the number of passes constant as roots, files, and policies increase.

The motivation includes the future GPU execution contract. This is a CPU reference, not a GPU speed measurement.
Batching alone does not establish a large CPU gain.

Evidence: [execution contract](gpu-producer-execution-contract.md) and [producer measurements](../fixtures/host/data-layer-port/profiling/batched-producer-evidence.json).

### 2026-10-21: complete SQL instrumentation

**Decision:** observe actual statement execution before another SQL-shape change.
The implementation at `e45dd638` imports statement capture, CPU counters, deferred plans, bytecode, timelines, and searchable reports.
Inclusive operation timings do not add to their child SQL timings.
VM instructions, sorts, and cursor visits supply explanations. They do not replace measured CPU.

Evidence: [profiler method and accounting](sql-workload-profiling.md).

### 2026-10-21: isolated SQL-shape experiments

The experiment implementation is `b6934ddf`.
Each row is a median of three fresh processes, with one binary and the pinned five-file quality workload.
All values are process CPU milliseconds. These are historical controlled measurements.

| Experiment | Cold | Edit | Decision |
| --- | ---: | ---: | --- |
| Baseline | 2,966 | 1,180 | Control |
| Method identity index | 2,822 | 1,011 | Keep |
| Forced owner-first joins | 2,842 | 1,048 | Reject forced order |
| Index and forced joins | 2,799 | 985 | Keep index alone |
| Return range index and bound | 2,943 | 1,120 | Keep |
| Narrow projections | 2,815 | 1,098 | Keep |
| UNION ALL root seeds | 3,027 | 1,197 | Reject |
| Materialized root CTE | 3,131 | 1,296 | Reject |
| Root unpivot | 3,080 | 1,247 | Reject |
| Virtual publication | 2,412 | 989 | Keep |
| SQL combination with UNION ALL roots | 2,548 | 835 | Retain as control |
| That combination and virtual publication | 1,996 | 718 | Retain as control |
| Accepted combination with original roots | 1,991 | 685 | Promote |

The method index lets SQLite select the better join order without a forced CROSS JOIN.
`self_stores` falls from about 8.1 million to 77,000 VM instructions.
The return index and upper start bound reduce excessive candidate scanning, while the sort count stays unchanged.
Narrow projections reduce unused column reads and retain row multiplicity and logical schemas.
They do not implement physical column removal or a shared cache of derived facts.

Virtual publication replaces output insertion and SQLite index construction with access to completed numeric arrays.
Diagnostic publication CPU falls from 671 ms to 19 ms, with some extra cost in SQL consumers.
The net workload gain is smaller than that publication saving.

All root rewrites regress in repeated CPU measurements.
UNION ALL removes sorts and lowers VM counts. The materialized CTE and unpivot reduce scans.
Those counter reductions do not offset their other execution costs.
The accepted combination retains the original root query.

Evidence: [experiment report](sql-shape-spikes.md) and [individual runs and counters](../fixtures/host/data-layer-port/profiling/sql-shape-spikes.json).

### 2026-10-21: promotion and removal of optional paths

**Decision:** one permanent execution path, implemented at `7da151b8`.
The method and return indexes, column narrowing, and completed-output virtual table become normal behavior.
The engine retains bounded 32-row insertion and always releases stages after their last consumer.
The three source domains remain separate.

Five execution switches, alternate input adapters, and older authoring controls disappear.
The profiling addon no longer depends on the experimental native-access library.
The module retains its source for SQLite reconnects and releases that ownership on DROP.
Live tables and cursors retain their own references.

Validation covers 400 native tests in each build, the producer gate, the positive corpus, production comparisons, and edit transitions.
Typechecking and Clippy pass in both build modes.
The [promotion evidence](../fixtures/host/data-layer-port/profiling/sql-shape-decisions.json) records the exact scope and hashes.

## Future entries

Each new entry must identify the hypothesis, baseline commit, workload hashes, build configuration, and exact commands.
It must record every timing run, parity scope, explanatory counters, and the keep-or-reject decision.
Diagnostic captures must remain separate from timing medians.
A different binary, allocator, workload, or build configuration needs an explicit comparison boundary.
New experiments must not add selectable execution paths to the engine.
