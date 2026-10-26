# Cold CPU architecture: spike results

Date: 2026-10-26.
Status: every planned aspect has a runnable experiment and a decision.
Production optimization remains separate.

The [workplan](cold-cpu-recovery-plan.md) defines the implementation sequence and acceptance gate.
The [attribution receipt](../fixtures/host/data-layer-port/profiling/cold-cpu-attribution-20261026.json) preserves the full diagnostic.
The [experiment receipt](../fixtures/host/data-layer-port/profiling/cold-cpu-architectural-spikes-20261026.json) preserves trials, identities, exact row digests, and qualifications.
The [tool guide](../scripts/profiling/preparation/cold-cpu.md) gives reproduction commands and model limits.
Raw evidence is under `.rerail/profiles/cold-cpu-spikes-20261026/`.

## The complete diagnostic identifies the main problem

All 9,021 files and 137,924,324 source bytes complete.
Expected and actual findings both equal 24,993.
False positives, false negatives, and engine failures are zero.
Logical compilation takes 1.958 CPU seconds and 2.170 wall seconds.

The traced cold run takes 3,586.594 CPU seconds and 4,308.203 wall seconds.
OS peak RSS is 7,530,659,840 bytes.
This run overlaps laboratory development and includes tracing.
These figures are diagnostic, not a new acceptance baseline or a measured regression.
The last uninstrumented all-60 reference remains 2,540.634 CPU seconds and 6.391 GiB peak RSS.
It precedes the final import-membership correction.

| Measured self CPU | Seconds | Span calls |
| --- | ---: | ---: |
| SQL analysis | 679.109 | 110,742 |
| Candidate materialization | 619.630 | 198,648 |
| Relational materialization | 455.858 | 110,780 |
| Relational indexes | 341.870 | 132,500 |
| Pool trimming | 190.128 | 22,098 |
| Candidate statement preparation | 161.016 | 203,989 |
| Candidate result reads | 145.748 | 5,341 |
| Consumer release | 132.883 | 11,045 |
| Relational result reads | 51.483 | 11,045 |

These counts describe spans, not engine-submitted statements.
An analysis or index call can reuse existing state and submit no SQL.
The table must not replace the separate SQL execution counter.

The executor admits up to **792 suspended SQL consumers** in four capacity partitions.
Peak reported suspended SQL storage is 249,905,204 bytes.
The admission limit is 256 MiB; the input limit is 64 MiB.
Those limits do not prevent expensive schema interactions between admitted consumers.

`Storage::analyze` invokes SQLite analysis during stage construction.
SQLite's bundled `sqlite3AnalysisLoad` clears schema statistics and reloads the complete `sqlite_stat1` relation.
It also visits schema tables and indexes.
Repeated small analyses therefore interact with the other live schemas.
This explains why optimizing the final predicate leaves substantial CPU unchanged.

Two supplemental Node samples corroborate this path.
`sqlite3FindTable` and `sqlite3FindIndex` dominate many active stacks alongside paging and `sqlite3AnalysisLoad`.
Their wall-time sample weights are not CPU shares.
The scheduled samples accidentally targeted `/usr/bin/time`; they provide no engine evidence.
A regression and sampler correction now select the workload child and record its PID.

Clause attribution places 776.922 measured seconds in `quality-contracts-1`.
`state-flow-1` accounts for 305.016; `authority-sites` accounts for 192.103.
`legacy-execution-graphs` accounts for 114.153.
The first quality clause is the first relational family to replace.

The reporter observes 3,204.752 seconds across measured spans.
Of this, 1,527.242 seconds lack a complete rule/clause identity, including shared candidate and pool work.
Untraced work remains separate.
The capture has 103 overlap flags and seven CPU-overdraw flags among 1,953,732 events.
These flags remain visible; this is not perfect CPU reconciliation.

## S1: choose one anchor before execution

The captured structural family has 333 result rows in four overlapping request scopes.
One persisted-statistics query chooses between function and identifier anchors.
It does not count each posting alternative at execution time.

