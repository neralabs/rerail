# TSLint conformance delta: comments against golden

- before: the golden summary (generated 2026-09-25T12:49:18.651Z; kernel 1, built 2026-09-25T12:22:17.310Z; not retained, this delta records the change)
- after: comments (generated 2026-10-08T20:57:05.485Z; kernel 1, built 2026-10-08T20:56:59.238Z); timings median of 5
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
| defaultBytes | 615314 | 614740 | -574 |
| totalBytes | 1287524 | 1283617 | -3907 |
| docs | 321 | 321 | 0 |
| checks | 478 | 478 | 0 |
| relations | 3163 | 3168 | +5 |
| not | 1227 | 1229 | +2 |
| any | 1473 | 1474 | +1 |
| all | 1111 | 1112 | +1 |
| evalMs (sum) | 79.0 | 59.1 | -19.9 |

## Verdict changes

| rule | raw | effective |
|---|---|---|
| no-reference-import | BLOCKED → PASS | BLOCKED → PASS |

## Changed rules

Rules whose verdict, score or port shape changed (eval ms alone is noise and does not qualify).

| rule | golden | effective | TP/FP/FN | out of scope | total bytes | Δ bytes | docs | checks | relations (in/has/pre/fol) | not | any/all | depth | eval ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| adjacent-overload-signatures | true | PASS | 15/0/0 | 0 | 26444 → 26378 | -66 | 2 | 4 | 45 (6/19/0/20) | 20 | 36/31 | 7 | 0.78 → 0.71 |
| ban-ts-ignore | true | PASS | 5/0/0 | 0 | 1243 → 1237 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.10 → 0.07 |
| ban-types | false | PASS | 6/0/0 | 0 | 4177 → 4144 | -33 | 1 | 2 | 2 (2/0/0/0) | 2 | 2/2 | 5 | 0.12 → 0.07 |
| member-access | true | PASS (repaired fixture) | 13/0/13 | 0 | 12007 → 11923 | -84 | 5 | 7 | 35 (18/7/1/9) | 15 | 13/0 | 4 | 0.54 → 0.47 |
| member-ordering | false | PASS | 34/0/0 | 0 | 150101 → 150017 | -84 | 14 | 14 | 325 (28/97/0/200) | 376 | 324/297 | 5 | 2.40 → 2.21 |
| no-any | true | PASS (repaired fixture) | 10/0/10 | 0 | 2738 → 2726 | -12 | 2 | 2 | 4 (3/1/0/0) | 1 | 0/2 | 7 | 0.25 → 0.20 |
| no-empty-interface | true | PASS (repaired fixture) | 0/0/3 | 0 | 3674 → 3641 | -33 | 1 | 2 | 6 (2/4/0/0) | 2 | 0/7 | 7 | 0.06 |
| no-for-in | true | PASS | 1/0/0 | 0 | 1361 → 1355 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.16 → 0.14 |
| no-import-side-effect | true | PASS | 8/0/0 | 0 | 3466 → 3454 | -12 | 2 | 2 | 5 (2/3/0/0) | 3 | 2/3 | 5 | 0.17 → 0.16 |
| no-inferrable-types | true | PASS | 25/0/0 | 0 | 10250 → 10232 | -18 | 3 | 3 | 33 (15/9/9/0) | 8 | 22/0 | 7 | 0.68 → 0.46 |
| no-internal-module | true | PASS | 13/0/0 | 0 | 1399 → 1393 | -6 | 1 | 1 | 1 (0/1/0/0) | 1 | 0/1 | 4 | 0.17 → 0.12 |
| no-magic-numbers | true | PASS | 15/0/0 | 0 | 13374 → 13350 | -24 | 4 | 4 | 45 (25/16/0/4) | 12 | 16/0 | 5 | 0.56 → 0.54 |
| no-namespace | true | PASS | 5/0/0 | 0 | 7916 → 7904 | -12 | 2 | 2 | 14 (6/8/0/0) | 5 | 9/10 | 9 | 0.13 → 0.16 |
| no-non-null-assertion | true | PASS | 1/0/0 | 0 | 1107 → 1101 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 1 | 0.04 → 0.05 |
| no-parameter-reassignment | false | PASS | 7/0/0 | 0 | 5736 → 5730 | -6 | 1 | 1 | 14 (7/3/3/1) | 1 | 4/7 | 9 | 0.20 → 0.14 |
| no-reference | false | PASS | 1/0/0 | 0 | 686 → 680 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.01 |
| no-unnecessary-type-assertion | false | INFEASIBLE | 2/0/37 | 0 | 2870 → 2864 | -6 | 1 | 1 | 2 (0/2/0/0) | 1 | 1/3 | 8 | 0.79 → 0.62 |
| no-var-requires | false | PASS | 1/0/0 | 0 | 2370 → 2364 | -6 | 1 | 1 | 1 (1/0/0/0) | 1 | 1/1 | 5 | 0.05 → 0.04 |
| only-arrow-functions | true | PASS | 12/0/0 | 0 | 11456 → 11438 | -18 | 3 | 3 | 20 (7/13/0/0) | 8 | 8/7 | 9 | 0.32 → 0.28 |
| prefer-for-of | false | PASS (repaired fixture) | 0/0/10 | 0 | 19566 → 19560 | -6 | 1 | 1 | 34 (16/10/5/3) | 8 | 21/11 | 16 | 0.65 → 0.68 |
| promise-function-async | false | PASS | 8/0/0 | 0 | 6588 → 6582 | -6 | 1 | 1 | 16 (0/14/0/2) | 2 | 5/8 | 8 | 0.38 → 0.30 |
| typedef | false | PASS (repaired fixture) | 24/0/26 | 0 | 37215 → 36873 | -342 | 12 | 22 | 143 (76/34/12/21) | 59 | 53/1 | 6 | 1.24 → 1.18 |
| unified-signatures | false | PASS | 12/0/0 | 0 | 48295 → 48289 | -6 | 1 | 1 | 163 (37/107/11/8) | 20 | 58/15 | 16 | 3.07 → 2.47 |
| await-promise | false | INFEASIBLE | 6/0/5 | 0 | 18892 → 18826 | -66 | 2 | 4 | 18 (2/10/2/4) | 2 | 12/10 | 10 | 0.68 → 0.56 |
| ban-comma-operator | true | PASS | 5/0/0 | 0 | 1754 → 1748 | -6 | 1 | 1 | 2 (1/0/1/0) | 1 | 0/2 | 5 | 0.12 → 0.09 |
| ban | false | PASS | 13/0/0 | 0 | 7571 → 7403 | -168 | 1 | 7 | 20 (10/0/10/0) | 0 | 1/10 | 4 | 0.24 → 0.18 |
| curly | false | PASS | 29/0/0 | 0 | 10146 → 9966 | -180 | 3 | 9 | 18 (2/14/1/1) | 17 | 3/11 | 7 | 0.81 → 0.63 |
| forin | false | PASS | 2/0/0 | 0 | 3163 → 3157 | -6 | 1 | 1 | 7 (0/5/1/1) | 3 | 1/4 | 10 | 0.14 → 0.11 |
| function-constructor | true | PASS | 6/0/0 | 0 | 1393 → 1387 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.12 → 0.08 |
| import-blacklist | false | PASS (repaired fixture) | 11/0/2 | 0 | 15879 → 15738 | -141 | 1 | 6 | 29 (18/11/0/0) | 1 | 17/20 | 11 | 0.19 → 0.16 |
| label-position | true | PASS | 2/0/0 | 0 | 2034 → 2028 | -6 | 1 | 1 | 2 (1/1/0/0) | 1 | 1/1 | 6 | 0.10 → 0.09 |
| no-arg | true | PASS | 1/0/0 | 0 | 1039 → 1033 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 1 | 0.06 → 0.05 |
| no-async-without-await | true | PASS (repaired fixture) | 0/0/14 | 0 | 4041 → 4035 | -6 | 1 | 1 | 5 (2/3/0/0) | 3 | 5/1 | 6 | 0.41 → 0.33 |
| no-bitwise | true | PASS | 3/0/0 | 0 | 2007 → 2001 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.07 → 0.06 |
| no-conditional-assignment | true | PASS | 25/0/0 | 0 | 11545 → 11539 | -6 | 1 | 1 | 25 (13/0/8/4) | 2 | 8/12 | 11 | 0.70 → 0.37 |
| no-console | true | PASS | 14/0/0 | 0 | 1873 → 1861 | -12 | 2 | 2 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.22 → 0.16 |
| no-construct | true | PASS | 3/0/0 | 0 | 1589 → 1583 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.07 → 0.05 |
| no-debugger | true | PASS | 1/0/0 | 0 | 1056 → 1050 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 1 | 0.05 |
| no-duplicate-super | false | PASS | 8/0/0 | 0 | 10634 → 10601 | -33 | 1 | 2 | 53 (29/11/12/1) | 18 | 23/10 | 11 | 1.03 → 0.88 |
| no-duplicate-switch-case | true | PASS (repaired fixture) | 4/0/2 | 0 | 1629 → 1623 | -6 | 1 | 1 | 5 (1/1/0/3) | 0 | 0/0 | 5 | 0.28 → 0.24 |
| no-dynamic-delete | false | PASS | 9/0/0 | 0 | 2841 → 2835 | -6 | 1 | 1 | 4 (2/2/0/0) | 2 | 1/4 | 9 | 0.17 → 0.12 |
| no-empty | true | PASS | 29/0/0 | 0 | 9981 → 9963 | -18 | 3 | 3 | 18 (5/13/0/0) | 7 | 7/7 | 9 | 0.63 → 0.53 |
| no-eval | true | PASS | 1/0/0 | 0 | 1351 → 1345 | -6 | 1 | 1 | 1 (1/0/0/0) | 0 | 0/1 | 3 | 0.05 |
| no-floating-promises | false | INFEASIBLE | 13/0/18 | 0 | 4382 → 4370 | -12 | 2 | 2 | 2 (0/2/0/0) | 0 | 6/6 | 6 | 0.53 → 0.43 |
| no-for-in-array | false | INFEASIBLE | 0/0/3 | 0 | 1593 → 1587 | -6 | 1 | 1 | 2 (0/2/0/0) | 1 | 2/1 | 5 | 0.15 → 0.13 |
| no-implicit-dependencies | false | BLOCKED | 20/1/6 | 0 | 13893 → 13863 | -30 | 5 | 5 | 15 (15/0/0/0) | 10 | 15/5 | 6 | 0.72 → 0.73 |
| no-invalid-template-strings | false | BLOCKED | 10/0/2 | 0 | 1054 → 1048 | -6 | 1 | 1 | 1 (1/0/0/0) | 0 | 0/1 | 3 | 0.25 → 0.18 |
| no-invalid-this | true | PASS (repaired fixture) | 0/0/6 | 0 | 8389 → 8377 | -12 | 2 | 2 | 13 (7/6/0/0) | 5 | 11/2 | 5 | 0.33 → 0.26 |
| no-misused-new | true | PASS | 5/0/0 | 0 | 5612 → 5552 | -60 | 1 | 3 | 14 (5/9/0/0) | 0 | 6/3 | 6 | 0.11 → 0.09 |
| no-null-keyword | true | PASS | 4/0/0 | 0 | 2015 → 2009 | -6 | 1 | 1 | 2 (2/0/0/0) | 2 | 1/1 | 5 | 0.10 → 0.09 |
| no-null-undefined-union | false | PASS | 1/0/0 | 0 | 1896 → 1890 | -6 | 1 | 1 | 7 (1/6/0/0) | 1 | 4/1 | 3 | 0.04 |
| no-object-literal-type-assertion | false | PASS | 6/0/0 | 0 | 4236 → 4224 | -12 | 2 | 2 | 5 (1/4/0/0) | 3 | 6/4 | 6 | 0.21 → 0.19 |
| no-promise-as-boolean | false | INFEASIBLE | 1/0/17 | 0 | 3088 → 3082 | -6 | 1 | 1 | 7 (6/0/1/0) | 0 | 4/2 | 6 | 0.29 → 0.27 |
| no-restricted-globals | false | BLOCKED | 10/5/0 | 0 | 4278 → 4272 | -6 | 1 | 1 | 2 (1/0/0/1) | 2 | 1/2 | 6 | 0.31 → 0.23 |
| no-return-await | true | PASS | 13/0/0 | 0 | 4773 → 4767 | -6 | 1 | 1 | 11 (7/1/1/2) | 3 | 6/0 | 4 | 0.34 → 0.25 |
| no-sparse-arrays | false | BLOCKED | 7/1/0 | 0 | 1143 → 1137 | -6 | 1 | 1 | 2 (1/0/1/0) | 0 | 0/1 | 3 | 0.14 → 0.10 |
| no-string-literal | false | PASS | 6/0/0 | 0 | 1326 → 1320 | -6 | 1 | 1 | 2 (1/0/0/1) | 0 | 0/1 | 3 | 0.14 → 0.11 |
| no-string-throw | true | PASS (repaired fixture) | 4/0/8 | 0 | 2055 → 2049 | -6 | 1 | 1 | 3 (0/3/0/0) | 0 | 1/0 | 3 | 0.33 → 0.27 |
| no-submodule-imports | true | PASS | 14/0/0 | 0 | 7094 → 7076 | -18 | 3 | 3 | 9 (9/0/0/0) | 2 | 9/3 | 6 | 0.37 → 0.30 |
| no-switch-case-fall-through | true | PASS | 6/0/0 | 0 | 24495 → 24489 | -6 | 1 | 1 | 90 (0/73/4/13) | 22 | 29/16 | 9 | 0.29 → 0.23 |
| no-tautology-expression | true | PASS | 20/0/0 | 0 | 3643 → 3637 | -6 | 1 | 1 | 1 (0/1/0/0) | 0 | 3/2 | 6 | 0.48 → 0.33 |
| no-this-assignment | true | PASS | 12/0/0 | 0 | 3644 → 3626 | -18 | 3 | 3 | 4 (0/4/0/0) | 2 | 1/3 | 5 | 0.22 → 0.19 |
| no-unbound-method | false | INFEASIBLE | 33/7/0 | 0 | 10526 → 10460 | -66 | 2 | 4 | 35 (25/6/0/4) | 6 | 8/6 | 8 | 1.03 → 0.69 |
| no-unnecessary-class | true | PASS (repaired fixture) | 51/0/3 | 0 | 24066 → 23907 | -159 | 4 | 9 | 78 (0/78/0/0) | 27 | 30/3 | 7 | 2.02 → 1.59 |
| no-unsafe-any | false | INFEASIBLE | 0/0/65 | 0 | 1033 → 1027 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 1.21 → 0.97 |
| no-unsafe-finally | true | PASS | 25/0/0 | 0 | 9188 → 9101 | -87 | 1 | 4 | 16 (12/4/0/0) | 6 | 6/4 | 6 | 0.70 → 0.53 |
| no-unused-expression | true | PASS | 121/0/0 | 0 | 80948 → 80924 | -24 | 4 | 4 | 256 (90/74/18/74) | 130 | 72/60 | 6 | 5.26 → 2.69 |
| no-unused-variable | false | INFEASIBLE | 0/0/0 | 0 | 777 → 771 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 1 | 0.00 |
| no-use-before-declare | false | INFEASIBLE | 1/0/9 | 0 | 1915 → 1909 | -6 | 1 | 1 | 3 (2/0/0/1) | 0 | 1/1 | 5 | 0.37 → 0.28 |
| no-var-keyword | true | PASS | 16/0/0 | 0 | 2112 → 2106 | -6 | 1 | 1 | 5 (5/0/0/0) | 2 | 0/1 | 6 | 0.30 → 0.24 |
| no-void-expression | false | INFEASIBLE | 3/0/3 | 0 | 7176 → 7164 | -12 | 2 | 2 | 12 (12/0/0/0) | 2 | 14/2 | 10 | 0.20 → 0.17 |
| prefer-conditional-expression | true | PASS | 10/0/0 | 0 | 9864 → 9852 | -12 | 2 | 2 | 34 (6/15/4/9) | 4 | 10/8 | 6 | 0.55 → 0.49 |
| prefer-object-spread | false | PASS | 14/0/0 | 0 | 3082 → 3049 | -33 | 1 | 2 | 5 (1/4/0/0) | 3 | 3/2 | 6 | 0.36 → 0.26 |
| radix | true | PASS | 6/0/0 | 0 | 1575 → 1569 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 1/0 | 2 | 0.18 → 0.12 |
| restrict-plus-operands | false | INFEASIBLE | 14/0/11 | 0 | 9653 → 9647 | -6 | 1 | 1 | 29 (0/15/7/7) | 4 | 11/7 | 6 | 0.68 → 0.43 |
| static-this | true | PASS | 10/0/0 | 0 | 1911 → 1905 | -6 | 1 | 1 | 3 (2/1/0/0) | 2 | 0/0 | 4 | 0.23 → 0.18 |
| strict-boolean-expressions | false | INFEASIBLE | 1/0/128 | 0 | 21492 → 21450 | -42 | 7 | 7 | 69 (35/14/14/6) | 0 | 41/7 | 6 | 1.75 → 1.53 |
| strict-comparisons | false | INFEASIBLE | 76/0/14 | 0 | 10630 → 10612 | -18 | 3 | 3 | 70 (0/38/16/16) | 6 | 27/16 | 4 | 2.35 → 1.98 |
| strict-string-expressions | false | INFEASIBLE | 0/0/30 | 0 | 7181 → 7115 | -66 | 2 | 4 | 8 (2/6/0/0) | 0 | 6/4 | 4 | 1.23 → 1.07 |
| strict-type-predicates | false | INFEASIBLE | 0/0/41 | 0 | 4777 → 4744 | -33 | 1 | 2 | 7 (0/7/0/0) | 0 | 3/3 | 6 | 1.27 → 1.12 |
| switch-default | true | PASS | 2/0/0 | 0 | 1519 → 1513 | -6 | 1 | 1 | 2 (0/2/0/0) | 1 | 0/1 | 5 | 0.11 → 0.10 |
| triple-equals | true | PASS | 6/0/0 | 0 | 6164 → 6065 | -99 | 3 | 6 | 14 (10/4/0/0) | 4 | 0/6 | 5 | 0.17 → 0.14 |
| typeof-compare | true | PASS | 14/0/0 | 0 | 2951 → 2945 | -6 | 1 | 1 | 3 (0/3/0/0) | 1 | 1/3 | 6 | 0.44 → 0.33 |
| unnecessary-constructor | false | PASS | 20/0/0 | 0 | 7620 → 7608 | -12 | 2 | 2 | 12 (0/12/0/0) | 4 | 5/2 | 7 | 1.01 → 0.84 |
| use-isnan | true | PASS | 8/0/0 | 0 | 1639 → 1633 | -6 | 1 | 1 | 2 (0/2/0/0) | 0 | 0/2 | 4 | 0.22 → 0.17 |
| cyclomatic-complexity | true | PASS | 21/0/0 | 0 | 13150 → 13132 | -18 | 3 | 3 | 3 (0/3/0/0) | 3 | 6/0 | 3 | 1.45 → 1.63 |
| invalid-void | true | PASS | 40/0/0 | 0 | 15223 → 15205 | -18 | 3 | 3 | 20 (15/5/0/0) | 2 | 15/8 | 9 | 0.73 → 0.52 |
| max-classes-per-file | false | PASS | 4/0/0 | 0 | 5660 → 5642 | -18 | 3 | 3 | 26 (11/7/0/8) | 3 | 14/4 | 4 | 0.15 |
| max-file-line-count | false | BLOCKED | 1/1/0 | 0 | 2715 → 2697 | -18 | 3 | 3 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.10 → 0.06 |
| no-default-export | true | PASS | 7/0/0 | 0 | 1581 → 1575 | -6 | 1 | 1 | 2 (2/0/0/0) | 1 | 0/1 | 4 | 0.16 → 0.10 |
| no-default-import | false | PASS | 14/0/0 | 0 | 5564 → 5498 | -66 | 2 | 4 | 18 (12/4/2/0) | 0 | 0/4 | 7 | 0.25 → 0.19 |
| no-duplicate-imports | true | PASS | 9/0/0 | 0 | 8485 → 8446 | -39 | 2 | 3 | 43 (12/25/0/6) | 1 | 9/4 | 9 | 0.33 → 0.36 |
| no-mergeable-namespace | true | PASS | 6/0/0 | 0 | 9854 → 9848 | -6 | 1 | 1 | 15 (2/11/0/2) | 0 | 16/6 | 13 | 0.15 → 0.12 |
| no-require-imports | true | PASS | 5/0/0 | 0 | 1497 → 1491 | -6 | 1 | 1 | 1 (1/0/0/0) | 0 | 1/1 | 4 | 0.10 → 0.08 |
| object-literal-sort-keys | false | BLOCKED | 9/1/35 | 0 | 10931 → 10907 | -24 | 4 | 4 | 33 (3/14/3/13) | 4 | 16/3 | 4 | 1.05 → 0.96 |
| prefer-const | false | BLOCKED | 1/0/43 | 0 | 1309 → 1303 | -6 | 1 | 1 | 3 (1/1/1/0) | 0 | 0/1 | 4 | 0.99 → 0.85 |
| prefer-readonly | false | PASS (repaired fixture) | 1/0/14 | 0 | 13682 → 13670 | -12 | 2 | 2 | 68 (34/32/2/0) | 12 | 18/17 | 7 | 0.51 → 0.43 |
| array-type | false | PASS | 48/0/0 | 0 | 5740 → 5665 | -75 | 3 | 6 | 9 (2/7/0/0) | 3 | 4/2 | 5 | 0.99 → 0.74 |
| arrow-return-shorthand | true | PASS | 17/0/0 | 0 | 11266 → 11200 | -66 | 2 | 4 | 41 (4/18/8/11) | 7 | 14/3 | 5 | 0.32 → 0.24 |
| binary-expression-operand-order | false | PASS | 9/0/0 | 0 | 2342 → 2336 | -6 | 1 | 1 | 1 (0/1/0/0) | 2 | 0/2 | 5 | 0.28 → 0.27 |
| callable-types | true | PASS | 19/0/0 | 0 | 3146 → 3140 | -6 | 1 | 1 | 5 (3/2/0/0) | 2 | 2/3 | 10 | 0.31 → 0.20 |
| class-name | true | PASS | 4/0/0 | 0 | 1741 → 1735 | -6 | 1 | 1 | 1 (1/0/0/0) | 0 | 1/1 | 4 | 0.07 → 0.06 |
| comment-format | false | PASS | 28/0/0 | 0 | 9693 → 9663 | -30 | 5 | 5 | 1 (0/0/0/1) | 20 | 5/11 | 8 | 0.74 → 0.56 |
| comment-type | false | PASS | 18/0/0 | 0 | 8818 → 8518 | -300 | 5 | 15 | 0 (0/0/0/0) | 0 | 0/15 | 2 | 0.21 → 0.19 |
| completed-docs | false | PASS (repaired fixture) | 76/0/53 | 0 | 91993 → 90925 | -1068 | 25 | 59 | 329 (123/130/0/76) | 129 | 133/171 | 10 | 7.36 → 2.64 |
| encoding | false | INFEASIBLE | 0/0/3 | 0 | 538 → 540 | +2 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.01 |
| file-header | false | PASS | 8/0/0 | 0 | 8424 → 8313 | -111 | 5 | 8 | 10 (0/5/0/5) | 15 | 1/0 | 7 | 0.95 → 0.63 |
| file-name-casing | false | BLOCKED | 11/1/0 | 0 | 9100 → 8944 | -156 | 8 | 12 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.51 |
| increment-decrement | true | PASS | 18/0/0 | 0 | 1670 → 1666 | -4 | 2 | 2 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.27 → 0.17 |
| interface-name | true | PASS | 5/0/0 | 0 | 2096 → 2092 | -4 | 2 | 2 | 2 (2/0/0/0) | 1 | 0/2 | 3 | 0.16 |
| interface-over-type-literal | true | PASS | 2/0/0 | 0 | 2313 → 2307 | -6 | 1 | 1 | 4 (1/3/0/0) | 1 | 0/3 | 9 | 0.08 → 0.07 |
| match-default-export-name | false | INFEASIBLE | 1/1/0 | 0 | 943 → 937 | -6 | 1 | 1 | 1 (1/0/0/0) | 0 | 0/1 | 3 | 0.04 |
| newline-per-chained-call | false | PASS | 12/0/0 | 0 | 8836 → 8830 | -6 | 1 | 1 | 8 (2/6/0/0) | 0 | 11/7 | 21 | 0.57 → 0.40 |
| no-angle-bracket-type-assertion | true | PASS | 20/0/0 | 0 | 1518 → 1512 | -6 | 1 | 1 | 1 (1/0/0/0) | 1 | 0/1 | 4 | 0.31 → 0.19 |
| no-boolean-literal-compare | false | INFEASIBLE | 13/1/0 | 0 | 2489 → 2483 | -6 | 1 | 1 | 2 (1/1/0/0) | 4 | 2/3 | 8 | 0.20 → 0.13 |
| no-parameter-properties | true | PASS | 4/0/0 | 0 | 1833 → 1827 | -6 | 1 | 1 | 1 (0/1/0/0) | 0 | 2/1 | 4 | 0.10 → 0.07 |
| no-redundant-jsdoc | false | PASS | 14/0/0 | 0 | 2796 → 2682 | -114 | 1 | 5 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.24 → 0.18 |
| no-reference-import | false → true | BLOCKED → PASS | 2/1/0 → 2/0/0 | 0 | 889 → 3093 | +2204 | 1 | 1 | 0 (0/0/0/0) → 5 (3/1/0/1) | 0 → 2 | 1/0 → 2/1 | 2 → 7 | 0.08 → 0.09 |
| no-unnecessary-callback-wrapper | false | PASS | 4/0/0 | 0 | 3333 → 3327 | -6 | 1 | 1 | 5 (0/5/0/0) | 3 | 3/1 | 6 | 0.23 → 0.15 |
| no-unnecessary-initializer | true | PASS | 8/0/0 | 0 | 6690 → 6657 | -33 | 1 | 2 | 16 (5/9/2/0) | 5 | 10/3 | 7 | 0.23 → 0.14 |
| no-unnecessary-qualifier | false | INFEASIBLE | 7/2/5 | 0 | 3319 → 3286 | -33 | 1 | 2 | 5 (5/0/0/0) | 1 | 4/2 | 5 | 0.49 → 0.30 |
| object-literal-key-quotes | false | PASS | 22/0/0 | 0 | 8095 → 8044 | -51 | 4 | 5 | 32 (19/8/0/5) | 8 | 10/1 | 4 | 0.56 → 0.38 |
| object-literal-shorthand | false | PASS | 32/0/0 | 0 | 6509 → 6410 | -99 | 3 | 6 | 8 (4/4/0/0) | 2 | 6/3 | 5 | 0.64 → 0.41 |
| one-variable-per-declaration | true | PASS | 19/0/0 | 0 | 7676 → 7583 | -93 | 2 | 5 | 15 (3/7/0/5) | 2 | 7/7 | 6 | 0.40 → 0.27 |
| ordered-imports | false | BLOCKED | 51/36/6 | 0 | 18929 → 18818 | -111 | 5 | 8 | 60 (8/24/8/20) | 4 | 20/32 | 8 | 2.42 → 1.75 |
| prefer-function-over-method | true | PASS | 14/0/0 | 0 | 14214 → 14190 | -24 | 4 | 4 | 51 (28/19/4/0) | 16 | 17/0 | 6 | 0.38 → 0.32 |
| prefer-method-signature | true | PASS | 4/0/0 | 0 | 2036 → 2030 | -6 | 1 | 1 | 3 (1/2/0/0) | 0 | 1/1 | 5 | 0.09 → 0.07 |
| prefer-switch | true | PASS (repaired fixture) | 0/0/8 | 0 | 14930 → 14918 | -12 | 2 | 2 | 65 (8/45/6/6) | 8 | 11/10 | 6 | 0.44 → 0.40 |
| prefer-template | true | PASS | 18/0/0 | 0 | 10939 → 10927 | -12 | 2 | 2 | 32 (2/17/11/2) | 11 | 8/12 | 6 | 0.31 → 0.27 |
| prefer-while | false | PASS | 2/0/0 | 0 | 867 → 861 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/1 | 2 | 0.10 → 0.08 |
| return-undefined | false | INFEASIBLE | 5/0/4 | 0 | 10580 → 10547 | -33 | 1 | 2 | 40 (4/36/0/0) | 18 | 19/2 | 6 | 0.30 → 0.27 |
| switch-final-break | false | PASS | 9/0/0 | 0 | 10261 → 10249 | -12 | 2 | 2 | 14 (5/3/6/0) | 10 | 14/12 | 11 | 0.30 → 0.21 |
| type-literal-delimiter | true | PASS | 13/0/0 | 0 | 6948 → 6855 | -93 | 2 | 5 | 13 (10/0/3/0) | 7 | 2/5 | 5 | 0.27 → 0.17 |
| unnecessary-bind | false | BLOCKED | 21/0/3 | 0 | 6435 → 6396 | -39 | 2 | 3 | 15 (3/12/0/0) | 5 | 5/3 | 6 | 0.83 → 0.57 |
| unnecessary-else | true | PASS (repaired fixture) | 0/0/34 | 0 | 11995 → 11983 | -12 | 2 | 2 | 51 (8/23/12/8) | 5 | 12/4 | 9 | 1.31 → 0.99 |
| variable-name | true | PASS (repaired fixture) | 13/0/80 | 0 | 27875 → 27800 | -75 | 8 | 9 | 83 (57/1/0/25) | 40 | 47/1 | 4 | 1.05 → 0.85 |
| arrow-parens | false | PASS (repaired fixture) | 2/1/12 | 0 | 3317 → 3305 | -12 | 2 | 2 | 7 (4/2/1/0) | 2 | 0/4 | 8 | 0.47 → 0.36 |
| import-spacing | false | PASS | 40/0/0 | 0 | 5764 → 5488 | -276 | 1 | 11 | 1 (1/0/0/0) | 0 | 0/8 | 4 | 0.46 → 0.29 |
| linebreak-style | true | PASS | 8/0/0 | 0 | 2183 → 2165 | -18 | 3 | 3 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.06 → 0.04 |
| newline-before-return | false | BLOCKED | 8/4/1 | 0 | 3564 → 3531 | -33 | 1 | 2 | 7 (2/0/3/2) | 2 | 2/4 | 9 | 0.40 → 0.28 |
| new-parens | true | PASS | 3/0/0 | 0 | 1374 → 1368 | -6 | 1 | 1 | 1 (0/1/0/0) | 1 | 0/1 | 4 | 0.06 → 0.05 |
| no-irregular-whitespace | true | PASS | 22/0/0 | 0 | 1229 → 1223 | -6 | 1 | 1 | 0 (0/0/0/0) | 0 | 0/0 | 0 | 0.11 → 0.04 |
| number-literal-format | true | PASS | 25/0/0 | 0 | 4197 → 4056 | -141 | 1 | 6 | 0 (0/0/0/0) | 0 | 0/6 | 2 | 0.24 → 0.13 |
| quotemark | true | PASS (repaired fixture) | 45/0/7 | 0 | 15565 → 15370 | -195 | 10 | 15 | 21 (18/2/1/0) | 9 | 2/18 | 7 | 1.10 → 0.98 |

