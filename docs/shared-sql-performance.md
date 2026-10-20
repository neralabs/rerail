# Performance after shared SQL grouping

The subsequent [SQL experiments](sql-performance-spikes.md) measure the proposals in this report.

## Scope and result

Keep the shared SQL implementation.
This investigation changes profiling tools, SQL probes, and documentation only.
The policy digest and native binary match the previous shared-SQL measurements.

The workload contains five pinned production files and 1,026,452 source bytes.
Every command uses the profiling controller with a deadline of at most 115 seconds.
All four workload runs pass the recorded parity checks.

| Run | Cold wall | Cold process CPU | Edited-file wall |
| --- | ---: | ---: | ---: |
| Tracing and sampling disabled | 14.810 s | 14.358 s | 3.778 s |
| Attributed tracing, no sampling | 15.267 s | 14.701 s | 4.437 s |
| External sampling, tracing disabled | 19.702 s | 14.740 s | 5.972 s |

The unsampled CPU time matches the previous 14.370-second result.
The lower wall time does not represent another implementation improvement.
External sampling increases elapsed time, so the unsampled run supplies the latency baseline.
A fourth run collects denser cold-only samples and records their driver-phase windows.

The [recorded evidence](../fixtures/host/data-layer-port/profiling/shared-sql-next-profile.json) contains fingerprints, stage timings, SQL cardinalities, and external samples.
Raw artifacts remain in `.rerail/profiles/shared-sql-next-2026-10-20/`.

## Measured CPU costs

Percentages use the attributed run's 14.701 seconds of cold process CPU.
The table adds disjoint leaf spans, not inclusive parent spans.

| Work | CPU | Share of cold CPU |
| --- | ---: | ---: |
| SQL table materialization | 6.578 s | 44.7% |
| SQL index construction | 3.021 s | 20.6% |
| AST input extraction | 1.778 s | 12.1% |
| SQLite input insertion | 0.446 s | 3.0% |
| SQL analysis, validation, and results | 0.192 s | 1.3% |
| Work outside these leaf spans | 2.685 s | 18.3% |

The final category includes workspace preparation, dispatch, matching setup, allocation, teardown, and instrumentation.
The current spans do not separate these costs sufficiently for individual estimates.
Do not interpret the aggregate field `manifestCandidateLookupNs` as a measured lookup operation.
That field also receives the profiler's default timer category.

External bottom-up samples confirm substantial work in `sqlite3VdbeExec`, B-tree navigation, record comparison, memory comparison, and memory copies.
Native matching samples expose tape child-field lookup, anchor selection, allocation, and budget counters.
The sampler attributes allocator and comparison leaves to SQLite when their caller chain contains SQLite.
Stack percentages describe sampled windows. They are not whole-run CPU percentages.

## 1. Descendant closure remains the largest individual cost

Materialization and indexes for `descendants` consume 3.977 seconds, or 27.1% of cold CPU.
The file-local quality-contract closure contains 1,095,879 rows for 113,953 syntax nodes.
Its roots include every syntax node.

The largest file, `commit_branch_steps.py`, produces 631,463 closure rows for 67,774 nodes.
The existing SQL computes all root/node pairs through a recursive `UNION` and builds two index permutations.
Query plans confirm an indexed edge lookup. This is not an accidental full scan of the edge table.

Stage materialization is eager.
A later SELECT cannot push root filters into a temporary table that an earlier statement already constructed.
The SQL construction queries must receive those restrictions explicitly.

The path, root, and node strings total 113,048,228 logical bytes across the five file-local closures.
Repeated paths account for 61,475,460 bytes, or 54.4% of that key text.
The largest file contributes 66,697,780 key-text bytes.
Both declared indexes contain these key fields.
These numbers exclude SQLite record overhead and do not measure simultaneous allocation or peak memory.

Next SQL experiments:

- Derive the required root sets from the actual consumers and restrict recursive seeds where equivalence permits.
- Avoid repeating a constant file path in file-local graph keys. Keep paths in repository-wide relations and final anchors.
- Compare compact SQL node identifiers against the current `kind:start:end` strings.
- Test whether this CST-specific graph can use `UNION ALL` after proving single-parent, acyclic, unique-node invariants.

