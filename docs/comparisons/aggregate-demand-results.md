# Aggregate and semantic demand repair

The full four-rule job falls from **1,058.8 to 14.3 CPU seconds**, about **74× faster**.
The aggregate repair and the selective semantic producers both contribute.
The 100-execution budget remains unmet. Statement reuse is a separate, documented follow-up.

The implementation is on `jeremy/async-family-integration-comparison`.
Read the [design](aggregate-demand-design.md) and [work journal](async-family-integration-journal.md).
The [evidence](../../benchmark/async-family-integration/demand-repair/comparison.json) and [provenance](../../benchmark/async-family-integration/demand-repair/manifest.json) accompany this report.

## Complete job comparison

Both unprofiled jobs process the same 5,725 files, 95,697,853 source bytes, and 19,007,630 syntax nodes.
Both read prepared programs from the store. File and cross-file result memos are disabled.
Each new measurement starts with a private, consistent backup of the baseline tape cache.

| Unprofiled measurement | Baseline | Repaired engine and rules |
|---|---:|---:|
| Process CPU | 1,058.754 s | 14.315 s |
| Wall time | 2,053.364 s | 37.055 s |
| SQL executions | 88,545 | 10,414 |
| Chunks | 234 | 73 |
| Peak RSS | 11,658 MB | 647 MB |
| Findings | 0 | 0 |

The earlier repaired run took 13.071 CPU seconds and 13.725 wall seconds, with 72 chunks and 10,272 executions.
Both samples are preserved; the table reports the final binary.
These are individual runs on a shared host. CPU includes worker threads; wall time depends on host contention.
Chunk counts are adaptive and differ between runs. The statement reduction does not establish that execution structures are now reusable.

The specialized reference's 163 ms starts from prepared tapes and measures its matching pipeline.
It is not a complete-job timing comparable to the 14.3-second result above.

## Proof that the aggregate itself is repaired

A separate diagnostic runs the **unchanged original policies** through the repaired compiler.
It completes with the same findings, at 301.200 CPU seconds and 600.802 wall seconds.
It executes 109,166 statements in 291 chunks. This diagnostic includes cold program preparation.
It is separate from the unprofiled comparison above.

The two former binding-existence plans each emitted 1,342,302,187 join outputs.
Their repaired plans each emit **one** matching witness.
Across the entire job, each plan reads 19,820 demanded keys and reaches only 54 ancestor candidates before probing bindings.
The [raw before/after plans](../../benchmark/async-family-integration/demand-repair/outer-binding-plans.txt) preserve these counters.

The resulting order is:

```text
rr_demand_agg_rr_agg59(scope, name)  -> 19,820 demanded keys
rr_inside_ks7(scope, outer_scope)  ->     54 ancestor candidates
bound_in(outer_scope, name)       ->      1 matching witness
```

Scope IDs retain document and world identity. The source and annotation derivations keep their corresponding binding inputs.

| Exclusive hotspot CPU | Baseline profile | Repaired aggregate profile |
|---|---:|---:|
| Both outer-binding joins | 430.055 s | 0.0057 s |
| Consumer input loading and indexing | 181.202 s | 0.0151 s |

The unchanged policies still perform broad name analysis and guest processing.
For example, their new profile spends 39.4 seconds directly in guest processing and 27.3 seconds in two ancestor propagations.
This is why repairing aggregates alone does not reach the final performance result.

## Selective semantic producers

The updated policies retain the posting-based call producer.
They select async call sites, screen target APIs, and then request semantic facts.
Arguments are requested only for qualified spawn calls. Receiver origins follow the `.open` attribute filter.

The complete repaired profile records:

| Measurement | Result |
|---|---:|
| Process CPU | 15.753 s |
| Wall time | 18.802 s |
| SQL executions | 10,272 |
| SQL VM instructions | 2,283,088 |
| Actual semantic node projections | 10 |
| Scope tables built | 5 |
| Demanded argument rows | 0 |

The corpus contains no relevant spawn calls, so it needs no argument rows.
The positive fixtures separately exercise argument extraction and all four rule labels.
The producer also screens secondary constructor requests; the reference projects 12 semantic nodes on this corpus.

In the unprofiled run, local rule execution costs 0.976 CPU seconds.
Worker parsing costs 2.676 seconds, and batch construction costs 7.750 seconds.
The remaining complete-job cost is now mainly preparing syntax facts for the batches.
The detailed profile attributes 6.018 seconds directly to batch projection and interning.

## Validation

The saved red tests preceded aggregate and semantic integration.
A subsequent native trace exposed a scan of 201 bindings before three demand keys.
That finding led to the demand-first physical planning change.
The production positive test also caught the incorrect cross-file classification of the new native operators.

All 17 final targeted tests pass:

- Zero-count anti-existence, demanded count and sum groups, duplicate witnesses, and empty groups.
- Explicit world keys, multiple consumers, safe recursive consumers, and refusal of recursive-only demand sources.
- At most three rows at every native helper step in the synthetic work-growth test.
- Load and nearest-scope guards in the actual pyflakes helper.
- Posting parity, conservative import screening, selective scopes, cached semantic facts, and demanded arguments.
- Prepared-program metadata, document-local execution, and refusal of cross-file target sets as local work.
- Distinct helper contracts with colliding names, plus sharing of identical contracts.
  The adversarial runtime fixture now returns both expected findings.
- All 120 expected findings in the 24-document positive fixture.

The 74 non-ignored RR regression tests pass, as do eight file-local tests.
One memo test first ran with memos disabled by the benchmark environment; it passes with normal memo settings.
Two RR tests and one file-local device test remain ignored by their existing definitions.

All four edge fixtures match **178 Ruff 0.15.7 findings**, with no missing or extra keys.
The old reference parity report still shows four additional findings, matching its previously documented misses.
Its expected files remain unchanged. The comparison script checks the separately preserved Ruff oracle.

The unchanged legacy policies retain their exact previous findings on every edge fixture.
Their 120-positive gate also passes. This checks that aggregate optimization preserves their existing semantics.

## Remaining execution work

The old profiled run had 45,004 DROP, 43,869 CREATE, and 41,914 INSERT executions.
The repaired profile still has 2,022 DROP, 2,031 CREATE, 1,382 INSERT, and 4,837 other executions.
Prepared queries do not remove these operations. The [design](aggregate-demand-design.md#deferred-execution-structure-work) records the follow-up.

There is also measured repeated request handling.
Publishing `async_reference` costs 784,512 VM instructions.
Its four identical demand reads cost 785,520 instructions; another read costs 196,308.
Together, these consume **77.4% of the repaired job's SQL VM instructions**.

The next execution change should retain native demand relations without publishing and rereading them through temporary tables.
Qualification consumers with the same candidate scope can share one target-union batch, then filter results by each original target set.
This must preserve policy scope and world identity. Merely packing unrelated rules together is not sufficient.
Reusable schemas, fewer publication boundaries, and skipping known-empty operator inputs must be measured against the existing 100-execution gate.
