# Production input lifecycle evidence

This comparison covers five production changes after the integration merge:
counted kind postings, demanded line lookup, direct validated fact construction,
demanded projection arrays/spans, and exact reservations. It excludes a retained full cache.

The baseline is `e3ea1ad6d66b38e4b9f4e532966ec06854c7f6c9`.
The merged integration tip is `4ab83ed5190d1af7675f6a66544f9b35b972d9b1`.
`manifest.json` records binary identities, source hashes, measurement boundaries, and limits.

## Results

The 5,725-file job uses a warm tape cache and four workers.
Both result memos are disabled. Each arm runs in a fresh process at nice 19.
Three adjacent pairs reverse order in the middle pair.

Median corpus CPU changes from 3.926 to 2.934 seconds.
Every pair saves CPU. SQL remains at 81 executions and 785,573 VM instructions.
`summary.json` contains medians and every paired difference.
`profile-costs.json` separates input phases and cumulative capacity at publication.
`validation.json` records 107 focused test executions, 178 edge findings, and uncached admission checks.
`axes-summary.json` records the independent large-file nearest-scope query.

The full corpus has no positive findings. The positive and edge fixtures supply correctness coverage.
Wall times vary with host contention. Cumulative publication capacity is not peak resident memory.
`containment.json` retains the slower exhaustive correctness-test result.
Its reference calculation and actual relation work have separate timing in the final test log.

## Evidence and reproduction

`raw-reports.tar.xz` contains raw JSON reports, test/build logs, and the earlier bulk-reader measurements.
It contains no executables, corpus files, or cache databases.

The drivers preserve the exact local paths used for measurement:

- `run.py`: focused tests, edge parity, warmup, three pairs, and separate profiles.
- `axes.py`: the nearest-scope query over the 100 largest corpus files.
- `extra.py`: exhaustive containment timings and uncached corpus admission.
- `audit.py`: assertions over completed reports and source hashes.

For another checkout, update the executable, corpus, cache, and workspace paths in the drivers.
Build both revisions with the profiling profile and the recorded Rust toolchain.
Run each driver with `nice -n 19 python3 <driver>`.
The audit requires the raw archive contents beside the scripts.

After the corpus measurements, only `syntax/tests.rs` changed to report containment phase CPU.
Production source hashes remain identical. The manifest distinguishes measured and final source hashes.
The documentation contains the complete interpretation:
`docs/comparisons/input-lifecycle-opportunities.md`.
