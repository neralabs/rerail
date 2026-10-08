# TSLint conformance delta: golden against poison-audit

- before: the poison-audit summary (generated 2026-09-24T13:18:25.538Z; kernel 1, built 2026-09-24T12:52:50.048Z; not retained, this delta records the change)
- after: golden (generated 2026-09-25T12:49:18.651Z; kernel 1, built 2026-09-25T12:22:17.310Z); timings median of 5
- rules scored: 149 → 149; 14 excluded (formatter) → 14 excluded (formatter)

## Totals

Over rules present in both runs.

| measure | before | after | Δ |
|---|---|---|---|
| golden | 0 | 74 | +74 |
| raw BLOCKED | 28 | 17 | -11 |
| raw INFEASIBLE | 23 | 23 | 0 |
| raw PARTIAL | 4 | 15 | +11 |
| raw PASS | 94 | 94 | 0 |
| effective BLOCKED | 14 | 14 | 0 |
| effective INFEASIBLE | 23 | 23 | 0 |
| effective PASS | 94 | 94 | 0 |
| effective PASS (repaired fixture) | 18 | 18 | 0 |
| tp | 1815 | 1815 | 0 |
| fp | 63 | 63 | 0 |
| fn | 794 | 794 | 0 |
| engineFailures | 59 | 59 | 0 |
| partialFiles | 26 | 26 | 0 |
| outOfScope | 0 | 0 | 0 |
| defaultBytes | 563391 | 615314 | +51923 |
| totalBytes | 1235601 | 1287524 | +51923 |
| docs | 321 | 321 | 0 |
| checks | 478 | 478 | 0 |
| relations | 3163 | 3163 | 0 |
| not | 1227 | 1227 | 0 |
| any | 1473 | 1473 | 0 |
| all | 1111 | 1111 | 0 |
| evalMs (sum) | 72.2 | 79.0 | +6.8 |

## Verdict changes

| rule | raw | effective |
|---|---|---|
| member-access | BLOCKED → PARTIAL | PASS (repaired fixture) |
| no-any | BLOCKED → PARTIAL | PASS (repaired fixture) |
| no-empty-interface | BLOCKED → PARTIAL | PASS (repaired fixture) |
| no-async-without-await | BLOCKED → PARTIAL | PASS (repaired fixture) |
| no-invalid-this | BLOCKED → PARTIAL | PASS (repaired fixture) |
| no-string-throw | BLOCKED → PARTIAL | PASS (repaired fixture) |
| no-unnecessary-class | BLOCKED → PARTIAL | PASS (repaired fixture) |
| prefer-switch | BLOCKED → PARTIAL | PASS (repaired fixture) |
| unnecessary-else | BLOCKED → PARTIAL | PASS (repaired fixture) |
| variable-name | BLOCKED → PARTIAL | PASS (repaired fixture) |
| quotemark | BLOCKED → PARTIAL | PASS (repaired fixture) |

## Changed rules

Rules whose verdict, score or port shape changed (eval ms alone is noise and does not qualify).

