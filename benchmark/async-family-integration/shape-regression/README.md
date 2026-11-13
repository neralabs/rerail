# Composition regression evidence

The [investigation](../../../docs/comparisons/sql-composition-regression.md) explains the measurements and production changes.
`manifest.json` records the source hashes, binary hashes, corpus manifest, and measurement boundaries.

## Production results

| Artifact | Meaning |
|---|---|
| `native-profile.json.gz` | Same-build native relational execution, before composition |
| `before-profile.json.gz` | Published composition with diagnostic support only |
| `fixed-profile.json.gz` | Final compiler, complete 5,725-file production profile |
| `positive-before.json.gz`, `positive-fixed.json.gz` | Nonempty 24-file workload; all 120 findings |
| `cost-red.json` | Both instruction ceilings fail against the published shape |
| `cost-final-green.json` | Both instruction ceilings and 100-execution budgets pass |
| `shape-final-green.json` | No rule-table publication or SQL scheduler loop |
| `edge-comparison.json`, `edges/` | All 178 recorded Ruff findings; known reference differences retained |
| `compose-final-tests.log.gz` | Nine composition tests, including constant-prefix and bound-suffix cases |
| `native-demand-tests.log.gz` | Four retained-demand tests |

`profile-comparison.json` describes the first fixed profile, before the bound-suffix guard.
The final SQL is identical; the adaptive scheduler uses 72 instead of 73 batches in the final run.
`compose-tests.log.gz` preserves the first test-fixture failure: it omitted production simplification.
That failure is distinct from the measured production cost failures.

## Controlled timings and shape experiments

`timing-summary.json` includes samples and medians. Each phase CPU includes user and system time.
The process-clock total includes worker CPU. The two `timings-processes.json.gz` files preserve commands, environments, outputs, and phase counters.
`paired/` compares native and composed execution in one build. `fix-paired/` compares published and corrected composition.
Wall measurements are retained, but shared-host contention prevents a causal wall-speed claim.

The case JSON files define literal SQL rewrites, isolated root queries, or exact-row snapshots.
The experiment directories contain emitted SQL, EXPLAIN plans, and per-batch JSONL records, compressed with gzip.

- `full/`: join order, CTE materialization, duplicate removal, retained structural/semantic rows, and isolated roots.
- `positive-detail/`: nonempty outputs, individual rules, and string-position amplification.
- `boundary/`: downstream driver choices after retaining native facts.
- `batch-receiver-final/`: attribute membership through the existing batched virtual-table interface.
- `batch-receiver-positive/`: result parity for that membership rewrite on the positive fixture.

Root costs are inclusive and not additive. Snapshot capture costs appear as separate `capture:` records.
Replay CPU uses already-bound, warmed sources and must not be substituted for whole-job CPU.
The aborted inlining case retains its instruction-limit failure; its measured instructions are a lower bound.
The two `rejected-config` logs preserve failed lab setups, not engine result regressions.

The membership rewrite reduces cursor calls but increases CPU. It is not included in production.
The generic inlining and snapshot rewrites are also experiments only.

## Check the archived production gates

```sh
nice -n 19 python3 benchmark/async-family-integration/check-query-cost.py \
  --corpus benchmark/async-family-integration/shape-regression/fixed-profile.json.gz \
  --positive benchmark/async-family-integration/shape-regression/positive-fixed.json.gz \
  --output /tmp/rerail-query-cost.json

nice -n 19 python3 benchmark/async-family-integration/check-composed.py \
  benchmark/async-family-integration/shape-regression/fixed-profile.json.gz \
  --output /tmp/rerail-query-shape.json
```