Consumers also request argument, annotation, binding, loop, and block roots.
Restricting roots to functions alone would change rule behavior.
A generic replacement of `UNION` would also require a separate correctness proof.
These are proposed experiments, not implemented optimizations.

## 2. Common syntax and ownership work still repeats

Across all scopes, `nodes` construction and indexes consume 1.675 seconds.
Ownership construction and indexes consume another 1.528 seconds.
Together, these operations account for 21.8% of cold CPU.

The three source groups remain independent SQL scopes.
In the largest file, all three produce 67,774 syntax rows and run the same ownership query.
Each ownership materialization executes exactly 5,299,816 VM instructions.
The ownership input columns come from the same file's syntax graph.

Construction, indexing, and analysis in the secondary file-local groups consume about 2.030 seconds for these two relations.
That is the measured work exposed to further sharing, not a guaranteed saving.
The primary group must still construct its facts, and a combined scope can introduce other costs.

Next, evaluate one common SQL fact scope with explicit detector and source gates.
Retain the callback and inventory gate semantics.
The repository-wide conditional graph remains separate because it uses different input partitions.
No runtime relation cache is required for this experiment.

Broad projections also copy data through `SELECT *` and `SELECT n.*` stages.
Primary syntax text alone totals 9,842,329 logical bytes before downstream copies.
Use explicit column projections and delay large text fields until a consumer needs them.
This text total does not explain the measured 1.335 GB peak RSS by itself.
Heap attribution remains necessary before making a physical-memory claim.

## 3. Input extraction is now visible

The cold pass performs 355 input extractions and produces 339,551 input rows across its SQL scopes.
These extractions consume 1.778 seconds of attributed CPU.
Typed input selectors and their field projections still execute separately.
The catch-all syntax selector must probe many optional fields.

External matching samples include `TapeNode::child_by_field_id` and relation-anchor selection.
The tape implementation searches a node's children for each field lookup.
Several field probes and repeated selectors therefore repeat navigation work.

Investigate narrower per-kind field projections and a shared extraction pass for compatible selectors.
Preserve each input's witness bound and suppression behavior.
The workspace's file-local scheduling probes already stop after one witness and clear their projections.
They do not build a second complete retained copy of every input relation.

## Smaller costs and execution order

Input insertion uses a transaction and a prepared statement, with one execute per row.
It costs about 3% of cold CPU, so bulk insertion follows the larger SQL costs.
SQL validation, statistics, and final result queries together cost about 1.3%.
The aggregate parser timer records only 0.074 seconds in the attributed cold pass.
A parser replacement is not supported by these measurements.

File-local relational execution remains serial.
First reduce repeated construction and graph size, then measure memory before adding file concurrency.
These results identify optimization opportunities rather than unsupported rule-representation features.

## Reproduce the investigation

Collect attributed timings without an external sampler:

```sh
npm run profile:code-rule-dsl -- \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --output .rerail/profiles/shared-sql-attributed
```

For external bottom-up samples, add `--trace off --external sample --sample-every 1 --sample-count 10 --cold-only`.
For the latency control, use `--trace off` without a sampler.
Use a new output directory for each command.
The report assigns external sample windows to driver phases even when native tracing is disabled.

Read cardinalities through a diagnostic policy copy:

```sh
npm run profile:code-rule-dsl -- \
  --trace off --output .rerail/profiles/shared-sql-statistics -- \
  node --import tsx scripts/profiling/sql-stats.ts \
  --policy packs/data-layer/candidates/rules/data-layer-serverplan-quality.json \
  --recording .rerail/profiles/shared-sql-attributed/selected-recording.json \
  --metrics fixtures/host/data-layer-port/profiling/quality-sql-metrics.json \
  --output .rerail/profiles/shared-sql-statistics-result.json
```

The probe retains the original inputs, intermediate SQL, indexes, and partition configuration.
It replaces final diagnostic SELECTs in memory and adds scalar aggregate queries.
It obeys the existing SQL authorization and work limits.
Its diagnostic rows and elapsed time are not parity or performance results.
Use `--tables nodes,descendants,ownership` to restrict collection.

Validation covers probe row counts, unchanged producer SQL, unknown relations, caller attribution, phase windows, and deadline handling.
