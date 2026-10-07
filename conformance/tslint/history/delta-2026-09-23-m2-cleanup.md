# TSLint conformance delta: m2-cleanup against m2

- before: the m2 summary (generated 2026-09-24T02:35:38.923Z; kernel 1, built 2026-09-24T02:26:29.717Z; not retained, this delta records the change)
- after: m2-cleanup (generated 2026-09-24T03:15:52.932Z; kernel 1, built 2026-09-24T02:26:29.717Z); timings median of 5
- rules scored: 163 → 149; 0 excluded → 14 excluded (formatter)

## Totals

Over rules present in both runs.

| measure | before | after | Δ |
|---|---|---|---|
| raw BLOCKED | 47 | 35 | -12 |
| raw INFEASIBLE | 23 | 23 | 0 |
| raw PARTIAL | 4 | 4 | 0 |
| raw PASS | 75 | 87 | +12 |
| effective BLOCKED | 35 | 22 | -13 |
| effective INFEASIBLE | 23 | 23 | 0 |
| effective PARTIAL | 4 | 0 | -4 |
| effective PASS | 75 | 87 | +12 |
| effective PASS (repaired fixture) | 12 | 17 | +5 |
| tp | 1723 | 1748 | +25 |
| fp | 146 | 101 | -45 |
| fn | 886 | 861 | -25 |
| engineFailures | 64 | 65 | +1 |
| partialFiles | 28 | 27 | -1 |
| defaultBytes | 3822243 | 2401215 | -1421028 |
| totalBytes | 13521987 | 5697489 | -7824498 |
| docs | 318 | 320 | +2 |
| checks | 641 | 644 | +3 |
| relations | 23769 | 14538 | -9231 |
| not | 10365 | 5644 | -4721 |
| any | 13665 | 8941 | -4724 |
| all | 21337 | 10796 | -10541 |
| evalMs (sum) | 110.3 | 106.6 | -3.7 |

## Verdict changes

| rule | raw | effective |
|---|---|---|
| adjacent-overload-signatures | BLOCKED → PASS | BLOCKED → PASS |
| no-parameter-reassignment | BLOCKED → PASS | BLOCKED → PASS |
| prefer-for-of | BLOCKED → PARTIAL | BLOCKED → PASS (repaired fixture) |
| unified-signatures | BLOCKED → PASS | BLOCKED → PASS |
| no-duplicate-super | PARTIAL → PASS | PARTIAL → PASS |
| no-duplicate-switch-case | BLOCKED → PARTIAL | BLOCKED → PASS (repaired fixture) |
| no-string-throw | BLOCKED | BLOCKED → PASS (repaired fixture) |
| no-switch-case-fall-through | BLOCKED → PASS | BLOCKED → PASS |
| no-unsafe-finally | BLOCKED → PASS | BLOCKED → PASS |
| prefer-conditional-expression | PARTIAL → PASS | PARTIAL → PASS |
| max-classes-per-file | PARTIAL → PASS | PARTIAL → PASS |
| no-duplicate-imports | BLOCKED → PASS | BLOCKED → PASS |
| no-mergeable-namespace | BLOCKED → PASS | BLOCKED → PASS |
| prefer-readonly | BLOCKED → PARTIAL | BLOCKED → PASS (repaired fixture) |
| array-type | PARTIAL → PASS | PARTIAL → PASS |
| completed-docs | BLOCKED → PARTIAL | BLOCKED → PASS (repaired fixture) |
| prefer-function-over-method | BLOCKED → PASS | BLOCKED → PASS |

## Changed rules

Rules whose verdict, score or port shape changed (eval ms alone is noise and does not qualify).