| Four-scope control | Median total CPU, ms | Median query CPU, ms |
| --- | ---: | ---: |
| Broad construction | 67.378 | 0.954 |
| Forced identifier anchor | 73.168 | 8.442 |
| Statistics-selected anchor | 66.203 | 0.826 |

The total includes the laboratory fact reconstruction.
That reconstruction dominates this small case.
Tests change statistics to reverse the choice and retain complete row parity.
An empty outer demand gate prevents entry into a failing inner operation.

**Decision:** select from SQL-owned statistics and emit one arm.
Extend execution ordering beyond the current compiler-complexity ordering in `Lower::plan`.
Keep compilation-complexity limits separate from cardinality estimates.

## S2: fuse SQL before creating physical cuts

Two consumers produce 17,033 rows over 256 scoped requests.
Every trial preserves the complete multiset.

| Physical shape | Median total CPU, ms | Median query CPU, ms |
| --- | ---: | ---: |
| CTAS, index, ANALYZE, read, release | 109.918 | 44.109 |
| Repeated inline computation | 124.949 | 58.363 |
| One shared materialized CTE | 101.101 | 36.347 |
| Fused consumer query | 97.916 | 33.847 |

The fused shape is the simplest candidate here.
A materialized CTE is useful when it avoids an actual second computation.
Repeated inline expansion loses.
These are structural controls, not a complete production-family rewrite.

The actual quality-policy overlay removes two terminal diagnostic cuts.
It preserves authored options, deferred work, recursive work, and native boundaries.
All 192 original quality observations pass: 130 positive and 62 clean.
Finding parity and execution-path parity are exact.

The first broader overlays fail the existing 16-KiB authored-query limit.
The bounded overlay respects that limit.
A small repository control shows no useful CPU improvement from this policy-only change.
It leaves the other construction and schema costs in place.

**Decision:** perform broad fusion after logical validation, inside the physical planner.
Keep the authored limits. Partition physical families at real producer boundaries and existing SQLite limits.
Do not promote the small policy overlay as the recovery solution.

## S3: preserve the batch boundary; reject an unsupported CPU claim

The generic first-field-child operator receives the complete request-ID batch through `sqlite3_vtab_in`.
It returns typed columns and applies the kind predicate after selecting the first field child.
It does not fall through to a later child with a preferred kind.

The regression covers 80,000 correlated requests and 80,000 outputs.
It uses one `xFilter`, two broad output waves, and at most 65,536 buffered rows.
`xNext` serves completed columns.
The first prototype visited unnecessary sibling children; the corrected operator matches `nera_child` semantics.

| Prepared operator, 20,556 outputs | Median total CPU, ms |
| --- | ---: |
| SQL child index | 106.832 |
| Scalar control | 100.773 |
| Native typed batch | 105.781 |

Native batching does not establish a CPU win for this cheap operator.
Its descriptor, transport, and output costs remain visible.

**Decision:** keep the typed whole-demand contract for expensive operators and future GPU execution.
Do not promote the native child implementation as a CPU optimization.
Use SQL for the selected production binding until a complete native comparison wins.
The scalar control is a measurement control, not the target execution shape.

## S4: transaction batching wins; retain existing triggers and cache budgets

This control publishes 1,024 copies of one complete captured file.
It preserves the production positional FTS declaration and complete value-posting bytes.
All variants pass statistics, rollback, replacement, and pinned SQLite-reader checks.

| Publication shape | Main cache | Median total CPU, ms |
| --- | ---: | ---: |
| Per-file transactions, triggers | 1 MiB | 3,458.821 |
| 32-file transactions, triggers | 1 MiB | 529.826 |
| 32-file transactions, grouped value statistics | 1 MiB | 489.235 |
| Grouped statistics | 8 MiB | 431.765 |
| Grouped statistics | 32 MiB | 449.563 |

Transaction batching is the clear improvement.
The additional statistics gain overlaps trial variation and adds maintenance complexity.
Larger caches increase process residency. They do not provide a consistent additional win.
The replicated tiny file cannot predict the complete repository gain.

