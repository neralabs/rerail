# TSLint conformance delta: comments against main

- before: the main summary (generated 2026-10-09T15:00:48.929Z; kernel 1, built 2026-10-09T15:00:45.999Z; not retained, this delta records the change)
- after: comments (generated 2026-10-09T15:01:03.068Z; kernel 1, built 2026-10-09T14:55:14.645Z); timings median of 5
- rules scored: 149 → 149; 14 excluded (formatter) → 14 excluded (formatter)

## Totals

Over rules present in both runs.

| measure | before | after | Δ |
|---|---|---|---|
| golden | 74 | 75 | +1 |
| raw BLOCKED | 17 | 16 | -1 |
| raw INFEASIBLE | 23 | 23 | 0 |
| raw PARTIAL | 15 | 15 | 0 |
| raw PASS | 94 | 95 | +1 |
| effective BLOCKED | 14 | 13 | -1 |
| effective INFEASIBLE | 23 | 23 | 0 |
| effective PASS | 94 | 95 | +1 |
| effective PASS (repaired fixture) | 18 | 18 | 0 |
| tp | 1815 | 1815 | 0 |
| fp | 63 | 62 | -1 |
| fn | 794 | 794 | 0 |
| engineFailures | 59 | 59 | 0 |
| partialFiles | 26 | 26 | 0 |
| outOfScope | 0 | 0 | 0 |
| defaultBytes | 612538 | 615034 | +2496 |
| totalBytes | 1281415 | 1283911 | +2496 |
| docs | 321 | 321 | 0 |
| checks | 478 | 478 | 0 |
| relations | 3163 | 3168 | +5 |
| not | 1227 | 1229 | +2 |
| any | 1473 | 1475 | +2 |
| all | 1111 | 1112 | +1 |
| evalMs (sum) | 62.3 | 61.6 | -0.6 |

## Verdict changes

| rule | raw | effective |
|---|---|---|
| no-reference-import | BLOCKED → PASS | BLOCKED → PASS |

## Changed rules

Rules whose verdict, score or port shape changed (eval ms alone is noise and does not qualify).

| rule | golden | effective | TP/FP/FN | out of scope | total bytes | Δ bytes | docs | checks | relations (in/has/pre/fol) | not | any/all | depth | eval ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| no-reference-import | false → true | BLOCKED → PASS | 2/1/0 → 2/0/0 | 0 | 891 → 3387 | +2496 | 1 | 1 | 0 (0/0/0/0) → 5 (3/1/0/1) | 0 → 2 | 1/0 → 3/1 | 2 → 7 | 0.06 → 0.13 |

