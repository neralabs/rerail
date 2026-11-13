# Execution workplan status — 2026-11-12

This status covers `jeremy/async-family-integration-comparison` through `fc3e8c4f9`.
It separates production repairs from offline experiments.
It does not qualify the current tips of sibling branches.
The most recent local integration included here is `20644b585`, through merge `d1dc128bb`.

## Objective and architecture

The objective is fast complete rule jobs, with correct findings and bounded incremental work.
The architecture uses narrow typed relations, early demands, and native batch operators behind relational contracts.
SQL composes those relations where appropriate. No SQL-hosted state machine is part of the selected architecture.
Statement counts and VM instructions remain diagnostics. Complete-job CPU, wall time, memory, and findings determine acceptance.
Caches use rule-defined keys and transformations, with ordinary fact access and reuse.

## Completed production work

| Area | Result | Evidence |
|---|---|---|
| Aggregate demands | Existence checks preserve demanded scope/name keys. The two global binding joins no longer produce billions of rows. | [Aggregate results](aggregate-demand-results.md) |
| Semantic producers | Async calls undergo structural and API screening before name resolution. Arguments remain demand-driven. | [Aggregate results](aggregate-demand-results.md) |
| Candidate planning | Candidate demands reach producers before broad projection. The candidate SQL publication round trip was removed. | [Early demands](early-candidate-demands.md), [retained demands](retained-semantic-demands.md) |
| Execution structures | Composed sessions retain reusable structures. Publication and frame-use updates consume bulk demand sets. | [Session repair](composed-session-publication-2026-11-10.md), journal |
| Maintenance joins | Changed-scope demands can precede walk probes. Empty required materialized inputs stop expansion. | Journal: affected joins and merge-regression repair |
| Memo replay | Consumers retain only required relations and statistics. Warm jobs reuse cached documents and file digests. | Journal: warm projection and observer-flush repair |
| Tape encoding | Buffer/cursor reuse and fewer native property reads reduce encoding overhead without changing the named-only format. | Journal: integration-regression repair |
| Statistics collection | Fallback samples obey an independent 8,192-row limit. Known statistics avoid source reads. | `be621c351`, journal |
| Join estimates | Bounded dense/sparse histograms survive the former 64-value cliff. Live counts precede priors. Literal priors use rows/NDV. | `fbc6814d5`, journal |
| Diagnostics | Built-in analysis and EXPLAIN expose late restrictions, reconstruction, cardinalities, frequencies, and estimate provenance. | [Structural analysis journal](async-family-integration-journal.md) |

The original complete four-rule job took 1,058.8 CPU seconds.
The first aggregate and semantic repairs reduced it to 14.3 seconds in that controlled comparison.
Later complete-job measurements use different tape, cache, and build states. They are not one continuous speedup series.
The prepared-tape reference's 163 ms measures matching, not complete corpus processing.

## Latest production qualification

The latest engine repair passes the unchanged O125 work ceilings and incremental/full output parity.
The targeted insertion check remains at 633 work units.

| Recorded join work | Before planner repair | After planner repair |
|---|---:|---:|
| Cold replay | 10,584,698 | 10,494,609 |
| Incremental replay | 7,251,458 | 7,143,870 |
| Full recomputation | 10,734,169 | 10,632,596 |

The repair removes about 97% of the sample-change regression.
Full recomputation remains 3,187 work units above the preceding repair, approximately 0.03%.
These counters omit some whole-plan work. They do not replace CPU measurements.

On 5,725 files, the latest paired execution CPU medians are 12.81 seconds cold and 110.2 ms warm.
Including rule loading, the medians are 12.88 seconds and 128.6 ms.
Cold samples range from 10.95 to 14.67 seconds on the shared host.
These measurements qualify the repair, but do not establish a stable speedup percentage.
The latest profile records 10.4 ms in local statistics sampling.

The requested 40–50 ms replanning target remains open.
A prior repaired replay measured 73.5 ms of replanning CPU. Later statistics results do not prove that target is met.
The journal retains the earlier failed candidates and the exact measurement boundaries.

## Recorded result: bounded statistics and probes

The [implemented spike](../../scripts/profiling/statistical_planning/README.md) and [results](../../scripts/profiling/statistical_planning/results.json) are committed at `fc3e8c4f9`.
The experiment uses 32 seeds, six real snapshots, three synthetic cases, and historical order controls.
Every enumerated supported join order matches native output. Nine focused tests pass.
The peak accounted statistics, probe cache, and planning scratch total is 286,976 bytes.
The job limits are 2 MiB, 512 sampled seed rows, and 50,000 probe work units.
Source facts and existing indexes are outside the statistics allowance.

Current captured orders are already optimal among the supported connected positive-join orders.
No production improvement is demonstrated on those inputs.
On historical controls, cold probes add 3.2% work without changing plans.
Probes with prior measurements from identical revisions reduce work by 17.3%, including probe work.
The prior execution needed to obtain those measurements is an additional acquisition cost.
Exact demanded-key counters support good cheap estimates. Conditional tuple samples add no measured benefit in this experiment.

Production planning remains unchanged by the spike.
Blanket collection and cold random probing are not accepted production changes.
The supported experiment excludes negation, general aggregates, scalar functions, and recursion.

## Remaining work, in priority order

1. **Resolve the remaining demonstrated estimate gaps.**
   The remaining cases involve `typeof_var` long-text fallbacks, decorated-definition walks, and parameter annotations.
   They need conditional selectivity or partial scalar-domain information beyond marginal histograms.
   The next implementation candidate is a bounded set of demanded-key counters, grouped by column mask during existing ingestion.
   Production acceptance needs a residual bad-plan capture, lower execution work, unchanged answers, and bounded collection cost.
   The spike's already-optimal current captures alone cannot establish that improvement.

2. **Finish selective replanning.**
   Dependency versions, signature reuse, empty-delta deferral, and separate delta pricing are implemented.
   Remaining searches and repricings still need attribution before another change.
   The acceptance target remains 40–50 ms on the pinned replay while preserving the 633-work check and other ceilings.
   No additional statistics pass is justified solely to reach that target.

3. **Reduce the remaining cold input cost.**
   Tape parsing, encoding, and fact construction dominate much of the complete cold job.
   Encoder repairs and bounded statistics removed demonstrated regressions, but the full audit remains far above the one-to-two-second product target.
   The next profile must separate source loading, parsing, encoding, fact admission, matching, and output.
   Warm reuse is already much cheaper. The prepared-tape reference does not measure these cold costs.

4. **Carry the repairs through integration.**
   This branch does not include or qualify every current sibling tip.
   The next integration merge needs the same work ceilings, positive findings, incremental/full parity, and complete-job measurements.
   The baseline must retain production allocation, actual worker counts, stable output paths, and verified observer/cache reuse.
   A reduction in execution-connection SQL does not prove store-side work is removed.

General cold random probing remains deferred.
Revision-keyed observed work is a possible later input, but it needs a demonstrated decision that repays probe and collection costs.
The current plan does not add an interpreter or per-rule SQL loops.