A separate 32-file trace records seven host submissions and 46 SQLite internal completions for packet publication with triggers.
The per-file control records 224 host submissions and 295 internal completions.
The trigger control also records 2,336 trigger entries; these are not additional host submissions.
FTS still maintains per-document metadata inside a host batch.

**Decision:** publish 32-file packets with the existing triggers, positional index, and 1-MiB cache budget.
Do not promote grouped statistics or larger caches from this evidence.

## S5: reduce schema fan-out, then reuse stable templates

For 64 consumers, increasing retained schemas from 128 to 1,024 raises repeated-analysis consumer CPU from 17.209 to 102.277 ms.
At 1,024 retained tables, omitting repeated analysis reduces consumer CPU to 22.220 ms.
Stable backing takes 1.365 ms; the fused control takes 0.060 ms.
The complete trials include retained-schema construction and cleanup.
Those common costs dominate their totals and must not be omitted.
This control does not measure the full run's exact schema count.

The 256-consumer planning control takes 118.637 ms with private schemas and 95.699 ms with one batch.
Reusing a statement takes 100.604 ms but still executes it 256 times.
A regression keeps preparation counts distinct from execution counts.

The real compiler replay preserves complete schemas for five recorded programs repeated 128 times.
Shared metadata takes 1,470.930 median CPU ms, versus 2,712.551 without sharing.
Inference statements change from 4,480 to 35.
The production engine already has this bounded cache.
This is evidence to retain it, not a newly implemented engine improvement.

**Decision:** gather demand before creating private SQL schemas and execute one physical family batch.
Reuse templates when the chosen plan stays unchanged.
Count changes alone must not invalidate a physical template.
Pool-trim and release spans still mix metadata scans and SQLite cleanup.
Instrument their visited objects and actual drops before proposing new owner indexes.

## S6: construction dominates the composed result

The composed 256-request test preserves all 17,033 result rows.

| Complete structural control | Median CPU, ms |
| --- | ---: |
| Staged SQL | 109.005 |
| Fused SQL with common reconstruction | 99.925 |
| Native operator with common reconstruction | 109.374 |
| Direct postings and scalar control | 23.895 |
| Direct postings and native batch | 30.764 |

Empty demand still costs about 59–65 ms when the broad laboratory fact table is constructed first.
The direct controls take less than one millisecond and never enter the native operator.
The direct FTS query gates demand outside the expensive join and scans the chosen term once.

This demonstrates the preparation boundary.
The laboratory table is a reconstruction of captured facts, not production publication.
Its removal is not a measured fourfold end-to-end engine improvement.
The actual policy control and full repository remain essential.

**Decision:** remove unnecessary construction before layering operator improvements on top.
Do not multiply the isolated ratios or present the native control as the recovery result.

## Memory remains a separate frontier

The full deferred snapshot reports about 7.2 GiB physical footprint and 4 GiB of swapped allocator pages.
A falling current RSS therefore does not demonstrate memory recovery.
The complete run's peak footprint is 7,906,637,232 bytes.

The separate 128-file census finds zero JSON projection fields.
It records 6,977,240 typed projection bytes and 23,776,878 resident document bytes after deferred execution.
Live allocator bytes fall from 160,906,480 to 14,461,872 after workspace closure.
Reserved allocator space remains much larger.
This confirms that the earlier JSON removal is real.
It does not assign the full repository's live memory to individual owners.

The next memory work must account for retained syntax, typed projections, source graphs, SQL schemas, and completed producers separately.
Reduce live execution fan-out first. Use ownership measurements to set document and producer lifetimes.
Do not trade memory for repeated parsing without a complete cold CPU comparison.

## Validation and remaining gate

The native DSL passes 1,540 tests with 23 explicit ignores.
All 51 production-shape tests pass.
The laboratory passes 70 tests; the profiler passes 30.
Static checks and the profiling suite pass after the analysis-module directory correction.
Clippy checks pass with warnings denied; final tooling checks remain recorded in the journal.

No production algorithm flags or Linear issues were added.
The work does not claim a new uninstrumented 50/60 performance result.
Implement the selected architecture, then run both complete workloads with tracing off and compile time recorded separately.
