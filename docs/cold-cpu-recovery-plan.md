# Cold CPU recovery: workplan and spike ledger

Date: 2026-10-26.
Status: design complete. Spikes are next. Production promotion remains separate.

## Problem and acceptance boundary

The complete repository contains 9,021 files and 137,924,324 source bytes.
Both frozen runs have exact finding parity and zero execution failures.

| Cold CPU, seconds | First 50 | All 60 |
| --- | ---: | ---: |
| File ingestion and candidate preparation | 628.327 | 1,148.124 |
| Deferred evaluation | 44.165 | 1,392.224 |
| Complete cold execution | 672.782 | 2,540.634 |
| Selection counter | 347.871 | 729.928 |
| Index preparation counter | 141.067 | 129.736 |
| Physical planning counter | 78.008 | 159.368 |
| OS peak RSS, GiB | 4.620 | 6.391 |

The phase totals and component counters are different views. Do not add both views.
Deferred evaluation includes file analyses and repository analyses. It is not one global query.
The older all-60 run failed seven times. Its 492.543-second deferred phase is not a complete reference.
The passing pre-SQL first-50 reference takes 125.804 cold CPU seconds.
No clean equivalent pre-SQL all-60 reference exists in the current evidence.

The final import-membership correction and allocator-profiling correction follow these frozen timings.
Every new report must record native, policy, workload, driver, and spike source identities.

Acceptance requires complete cold CPU, wall time, OS peak RSS, and finding parity on both repository workloads.
Logical rule compilation stays separate. Physical planning, publication, construction, and cleanup stay inside cold execution.
The first-50 recovery target remains 125.804 CPU seconds. Smaller improvements do not close recovery.
The 100-submission reference and its regression tests remain intact as diagnostics.
Work allowances, source scope, findings, and fault isolation remain unchanged.
No Linear issue is created without user approval.

## Target execution shape

The executor has four specialized phases. It needs no generic graph scheduler.

1. Publish immutable tape versions, positional kind/value indexes, and SQL-owned statistics in bounded batches.
2. Gather exact consumer demand and choose one physical access path from stored statistics.
3. Execute narrow operator families over the whole demand batch.
4. Consolidate findings and release consumer state.

A demand row carries consumer, source version, file, range, producer, parameters, and requested columns.
Separate IN lists must not create a Cartesian product of those fields.
A native binding receives the complete demand batch in one xFilter and returns typed result rows.
xNext emits output rows. It does not implement an interpreter or request another scalar node operation.
The same relation contract also has a SQL binding.
These contracts preserve the future CPU, JIT, and GPU boundary.

Shared producers require equal definitions, versions, parameters, columns, and candidate scopes.
A shared producer completes once within that exact execution domain, including a complete empty result.
Different scopes remain separate even when their ranges overlap.
A measured second consumer justifies materialization. Repeated query text alone does not.
A native producer implements a generic relation operator. It does not encode a data-layer rule.

## Measurement ladder

1. Reproduce the production shape and preserve its complete output rows.
2. Compare setup-inclusive CPU and OS RSS in fresh processes.
3. Change files, demand rows, source nodes, consumers, and scope overlap independently.
4. Repeat the original positive and clean observations for affected rules.
5. Measure the complete repository after a selected implementation delivery.

Normal experiment commands have a 110-second target deadline and a 115-second outer limit.
Each command uses a separate result directory. A timeout remains an incomplete result.
Approved optimized builds and complete repository runs can finish with progress reports.
Full diagnostic profiling can overlap development, but its timing cannot establish acceptance.
Acceptance timing runs have no overlapping work from this worktree.

CPU tracing excludes allocator enumeration by default. Explicit memory census runs remain separate.
External samples use bottom-up stacks. Wall-time stack percentages are not CPU shares.
SQL VM counts, callback counts, preparation counts, and statements explain results. They are not performance gates.
Matched trials rotate variant order. RSS comparisons use separate processes through exit.
Every spike records a keep, reject, or unresolved decision and the remaining integration risk.

## WP0 / S0: reconcile the complete execution

**Question:** Which clauses and operations consume the 1,392-second deferred apply?

Add stable logical stage labels and inexpensive CPU spans around consumer execution, cleanup, and admission accounting.
Keep parent and child CPU separate. Attribute shared producer work separately instead of charging every consumer.
Report observed CPU, unattributed CPU, stage calls, source nodes, demanded keys, produced rows, and native boundary crossings.
Record private schema counts, completed producer counts, retained metadata, and SQLite page/cache counters at coarse boundaries.
Do not enumerate allocator zones in CPU runs.

