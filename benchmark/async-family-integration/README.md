# Reproduce the async family comparison

Read [the comparison](../../docs/comparisons/async-family-integration.md) for boundaries and conclusions.
`manifest.json` records the integration pin, source hashes, binary hashes, build settings, and host.
`corpus-manifest.json` records every input blob. Source corpus files and executable binaries are not committed.

## Build and inputs

Use Rust 1.97.1 or a compatible repository toolchain. The measured build uses `profiling`, optimization level 3, 256 codegen units, and no LTO.
From the repository root:

```sh
cargo test -p rerail-engine --lib --profile profiling --no-run
python3 benchmark/async-family-integration/restore_corpus.py \
  --repo /path/to/local/nera-data-layer \
  --out /path/to/comparison-corpus
```

The restore command reads the pinned Git blobs and verifies SHA-256. It refuses to overwrite different content.
Select the `rerail_engine-*` test executable printed by Cargo.
Set `BENCH_BINARY`, `BENCH_CORPUS`, and `BENCH_OUT` to absolute paths:

```sh
python3 benchmark/async-family-integration/run.py \
  --binary "$BENCH_BINARY" --corpus "$BENCH_CORPUS" --out "$BENCH_OUT" --stage reference
python3 benchmark/async-family-integration/run.py \
  --binary "$BENCH_BINARY" --corpus "$BENCH_CORPUS" --out "$BENCH_OUT" --stage rr-positive
python3 benchmark/async-family-integration/run.py \
  --binary "$BENCH_BINARY" --corpus "$BENCH_CORPUS" --out "$BENCH_OUT" --stage rr
python3 benchmark/async-family-integration/run.py \
  --binary "$BENCH_BINARY" --corpus "$BENCH_CORPUS" --out "$BENCH_OUT" --stage rr-profile
python3 benchmark/async-family-integration/run.py \
  --binary "$BENCH_BINARY" --corpus "$BENCH_CORPUS" --out "$BENCH_OUT" --stage producers
python3 benchmark/async-family-integration/run.py \
  --binary "$BENCH_BINARY" --corpus "$BENCH_CORPUS" --out "$BENCH_OUT" --stage edges
python3 benchmark/async-family-integration/summarize.py "$BENCH_OUT" > "$BENCH_OUT/summary.json"
```

The edge command is expected to fail on the measured baseline. It saves differences before exiting.
The admission check verifies accounting, not complete admission; inspect `completeFamilyAdmitted`.
The summary reports whether the production runner meets the 100-statement budget. The baseline does not.
The wrapper disables file-result and cross-result memoization and shares a private tape cache between stages.
Program preparation uses the engine's store. Record `prepared` and process environment rather than assuming a cold compile.
The measured unprofiled run preceded the addition of `RERAIL_PROF_CHUNKS=1` to the wrapper; its exact environment is recorded.

## Independent semantic check

The measured comparator was Ruff 0.15.7:

```sh
ruff check --isolated --no-cache \
  --select ASYNC210,ASYNC221,ASYNC222,ASYNC230 \
  --output-format json "$BENCH_OUT/edge-fixtures" > "$BENCH_OUT/ruff-edges.json"
ruff check --isolated --no-cache --no-respect-gitignore --config 'exclude=[]' \
  --select ASYNC210,ASYNC221,ASYNC222,ASYNC230 \
  --output-format json "$BENCH_CORPUS" > "$BENCH_OUT/ruff-full.json"
```

Ruff exits nonzero when it finds violations. The full-corpus file list was separately checked to contain all 5,725 files.
The stored `ruff-edge-parity.json` records comparisons by code, relative path, and line.
Ruff timing is not part of the performance comparison.

## Evidence files

- `reference-full.json`: final three-run comparison with stable native plan identities, SQL, EXPLAIN, and counters.
- `reference-before-adapter-fix.json`: initial measurement showing the copied adapter's cache-lifetime defect.
- `historical-reference.json`, `historical-manifest.json`: original published reference evidence.
- `rr.json`: unprofiled combined production job; `rr-positive.json`: 120-finding positive test.
- `profile-rr-profile.json.gz`: full profiled report, including statements and spans.
- `profile-{profile.txt,native.tsv,sql.tsv,spans.tsv}.gz`: original detailed profile exports.
- `hotspots.json`, `hotspot-plans.txt`: compact extracts of dominant native work and join order.
- `producers.json`: three interleaved producer passes, exact call-ID parity, separate core-fact cost.
- `edge-fixtures/`, `edge-parity.json`, `ruff-edge-parity.json`: reproducible semantic disagreements.
- `*-processes.json`: executed commands, environment, status, and process wall time.
- `port-audit.json`: reference-to-integration file mapping at the pinned base, before harness edits.
- `measured-tests.rs.gz`: exact v1 harness used for production measurements; see `manifest.json` for reconstruction.

The final reference uses binary v2. Production, producer, and edge measurements use v1.
The difference is confined to test-oracle plan lifetime and measurement metadata; production code is identical.
The final reference run includes both native oracles in the same binary and asserts raw finding parity.
A passing reference gate does not qualify the authored compiler or its separate JSON admission path.
