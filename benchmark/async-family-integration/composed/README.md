# Production query composition evidence

The [implementation report](../../../docs/comparisons/composed-production-queries.md) explains the results and remaining gaps.

- `manifest.json` records the tested binary hash, changed source hashes, merge snapshot, and corpus identity.
- `shape-final.json` records the complete-job budget and query-boundary checks.
- `rr-profile.json.gz`, `local.sql`, `cross.sql`, and `explain.json` contain the final execution evidence.
- `input-work.json` identifies cursor work inside the composed queries.
- `warm-timings.json` and the three compressed warm reports contain separate unprofiled measurements.
- `parity.json` compares the complete workload and all edge fixtures with preserved results.
- The test logs record 115 passing active tests and three existing ignored tests.
- `budget-red.json`, `dead-export-red.json`, and `scalar-null-red.log` preserve failures that drove the fixes.
- `attempts.json` and the earlier profiles preserve the intermediate measurements.

The first attempt enabled `code-rule-profile`. Later attempts and the final binary use default features.
The intermediate timings are not a controlled speed comparison.

`edge-parity.json` uses the older independent reference, which misses four findings that Ruff and production report.
Its four discrepancies remain visible. `parity.json` records the complete 178-finding Ruff comparison.

The old `check-retained.py` gate requires the candidate relation to remain entirely outside SQL.
That physical contract differs from this compiler's shared demand CTE and immutable frame.
The new gate prohibits candidate export and per-intermediate publication, while enforcing the complete-job statement budget.
It does not prove two statements across the whole corpus or a performance improvement.

From the repository root, reproduce the profile with the same corpus and an optimized test binary:

```sh
nice -n 19 python3 benchmark/async-family-integration/run.py \
  --binary /path/to/rerail_engine-test-binary \
  --corpus /path/to/corpus-fce4dc72 \
  --out /path/to/results --stage rr-profile
nice -n 19 python3 benchmark/async-family-integration/check-composed.py \
  /path/to/results/rr-profile.json --output /path/to/results/shape.json
```

Use `--stage rr` for unprofiled timings. The harness records its environment and process result.
Use the same output directory to preserve its tape and prepared-program cache between runs.