Run a complete repository diagnostic with the corrected tracer across all ten additional rules.
Keep the complete repository context for global clauses.
Use 16 large files and 128 distributed files only to develop the tooling and select bounded reproductions.
Compare early and late consumers to detect growth with retained repository or scheduler state.

**Delivery:** a reproducible attribution report and reduced cases for the largest costs.
**Proof:** reconcile disjoint measured CPU with the parent interval. Preserve an explicit residual instead of inventing attribution.
**Decision:** rank WP1–WP5 from observed covered CPU before production promotion.

## WP1 / S1: admit demand and select access paths before execution

**Question:** Does the producer enter only for the required file, range, kind, and value demand?

Survey the largest captured selection statements and correlated consumers from S0.
Compare the current plan with one chosen anchor arm and early scope gates.
Read statistics once per plan batch from kfreq, file_kind_counts, and value summaries.
Do not count candidate postings to price every arm at execution time.
Use the gate outside the expensive join or in the recursive seed.
Retain regex coverage fallback and the existing matcher as the semantic authority.

Use a retained vocabulary with small admitted demand. Vary unrelated files and term counts independently.
Vary rare and common anchor kinds to force the correct physical choice to change.
Use empty demand, duplicate demand, colliding node IDs across files, and overlapping ranges.
Use a producer-entry counter or a failing unchosen arm to prove that rejected demand never enters.

**Spike scope:** one captured candidate family and one correlated fact consumer.
**Deliverable:** emitted SQL, chosen-arm evidence, exact row comparison, CPU/RSS, and input/output counts.
**Keep criterion:** setup-inclusive improvement without broader scope, unsafe absence, or per-node SQL calls.
**Production boundary:** shared access lowering and exact demand. No rule-specific native evaluator.

## WP2 / S2: replace repeated stage construction with fused narrow families

**Question:** Which physical cuts remove work across all consumers rather than repeat it?

Compare three complete schedules: current stages, a fused statement, and one narrow shared cut with two real consumers.
Include table construction, indexes, ANALYZE, both consumer reads, and retirement in the timed interval.
Retain the same predicates, NULL behavior, affinity, recursive termination, and row multiplicity.
Run one consumer, two identical-scope consumers, unequal scopes, empty demand, and a disabled expensive arm.
Reject a shared relation over the union of different scopes.
Measure source bytes and typed fields read, not only relation count.

**Spike scope:** one captured relational family and one captured candidate family.
**Deliverable:** physical producer boundaries, dependency order, row/schema parity, and construction-inclusive CPU/RSS.
**Keep criterion:** the fused path or shared cut wins after its complete preparation cost.
**Production boundary:** one selected physical shape per family, with measured materialization owners.

## WP3 / S3: move broad native work behind typed batch relations

**Question:** Can the native binding return final operator rows instead of exposing every intermediate node to SQL?

Use a generic structural operator already required by the captured family, such as field-qualified children or ownership.
Compare its SQL binding, scalar access, and a whole-demand virtual table.
Pass correlated scope IDs through sqlite3_vtab_in or an equivalent typed frame.
Do not send independent file and node IN lists.
Preload cardinality metadata from SQL statistics before xBestIndex. Do not query SQLite recursively during planning.
Replace the fixed 32-row range estimate in the selected binding with an estimate for its actual constraint shape.
Preserve duplicate request semantics, missing values, source versions, and per-file failures.

**Spike scope:** one generic operator, several files, and at least 256 requests.
**Deliverable:** SQL/native row parity, xFilter count, input visits, output rows, column callbacks, CPU/RSS, and row estimates.
**Keep criterion:** one xFilter per operator batch, output-only iteration, and lower setup-inclusive cost.
**GPU check:** the batch uses typed contiguous demand/output columns and a bounded number of broad operator launches.
**Production boundary:** the operator contract. No new rule interpreter or parallel implementation of rule semantics.

## WP4 / S4: batch publication and maintain statistics once

**Question:** How much cold publication CPU comes from transaction, trigger, and cache shape?

