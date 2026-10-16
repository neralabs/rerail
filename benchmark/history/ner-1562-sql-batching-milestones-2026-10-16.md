# NER-1562 SQLite batching milestones

## Purpose

This report measures each SQLite batching milestone against the data-layer workload.
It also identifies the costs that remain after clause-major execution.

## Fixed inputs

The measurements used these inputs:

- Data-layer source: `fce4dc7208c0b892c11a549c8be43fc285580d6b`
- Catalog: 12 policies and 829 clauses
- Complete corpus: 5,725 files and 95,697,853 source bytes
- Diagnostic corpus: the first 100 sorted accepted files
- Computer: Apple M5 Max with 18 logical CPUs
- Rerail worker count: 4
- Node.js: v26.8.1
- Build: release with `code-rule-profile`
- Engine: SQLite

The diagnostic corpus completed at every milestone.
It produced 54 violations and no failures at every milestone.

The complete production-budget runs did not complete all rule work.
Therefore, their times do not replace the historical unlimited-work baseline.

## Diagnostic milestone results

| Milestone | Commit | `applyAsync` | AST match | SQLite statements |
| --- | --- | ---: | ---: | ---: |
| Restored file-major engine | `39708045` | 5.016 s | 0.203 s | Not recorded |
| 16-file terminal batches | `182fbb62` | 4.997 s | 0.202 s | Not recorded |
| Full-file batches with parallel preparation | `db4232b9` | 5.952 s | 0.135 s | Not recorded |
| One text statement per batch | `5a4e9baf` | 5.080 s | 0.115 s | 5,620 |
| Clause-major AST statements | `25f6ff55` | 5.011 s | 0.108 s | 504 |
| Isolated file budgets | `e8a2d1fa` | 4.787 s | 0.104 s | 504 |

Clause-major execution reduced the diagnostic statement count by 91.0 percent.
The complete diagnostic runtime decreased by 4.6 percent from the restored engine.
The AST-match time decreased by 48.9 percent.

The fixed cost outside AST matching dominates this small workload.
Thus, the diagnostic runtime does not show the large-corpus effect.

## Complete production-budget results

| Milestone | `applyAsync` | User CPU | Peak RSS | Violations | Failures |
| --- | ---: | ---: | ---: | ---: | ---: |
| `39708045` | 69.2 s | 209.8 s | 4.35 GiB | 10,018 | 37,895 |
| `182fbb62` | 66.9 s | 205.9 s | 4.61 GiB | 10,835 | 37,895 |
| `db4232b9` | 165.2 s | 175.4 s | 6.12 GiB | 10,835 | 37,895 |
| `5a4e9baf` | 175.6 s | 184.1 s | 6.07 GiB | 10,835 | 37,895 |
| `25f6ff55` | 134.6 s | 138.9 s | 5.26 GiB | 5,627 | 171,768 |
| `e8a2d1fa` | 651.9 s | 633.3 s | 14.88 GiB | 16,183 | 15,204 |
| Parallel compatibility groups | 293.1 s | 716.2 s | 19.95 GiB | 16,183 | 15,204 |

The clause-major implementation at `25f6ff55` let one file budget interrupt its full batch.
This defect caused 143,347 `adapter-error` failures.
Commit `e8a2d1fa` isolated the file budgets and removed those failures.

The corrected run completed much more rule work than the earlier milestones.
It still returned 15,199 `budget-exceeded` failures and 5 source failures.
Therefore, the complete milestone times are not direct throughput comparisons.

Parallel compatibility-group execution reduced corrected wall time by 55.0 percent.
Its peak RSS increased by 34.1 percent.
This result shows a CPU and memory tradeoff.

## Query-shape effect

The last file-major milestone executed 1,142,480 SQLite statements.
The corrected clause-major engine executed 61,805 statements.
This change removed 94.6 percent of statement executions.

The corrected engine registered these totals:

- 154 SQLite batches
- 10,152 file registrations across those batches
- 61,805 statements
- 3,921,804 result rows
- 232 terminal executions
- 232 terminal virtual-table filters

Compatibility partitions still multiply clause execution.
The partitions come from different languages and candidate-clause sets.

## New bottlenecks

AST matching remains the main CPU cost.
The parallel run recorded 785.2 CPU-seconds in the AST-match phase.
Parsing used 7.2 CPU-seconds, and regular expressions used 0.3 CPU-seconds.

The clause-major engine now uses all four workers across compatibility groups.
This change reduced wall time, but concurrent batch state increased peak memory to 19.95 GiB.

A second profile added counters to the complete parallel run.
The counter instrumentation increased wall time from 293.1 seconds to 372.1 seconds.
Therefore, the first parallel run remains the runtime result.

The detailed profile measured these counts:

| Operation | Count |
| --- | ---: |
| Node virtual-table rows | 3,486,284,310 |
| Scalar callbacks | 2,696,546,413 |
| Environment handles | 100,521,003 |
| Semantic-fact cache hits | 11,490,154 |
| Semantic-fact builds | 7,232,793 |
| Output rows | 3,921,804 |

Node walks and scalar callbacks dominate the new execution shape.
SQLite emits 889 node rows for each output row.
It also invokes 688 scalar callbacks for each output row.

Environment retention is the next memory target.
The batches created more than 100 million environment handles.

The first correction must reduce repeated node walks and scalar callbacks together.
Bulk structural relations are the measured next boundary.
The engine must not add more compatibility partitions.

## Historical comparison

The historical unlimited-work run took 1,404.7 seconds and used 15.1 GiB.
It returned 23 failures after complete rule work.

The new production-budget run took 293.1 seconds after group parallelism.
It returned 15,204 failures and did not complete the same rule work.
Thus, these times do not show a 4.8-fold complete-work speedup.

An unlimited-work build is required for the next direct historical comparison.

## Local artifacts

The raw results remain outside the repository under `/tmp`.
Their names start with `/tmp/ner-1562-` and include the commit and corpus size.
The built milestone add-ons remain under `/tmp/ner1562-binaries`.
