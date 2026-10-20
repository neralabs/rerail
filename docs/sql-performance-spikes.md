# SQL performance experiments

These experiments address the proposals in [the shared SQL profile](shared-sql-performance.md).
The shipped policy keeps its existing three file-local SQL scopes.
The SQL policy variants and shared input index require explicit selection.
SQLite executes every relational operation.
Input batching now runs by default in ordinary builds.
The measurements in this report precede that promotion and retain their original batch caps.

## Method

The main workload contains five pinned production files and 1,026,452 source bytes.
All comparisons use one release binding, thin LTO, and 256 code-generation units.
The allocator uses the existing system-malloc shim.
Each command has a 115-second deadline.
The controller reserves three seconds for termination and reports.

The host runs other work, so wall times vary substantially.
The first two baseline runs consume 14.225 and 14.213 seconds of cold process CPU.
The measurements separate compilation from cold evaluation.
Later baseline controls consume 14.385 and 14.459 seconds of CPU.
Differences near one or two percent do not establish an optimization gain.
Peak RSS covers the process, including compilation and retained workspace data.
RSS differences do not identify particular heap allocations.

The [evidence file](../fixtures/host/data-layer-port/profiling/sql-spikes.json) records fingerprints, timings, parity totals, and profile summaries.
Raw artifacts remain in `.rerail/profiles/sql-spikes-2026-10-20/`.

## Isolated experiments

| Experiment | Cold CPU | Peak RSS | Finding |
| --- | ---: | ---: | --- |
| `baseline` | 14.225 s | 1.374 GB | Control |
| `union-all` | 12.526 s | 1.379 GB | Forest only |
| `roots` | 13.830 s | 1.198 GB | Corrected; small gain |
| `no-path` | 12.328 s | 1.190 GB | Useful file-local change |
| `compact-keys` | 13.138 s | 1.267 GB | Mapping cost included |
| `compact-no-path` | 12.279 s | 1.045 GB | Largest isolated RSS reduction |
| `narrow-selectors` | 14.142 s | 1.332 GB | No established gain |
| `narrow-projections` | 13.857 s | 1.313 GB | Small gain |
| `all-shared` | 9.777 s | 1.310 GB | Needs producer gates |
| `insert32` | 14.279 s | 1.384 GB | No established gain |
| `shared-input-index` | 14.063 s | 1.373 GB | No established gain |

The path-free closure retains the path in final diagnostic anchors.
It omits the constant path from file-local closure rows and indexes.
The compact variant assigns integer keys through SQL tables and restores string identities at consumer boundaries.
The measurements include those mapping tables and joins.

The projection experiment uses SQLite column reads and SQLGlot to calculate column dependencies.
It retains index columns and preserves set operations and `DISTINCT` semantics.
It skips syntax serialization helpers and retains original SQL above the 16,384-byte limit.
SQLGlot is an optional authoring dependency, pinned in `scripts/profiling/requirements-lab.txt`.

The selector experiment removes optional field probes whose captures never reach that input's projection.
It changes generated selectors without changing the matcher.
The shared input index reuses immutable kind postings within one restored document.
It preserves cancellation polls and cannot reuse postings across document replacements.
It does not fuse matcher evaluation, projections, suppression, or witness limits.
The common SQL scope already shares extraction of its common inputs.

The insert experiment reuses prepared multi-row statements within the existing transaction.
It handles tails, nullable values, Unicode, row budgets, and SQLite variable limits.
At measurement time, both native experiments required `code-rule-profile` and defaulted to disabled.
Input batching now uses up to 32 rows per statement in ordinary builds.
AST inputs and reference edges share one helper with a 1 MiB payload target and parameter-limit checks.
The profiling controller defaults to 32 and retains `--insert-batch 1` as a control.
The historical measurements remain unchanged.

## Correctness findings

The first restricted-root variant missed four positive corpus cases.
Three SPQ-043 cases require the comprehension containing a `for_in_clause`.
One SPQ-035 case requires unfielded comparison operands.
Named fields and function roots alone do not cover these consumers.
The corrected variant includes both root sets.
The regression ladder first reproduces those failures, then passes the four positives and a clean control.
The full corpus supplies the final parity check.