| rule | effective | TP/FP/FN | total bytes | Δ bytes | docs | checks | relations (in/has/pre/fol) | not | any/all | depth | eval ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| adjacent-overload-signatures | BLOCKED → PASS | 2/0/13 → 15/0/0 | 4954 → 151971 | +147017 | 1 → 2 | 3 → 4 | 12 (0/0/0/12) → 216 (6/170/0/40) | 6 → 50 | 0/7 → 188/92 | 6 → 19 | 0.41 → 0.85 |
| member-ordering | BLOCKED | 20/0/14 → 22/0/12 | 8610534 → 1062430 | -7548104 | 14 | 140 | 11909 (140/11349/0/420) → 1766 (140/1486/0/140) | 5478 → 743 | 6156/12255 → 1193/1639 | 12 → 10 | 9.46 → 2.53 |
| no-parameter-reassignment | BLOCKED → PASS | 7/2/0 → 7/0/0 | 4372 → 5736 | +1364 | 1 | 1 | 10 (7/0/3/0) → 14 (7/3/3/1) | 0 → 1 | 2/6 → 4/7 | 6 → 9 | 0.16 → 0.25 |
| prefer-for-of | BLOCKED → PASS (repaired fixture) | 10/9/0 → 0/0/10 | 4927 → 19566 | +14639 | 1 | 1 | 4 (0/4/0/0) → 34 (16/10/5/3) | 1 → 8 | 4/5 → 21/11 | 9 → 16 | 0.98 → 0.41 |
| unified-signatures | BLOCKED → PASS | 5/4/7 → 12/0/0 | 18799 → 124133 | +105334 | 1 | 1 | 39 (2/25/1/11) → 311 (57/186/38/30) | 15 → 38 | 7/21 → 135/69 | 13 → 16 | 0.52 → 17.81 |
| no-duplicate-super | PARTIAL → PASS | 7/0/1 → 8/0/0 | 726652 → 42518 | -684134 | 1 | 2 | 244 (112/91/41/0) → 88 (37/32/17/2) | 149 → 25 | 337/171 → 54/23 | 25 → 17 | 1.88 → 3.09 |
| no-duplicate-switch-case | BLOCKED → PASS (repaired fixture) | 0/0/6 → 4/0/2 | 901 → 1162 | +261 | 1 | 1 | 0 (0/0/0/0) → 5 (1/1/0/3) | 0 | 0/0 | 1 → 5 | 0.26 → 0.32 |
| no-misused-new | PASS | 5/0/0 | 8370 → 5165 | -3205 | 1 | 3 | 9 (5/4/0/0) → 14 (5/9/0/0) | 0 | 5/3 → 6/3 | 5 → 6 | 0.14 → 0.09 |
| no-string-throw | BLOCKED → PASS (repaired fixture) | 4/0/8 | 88482 | 0 | 1 | 1 | 63 (0/63/0/0) | 0 | 63/63 | 19 | 0.32 → 0.29 |
| no-switch-case-fall-through | BLOCKED → PASS | 6/0/0 | 20205 | 0 | 1 | 1 | 25 (0/22/0/3) | 7 | 16/7 | 13 | 0.34 → 0.24 |
| no-unsafe-finally | BLOCKED → PASS | 23/0/2 → 25/0/0 | 10071 → 8735 | -1336 | 1 | 5 → 4 | 18 (15/3/0/0) → 16 (12/4/0/0) | 7 → 6 | 6/5 → 6/4 | 5 → 6 | 0.60 → 0.61 |
| prefer-conditional-expression | PARTIAL → PASS | 10/7/0 → 10/0/0 | 61027 → 36915 | -24112 | 2 | 2 | 16 (10/4/0/2) → 88 (22/37/11/18) | 8 → 20 | 8/6 → 21/22 | 11 → 22 | 1.54 → 1.00 |
| max-classes-per-file | PARTIAL → PASS | 2/0/2 → 4/0/0 | 13757 → 40868 | +27111 | 3 | 3 | 17 (8/4/0/5) → 54 (19/19/0/16) | 3 | 25/11 → 73/15 | 8 → 14 | 0.13 → 0.17 |
| no-duplicate-imports | BLOCKED → PASS | 7/8/2 → 9/0/0 | 1065 → 26726 | +25661 | 1 → 2 | 1 → 3 | 1 (0/0/0/1) → 108 (12/90/0/6) | 0 → 6 | 0/1 → 12/27 | 3 → 17 | 0.31 → 0.41 |
| no-mergeable-namespace | BLOCKED → PASS | 6/4/0 → 6/0/0 | 1866 → 9391 | +7525 | 1 | 1 | 5 (2/2/0/1) → 15 (2/11/0/2) | 0 | 1/1 → 16/6 | 6 → 13 | 0.15 → 0.18 |
| prefer-readonly | BLOCKED → PASS (repaired fixture) | 1/1/14 → 1/0/14 | 6164 → 55773 | +49609 | 2 | 3 → 4 | 12 (5/7/0/0) → 126 (66/54/6/0) | 5 → 24 | 3/9 → 42/44 | 6 → 16 | 0.38 → 0.41 |
| array-type | PARTIAL → PASS | 47/0/1 → 48/0/0 | 73527 | 0 | 3 | 6 | 47 (2/45/0/0) | 3 | 32/62 | 18 | 0.86 → 0.80 |
| completed-docs | BLOCKED → PASS (repaired fixture) | 76/10/53 → 76/0/53 | 176415 → 228174 | +51759 | 25 | 59 | 279 (163/27/0/89) → 471 (112/173/24/162) | 125 → 145 | 181/222 → 240/220 | 11 | 2.92 → 3.24 |
| no-reference-import | BLOCKED | 2/1/0 | 835 | 0 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.09 → 0.06 |
| no-unnecessary-callback-wrapper | PASS | 4/0/0 | 3296 → 3333 | +37 | 1 | 1 | 2 (0/2/0/0) → 5 (0/5/0/0) | 3 | 3/1 | 6 | 0.20 → 0.19 |
| prefer-function-over-method | BLOCKED → PASS | 13/0/1 → 14/0/0 | 18938 → 25014 | +6076 | 4 | 4 | 36 (16/16/4/0) → 51 (28/19/4/0) | 18 → 25 | 12/16 | 11 → 12 | 0.38 → 0.40 |

## Newly excluded

Scored before, excluded now; not part of the totals above.

| rule | before (effective, TP/FP/FN) | reason |
|---|---|---|
| one-line | PASS (repaired fixture), 24/0/20 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| space-before-function-paren | OUT_OF_SCOPE_FORMATTER, 26/28/54 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| space-within-parens | OUT_OF_SCOPE_FORMATTER, 17/0/14 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| align | OUT_OF_SCOPE_FORMATTER, 0/0/30 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| eofline | OUT_OF_SCOPE_FORMATTER, 1/1/3 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| indent | OUT_OF_SCOPE_FORMATTER, 94/20/0 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| jsdoc-format | OUT_OF_SCOPE_FORMATTER, 3/0/9 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| max-line-length | OUT_OF_SCOPE_FORMATTER, 10/4/0 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| no-consecutive-blank-lines | OUT_OF_SCOPE_FORMATTER, 1/10/7 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| no-trailing-whitespace | OUT_OF_SCOPE_FORMATTER, 59/13/0 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| semicolon | OUT_OF_SCOPE_FORMATTER, 64/121/113 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| trailing-comma | OUT_OF_SCOPE_FORMATTER, 245/55/13 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| typedef-whitespace | OUT_OF_SCOPE_FORMATTER, 624/0/914 | formatter concern; removed from equivalence set by decision 2026-09-23 |
| whitespace | OUT_OF_SCOPE_FORMATTER, 37/0/49 | formatter concern; removed from equivalence set by decision 2026-09-23 |

