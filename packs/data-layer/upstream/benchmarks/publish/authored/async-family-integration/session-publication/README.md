# Composed session retention and frame publication

Integration baseline: `5ea1d2f75`, merged as `d65a24d47`.
The baseline includes only the duplicate-dependency repair needed to build the merge.

Build the engine library tests with `--profile profiling --features p-anchor-corpus --lib --no-run`.
Run the baseline or repaired binary with:

```sh
python3 benchmark/async-family-integration/session-publication/check.py \
  --binary /absolute/path/to/rerail_engine-test-binary \
  --out /new/validation/directory --mode composed
```

Use `--mode native` for the integration default. Use `--baseline` to reproduce the three original failures.
The script isolates tests in separate processes and explicitly selects the execution mode.
It fails if a candidate test fails or the binary contains no matching test.

`baseline.json` records the original composed failures. `baseline-native.json` records all thirteen native passes.
`candidate-composed.json` records the intermediate repair: seven passes and six failures.
Five failures expose SQLite's reference limit; the sixth exposes missing statistics for a restored relation.
`negation-maintenance-before.sql` is the failing 38-CTE query captured at preparation.

`frame-red.log` records 782 publication statements before batching.
`frame-green.log` records 18 statements, 74,776 VM instructions, and eight passing storage tests.

Whole-job measurements use `../fact-admission/run.py`, with `--engine composed` or `--engine native`.
Each output directory gets its own corpus copy and fresh fact store.

The final build uses `CARGO_INCREMENTAL=0`. `compiler-green.log` has thirteen compiler passes,
including actual SQLite refusal before staging and unchanged resident identity after quiet maintenance.
`session-composed-green.json` has all thirteen repaired session/cache passes.
`session-native-green.json` records the unchanged native control's thirteen passes.
`positive-green.log` checks 120 findings; `architecture-green.log` checks the three cache architecture rules.

`composed-measurements.json` and `native-measurements.json` use the same final binary.
Their manifests record its hash and the environment. The composed job passes every execution ceiling.
The native cold job fails at 396 executions; the remaining native conditions pass.
Use `--record-budget-failures` to finish that comparison. The runner still exits with failure.
The final [report](../../../docs/comparisons/composed-session-publication-2026-11-10.md) explains the scope of the counts.

`over-eager-*` preserves the rejected automatic-cut shape. Its 154 executions failed the unchanged ceiling.
`uncut-cold-green.log` and `uncut.sql.gz` prove that SQLite accepts the full async query.
The final planner prepares risky complete graphs once and cuts only after a structural-limit refusal.