`UNION ALL` preserves closure rows for a unique-path forest.
Direct SQLite tests expose duplicate rows on diamonds and nontermination on cycles.
The five production files contain no duplicate node identities, duplicate edges, multiple parents, or self edges.
Those observations do not establish the identity invariant for every malformed CST.
The recommended combination therefore retains `UNION`.

The path and integer rewrites pass direct SQLite comparisons for forests, diamonds, cycles, dangling edges, and Unicode paths.
Additional tests preserve explicit root restrictions through composed rewrites.
The tests also protect option cases named `all` from selector pruning.

Each accepted isolated variant passes all 200 oracle-recorded quality cases.
These cases contain 192 recorded test calls and eight supplemental probes.
These cases contain 135 positive calls and 65 clean calls.
Replay compares diagnostic multiplicity, path, line, and code through stateless and workspace execution.
The combined variants pass the same corpus.

Oracle-mode parity does not cover production source gates.
The separate production harness compares unchanged default policies across 219 cases, with 130 positives and 89 clean cases.
It relocates real positive sources across primary, callback, test, and unrelated paths.
It compares complete diagnostic spans and messages while ignoring internal clause identities.
The shared-scope, combined, and guarded combined variants pass these comparisons.
These comparisons use the baseline policy as their reference, not the upstream oracle.

The harness also checks clean edits, restored sources, and removals across 19 boundary cases.
Each leading variant passes 57 transitions.
The native suite covers batched insertion tails and nullable projections.
The Rust regression covers shared-posting ownership and cancellation.

## Combined results and next architecture

| Combination | Cold CPU | Peak RSS |
| --- | ---: | ---: |
| Shared scope + path removal | 8.463 s | 1.169 GB |
| Shared scope + roots + compact path-free keys + narrow columns/selectors | 7.604 s | 0.980 GB |
| Previous combination + `UNION ALL` | 7.478 s | 1.024 GB |
| Previous combination + inserts/index reuse | 7.327 s | 1.012 GB |
| Set closure + producer gates | 7.501 s | 0.977 GB |

The guarded combination reduces cold CPU by about 47%, or 1.9 times the baseline speed.
The recorded run retains recursive set semantics, single-row insertion, and no shared input index.

| Scope-cost control | Cold CPU | Peak RSS |
| --- | ---: | ---: |
| Three original scopes | 3.864 s | 0.767 GB |
| One broad scope | 5.686 s | 1.148 GB |
| Combined rewrites | 4.591 s | 0.874 GB |
| Combined rewrites + producer gates | 2.748 s | 0.766 GB |

The scope-cost control relocates the largest pinned file outside the primary paths.
Its three expected inventory findings come from the baseline policy.
This control isolates scope cost and does not provide a new upstream oracle recording.

In the full-phase pair, edited-file CPU decreases from 3.776 seconds to 2.126 seconds.
Unchanged replay takes 11.97 ms and 10.84 ms, respectively.
Both runs pass cold, unchanged, and edited-result comparisons.

The first common scope loses producer-level path gates.
The scope-cost control exposes the resulting regression outside primary paths.
The refined variant keeps common producers and guards primary-only producers through SQL predicates.
It moves the file-domain relation immediately after syntax nodes.
The gates apply only to production execution and preserve explicit oracle entry points.
A direct SQLite test confirms that a false gate skips the producer expression.

The common SQL scope contains 61 stages.
The integer mapping adds two stages, so the combined graph fits the existing 64-stage limit.
The repository-wide conditional graph keeps its separate partition.
Production diagnostic SELECTs retain their individual path and source predicates.

The implementation sequence starts with common SQL producers and narrower file-local keys.
Producer gates prevent unrelated files from constructing primary-only graphs.
The guarded off-domain closure executes only 46 SQLite VM instructions in the attributed control.
Root restrictions require explicit dependencies from every closure consumer.
The comprehension and comparison regressions demonstrate why a hand-maintained function-only root set fails.
An authoring dependency pass can calculate roots and columns before materialization.
SQLite then builds and indexes the resulting temporary tables once per file and SQL scope.