Retain the ast-index-spike positional FTS layout and complete exact value postings.
Compare per-file publication with a bounded multi-file transaction and set-based statistics maintenance.
Use the same input images, source versions, posted values, and kind positions.
Preserve deletion, replacement, pinned old versions, rollback, and retry.
Measure image construction separately and include it in the total.
Retain SQL ownership of statistics. Do not replace indexed values with an always-resident Rust index.
Compare modest cache budgets with cache hit, miss, write, and spill counters plus process RSS.

**Spike scope:** kind and value publication for several packet sizes, including 1 and 32 files.
**Deliverable:** exact posting/statistic comparisons, transaction counts, internal executions, CPU/RSS, and rollback results.
**Keep criterion:** less complete publication CPU with bounded memory and identical committed state.
**Production boundary:** publication and maintenance batches. No per-rule index rebuild.

## WP5 / S5: reuse physical plans and bound execution metadata

**Question:** Does work grow with changed decisions, live producers, or all historical consumers?

Measure lowering, composition, rendering, SQLite preparation, producer identity, schema setup, accounting, and cleanup separately.
Compare repeated unchanged decisions with changed statistics that retain the same chosen anchor.
A changed count alone must not force a different physical template.
Compare stable structural fragments with repeated SQL parsing and string rewriting.
Exercise empty successful producers, changed source versions, rollback, and repeated file edits.
Measure retained schemas, identity definitions, private aliases, and cleanup work as consumer count grows.
Prototype constant-time retained-byte accounting or owner postings only when the profile proves repeated scans are material.

**Spike scope:** the production compiler/replay path and one resident SQL owner across increasing consumer counts.
**Deliverable:** exact outputs, compile/preparation CPU, cache hit/miss reasons, metadata growth, and release behavior.
**Keep criterion:** bounded live metadata and less complete planning/setup CPU without stale reuse.
**Production boundary:** existing specialized phases and producer contracts. No general graph scheduler.

## WP6 / S6: compose the selected results

Select one result from each aspect after its evidence review. Remove losing prototype alternatives from any promoted production path.
Use the existing replay host for an integrated physical-family comparison over unchanged captured inputs.
Include publication/statistics, planning, producer construction, every consumer read, output collection, and cleanup where the capture represents them.
List omitted costs explicitly. A candidate replay is not an end-to-end linter benchmark.
Check interactions between demand fences, materialization cuts, native cardinalities, cache reuse, and publication batches.
Compare each selected component with the combined result. Do not add isolated speedup ratios.

**Deliverable:** a composed replay, complete row parity, resource measurements, and an implementation order ranked by covered cold CPU.
**Promotion ladder:** regression tests, complete original observations, unchanged/edit checks, then both full repository workloads.
A promoted delivery must demonstrate an aggregate improvement. A spike alone does not close recovery.

## Dependencies and completion record

S0 attribution starts first. Bounded S1–S5 prototypes can run while the full diagnostic proceeds.
S6 depends on the S0–S5 decisions. Production implementation follows this spike review.
All spikes live in tools or the isolated replay host. No production algorithm flags are introduced.

| Spike | Status | Evidence | Decision |
| --- | --- | --- | --- |
| S0 attribution and scaling | Pending | — | — |
| S1 exact demand and anchor choice | Pending | — | — |
| S2 fusion and materialization ownership | Pending | — | — |
| S3 native typed batch boundary | Pending | — | — |
| S4 publication and statistics | Pending | — | — |
| S5 planning and metadata reuse | Pending | — | — |
| S6 composed execution | Pending | — | — |

## Prior evidence and design constraints

The [direction document](../../Pulling-it-all-together/docs/rule-engine-direction/README.md) defines statistics, pushdown, and full measurement gates.
The sibling path is workspace-local. It is not a repository dependency.
The [previous recovery plan](sql-performance-recovery-workplan.md) and [journal](data-layer-performance-journal.md) retain branch evidence.

The prior-work indexes cite these relevant experiments:

- PR 2539 skinny fact families and shared fact preparation: lessons 7, 11, 20, 28, and 35.
- ast-index-spike materialization versus fused plans: DESIGN S155 and history topics on physical cuts.
- Scope-identical sharing versus union scope: history S149 and S177, lines 2509–2527 and 3221–3270.
- Counter-driven optimization traps: history S225, lines 6126–6156.
- Native row callbacks and honest planning estimates: lessons 4, 9, and 35.
- Query-independent index lifetime and publication: lessons 23–27.

These sources motivate experiments. Their isolated ratios are not expected gains for this branch.
The new evidence must explain this branch's remaining CPU before any architectural result is promoted.
