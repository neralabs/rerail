# Adopted anchor-first SQL walks

The compiler now emits plain `CROSS JOIN` for candidate walks whose bounds require an anchor row.
This is the single implementation for these walks.
It requires no runtime switch, extra CTE, frame copy, or `ANALYZE` command.
The [shape experiments](sql-query-shape-spikes.md) explain the alternatives and their tradeoffs.

## Compiler behavior

The generated order is input frame, anchor point lookup, then bounded candidates:

```sql
FROM input_frame f
CROSS JOIN node fn ON fn.file = f.file AND fn.id = f.node
CROSS JOIN node c ON c.file = f.file
  AND c.parent = fn.wpar
  AND c.id <= fn.aprev
  AND c.kind IN (215)
```

Previously, ordinary joins let SQLite start with global candidate-kind postings.
It repeatedly visited the file's input frames before checking each anchor's parent and sibling boundary.
The new order supplies the candidate cursor's parent and range constraints when that cursor opens.
The candidate access path changes from `k=` to `f=p=i{k=` in the hot sibling query.

This change covers anchor-dependent parent, descendant, sibling, and field-parent candidates.
Sibling walks with rule stops also keep their anchor ahead of the existing stop-set and candidate joins.
Other walk shapes retain their existing joins.
Capture guards, parent keys, candidate ordinals, and recursive scheduling retain their existing behavior.

## Addon workload comparison

The comparison uses frozen release addons before and after this change, plus the native engine.
Each variant runs in five fresh processes per corpus, with reversed order on alternating rounds.
All 30 runs preserve complete native output parity, including captures, failures, and blind-file results.
The workloads contain 135 policies and compile to 141 SQL clauses.
The 32-file corpus produces 33 findings; the 138-file corpus produces 580 findings.
Both use four workers and two files per SQL batch.

| Corpus | Native apply CPU | SQL before | SQL after | Observed reduction |
|---|---:|---:|---:|---:|
| 32 files, 1.32 MB | 1,300.2 ms | 5,077.7 ms | 4,826.2 ms | 5.0% |
| 138 files, 7.34 MB | 7,551.0 ms | 27,607.4 ms | 27,256.8 ms | 1.3% |

The tables report median process CPU.
The host is an Apple M5 Max with variable concurrent load.
Both addons use the same diagnostic system allocator and `unbounded-work` feature.
The comparison therefore describes this diagnostic configuration and workload.
It does not reproduce the unavailable original 818-clause catalogue.

The large-corpus comparison initially used three rounds.
Its CPU variation exceeded the observed median improvement, so the comparison added two paired rounds.
Per-round reductions range from 0.7% to 5.4% on 32 files and −2.4% to 4.5% on 138 files.
The large-corpus result remains small relative to that variation.
The instruction-count reduction below gives stronger evidence for the corrected join plan than the large-corpus timing difference.

The SQL/native apply-CPU ratio falls from 3.91× to 3.71× on 32 files and from 3.66× to 3.61× on 138 files.
This removes approximately 6.7% and 1.7% of the respective excess CPU over native.
It fixes a severe local inefficiency while leaving most of the engine gap.

Cold compilation, setup, and apply together show similar reductions:

| Corpus | Native total CPU | SQL before | SQL after |
|---|---:|---:|---:|
| 32 files | 1,416.8 ms | 5,303.5 ms | 5,041.4 ms |
| 138 files | 7,661.8 ms | 27,829.0 ms | 27,458.8 ms |

The driver finalizes the SQL program during setup, before timing apply.
On 32 files, median SQL-statement execution CPU falls from 4,264.4 ms to 4,016.7 ms.
Parsing and planning together remain near 285 ms.
The observed apply reduction therefore aligns with reduced statement execution, rather than a parsing or setup change.

## Profile evidence

Paired 32-file captures use the frozen before and after addons.
Both captures pass complete native output parity.
Profile CPU includes instrumentation overhead and does not estimate unprofiled speedups.
Query identifiers can differ between captures; comparisons use the generated SQL text.

| Measure | Before | After | Reduction |
|---|---:|---:|---:|
| Total SQLite VM instructions | 251,656,621 | 233,296,713 | 7.3% |
| Total frame advances | 12,623,013 | 11,225,553 | 11.1% |
| Dependent candidate VM instructions | 31,491,396 | 13,131,672 | 58.3% |
| Dependent candidate frame advances | 2,215,914 | 818,454 | 63.1% |
| Hot sibling VM instructions | 15,426,221 | 65,535 | 99.6% |
| Hot sibling frame advances | 1,178,918 | 1,055 | 99.9% |

