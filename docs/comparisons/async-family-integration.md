# Async family: reference versus integration

This is the baseline comparison. The subsequent [demand repair and measured results](aggregate-demand-results.md) implement its recommendations.

Keep the integration compiler. Connect it to the reference's demand-driven semantic facts, and fix aggregate demand propagation before further scheduling work.
The reference is a useful executable specification and performance gate. It is not yet the generic compiler's output.

The measured integration expands a binding-existence check into **2.68 billion join outputs** across two equivalent-looking source and annotation plans.
Those joins consume 430 CPU seconds. Loading and indexing inputs for their consumer consumes another 181 seconds.
This is the principal measured structural gap. Changing statement packing alone cannot remove it.

## Reproduction of the quoted result

All arms use the same 5,725 Python files from data-layer commit `fce4dc7208c0b892c11a549c8be43fc285580d6b`.
The manifest verifies 95,697,853 source bytes. The corpus contains 19,007,630 syntax nodes.

| Measurement | Quoted historical result | Fresh reference, median of three |
|---|---:|---:|
| Indexed native matching CPU | 1,342 ms | 643.561 ms |
| Independent reference CPU | 336 ms | 166.865 ms |
| Generated reference-plan CPU | 350 ms | 163.056 ms |
| Semantic requests | 65,302 → 12 | 12 |
| Generated SQL VM instructions | 2,030,306 → 1,886 | 1,886 |
| Generated SQL statements | 2 | 2 |

The fresh run reproduces the final structural counters exactly.
It does not rerun the historical pre-optimization plan represented by the left sides of the arrows.
The historical and fresh builds have different code-generation settings and dependencies. The host was not isolated.
An earlier fresh run measured 266/264 ms for reference/generated execution. Timing variation is substantial; counters are stable.
Do not interpret the lower fresh timings as a measured compiler improvement.

“Generated SQL execution CPU” includes native fact production and semantic work for the generated plan.
It is not the SQLite instruction loop alone. Parse/tape construction and SQL preparation are separate.
The reference still scans 691,328 calls and finds 65,399 async calls.
Its import screen rejects all 65,292 primary semantic candidates on this corpus.
Dependent receiver cases require 12 secondary semantic requests across five scopes.
Argument extraction receives no demands, so it produces zero argument rows.

The current per-file native oracle takes 3,221.845 ms, including its per-file index and selection work.
It visits 7,484,956 candidate roots, versus 269,128 for the retained corpus-indexed oracle.
These are different native baselines; they must not share an ambiguous “native” label.

The retained indexed oracle also costs 40.951 seconds to construct its index and 29.040 seconds for first selection.
Repeat selection costs 0.300 and 0.318 ms. The copied test adapter previously rebuilt pointer-keyed plans each run, defeating that cache.
This branch fixes their lifetime. The earlier, defective measurement remains in `reference-before-adapter-fix.json`.

Parse/tape construction costs 9.737 seconds CPU. The first generated phase-sum job cost is 9.900 seconds; current native is 12.962 seconds.
These sums exclude source loading, suppressions, and payload generation. They are not complete CLI timings.
Matching CPU alone must not be presented as whole-job CPU.

## Actual integration execution

The measured integration is commit **`49035cc9098d3cafb19a1e4c33bc45ab796dce28`**, the current committed integration when this comparison began.
The active descendant is `wolfeel` / `jeremybarnes/final-rerail-shape`; the literal `Pulling-it-all-together` directory is stale.
The four authored `.rr` rules run together through production `corpus::run_with_options`, with module includes preserved.
The input list explicitly includes all 5,725 files, including hidden paths.
This avoids the separate-policy CLI loop and gives the compiler an opportunity to share their libraries.

| Measurement | Unprofiled combined `.rr` job | Separate profiled job |
|---|---:|---:|
| Process CPU | 1,058.754 s | 863.727 s |
| Wall time | 2,053.364 s | 1,386.382 s |
| Local-phase CPU | 998.581 s | 823.492 s |
| Cross-file CPU | 32.359 s | 21.601 s |
| SQL executions | 88,545 | 141,771 |
| SQL VM instructions | Not instrumented | 58,047,536 |
| Chunks | 234 | 391 |
| Peak RSS | 11.66 GB | 11.57 GB |
| Embedded-source requests | 538,412 | 538,412 |
| Findings | 0 | 0 |

The 100-execution budget fails in both production runs.
Preparation comes from the store; the unprofiled initial stats/preparation phase costs only 0.493 seconds CPU.
Local processing includes further planning, guest parsing, and derivation. Its total is not a pure matching measurement.
Per-file and cross-file result memoization are disabled. The tape cache is private to the comparison and shared across stages.
The profiled run follows the timed run. Cache state, host load, and chunk partition differ; its shorter time is not an optimization result.

