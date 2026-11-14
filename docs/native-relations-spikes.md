# Native SQL relation experiments

This document records historical experiments. The [final decisions](sql-shape-spikes.md#final-decisions) describe the current execution path.
The current branch removes experimental execution flags and alternate input adapters.
Historical commands require the corresponding older commit.

Historical measurements. The [implementation report](native-relations-implementation.md) describes the adopted features and new benchmarks.

Keep SQL for rule decisions, joins, aggregation, and diagnostics.
Native algorithms can supply facts through functions and virtual tables.
These experiments preserve that boundary and leave production defaults unchanged.

The strongest candidate is native tree indexing behind the existing reachability contract.
Broad virtual tables, automatic column demand, and generic inference need more work before adoption.
The measurements below include preparation costs.

## Interfaces and storage

| Experiment | Interface | Implementation |
| --- | --- | --- |
| AST metadata | `native_nodes` virtual table | Reads the real parser tape; projects text only when requested |
| Point access | `node_text`, `owner`, `is_ancestor` functions | Reads the same tape and owner metadata |
| Structural relations | Existing `reachability` virtual table and `is_reachable` function | Shares a strict forest index; the general graph adapter remains available |
| Bindings | `native_bindings` table and `binding_start` function | Builds the existing `ScopeTable` once, without a second resolver |
| Resolved calls | SQL joins over calls and bindings | Keeps call selection and resolution decisions in existing components |
| Finite-set inference | `flow_has` function and inferred relation | Shares one bounded worklist result with dense bitsets |
| Existing capture inputs | `--native-input captures` | Retains shared capture projections and reads requested fields lazily |
| Owned inputs | `--native-input rows` | Retains complete SQL row vectors; this is an allocation control |
| Narrow materialization | `narrow` tape backend | Stores metadata in SQLite and gets text through a function |
| Column demand and lifetime | Replay compiler | Uses SQLite bytecode dependencies, narrow tables, logical views, and last-consumer release |

The native interfaces compile only with `code-rule-profile`.
The tape experiment exports the `NativeSqlSpike` class through the profiling addon.
Its current parser adapter supports Python.
Each instance owns an immutable source snapshot and connection.
Node ordinals belong to that snapshot.
They are not persistent identities across edits or files.

Equality and `IN` lookups use native postings where available.
SQLite rechecks predicates, including type and collation behavior.
Unsupported affinity conversions use a scan.
Point functions and relation queries share the same underlying result.
Lazy inference starts during execution, not query planning.

The binding and resolved-call experiment is file-local.
Existing reference-edge inputs still read the engine's native graph through the owned-row adapter.
The experiment adds no cross-file resolution semantics.

The owner adapter originally scanned every owner row for each lookup.
It now builds reverse postings for function, class, and outer-function owners once.
Lookup work is proportional to requests plus returned memberships.
The regression issues 200 owner requests and checks this work count.
The postings contain at most three memberships per tape node.
The constructor eagerly builds these arrays and postings, so cold measurements include their cost.

The capture adapter shares existing `Arc<Value>` projections and interns paths.
It avoids another complete SQL cell vector and projected-string copies.
It still retains all matcher projection values.
It is not a canonical typed column store, and it does not eliminate extraction costs.
The owned-row adapter continues to store complete vectors.

## Measurements

The workload contains five pinned data-layer files, with 1,026,452 source bytes.
The source commit is `2b1504adc4291daaf080b2e932962b134da1a69a`.
[The evidence file](../fixtures/host/data-layer-port/profiling/native-relations-metrics.json) records hashes, counts, phase timings, and artifact names.
Raw profiles remain under `.rerail/profiles/native-relations-spike/`.

### Complete captured-input SQL replay

Each row includes input loading, planning, stage execution, and result retrieval.
Parsing and matcher extraction are outside this replay.
The control uses shared SQL producers and batched recursive SQL.
All six variants preserve complete captured finding rows, including multiplicity.

| Backend | Pipeline CPU | Process peak RSS |
| --- | ---: | ---: |
| SQL control | 7.30 s | 640 MB |
| Owned-row virtual inputs | 7.19 s | 727 MB |
| Column demand and lifetime | 7.81 s | 589 MB |
| Native forest traversal | 6.70 s | 669 MB |
| Native IR-name propagation | 7.55 s | 666 MB |
| Combined experiments | 7.44 s | 733 MB |

These are single runs on a shared host.
Small differences do not establish a speed improvement.
RSS includes the captured JSON, interpreter, and SQLite allocations.

The forest experiment reduces graph-stage CPU from 2.67 s to 1.99 s.
Traversal and pair indexing fall from 1.23 s to 0.37 s.
Native loading and indexing add 0.17 s.
Compact-key preparation still costs about 1.44 s.
The next graph improvement is to remove that bridge by reading tape identities directly.

The generic worklist makes the existing `ir_names` stage slower: about 152 ms versus 9.5 ms in SQL.
The follow-up breakdown attributes about 64 ms to SQL preparation and graph retrieval, 20 ms to serialization, and 55 ms to loading and solving.
Some files have no seeds but still pay preparation costs.
Keep this particular inference in SQL.
The native algorithm remains useful for testing larger cyclic analyses, but this workload does not justify adopting it.

Automatic demand removes only six of 1,993 logical columns in the main shared pipeline.
The conditional pipeline falls from 1,363 columns to 1,254.
Wide intermediate consumers and window operations keep most fields live.
The experiment reduces process RSS but adds planning and view costs.
A stronger solution needs explicit narrow producer contracts and demand before extraction.

### Actual engine input adapter

Both runs use the same addon, policy, source, allocator, tracing mode, and five-file workload.

| Input backend | Cold CPU | Process peak RSS |
| --- | ---: | ---: |
| SQLite tables | 36.92 s | 1,116 MB |
| Shared capture access | 36.99 s | 1,104 MB |

The capture adapter reduces input-stage CPU from about 1.58 s to 0.27 s in the sampled comparison.
Subsequent SQL materialization grows by about 1.10 s.
It moves work rather than establishing an overall improvement.
Fewer input copies alone do not solve repeated wide consumers.

The native crate uses `opt-level=0` and 256 codegen units for these experiments.
Dependencies and the standalone extension use release optimization.
Do not compare these engine totals with earlier `opt-level=1` reports.
The unsampled comparison records 45.94 s and 38.37 s wall time, but almost identical CPU.
Shared-host scheduling prevents treating that wall-time difference as an algorithmic improvement.

### Tape access suite

This suite measures real tape construction and SQL access, rather than full linter execution.
It includes metadata scans, parent joins, owner joins, lazy text, descendants, bindings, and resolved-call joins.
Every backend returns identical result digests across five files and two query passes.

| Backend | Total CPU | Process peak RSS |
| --- | ---: | ---: |
| Direct tape virtual table | 1.38 s | 395 MB |
| Full SQL materialization | 1.27 s | 448 MB |
| Narrow SQL materialization | 1.24 s | 429 MB |

Virtual access saves memory here, but repeated callbacks can outweigh initial materialization.
This supports a hybrid choice based on measured reuse and required columns.
It does not support replacing every table with a virtual table.

## External profiler findings

The bounded runner captured three macOS `sample` profiles for actual engine execution and three for combined SQL replay.
The reports include bottom-up self and inclusive stacks.
Sampled runs are separate from the unsampled comparison.

The first two engine samples concentrate in matcher selection and tape navigation.
They include `FileAnchorIndex::relation_selection`, binary searches, and memory movement.
The third sample includes SQLite record comparisons and virtual-machine execution.
Trace output also appears in that sample and is measurement overhead.
These observations apply to this experimental build.

The replay samples concentrate in SQLite execution, B-tree lookups, record comparison, sorting, allocation, and copying.
They agree with the measured cost of wide materialized relations and indexes.
The native inference solver does not become the dominant sampled cost.

## Recommended architecture

1. Keep shared SQL producers and the existing batched request/result contract.
   Carry scope guards into shared producers so unrelated files cannot contribute facts.
2. Add snapshot-scoped native fact sources with explicit identity, completion, budget, and lifetime contracts.
   Reuse existing binding and call analysis.
3. Use scalar functions for point membership and single-value access.
   Use virtual tables for enumeration, batched lookup, reverse lookup, and joins.
4. Offer native forest indexing as a graph adapter with direct tape keys.
   Keep general graph semantics explicit.
   Reject cycles and multiple parents in the strict tree adapter.
5. Materialize narrow, frequently reused relations in SQLite.
   Leave broad source text behind a lazy interface.
6. Add explicit field demand before matcher projection and extraction.
   Cache resolved demand by policy, options, schema, and source version.
   Release physical relations after their last consumer.
7. Keep the current IR-name inference in SQL.
   Require a demonstrated expensive fixed point before selecting a native solver.

[PR 2539](https://github.com/neralabs/nera-data-layer/pull/2539) supports the demand and storage direction.
Its transferable ideas are typed fact catalogs, field demand, shared producers, narrow rows, and explicit invalidation.
The bytecode experiment shows why demand added after broad extraction has limited value here.

The experimental worklist does not implement a language type system or control-flow analysis.
It computes finite-set propagation over supplied graph edges, generators, and kills.
The replay supplies actual SQL-selected `ir_names` seeds over reversed syntax edges.
Rule selection and diagnostic logic remain in SQL.
The solver retains result rows and bitsets, so it is not memory-free enumeration.
Its state cap permits up to eight million words per bitset matrix, with three working matrices plus output overhead.
Forest enumeration also allocates and sorts each requested result vector.
Only point ancestry tests use constant-time interval checks.

## Validation and reproduction

- Both native input adapters pass all 200 upstream corpus cases: 135 positive and 65 clean.
- The combined replay passes all 200 SQL cases: 134 positive and 66 clean.
  One unchanged nonrelational positive accounts for the difference.
- Production scope probes pass 19 cases and 57 clean, restore, and removal transitions against SQLite input tables.
- Full native inference rows match the SQL `ir_names` relation on the five-file workload.
- The shared-capture runtime and tape suite pass 213 tests.
  The owned-input runtime passes 206 tests.
- All 48 Python profiling and differential tests pass with the extension loaded.
- Type checks, repository checks, formatting, and Clippy pass.

Builds, tests, replays, and profiles use a maximum 115-second controller budget.
The controller terminates the process group before that limit.
The longest successful native build took about 95 seconds.
Early loader attempts hit macOS code-signature validation.
Copied, signed artifacts resolve that problem.
Those attempts and invalid subset-path probes are excluded from the measurement tables.

Build the optional addon with `code-rule-profile` and select it through `RERAIL_ENGINE_PATH`.
The standalone extension builds from `scripts/profiling/reachability/Cargo.toml`.
The scripts below accept paths to local captures and compiled artifacts:

```sh
python3 scripts/profiling/run.py --deadline 115 --trace off \
  --native-input captures --recording RECORDING.json --cold-only --output RUN_DIR

python3 scripts/profiling/run.py --deadline 115 --trace off --output RUN_DIR -- \
  python3 scripts/profiling/native_relations_replay.py \
  --capture CAPTURE.json --extension EXTENSION --backend combined \
  --offset 0 --limit 40 --output RESULTS.json

python3 scripts/profiling/run.py --deadline 115 --trace off --output RUN_DIR -- \
  node --expose-gc --import tsx scripts/profiling/native-tape.ts \
  --recording RECORDING.json --backend narrow --limit 5 --output RESULTS.json
```

Use a fresh output directory for each command.
Use `--verify-intermediates` for inference correctness runs, separately from timing runs.
Use `--external sample` for bottom-up sampling on macOS.
A selected file changes project absence checks, so subset replay compares against SQL on that same subset.
Whole-corpus replay compares against complete captured native findings.