The hot sibling query executes seven times and produces the same 1,875 candidate rows in both captures.
Its recorded CPU falls from 125.60 ms to 0.82 ms.
Across 1,962 dependent candidate executions, recorded CPU falls from 313.01 ms to 157.70 ms.
Those executions produce the same 115,642 candidate rows.
The complete workload still publishes 3,604,670 frame rows and executes 34,487 rule, stage, result, and control SQL statements.
This change removes unnecessary visits without removing those statements or intermediate results.

## Remaining work

The after profile records 4,555.2 ms of SQL and exclusive native-operation CPU.
Rule, stage, result, and control SQL accounts for 3,548.8 ms, or 77.9%.
Native operations and submission overhead account for 797.6 ms, or 17.5%.
Setup and transaction SQL accounts for 207.9 ms, or 4.6%.
These recorded spans do not partition apply CPU; parsing and some preparation fall outside them.

Fact-terminal queries take 572.4 ms across 1,158 executions.
Pattern-terminal queries take 441.9 ms across 1,022 executions.
Their CPU includes native matching, demand collection, materialization, and result rejoining.
Their VM instruction counts remain unchanged.
The next measurement must separate native terminal evaluation from SQLite bookkeeping before selecting a compiler change.
Removing materialization alone can restore expensive per-row terminal calls.

Capture-demand insertion takes 300.4 ms across 1,566 executions.
The hottest remaining statement inserts canonical demand keys and takes 95.7 ms and 6,698,391 VM instructions.
Its associated expansion takes another 65.2 ms and 3,869,812 instructions.
These statements repeat canonical atom lookups and key comparisons.
The next compiler experiment should carry computed integer keys through these operators without adding a wide materialized CTE.
The earlier key-CTE experiment cost more than it saved.

Frame index construction still takes approximately 5.1 ms across the entire capture.
Selected-root publication takes 0.63 ms across 16 slice publications.
Neither explains the remaining engine gap.

## Validation and artifacts

The SQL suite passes 159 tests, with 16 ignored experiments.
The generated differential run compares 500 cases with zero differences.
The generator records the same two unsupported forms as the baseline.
The plan audit checks 722 clauses and 33,039 statements.
The new regression checks first captures and bounded frame visits for both sibling directions and both verdict modes.

Clippy with warnings denied, the production library check, formatting, and script tests pass.
The script suite contains 16 JavaScript tests and six Python documentation tests.
Boundary and Windows-path checks pass.
The structure check retains two existing failures: misplaced benchmark fixture data and fixture imports in `policy_guards.rs`.
These paths have no changes in this commit.

The 30 workload runs check the selected engine, four-worker configuration, compiled clause count, and frozen addon hash.
`validation.json` records these checks and both profiled outcome comparisons.

Artifacts are under `/tmp/rerail-anchor-first-20261021/`:

- `before.node`, `after.node`, and their build manifests record addon hashes, source hashes, commit, and build command.
- `sweep-32.json`, `sweep-138.json`, and `sweep-138-extra.json` define the unprofiled comparisons.
- `comparison-32.json` and `comparison-138-five.json` contain every run and its complete outcome digest.
- `merge-large.py` combines the original three large-corpus rounds with the two continuation rounds.
- `profile-before-32/` and `profile-after-32/` contain raw traces, query plans, counters, and interactive reports.
- `profile-comparison.json` matches changed statements and records family totals.
- `sql-suite.log`, `clippy.log`, `production-check.log`, and `script-tests.log` record validation.

The addon build command is identical before and after:

```sh
env JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
  cargo rustc --release --features unbounded-work --manifest-path native/Cargo.toml \
  -- -C codegen-units=16 -C incremental=target/sql-tuning-incremental
```

The benchmark and profile commands are:

```sh
node benchmark/sql-set-tuning.mjs /tmp/rerail-anchor-first-20261021/sweep-32.json
node benchmark/sql-set-tuning.mjs /tmp/rerail-anchor-first-20261021/sweep-138.json
node benchmark/sql-set-tuning.mjs /tmp/rerail-anchor-first-20261021/sweep-138-extra.json
python3 /tmp/rerail-anchor-first-20261021/merge-large.py
node benchmark/sql-profile.mjs /tmp/rerail-anchor-first-20261021/workload-before-32.json /tmp/rerail-anchor-first-20261021/profile-before-32
node benchmark/sql-profile.mjs /tmp/rerail-anchor-first-20261021/workload-after-32.json /tmp/rerail-anchor-first-20261021/profile-after-32
```

The profile commands require fresh output directories.
These local artifacts depend on the recorded external corpus paths and are not repository fixtures.
