# Code-rule profiling

Rerail's data-layer workload spends most evaluation time constructing SQL relations and indexes.
The same relations recur across clauses. File partitions also execute serially in the global evaluator.

The profiling command records these costs by rule, clause, file, input, and stage.
It can attach macOS `sample` and produce a bottom-up report from the captured call trees.

The [profile after shared SQL grouping](shared-sql-performance.md) records the remaining costs and SQL statistics probes.
The [SQL experiments](sql-performance-spikes.md) add generated policy variants and production-gate comparisons.
The [reachability experiments](reachability-spikes.md) compare batched SQL with Rust SQLite extensions on captured native inputs.

## Run a bounded workload

Use Python 3.9 or later. Use the normal Node development environment.
Native spans require a native binding built with Cargo feature `code-rule-profile`.
Without that feature, external samples and end-to-end timings still work; the command reports missing native spans.

Start with the one-file dispatch selection:

```sh
npm run profile:code-rule-dsl -- \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/dispatch-one.json \
  --output .rerail/profiles/dispatch
```

The selections require source commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
The harness checks every selected source hash before evaluation.
The selection contains findings filtered mechanically from the verified repository recording.
Source construction therefore needs no full recording or Python environment.
Alternatively, pass `--recording <repository-recording.json>` with the selection.

Use the seven-file state selection for a short external profile:

```sh
npm run profile:code-rule-dsl -- \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/state-seven.json \
  --external sample --sample-every 3 --sample-count 3 \
  --output .rerail/profiles/state
```

Use the five-file quality selection for the largest repeated-stage cost:

```sh
npm run profile:code-rule-dsl -- \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --cold-only --external sample --sample-every 5 --sample-count 4 \
  --output .rerail/profiles/quality
```

The quality selection includes all four canonical conditional-owner files.
Removing these files changes the meaning of its cross-file checks.
All three selections retain the full authored rule configurations.
These production selections expect zero findings. Positive detection remains covered by the linter corpus and native regression tests.

The default deadline is 110 seconds. The maximum accepted deadline is 115 seconds.
On macOS and Linux, the controller reserves three seconds to terminate the process group and write reports.
Each external sampler also has a timeout. A timed-out workload exits with code 124 and retains partial evidence.
Use a new output directory for each run.

External samples select stacks with the Rerail native module by default.
For a standalone SQL host, use `--sample-module '*'` to include all active stacks.
Repeat `--sample-module` to select module or symbol substrings instead.
Reports record this selection, and comparisons require matching selections.

The same controller can profile another Rerail driver or bound a build command:

```sh
npm run profile:code-rule-dsl -- \
  --trace off --deadline 115 --output .rerail/profiles/build \
  -- cargo rustc --release --manifest-path native/Cargo.toml \
     --features code-rule-profile --lib -- -C lto=thin -C codegen-units=256
```

Install the resulting library as the platform's native binding through the normal build process.
For manual replacement, copy to a temporary filename and rename it atomically.
For example, with Cargo's default target directory on macOS arm64:

```sh
cp target/release/librerail_engine.dylib native/rerail_engine.darwin-arm64.next.node
mv native/rerail_engine.darwin-arm64.next.node native/rerail_engine.darwin-arm64.node
```

Grammar generation and native compilation can require separate bounded attempts on a busy machine.

## Read the output

| Artifact | Contents |
| --- | --- |
| `summary.txt` | Phase timings, parity, largest stages, and sampled leaf functions |
| `profile.json` | Full aggregates, host load, native samples, and benchmark metadata |
| `parity.json` | Compilation, cold evaluation, unchanged replay, edit, CPU, RSS, and finding differences |
| `trace.jsonl` | Attributed native spans, CPU time, SQL counters, and cache decisions |
| `trace.chrome.json` | Timeline for Perfetto or a Chrome trace viewer |
| `sample-*.txt` | Original external profiler call trees and flat listings |
| `stderr.log`, `stdout.log`, `sampler.log` | Original process output |
| `selection.json`, `selected-recording.json` | Checked selection and exact workload |

Compilation, cold evaluation, unchanged replay, and edit have separate phase markers.
Other drivers can emit `RERAIL_PROFILE_PHASE {"phase":"cold"}` on stderr to separate their own phases.
Use `--cold-only` to omit incremental phases. Their timings then remain null.
Use `--trace off` to disable attributed spans on the same binary.
This switch retains existing aggregate instrumentation in a profiling build.

SQLite input insertion uses batches of up to 32 rows in ordinary builds.
AST evidence and reference edges use the same insertion helper.
The helper also respects SQLite's parameter limit and a 1 MiB payload target.
A larger individual row uses its own statement.
Prepared statements and conversion buffers serve multiple batches within the existing transaction.
Each row retains its budget and cancellation checks.

