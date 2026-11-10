# Retain semantic demands across native operators

## Merge boundary

The literal `Pulling-it-all-together` tip, `cbee7a03c`, was already an ancestor.
This work merges its committed successors:

- Integration hub: `aa2ef00ef`, branch `jeremybarnes/final-shape-serverplan-quality`.
- Executor: `8bd924c72`, branch `jeremybarnes/final-rerail-shape`.

Uncommitted work in those worktrees stays there.
The only merge conflict concerns export reads in `corpus.rs`.
The hub's keyed `rows_of` implementation remains; it avoids a snapshot scan.

## Measured problem

The previous profile used 1,766,340 VM instructions to publish and reread `async_reference`.
That is 77.4% of its 2,283,088 SQL VM instructions.
The native set engine already owns these rows before publication.
The semantic operator forces a table boundary, and four qualification consumers read the same keys.
A subsequent native join then reloads those published rows.

## Required shape

```text
posting facts -> native structural joins -> retained candidate relation
                                               |
                                 one demand snapshot per relation
                                               |
                                  batched semantic fact operators
                                               |
                                      retained result relations
                                               |
                               native joins, or SQL when required
```

Use the integration hub's retained relation and source interfaces.
Read native demands directly. Cache their distinct keys for this run only.
Consumers share keys only when they name the same completed relation.
Their target sets remain separate. Never broaden candidate scope to combine different policies.
File and world identity remain in node keys.

Keep typed semantic results in the native relation store.
Compute planner statistics from those rows through the existing source interface.
Materialize only when a later SQL consumer needs a table.
An SQL-only producer remains a valid source: read its completed table once.
No new execution option is required.

The cache belongs to one run and expires after the final reader.
It must not reuse keys across chunks or session updates.
The retained path requires completed demands. It rejects recursive semantic components.

## Gates

First measure the merged engine before changing this boundary.
Preserve its full-corpus profile and failing retained-candidate gate.
The repaired profile must contain no executed SQL that names `async_reference`.
Instrumentation must identify direct demand reads and cache reuse.

Check positive findings, empty demands, exact target isolation, repeated relation names across runs, and an SQL consumer of retained outputs.
Keep all aggregate and semantic tests from the previous change.
Run all four edge fixtures against the preserved Ruff oracle.
Report whole-job CPU, wall time, RSS, statements, and VM instructions for the same 5,725 files.
Use repeated warm-program runs to separate runtime variation from structural savings.

This change does not claim to remove all temporary tables or meet the 100-execution budget.
The lifecycle work remains a separate measured follow-up.

## Prior work

The rule-engine direction requires batched pushdown and narrow typed relations.
Its lessons ledger rejects unnecessary materialization, sharing over different scopes, and proxy-only performance claims.
Decision 0090 retains relations across session steps.
Hub commits `a34441da4` and `aa2ef00ef` keep statistics and indexes with retained rows.
This change extends that ownership through semantic fact operators.

## Merged baseline

The three warm samples use 13.682, 13.596, and 13.942 CPU seconds.
Their median is 13.682 CPU seconds and 28.910 wall seconds.
The profile still records exactly 2,283,088 VM instructions and 10,272 SQL executions.
The candidate-boundary gate fails with 1,773,972 VM instructions attributed to statements that name `async_reference`.
These include setup and statistics, besides the 1,766,340 instructions in publication and the major reads.

The merge also exposed a legacy-extension compatibility regression: `unknown join item sum`.
The planner now reserves the new numeric reduction instructions for the production engine.
The frozen text-symbol extension uses SQL for these reductions.
The aggregate growth gate checks both output parity and the production native sum path.

## Final results

Both variants process the same 5,725 files, 95,697,853 source bytes, and 19,007,630 nodes.
Prepared programs come from the store. File and cross-file result memos remain disabled.
The primary timing comparison alternates the saved merged binary and the final binary, with three runs each.
The order is before/after, after/before, before/after.

| Measurement | Merged baseline | Retained demands |
|---|---:|---:|
| Median whole-job CPU | 9.358 s | 8.510 s |
| Median wall time | 8.283 s | 7.332 s |
| Median local execution CPU | 0.674 s | 0.567 s |
| SQL executions | 10,272 | 3,849 |
| Profile SQL VM instructions | 2,283,088 | 336,517 |
| Candidate-related SQL VM instructions | 1,773,972 | 0 |
| Native candidate snapshots | — | 72 |
| Reuse of those snapshots | — | 216 |
| Findings | 0 | 0 |

The measured CPU reduction is about 9%; SQL VM instructions fall 85.3%, and executions fall 62.5%.
These are three-run medians on a shared host, not a controlled hardware benchmark.

The first separate timing series gave medians of 13.682 and 8.575 CPU seconds.
Parsing and batch construction improved by a similar proportion, so that difference cannot be attributed to this fix.
The report uses the later alternating comparison. Both series remain in the evidence.

The final profile spends 4.630 CPU seconds in batch construction and 1.660 seconds in worker parsing.
Its largest exclusive span is batch projection and interning, at 3.803 seconds.
Actual semantic projection remains ten nodes across five scope tables.
The optimization preserves target selection; it removes publication, repeated reads, and result-table setup.

## Final validation

- All 22 focused tests pass, including SQL fallback, target isolation, duplicate rows, empty demands, and cache lifetime.
- All 93 active RR tests pass. Two existing tests remain ignored.
- Nine file-local tests, scalar reduction parity, and the frozen native extension goldens pass.
- All four edge fixtures match the preserved 178 Ruff findings, with no missing or extra keys.
- The older reference-only edge report still flags its four documented misses. Its expected files remain unchanged.
- The complete-job findings and corpus size match the merged baseline.
- The retained-candidate gate passes: no executed SQL names `async_reference`, with one snapshot per chunk.

## Remaining work

The 100-execution budget still fails: this job executes 3,849 statements.
The profile records 915 CREATE, 906 DROP, 909 INSERT, 897 SELECT, and 222 other executions.
Preparation alone does not remove them. Reusable execution structures remain a separate task.

The fixed positions table consumes 110,608 VM instructions; document-range publication consumes 74,433.
Together they account for 55% of the remaining SQL VM instructions.
Whole-job CPU is dominated by syntax batch preparation, so further SQL changes must retain the complete-job measurement.

## Evidence

[Recorded profiles, tests, and timings](../../benchmark/async-family-integration/retained-demands/)
include the failed merged baseline gate and the passing final gate.
The provenance file records both merged tips, the baseline commit, source hashes, and binary hashes.
The journal records the merge conflict, compatibility repair, fixture failures, and final measurements.