SQL construction remains the larger measured cost.
Batching now forms part of normal ingestion despite the inconclusive result on this workload.
The shared-posting prototype supplies a bounded foundation for future extraction work.
A complete shared extraction pass still needs compatible selectors, capture environments, suppression rules, and separate witness bounds.
This experiment does not establish a gain for that larger design.

## Remaining costs

| Attributed cold CPU | Baseline | Guarded combination |
| --- | ---: | ---: |
| SQL materialization | 6.741 s | 3.885 s |
| SQL indexes | 3.085 s | 1.212 s |
| Input extraction | 1.908 s | 0.654 s |
| Input insertion | 0.431 s | 0.177 s |
| Outside the measured leaf spans | 2.772 s | 2.051 s |

These runs use tracing and have higher CPU totals than the latency controls.
The guarded trace consumes 8.096 seconds of process CPU.
Closure construction and indexes still consume 1.809 seconds.
The integer maps and edges add 0.261 seconds.
Common syntax nodes and ownership consume another 1.211 seconds.
The remaining scope and function traversals consume about 0.614 seconds.

Four external sample windows lie entirely within cold evaluation.
They contain 3,292 active native samples, including 3,222 samples with SQLite caller chains.
The largest leaves include SQLite VM execution, B-tree navigation, record comparison, memory comparison, and memory copies.
These windows favor SQL work and do not estimate the whole-workload CPU distribution.
The retained bottom-up stacks preserve the caller chains for allocation and comparison leaves.

The next SQL work can reduce closure consumers and repeated scope traversals.
The unassigned 2.051 seconds also require finer workspace and allocation attribution.
The measurements do not justify a parser replacement or file concurrency before these costs decrease.

## Reproduce

Generate one variant into an experiment directory:

```sh
python3 scripts/profiling/sql_variants.py \
  --policy packs/data-layer/candidates/rules/data-layer-serverplan-quality.json \
  --variant compact-no-path \
  --output .rerail/profiles/example-policy/data-layer-serverplan-quality.json
```

Generate the common scope with `data_layer_generate_batch_six.py --quality-all-sql` and the existing pinned source arguments.
Compose variants through repeated `--variant` arguments.
Apply `roots` before a path or integer rewrite.
Apply `union-all` last for its separate forest experiment.
For producer gates, apply `--variant scope-stages --scope-reference <three-scope-policy.json>` after the other SQL rewrites.
For projection pruning, install `requirements-lab.txt` into an isolated Python environment.

Run each measurement separately:

```sh
python3 scripts/profiling/run.py --trace off --cold-only --deadline 115 \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --policies .rerail/profiles/example-policy \
  --output .rerail/profiles/example-run
```

Use `--insert-batch 1` for a single-row control or `--shared-input-index` for the optional input-index experiment.
The historical single-row measurements require `--insert-batch 1`.
The controller clears inherited experiment flags and records the selected values.
The external sampler converter retains unsymbolicated frames so their children keep the correct stack depth.
Malformed captures retain their raw data and a parse error without aborting the workload.
Use `compare.py --allow-policy-changes` for different generated SQL policies.
The comparison still requires matching workloads, binaries, allocators, phases, and profiler modes.

Run upstream replay against a variant with the existing `conformance/data-layer/replay.ts` harness.
Use quality-only recordings and split the corpus into 40-case chunks on a busy host.
Keep positive and clean cases in each chunk.

Run the production comparison separately:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/example-production-gates -- \
  node --import tsx scripts/profiling/sql-parity.ts \
  --baseline packs/data-layer/candidates/rules/data-layer-serverplan-quality.json \
  --candidate .rerail/profiles/example-policy/data-layer-serverplan-quality.json \
  --recording /path/to/quality-only-recording.json \
  --offset 0 --limit 40 \
  --output .rerail/profiles/example-production-gates.json
```

Use successive offsets to cover all cases.
The output records the total case count and distinguishes baseline-relative findings from upstream expectations.
Add `--transitions` for edit, restore, and removal checks.
Use a new output directory for every bounded command.