In profiling builds, use `--insert-batch 1` for the single-row control.
The controller defaults to 32 rows and records the selected cap in `profile.json`.
The `sql-input` spans record actual statement counts and maximum buffered payload bytes.
Use `--shared-input-index` for optional document-scoped kind postings between selectors.
That experiment requires `code-rule-profile` and defaults to disabled.
The controller clears inherited flags.

The native spans separate AST input extraction, SQLite input insertion, statement validation,
table materialization, index construction, statistics collection, and result evaluation.
Materialization spans include SQLite statement counters: VM instructions, full scans, sort operations, and automatic-index rows.
File spans record final-result cache hits and misses.

Parent spans include their children. Add only the disjoint leaf categories for a worker-time breakdown.
Sums across workers can exceed elapsed time. Process CPU timings include all Node and native threads.

The bottom-up converter preserves weighted stack residuals, including truncated stacks.
Its self counts sum to active native samples. Inclusive counts include callees and count recursion once per stack.
Waiting threads and non-native active stacks have separate counts.
Percentages use active Rerail stacks as their denominator, rather than all Node threads.
These percentages describe the captured intervals; they do not estimate whole-run wall percentages.
The report also retains reversed caller chains and active thread identities.
Sample-to-span overlap uses the sampler launch time and is approximate.

Detailed matcher descriptions require the separate `RERAIL_CODE_RULE_ANCHORS` environment variable.
The controller disables these dumps. Attributed spans and aggregate summaries tolerate closed or nonblocking stderr.
Input-extraction spans cover the stateless relational path.
Workspace producer scans remain in the older aggregate AST counters and the unassigned wall remainder.

## Measured results

These measurements used the existing release build settings: thin LTO and 256 code-generation units.
The allocator was the existing system-malloc shim, not jemalloc.
The native binding hash and policy digests are recorded in each result.
[The evidence file](../fixtures/host/data-layer-port/profiling/evidence.json) retains the measurements and external sampling summaries.

| Workload | Files / UTF-8 bytes | Compile | Cold | Unchanged | Edit | Peak RSS |
| --- | --- | --- | --- | --- | --- | --- |
| Dispatch, traced | 1 / 136,810 | 54 ms | 1.218 s | 1.206 ms | 1.488 s | 411 MB |
| State, traced + sampled | 7 / 1,123,712 | 366 ms | 13.638 s | 9.998 ms | 1.334 s | 1,067 MB |
| Quality, traced + sampled | 5 / 1,026,452 | 723 ms | 48.839 s | 8.272 ms | 11.119 s | 1,350 MB |

All workloads passed cold and incremental parity, with no engine failures.
These are individual runs on a shared machine with 18 logical CPUs.
Recorded load averages ranged above the CPU count. Quality's later trace-disabled cold run took 88.599 seconds.
These samples cannot establish a stable profiling overhead or a production speed ratio.
The smaller state pair consumed 13.133 CPU seconds with spans and 12.824 CPU seconds without spans.
That single pair also cannot establish a precise overhead percentage.
The current harness leaves subset oracle timings null. Earlier artifacts use a zero placeholder.
No subset oracle duration was measured; the original repository timing does not measure these subsets.

Quality's cold phase had this breakdown:

| Disjoint operation | Worker wall time | Share of cold elapsed time |
| --- | --- | --- |
| SQL table materialization | 25.769 s | 52.8% |
| SQL index construction | 12.392 s | 25.4% |
| AST input extraction | 5.137 s | 10.5% |
| SQLite input insertion | 1.334 s | 2.7% |
| SQL statistics, validation, and results | 0.535 s | 1.1% |

The remainder includes workspace preparation, decoding, result handling, and instrumentation overhead.
The table does not add inclusive file or relational spans.

## Causes and implementation priorities

### 1. Repeated materialization dominates

Quality ran 890 materialized stages, 1,362 index builds, and 1,155 AST input extractions in its cold phase.
The `descendants` tables alone cost 17.380 seconds to materialize and 8.338 seconds to index.
Together, these operations consumed 52.7% of cold elapsed time.

For `commit_branch_steps.py`, six clauses executed the same recursive `descendants` query.
Each execution used exactly 23,860,081 VM instructions. Materialization alone cost 9.695 seconds across those six executions.
The authored stage definition also recurs across the contracts clauses.
Related `nodes`, `edges`, `functions`, and ownership relations recur as well.
The original generator split detector groups to satisfy the 32-stage and SQL-size bounds.
Those groups repeated common prerequisites in separate SQL scopes.
The [shared SQL implementation](relational-execution-design.md) now groups compatible quality detectors and permits 64 stages.