| rule | golden | effective | TP/FP/FN | out of scope | total bytes | Δ bytes | docs | checks | relations (in/has/pre/fol) | not | any/all | depth | eval ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| adjacent-overload-signatures | false → true | PASS | 15/0/0 | 0 | 25969 → 26444 | +475 | 2 | 4 | 45 (6/19/0/20) | 20 | 36/31 | 7 | 0.74 → 0.78 |
| ban-ts-ignore | false → true | PASS | 5/0/0 | 0 | 798 → 1243 | +445 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.08 → 0.10 |
| member-access | false → true | PASS (repaired fixture) | 13/0/13 | 0 | 10844 → 12007 | +1163 | 5 | 7 | 35 (18/7/1/9) | 15 | 13/0 | 4 | 0.40 → 0.54 |
| no-any | false → true | PASS (repaired fixture) | 10/0/10 | 0 | 2161 → 2738 | +577 | 2 | 2 | 4 (3/1/0/0) | 1 | 0/2 | 7 | 0.19 → 0.25 |
| no-empty-interface | false → true | PASS (repaired fixture) | 0/0/3 | 0 | 3219 → 3674 | +455 | 1 | 2 | 6 (2/4/0/0) | 2 | 0/7 | 7 | 0.04 → 0.06 |
| no-for-in | false → true | PASS | 1/0/0 | 0 | 924 → 1361 | +437 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.15 → 0.16 |
| no-import-side-effect | false → true | PASS | 8/0/0 | 0 | 3005 → 3466 | +461 | 2 | 2 | 5 (2/3/0/0) | 3 | 2/3 | 5 | 0.22 → 0.17 |
| no-inferrable-types | false → true | PASS | 25/0/0 | 0 | 8327 → 10250 | +1923 | 3 | 3 | 33 (15/9/9/0) | 8 | 22/0 | 7 | 0.67 → 0.68 |
| no-internal-module | false → true | PASS | 13/0/0 | 0 | 944 → 1399 | +455 | 1 | 1 | 1 (0/1/0/0) | 1 | 0/1 | 4 | 0.14 → 0.17 |
| no-magic-numbers | false → true | PASS | 15/0/0 | 0 | 11905 → 13374 | +1469 | 4 | 4 | 45 (25/16/0/4) | 12 | 16/0 | 5 | 0.52 → 0.56 |
| no-namespace | false → true | PASS | 5/0/0 | 0 | 7473 → 7916 | +443 | 2 | 2 | 14 (6/8/0/0) | 5 | 9/10 | 9 | 0.14 → 0.13 |
| no-non-null-assertion | false → true | PASS | 1/0/0 | 0 | 646 → 1107 | +461 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 1 | 0.04 |
| only-arrow-functions | false → true | PASS | 12/0/0 | 0 | 10997 → 11456 | +459 | 3 | 3 | 20 (7/13/0/0) | 8 | 8/7 | 9 | 0.41 → 0.32 |
| ban-comma-operator | false → true | PASS | 5/0/0 | 0 | 947 → 1754 | +807 | 1 | 1 | 2 (1/0/1/0) | 1 | 0/2 | 5 | 0.10 → 0.12 |
| function-constructor | false → true | PASS | 6/0/0 | 0 | 744 → 1393 | +649 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.10 → 0.12 |
| label-position | false → true | PASS | 2/0/0 | 0 | 1587 → 2034 | +447 | 1 | 1 | 2 (1/1/0/0) | 1 | 1/1 | 6 | 0.09 → 0.10 |
| no-arg | false → true | PASS | 1/0/0 | 0 | 522 → 1039 | +517 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 1 | 0.06 |
| no-async-without-await | false → true | PASS (repaired fixture) | 0/0/14 | 0 | 2548 → 4041 | +1493 | 1 | 1 | 5 (2/3/0/0) | 3 | 5/1 | 6 | 0.27 → 0.41 |
| no-bitwise | false → true | PASS | 3/0/0 | 0 | 1044 → 2007 | +963 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.08 → 0.07 |
| no-conditional-assignment | false → true | PASS | 25/0/0 | 0 | 11076 → 11545 | +469 | 1 | 1 | 25 (13/0/8/4) | 2 | 8/12 | 11 | 0.72 → 0.70 |
| no-console | false → true | PASS | 14/0/0 | 0 | 1434 → 1873 | +439 | 2 | 2 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.21 → 0.22 |
| no-construct | false → true | PASS | 3/0/0 | 0 | 888 → 1589 | +701 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.06 → 0.07 |
| no-debugger | false → true | PASS | 1/0/0 | 0 | 529 → 1056 | +527 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 1 | 0.04 → 0.05 |
| no-duplicate-switch-case | false → true | PASS (repaired fixture) | 4/0/2 | 0 | 1162 → 1629 | +467 | 1 | 1 | 5 (1/1/0/3) | 0 | 0/0 | 5 | 0.28 |
| no-empty | false → true | PASS | 29/0/0 | 0 | 9545 → 9981 | +436 | 3 | 3 | 18 (5/13/0/0) | 7 | 7/7 | 9 | 0.57 → 0.63 |
| no-eval | false → true | PASS | 1/0/0 | 0 | 698 → 1351 | +653 | 1 | 1 | 1 (1/0/0/0) | 0 | 0/1 | 3 | 0.05 |
| no-invalid-this | false → true | PASS (repaired fixture) | 0/0/6 | 0 | 7940 → 8389 | +449 | 2 | 2 | 13 (7/6/0/0) | 5 | 11/2 | 5 | 0.18 → 0.33 |
| no-misused-new | false → true | PASS | 5/0/0 | 0 | 5165 → 5612 | +447 | 1 | 3 | 14 (5/9/0/0) | 0 | 6/3 | 6 | 0.12 → 0.11 |
| no-null-keyword | false → true | PASS | 4/0/0 | 0 | 1170 → 2015 | +845 | 1 | 1 | 2 (2/0/0/0) | 2 | 1/1 | 5 | 0.09 → 0.10 |
| no-return-await | false → true | PASS | 13/0/0 | 0 | 2970 → 4773 | +1803 | 1 | 1 | 11 (7/1/1/2) | 3 | 6/0 | 4 | 0.39 → 0.34 |
| no-string-throw | false → true | PASS (repaired fixture) | 4/0/8 | 0 | 1258 → 2055 | +797 | 1 | 1 | 3 (0/3/0/0) | 0 | 1/0 | 3 | 0.30 → 0.33 |
| no-submodule-imports | false → true | PASS | 14/0/0 | 0 | 6635 → 7094 | +459 | 3 | 3 | 9 (9/0/0/0) | 2 | 9/3 | 6 | 0.36 → 0.37 |
| no-switch-case-fall-through | false → true | PASS | 6/0/0 | 0 | 24022 → 24495 | +473 | 1 | 1 | 90 (0/73/4/13) | 22 | 29/16 | 9 | 0.30 → 0.29 |
| no-tautology-expression | false → true | PASS | 20/0/0 | 0 | 3178 → 3643 | +465 | 1 | 1 | 1 (0/1/0/0) | 0 | 3/2 | 6 | 0.41 → 0.48 |
| no-this-assignment | false → true | PASS | 12/0/0 | 0 | 3189 → 3644 | +455 | 3 | 3 | 4 (0/4/0/0) | 2 | 1/3 | 5 | 0.19 → 0.22 |
| no-unnecessary-class | false → true | PASS (repaired fixture) | 51/0/3 | 0 | 20821 → 24066 | +3245 | 4 | 9 | 78 (0/78/0/0) | 27 | 30/3 | 7 | 1.87 → 2.02 |
| no-unsafe-finally | false → true | PASS | 25/0/0 | 0 | 8735 → 9188 | +453 | 1 | 4 | 16 (12/4/0/0) | 6 | 6/4 | 6 | 0.56 → 0.70 |
| no-unused-expression | false → true | PASS | 121/0/0 | 0 | 80489 → 80948 | +459 | 4 | 4 | 256 (90/74/18/74) | 130 | 72/60 | 6 | 5.82 → 5.26 |
| no-var-keyword | false → true | PASS | 16/0/0 | 0 | 1479 → 2112 | +633 | 1 | 1 | 5 (5/0/0/0) | 2 | 0/1 | 6 | 0.26 → 0.30 |
| prefer-conditional-expression | false → true | PASS | 10/0/0 | 0 | 9387 → 9864 | +477 | 2 | 2 | 34 (6/15/4/9) | 4 | 10/8 | 6 | 0.62 → 0.55 |
| radix | false → true | PASS | 6/0/0 | 0 | 944 → 1575 | +631 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.13 → 0.18 |
| static-this | false → true | PASS | 10/0/0 | 0 | 1200 → 1911 | +711 | 1 | 1 | 3 (2/1/0/0) | 2 | 0/0 | 4 | 0.22 → 0.23 |
| switch-default | false → true | PASS | 2/0/0 | 0 | 956 → 1519 | +563 | 1 | 1 | 2 (0/2/0/0) | 1 | 0/1 | 5 | 0.08 → 0.11 |
| triple-equals | false → true | PASS | 6/0/0 | 0 | 5718 → 6164 | +446 | 3 | 6 | 14 (10/4/0/0) | 4 | 0/6 | 5 | 0.14 → 0.17 |
| typeof-compare | false → true | PASS | 14/0/0 | 0 | 2503 → 2951 | +448 | 1 | 1 | 3 (0/3/0/0) | 1 | 1/3 | 6 | 0.35 → 0.44 |
| use-isnan | false → true | PASS | 8/0/0 | 0 | 1201 → 1639 | +438 | 1 | 1 | 2 (0/2/0/0) | 0 | 0/2 | 4 | 0.19 → 0.22 |
| cyclomatic-complexity | false → true | PASS | 21/0/0 | 0 | 12689 → 13150 | +461 | 3 | 3 | 3 (0/3/0/0) | 3 | 6/0 | 3 | 1.43 → 1.45 |
| invalid-void | false → true | PASS | 40/0/0 | 0 | 14779 → 15223 | +444 | 3 | 3 | 20 (15/5/0/0) | 2 | 15/8 | 9 | 0.61 → 0.73 |
| no-default-export | false → true | PASS | 7/0/0 | 0 | 1131 → 1581 | +450 | 1 | 1 | 2 (2/0/0/0) | 1 | 0/1 | 4 | 0.11 → 0.16 |
| no-duplicate-imports | false → true | PASS | 9/0/0 | 0 | 7192 → 8485 | +1293 | 2 | 3 | 43 (12/25/0/6) | 1 | 9/4 | 9 | 0.38 → 0.33 |
| no-mergeable-namespace | false → true | PASS | 6/0/0 | 0 | 9391 → 9854 | +463 | 1 | 1 | 15 (2/11/0/2) | 0 | 16/6 | 13 | 0.20 → 0.15 |
| no-require-imports | false → true | PASS | 5/0/0 | 0 | 1041 → 1497 | +456 | 1 | 1 | 1 (1/0/0/0) | 0 | 1/1 | 4 | 0.09 → 0.10 |
| arrow-return-shorthand | false → true | PASS | 17/0/0 | 0 | 10803 → 11266 | +463 | 2 | 4 | 41 (4/18/8/11) | 7 | 14/3 | 5 | 0.33 → 0.32 |
| callable-types | false → true | PASS | 19/0/0 | 0 | 2698 → 3146 | +448 | 1 | 1 | 5 (3/2/0/0) | 2 | 2/3 | 10 | 0.27 → 0.31 |
| class-name | false → true | PASS | 4/0/0 | 0 | 678 → 1741 | +1063 | 1 | 1 | 1 (1/0/0/0) | 0 | 1/1 | 4 | 0.06 → 0.07 |
| increment-decrement | false → true | PASS | 18/0/0 | 0 | 1086 → 1670 | +584 | 2 | 2 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.20 → 0.27 |
| interface-name | false → true | PASS | 5/0/0 | 0 | 1271 → 2096 | +825 | 2 | 2 | 2 (2/0/0/0) | 1 | 0/2 | 3 | 0.14 → 0.16 |
| interface-over-type-literal | false → true | PASS | 2/0/0 | 0 | 829 → 2313 | +1484 | 1 | 1 | 4 (1/3/0/0) | 1 | 0/3 | 9 | 0.06 → 0.08 |
| no-angle-bracket-type-assertion | false → true | PASS | 20/0/0 | 0 | 701 → 1518 | +817 | 1 | 1 | 1 (1/0/0/0) | 1 | 0/1 | 4 | 0.22 → 0.31 |
| no-parameter-properties | false → true | PASS | 4/0/0 | 0 | 721 → 1833 | +1112 | 1 | 1 | 1 (0/1/0/0) | 0 | 2/1 | 4 | 0.07 → 0.10 |
| no-unnecessary-initializer | false → true | PASS | 8/0/0 | 0 | 6219 → 6690 | +471 | 1 | 2 | 16 (5/9/2/0) | 5 | 10/3 | 7 | 0.20 → 0.23 |
| one-variable-per-declaration | false → true | PASS | 19/0/0 | 0 | 7201 → 7676 | +475 | 2 | 5 | 15 (3/7/0/5) | 2 | 7/7 | 6 | 0.32 → 0.40 |
| prefer-function-over-method | false → true | PASS | 14/0/0 | 0 | 12575 → 14214 | +1639 | 4 | 4 | 51 (28/19/4/0) | 16 | 17/0 | 6 | 0.36 → 0.38 |
| prefer-method-signature | false → true | PASS | 4/0/0 | 0 | 1571 → 2036 | +465 | 1 | 1 | 3 (1/2/0/0) | 0 | 1/1 | 5 | 0.10 → 0.09 |
| prefer-switch | false → true | PASS (repaired fixture) | 0/0/8 | 0 | 14485 → 14930 | +445 | 2 | 2 | 65 (8/45/6/6) | 8 | 11/10 | 6 | 0.28 → 0.44 |
| prefer-template | false → true | PASS | 18/0/0 | 0 | 10490 → 10939 | +449 | 2 | 2 | 32 (2/17/11/2) | 11 | 8/12 | 6 | 0.41 → 0.31 |
| type-literal-delimiter | false → true | PASS | 13/0/0 | 0 | 6485 → 6948 | +463 | 2 | 5 | 13 (10/0/3/0) | 7 | 2/5 | 5 | 0.25 → 0.27 |
| unnecessary-else | false → true | PASS (repaired fixture) | 0/0/34 | 0 | 11544 → 11995 | +451 | 2 | 2 | 51 (8/23/12/8) | 5 | 12/4 | 9 | 0.74 → 1.31 |
| variable-name | false → true | PASS (repaired fixture) | 13/0/80 | 0 | 25912 → 27875 | +1963 | 8 | 9 | 83 (57/1/0/25) | 40 | 47/1 | 4 | 0.81 → 1.05 |
| linebreak-style | false → true | PASS | 8/0/0 | 0 | 1734 → 2183 | +449 | 3 | 3 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.06 |
| new-parens | false → true | PASS | 3/0/0 | 0 | 935 → 1374 | +439 | 1 | 1 | 1 (0/1/0/0) | 1 | 0/1 | 4 | 0.05 → 0.06 |
| no-irregular-whitespace | false → true | PASS | 22/0/0 | 0 | 764 → 1229 | +465 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.12 → 0.11 |
| number-literal-format | false → true | PASS | 25/0/0 | 0 | 3736 → 4197 | +461 | 1 | 6 | 0 (0/0/0/0) | 0 | 0/6 | 2 | 0.27 → 0.24 |
| quotemark | false → true | PASS (repaired fixture) | 45/0/7 | 0 | 15128 → 15565 | +437 | 10 | 15 | 21 (18/2/1/0) | 9 | 2/18 | 7 | 1.03 → 1.10 |