The reference consumes prepared tapes; the `.rr` job includes corpus processing and output writing.
Therefore 1,058 seconds versus 0.163 seconds is **not** a controlled end-to-end speed ratio.
The phase breakdown and billions of unnecessary join outputs establish a structural problem without that comparison.

During measurement, the sibling advanced through `6ae30c48a` to `d8fad367b`.
The added code reuses collection leases, speeds source-row access, caches combined programs, and persists their file/cross split.
They may reduce constants in these paths. These are not measurements of that newer head.
Its new O143 note reports 8.7 seconds warm and 61.3 seconds cold for 534 programs over 486 files.
That is a different workload; the warm result uses per-file memo replay, which this comparison disables.
The O143 annotation join-order fix, `5e335eb4d`, already precedes our measured base. It does not fix the helper expansion identified here.
The dominant aggregate expansion does not depend on repeated compilation or lock-file overhead.

## The precise expansion

The source is `docs/rule-syntax/lib/python/pyflakes.rr:150–153`:

```text
unbound_ref(n, x) :-
    load_ref(n, x), look(n, x, s), nearest(n, s, scope),
    (kind(s, "module") ; kind(s, "class_definition")),
    ...,
    count : { inside(s, p, lfscope), bound_in(p, x) } = 0,
    !annotated_on_chain(n, x).
```

The aggregate lowering produces this independent helper:

```text
rr_agg59(p, s, x) :- rr_inside_ks7(s, p), bound_in(p, x).
```

Instead of asking whether an outer scope binds the demanded name, it enumerates bound names for every relevant ancestor pair.
The helper also loses the consumer constraint that `s` is a selected scope; it starts from the general `inside` relation.
The profiled plan reads 24,793,868 ancestor pairs and emits **1,342,302,187 bindings** from the name join.
The annotation-world copy emits the same number. These are cumulative join outputs across chunks, before final relation deduplication.
They are not a claim that 2.68 billion unique rows reside in memory simultaneously.

| Exclusive CPU hotspot | CPU |
|---|---:|
| `rr_agg59`: native joins | 217.614 s |
| `rr_in_annotation_1_rr_agg59`: native joins | 212.441 s |
| `unbound_ref`: input loading and index construction | 181.202 s |

Together these consume 611.257 seconds, about 71% of the profiled job CPU.
Planning these strata costs 0.006, 0.008, and 0.045 seconds respectively.
The expensive work occurs in the native relational engine, underneath SQL orchestration.
SQLite VM counters do not account for those native joins. Moving execution out of SQLite did not remove their amplification.

There is also a poor consumer join order.
One module branch performs aggregate checks and other work on hundreds of thousands of bindings.
Only its last `nearest` membership test reduces 661,051 bindings to 22.
A class branch reaches that test with 725,605 bindings, reduces them to 96, then rejects all 96 by scope kind.
The profile preserves the exact plans and step counts in `hotspot-plans.txt` and `hotspots.json`.

### Required replacement

Preserve the consumer's bound variables when lowering the aggregate. Construct demands only after cheap structural guards.
For this count-equals-zero case, the intended shape is an anti-existence probe:

```sql
SELECT d.n, d.x
FROM demanded_loads AS d
WHERE NOT EXISTS (
    SELECT 1
    FROM scope_ancestors AS a
    JOIN bound_names AS b ON b.scope = a.outer_scope
    WHERE a.scope = d.scope AND b.name = d.x
);
```

Here `demanded_loads` already satisfies the relevant scope kind, nearest-scope, load-context, and consumer constraints.
This is schematic SQL for the binding test, not a complete replacement for Python name resolution.
It requires indexes/access paths keyed by scope and by `(scope, name)`.
A batched native resolver can implement the same demand contract when resolution needs sequential Python semantics.
The output is keyed by the requested reference or `(scope, name)`; it must not enumerate every possible binding.

For general nonzero counts, restrict aggregation to demanded grouping keys before materialization.
For existence, stop at the first witness. Preserve source/annotation world identity until equivalence is established.
Do not merge those worlds merely because this corpus produces identical row counts.

The rule family should also push target-API and async-context demands ahead of general resolution.
The reference provides an executable model: call candidates → conservative import screen → batched semantic facts → candidate SQL → demanded arguments → final SQL.
A negative screen is valid only if it cannot reject a real match. Unknown cases must reach semantic resolution.

## Correctness: the negative corpus is insufficient

Both engines and Ruff 0.15.7 report zero findings on all 5,725 files.
Both reference and authored `.rr` execution pass the 24-file positive fixture with 120 findings.
However, the exported edge suite exposes differences:

| Against isolated Ruff, four async rules | Missing calls | Wrong report location |
|---|---:|---:|
| Native/reference | 4 | 0 |
| Authored `.rr` | 7 | 1 |