Each relational evaluation creates a fresh SQLite connection and rebuilds its inputs, stages, and indexes.
See [SQL evaluation](../native/src/code_rule_dsl/relational.rs) and the [relation generator](../scripts/dsl/data_layer_six/ast_relations.py).
The final-result cache is keyed by clause and file version.
It does not share common relation prefixes between clauses or rules.
The parsed-tape slot avoids repeated tree restoration within a clause, but does not share projected input relations between clauses.

Share compatible consumers through SQL before introducing another cache.
The quality generator now loads common inputs once and builds shared indexed temporary tables in one clause.
Diagnostic SELECTs read these tables, then combine their outputs with `UNION ALL`.
Different source-trigger groups retain separate execution scopes.

Add `--query-plans` to record SQLite `EXPLAIN QUERY PLAN` output in `sql-plan` trace events.
Use `--policies` to select generated SQL variants.
The comparison command requires `--allow-policy-changes` for different authored SQL and reports every changed policy digest.

### 2. All-root transitive closure creates large ordered relations

The common quality query seeds a walk from every syntax node and expands every ancestor-descendant pair.
Recursive `UNION` also deduplicates rows. Two indexes then order the materialized relation.
Keys contain file paths and textual node identities, which increase comparison and copying costs.
The number of pairs grows with node count and nesting depth; a chain has quadratic worst-case growth.

The external profiler confirms these operations from the bottom up.
The baseline quality interval had 1,594 active native samples on one libuv worker:

| Leaf function | Self samples | Share |
| --- | --- | --- |
| `sqlite3VdbeRecordCompareWithSkip` | 224 | 14.1% |
| `_platform_memcmp` | 206 | 12.9% |
| `sqlite3VdbeExec` | 135 | 8.5% |
| `_platform_memmove` | 130 | 8.2% |
| `sqlite3BtreeIndexMoveto` | 112 | 7.0% |
| `vdbeSorterCompareText` | 73 | 4.6% |
| `vdbeSorterSort` | 70 | 4.4% |

The reversed caller chains lead through SQLite execution into `relational::create_stage`.
All active samples in that interval included stage construction.
Three trace-disabled state samples independently showed the same SQLite and memory-operation costs.
Stage construction accounted for 62.2–86.4% of active samples in those intervals.
This confirms that native span logging did not create the underlying SQL bottleneck.

Next, restrict recursive roots to the consumers that need them.
Use direct ownership or parent walks when a detector needs only an enclosing function or class.
Keep self rows and depth semantics explicit in parity tests.
Evaluate narrower keys and indexes after reducing duplicate relation construction.
Full-scan counters alone do not prove a missing index; recursive seed scans can be necessary.

### 3. Global file evaluation is serial

`evaluate_file_relations` loops over paths and evaluates each cache miss synchronously.
The [workspace global evaluator](../native/src/code_rule_dsl/workspace/global.rs) contains the cache and this serial loop.
Repository-wide clauses also run in the global pass after deferred input ingestion.
The configured four native workers therefore do not imply four concurrent relational evaluations.
Every captured interval showed one active native worker, while other worker stacks waited.

After reducing duplication, evaluate independent file partitions with bounded concurrency.
Preserve per-file budgets, cancellation, deterministic results, and exact cache invalidation.
Use an input-byte or retained-memory bound as well as a worker count.
The quality workload already reached 1.35 GB RSS for five files; unrestricted parallelism can increase memory pressure.

### 4. A changed file repeats the pipeline

Unchanged replay is fast because the workspace retains final clause results.
A changed source version invalidates those results, including for a trailing newline.
The changed file then repeats input extraction, SQL materialization, and index creation for its clauses.
This explains quality's 11.119-second edit despite an 8.272-millisecond unchanged replay.

Shared input and relation work should therefore precede a general incremental-SQL mechanism.
Validate clean-to-positive, positive-to-clean, file removal, option changes, and companion-file edits before accepting reuse.
Use the existing positive linter corpus in bounded chunks when validating each optimization.

## Profiling correctness and validation

The investigation found an aggregate-profiler bug: Rayon can execute a one-item iterator on the apply thread.
`WorkerProfile` cleared that thread's enclosing profile and timer stack, which could omit the final apply summary.
It now preserves an existing session. A Rust regression test covers the nested worker case.
Relational execution is also classified as `other` in the older aggregate CPU report, rather than candidate lookup.
The attributed spans provide the detailed relational breakdown.

Framework tests cover weighted bottom-up counts, recursive stacks, waiting threads, source hashes,
positive finding retention, phase attribution, timeline identity, partial logs, and deadline termination.
CI runs them through `npm run test:profiling`.
Malformed or interleaved log lines produce warnings without discarding the remaining evidence.

This change adds profiling and corrects profiler accounting. It does not implement the proposed engine optimizations.
