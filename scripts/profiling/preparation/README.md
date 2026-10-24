# Candidate SQL replay host

This host compares physical SQL layouts over captured candidate inputs.
It imports the production SQLite adapters, span codec, CPU clock, storage settings, and replay implementation directly.
It does not parse ASTs or implement rule semantics.
It has no active rule budget. The profiling controller supplies the hard command deadline.

Build the host with the same optimization level as the engine controls:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/sql-preparation/lab-build -- \
  cargo build --release --locked --manifest-path scripts/profiling/preparation/Cargo.toml
```

Run one rotated comparison against a SQL-profile capture:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/sql-preparation/layout-trial-0 -- \
  scripts/profiling/preparation/target/release/rerail-sql-preparation-lab \
  .rerail/profiles/sql-preparation/diagnostic/sql/raw 0
```

Use trials 1 and 2 for the other layout orders.
Each trial reads normal production SQL after duplicate-expression removal.
It checks exact candidate rows and writes its results beside the capture.
The host compares temporary tables, retained postings, and retained shared sets.
The [profiling guide](../../../docs/sql-workload-profiling.md#candidate-preparation-captures) describes capture creation and interpretation.

The CPU totals include replay setup and cache construction.
They exclude parsing, initial publication, earlier structural preparation, and final matcher evaluation.
Complete-pipeline controls remain necessary before an engine promotion.

## Cache-budget controls

The `cache_budget` binary replays the scheduled SQL with explicit main and temporary cache targets, in KiB.
It changes no rule algorithm and adds no production configuration.
Its snapshots use SQLite counters without a `dbstat` scan of retained tables.
The counters report hits, misses, writes, spills, and cache bytes.
Stage and result statements also report VM steps, full-scan steps, and sorts.
Those VM totals exclude index and release batches.

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/cache-budget/control -- \
  scripts/profiling/preparation/target/release/cache_budget \
  path/to/candidate.json 1024 512 .rerail/profiles/cache-budget/control-result.json
```

Use fresh processes and separate outputs for each budget and trial.
Compare complete `.rows.json` multisets before comparing CPU.
Check VM totals to distinguish changed query work from changed storage traffic.
SQL CPU covers preparation, execution, indexes, result collection, and relation release.
Copying, capture parsing, scheduling, and counter snapshots remain outside that SQL timer.
The outer profiler separately records process CPU and RSS, including setup.
Captured inputs and backing files remain unchanged.
An ownership replay still uses the `lifetimes` binary; its table scans can alter page-cache contents.
Use the counter-only command for cache-capacity comparisons.

## Composition kernels

The `composition` binary compares four SQL shapes over a private copy of a captured database.
It imports the same production SQLite functions, streaming node table, and packed decoders.
Its bounded column table is an isolated terminal prototype.
The engine does not import this binary or its experimental shapes.

Build the kernel host:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/composition-build -- \
  cargo build --release --locked --manifest-path scripts/profiling/preparation/Cargo.toml --bin composition
```

Run a rotated comparison:

```sh
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/composition-range-controller -- \
  python3 scripts/profiling/composition.py \
  --capture .rerail/profiles/sql-preparation/diagnostic-positive/sql/raw/candidate-23476-0.sqlite \
  --files 3 --experiment range --trials 6 \
  --output .rerail/profiles/composition-range
```

Use a closed candidate capture with complete `decoded_properties` for the requested files.
The host checks complete contiguous preorder trees and one vocabulary before each kernel.
Input captures remain unchanged.
File IDs belong to each capture. The recorded source digests identify the measured files.
The [profiling guide](../../../docs/sql-workload-profiling.md#candidate-preparation-captures) describes capture creation.

| Experiment | Alternatives | Measured selection |
| --- | --- | --- |
| `range` | All-root closure, demanded-root closure, interval join | File `3` |
| `demand` | Eager text, selective syntax, materialized demand keys | File `3` |
| `utility` | Inline expression tree, named CTEs, indexed physical cuts | File `3`, depths `4` and `8` |
| `batch` | Scalar decoders, streaming node table, bounded column chunks | Files `1,2,3` |

Use `--depth 4` or `--depth 8` for the utility stress.
Use a new output directory for each comparison.
Use `--start-trial 3` for an additional matched group after a three-trial comparison.
Each child has a 20-second timeout.
The outer controller enforces the command deadline.

The driver checks complete sorted result multisets before it writes a success summary.
The summary includes individual commands, binary and capture hashes, CPU, RSS, query plans, VM counters, and native decoder counters.
Kernel CPU includes variant construction, indexes, preparation, execution, and result collection.
Common typed fact construction has a separate timer and also appears in the setup-inclusive total.
Input integrity checks, copying, serialization, and EXPLAIN diagnostics remain outside those totals.
Peak RSS includes parity output storage and diagnostic preparation.
Temporary page counts exclude transient CTE spools and journal files.
The prototype's `batch.waves` counter counts CPU decode chunks rather than GPU launches or dependency waves.

Run the focused checks:

```sh
cargo test --release --locked --manifest-path scripts/profiling/preparation/Cargo.toml --lib
python3 test/scripts/test_sql_composition.py
```

The [design](../../../docs/sql-composition-opportunities.md) and [journal](../../../docs/data-layer-performance-journal.md) record decisions and integration limits.


## FTS publication boundaries

The `fts_publication` binary isolates publication with the exact production positional FTS declaration.
It compares 32 bulk inserts with separate transactions, those inserts within one transaction, and one complete bulk insert.
It stages 1,024 files with 256 tokens each before measurement and rotates the order across three trials.
CPU includes FTS publication and commit. Setup and verification remain outside that interval.
Every variant must preserve positional vocabulary rows, deletion, and replacement.

```sh
cargo build --release --locked --manifest-path scripts/profiling/preparation/Cargo.toml --bin fts_publication
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/fts-publication/control -- \
  scripts/profiling/preparation/target/release/fts_publication \
  .rerail/profiles/fts-publication/result.json
```

Use new controller and result paths. The executable refuses to overwrite a result.
This binary has no production execution switch.
The [recorded experiment](../../../fixtures/host/data-layer-port/profiling/fts-bulk-publication-20261023.json) preserves raw trials and build identity.
The result motivates broad publication across ingestion packets. It does not establish a complete-pipeline speedup.

## Private owner projection replay

The `owner_projection` binary applies the production CTE rewrite to an existing candidate SQL capture.
It streams each owner's final `(fid, pre)` projection into CTAS while preserving upstream shared CTEs.
It copies the database and preserves parameters, stages, packet consumer IDs, and the dependency schedule.
It does not execute rule decisions or select a different engine algorithm.

```sh
cargo build --release --locked --manifest-path scripts/profiling/preparation/Cargo.toml --bin owner_projection --bin cache_budget
scripts/profiling/preparation/target/release/owner_projection \
  /path/to/candidate.json .rerail/profiles/owner-projection/candidate.json
python3 scripts/profiling/run.py --trace off --deadline 115 \
  --output .rerail/profiles/owner-projection/measurement -- \
  scripts/profiling/preparation/target/release/cache_budget \
  .rerail/profiles/owner-projection/candidate.json 1024 512 \
  .rerail/profiles/owner-projection/memory.json
```

Use a fresh output directory. The rewrite refuses an existing output file or copied database.
Compare the original and rewritten row multisets, VM counters, compilation peaks, and stepping heap peaks.
The copied packet IDs permit an exact intermediate comparison across the two programs.
Allocation-accounted replay timings are diagnostic. Run complete-process CPU controls separately with tracing disabled.
This rewrite removes VM materialization work; it does not reduce engine-submitted statement counts.
The [journal](../../../docs/data-layer-performance-journal.md#selector-sql-owners-2026-10-24) records the accepted cleanup and rejected family-packing experiments.

## Shared candidate ownership replay

Candidate captures now use format v3. They include store setup, producer slots, batch members, and the executable schedule.
Use `cache_budget` to replay that schedule directly. Do not reconstruct a schedule from the rendered SQL.
The shared store's writes would hide the separate logical input lifetimes.

For an older frozen capture, `candidate_owner` applies the production ownership compiler to its original logical graph.
It preserves consumer ordinals for an exact intermediate multiset comparison. Use a fresh output directory.

```sh
cargo build --release --locked --manifest-path scripts/profiling/preparation/Cargo.toml --bin candidate_owner --bin cache_budget
scripts/profiling/preparation/target/release/candidate_owner \
  /path/to/candidate.json /path/to/logical-graph.json \
  .rerail/profiles/candidate-owner/candidate.json
scripts/profiling/preparation/target/release/cache_budget \
  .rerail/profiles/candidate-owner/candidate.json 1024 512 \
  .rerail/profiles/candidate-owner/memory.json
```

Current captures include the logical graph in their `graph` field. Keep its array with the frozen source capture.
The recompiler rejects an existing result, an existing copied database, or an already shared program.
It adds no runtime engine option. Compare complete-process CPU separately with tracing and allocation accounting disabled.
The fixed guard also records `chunk-a-budget.json`. This progress check requires all 3,474 expected removals.
The final 100-submission gate remains independent and mandatory.
SQL batch duration belongs to the batch. Use its captured logical membership when analyzing producer costs.
