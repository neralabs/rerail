# NER-1562 historical SQL reproduction

## Result

The complete historical workload reproduced the SQLite performance problem.
The `applyAsync` call took 1,404.7 seconds and used 15.1 GiB of memory at its peak.

The removal commit reported 1,141 seconds for the same class of whole-tree run.
This reproduction was 23 percent slower than that result.
The build included `policy-profile` instrumentation, and the operating-system versions differed.

## Fixed inputs

The run used these inputs:

- SQL engine source: `30a03c642cdd993bec39357c27eb432b28043136`
- Benchmark harness: `aa63f36b580d4c90b9b955c77db9066f0733eb38`
- Data-layer source: `fce4dc7208c0b892c11a549c8be43fc285580d6b`
- Catalog: 12 policies and 829 clauses
- Corpus: 5,725 accepted files and 95,697,853 source bytes
- Worker count: 4
- Computer: Apple M5 Max with 18 logical CPUs
- Node.js: v26.8.1

The engine used the historical benchmark-only `no-file-deadline` behavior.
This behavior sets `FILE_WORK_BUDGET` to `u64::MAX`.
The change does not alter the SQL compiler or evaluator.

## Measurements

| Measurement | Value |
| --- | ---: |
| `applyAsync` wall time | 1,404.7 s |
| Process wall time | 1,435.0 s |
| User CPU time | 2,199.9 s |
| System CPU time | 100.4 s |
| Peak RSS | 15.1 GiB |
| Violations | 18,264 |
| Failures | 23 |
| Visited nodes | 27,196,611 |
| Witnesses | 38,308 |

The profile attributed these costs:

| Phase | Time or count |
| --- | ---: |
| AST match CPU | 2,240.1 s |
| Named worker CPU | 2,298.9 s |
| Anchor selection | 31.5 s |
| Manifest candidate lookup | 9.5 s |
| Parse | 7.7 s |
| Regex | 1.5 s |
| Rule-file evaluations | 1,167,586 |
| Selected root pairs | 4,687,139 |
| Rejected rule-file pairs | 866,997 |

The AST matcher and its SQL callback path dominate the run.
Parsing, regular expressions, and candidate lookup do not explain the elapsed time.

## Production-budget control

A control run used the normal work budget.
It completed `applyAsync` in 194.8 seconds.
It also returned 126,142 `budget-exceeded` failures.
Thus, that run did not complete the historical workload.

The earlier 7-second tests used the bundled 16-rule benchmark catalog.
They did not use the 12 expensive data-layer ports.
Therefore, those tests did not measure the workload from the removal commit.

## Local artifacts

The uncommitted raw results are in these local files:

- `/tmp/ner-1562-removal-parent-sql-unlimited-whole-tree.json`
- `/tmp/ner-1562-removal-parent-sql-unlimited-whole-tree.time`
- `/tmp/ner-1562-removal-parent-sql-whole-tree.json`
- `/tmp/ner-1562-removal-parent-sql-whole-tree.time`

The JSON files contain all policy hashes, compiled digests, engine hashes, machine data, and result counts.