The reference misses calls around comments/parentheses and a Unicode spelling of `open`.
The `.rr` path correctly adds three of those calls.
It misses other calls involving parenthesized or starred unpacking, an annotated assignment, and a parenthesized `with` target.
Both miss the Unicode case. One parenthesized multiline HTTP call gets the wrong report line in `.rr`.
Raw comparison with the reference gives seven missing keys and four extras; one pair is the same call at different lines.

Ruff is an independent comparator, not an automatic replacement for the project's declared semantics.
Record each intended behavior explicitly. Do not force all reference quirks into the compiler to make a parity test green.
The sources and results are preserved, including the failing edge gate.

The separate JSON-to-packed-matcher admission path accepts only ASYNC210 and ASYNC222.
It refuses ASYNC221 (`arguments binds`) and ASYNC230 (`resolves.binding`).
The actual integrated lowering returns two clauses from four offered clauses.
The accounting test passes because refusals are recorded; complete-family qualification is **false**.
This does not mean the authored `.rr` compiler cannot express those two rules.

## Opportunities to preserve from both branches

| Mechanism | Current state | Decision |
|---|---|---|
| Reference typed facts, import screen, selective arguments | Copied into integration; selective consumers remain in reference tests | Connect to production demand planning |
| Posting-based Python call producer | Production implementation exists | Keep and combine with demand pruning |
| Independent SQL/reference plan and counters | Copied, now measured beside actual `.rr` execution | Keep as oracle and shape gate |
| General anti-join, range, aggregate compiler | Integration supports more than the small reference compiler | Keep; repair its demand propagation |
| Common-label factoring, capture-delta memo, shared scopes, hardware counters | Already integrated | Avoid duplicate ports |
| Old continuation/cohort scheduler | Recorded full-workload CPU was 1.788× native | Preserve evidence; do not merge scheduler |
| Negative resolution cache and import-chain reuse from `c6ff81104` | Small unported candidates | Adapt to current semantics; measure later |

The producer experiment compares exact async call IDs for every file.
The integration's posting-based producer takes a median **49.040 ms**, versus **134.972 ms** for the reference structural scan.
It also eagerly produces **1,005,452 argument rows**, while the selected reference demands need zero.
Core-fact construction costs another **1,057.690 ms** and is excluded from those producer times.
Thus use postings when core facts are available, but separate argument extraction from basic call enumeration.
This is a measured opportunity from the integration, not a reason to replace it with the older implementation.

The negative-resolution cache must respect current load-pair and global-binding semantics.
The earlier suffix guard only avoids building an import chain: current code already rejects a suffix mismatch before scope resolution.
These candidates have no measured benefit on the current integration yet.

A separate source audit found that shared `rr_embed_req` loses consumer provenance before relation-level liveness pruning.
If one live consumer reads guests, requests from an unused consumer can survive through the shared relation.
This is a concrete design risk, but its contribution to this run's 538,412 requests has not been isolated.
The proposed minimal test adds an unused unrestricted `parsed` rule beside a live restricted rule.
The added rule must not parse unrelated prose. That regression fixture remains unexecuted.
Preserve request ownership until liveness closure is complete, then merge equivalent requests.

## What to do with this branch

Use `jeremy/async-family-integration-comparison` as the reviewable evidence and harness branch.
It starts from the integration pin and changes tests, measurement tools, and documentation; it does not change the production engine.
Port these comparison additions to the active integration, resolving test-layout differences there.
Keep the old experiment branches as evidence. Do not merge their compiler and scheduler wholesale.

The next implementation should have this bounded acceptance sequence:

1. Add a parameterized aggregate/anti-existence fixture with many names and few demanded references.
   Count emitted join bindings, not only SQL statements. Growth must follow demanded keys and relevant ancestor depth.
2. Preserve aggregate demand keys and place nearest/kind/load guards before expensive probes.
   Remove the unconditional ancestor-by-all-names helper. Check source and annotation worlds separately.
3. Connect the shared call/import/semantic producers to the authored async family.
   Reuse posting access and defer arguments until SQL supplies surviving call IDs.
4. Run the fixed positive and semantic edge cases, then the exact full corpus once without profiling and once with profiling.
   Require all four rules, the hard 100-execution budget, and disappearance of the billion-output helpers.
5. Compare actual production matching CPU with the reference's existing 1.25× execution gate.
   Also measure whole-job CPU/wall with the same cache boundary. Report failures instead of substituting the specialized reference compiler.

The 12-request and 1,886-VM results are diagnostic targets for this corpus, not universal language limits.
A general compiler may use different operators. It must demonstrate comparable demanded work and preserve the agreed semantics.

See the [journal](async-family-integration-journal.md) and [reproduction package](../../benchmark/async-family-integration/README.md) for commands, raw results, and provenance.
