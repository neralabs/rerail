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
