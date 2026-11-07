| corpus | files | nodes | source B/node | source zstd B/node |
|---|---|---|---|---|
| cpython-3.13-Lib | 1702 | 7,897,149 | 4.18 | 0.97 |
| nera-data-layer-impl | 3043 | 11,619,212 | 5.03 | 1.02 |
| rerail-ts | 559 | 2,029,743 | 3.44 | 1.04 |
| rerail-rust | 413 | 2,454,794 | 3.56 | 0.96 |
| rerail-php-fixtures | 534 | 161,595 | 3.26 | 1.23 |

Logical B/node

| corpus | v7 | v8 | v8i | v8k | v8np | v9 | v9np | v9r | v9i |
|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 8.49 | 13.12 | 13.12 | 12.12 | 10.00 | 10.50 | 8.38 | 10.50 | 8.38 |
| nera-data-layer-impl | 8.47 | 13.50 | 13.50 | 12.50 | 10.21 | 10.84 | 8.55 | 10.84 | 8.55 |
| rerail-ts | 8.47 | 11.69 | 11.69 | 10.69 | 8.62 | 10.39 | 8.32 | 10.39 | 8.32 |
| rerail-rust | 8.47 | 11.82 | 11.82 | 10.82 | 8.82 | 10.25 | 8.25 | 10.25 | 8.25 |
| rerail-php-fixtures | 8.93 | 11.05 | 11.05 | 10.05 | 8.05 | 10.31 | 8.31 | 10.31 | 8.31 |

zstd -3 B/node (per document)

| corpus | v7 | v8 | v8i | v8k | v8np | v9 | v9np | v9r | v9i |
|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 1.74 | 3.29 | 3.67 | 3.14 | 2.77 | 2.15 | 1.77 | 2.10 | 2.14 |
| nera-data-layer-impl | 1.85 | 3.48 | 3.89 | 3.33 | 2.94 | 2.36 | 1.97 | 2.34 | 2.32 |
| rerail-ts | 2.18 | 3.80 | 4.03 | 3.63 | 3.17 | 2.50 | 2.05 | 2.49 | 2.43 |
| rerail-rust | 2.07 | 3.72 | 3.98 | 3.54 | 3.10 | 2.46 | 2.01 | 2.43 | 2.39 |
| rerail-php-fixtures | 2.69 | 3.96 | 4.28 | 3.77 | 3.25 | 3.04 | 2.53 | 3.10 | 2.85 |

Ratio to v7 (logical / zstd)

| corpus | v8 | v8i | v8k | v8np | v9 | v9np | v9r | v9i |
|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 1.55 / 1.90 | 1.55 / 2.11 | 1.43 / 1.81 | 1.18 / 1.60 | 1.24 / 1.24 | 0.99 / 1.02 | 1.24 / 1.21 | 0.99 / 1.23 |
| nera-data-layer-impl | 1.59 / 1.88 | 1.59 / 2.10 | 1.48 / 1.79 | 1.21 / 1.58 | 1.28 / 1.28 | 1.01 / 1.06 | 1.28 / 1.26 | 1.01 / 1.25 |
| rerail-ts | 1.38 / 1.74 | 1.38 / 1.85 | 1.26 / 1.67 | 1.02 / 1.46 | 1.23 / 1.15 | 0.98 / 0.94 | 1.23 / 1.14 | 0.98 / 1.12 |
| rerail-rust | 1.40 / 1.79 | 1.40 / 1.92 | 1.28 / 1.71 | 1.04 / 1.49 | 1.21 / 1.19 | 0.97 / 0.97 | 1.21 / 1.17 | 0.97 / 1.15 |
| rerail-php-fixtures | 1.24 / 1.47 | 1.24 / 1.59 | 1.13 / 1.40 | 0.90 / 1.20 | 1.15 / 1.13 | 0.93 / 0.94 | 1.15 / 1.15 | 0.93 / 1.06 |

Rust decode, ns (random: per probe of kind+parent+size+span+named over 2M random nodes; seq: per node)

| corpus | random_v7 | random_v8 | random_v9 | random_span_v7 | random_span_v8 | random_span_v9 | seq_v7 | seq_v8 | seq_v9 | v7_restore_to_u32x5 | v9np_parent_rebuild | parity |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 245.1 | 41.0 | 78.5 | 222.8 | 11.9 | 45.9 | 3.9 | 3.8 | 8.8 | 3.7 | 4.2 | True |
| nera-data-layer-impl | 489.8 | 96.0 | 198.5 | 364.4 | 13.3 | 56.5 | 4.3 | 3.8 | 24.3 | 7.8 | 4.1 | True |
| rerail-ts | 179.8 | 73.3 | 93.8 | 78.1 | 6.9 | 47.6 | 4.0 | 3.2 | 9.8 | 3.8 | 4.6 | True |
| rerail-rust | 258.4 | 76.8 | 91.9 | 183.3 | 8.3 | 37.4 | 3.8 | 3.1 | 9.2 | 3.8 | 4.5 | True |
| rerail-php-fixtures | 113.4 | 10.4 | 23.2 | 32.7 | 3.5 | 13.0 | 3.9 | 2.9 | 9.6 | 4.0 | 5.1 | True |

Widths, code space, invariants

| corpus | docs | docs u32 par/size | docs u32 start/len (v8) | nodes in those | v9 docs u32 off/tail | max kind / catalog | max field | folded codes | aliases | non-uniform extra kinds (minority nodes) | extra | missing | error | internal | end≠end(last) | start≠start(first) | leaves ≥256 B | v7 side tables B/node |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 1702 | 6 | 124 | 3,703,217 | 0/0 | 273/274 | 32 | [209] | 0 | 1 (1) | 91021 | 0 | 13 | 2,672,175 | 8171 (0.31%) | 19596 | 2720 | 0.046 |
| nera-data-layer-impl | 3043 | 17 | 171 | 5,564,877 | 0/0 | 272/274 | 32 | [194] | 0 | 0 (0) | 26062 | 0 | 83 | 3,925,410 | 3944 (0.10%) | 1067 | 3588 | 0.011 |
| rerail-ts | 559 | 1 | 9 | 279,160 | 0/0 | 382/383 | 40 | [257] | 0 | 1 (2) | 6784 | 0 | 5 | 657,612 | 559 (0.09%) | 0 | 904 | 0.013 |
| rerail-rust | 413 | 0 | 18 | 500,634 | 0/0 | 354/355 | 31 | [239] | 0 | 0 (0) | 16691 | 0 | 0 | 758,083 | 4758 (0.63%) | 326 | 85 | 0.027 |
| rerail-php-fixtures | 534 | 0 | 0 | 0 | 0/0 | 431/432 | 31 | [277] | 0 | 0 (0) | 1518 | 0 | 7 | 58,982 | 265 (0.45%) | 28 | 12 | 0.038 |

Encode ns/node (after parse; noisy under load)

| corpus | parse ns/node | v7 | v8 | v9 |
|---|---|---|---|---|
| cpython-3.13-Lib | 1304 | 26.7 | 34.5 | 39.0 |
| nera-data-layer-impl | 1575 | 38.2 | 17.9 | 50.4 |
| rerail-ts | 1044 | 11.4 | 9.1 | 11.7 |
| rerail-rust | 904 | 6.4 | 77.3 | 29.3 |
| rerail-php-fixtures | 1671 | 10.2 | 14.4 | 15.0 |
