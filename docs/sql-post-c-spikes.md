# Post-C SQL execution experiments

These experiments cover every bucket in the [975-submission survey](sql-execution-opportunities.md#post-c-survey-current-975-submissions).
The production engine and policies retain C. The experiments add no production algorithm flag.
The [reproduction guide](../scripts/profiling/post_c_spikes/README.md) identifies each executable prototype.

## Selected shapes

**Admission:** use the existing `requireNonemptyInput` precondition for the 28 remaining event clauses.
Complete empty evidence skips execution. Compilation still validates SQL, including queries whose input is empty.
Incomplete evidence remains a failure. No new admission mechanism is needed.

**Relational pipelines:** place four nonrecursive, single-consumer flow stages inside their consumer as materialized CTEs.
The stages are `string_values`, `expression_edges`, `direct_events`, and `signed_exceptions`.
Keep recursive, native, and multiply consumed stages at their existing boundaries.
Count consumers across option cases, defaults, and templates before fusion.
Keep one local materialization when a consumer references its private input more than once.

**Native bindings:** register the existing immutable sources as eponymous SQLite modules.
Registration and release use SQLite's module API rather than CREATE/DROP statements.
Resolve declarations before installing the authored-query authorizer.
Use executable empty sources for empty runtime domains. Compiler schemas deliberately reject execution.
Readers retain separate bindings, and open cursors retain their source ownership.

**Provenance:** read file versions from the corpus metadata already held by the capture caller.
Do not query SQL again to recover the same versions.
These four observer statements remain part of the baseline accounting.

**Candidate boundaries:** preserve required posting and interval indexes.
Only incidental decoder cuts can become private pipelines when their SQL expansion remains bounded.
Count repeated output arms as one reader within a packet. Count separate packets as separate readers.
Retain integer affinity, genuine decoder barriers, and shared owners.

The frozen candidate experiment also simplifies set operations with empty inputs.
UNION and EXCEPT identity rewrites retain DISTINCT, because their inputs can contain duplicates.
The rewrites preserve complete result multisets, but their SQLite stepping heap exceeds the baseline.
Do not promote this generated-SQL rewriter. Any later simplification belongs in the existing typed graph and needs another memory check.

**Physical storage:** prefer an immutable SQLite schema/catalog seed and private database clones.
The seed contains no source data, decoded properties, completion records, or rule results.
SQLite's backup API copies it into each domain. Connection settings and TEMP schema remain private.
Keep positional FTS, typed bulk publication, and SQLite's existing maintenance.
Include seed creation, catalog writes, backups, connection setup, and retirement in accounting.

Clearing and reusing populated stores is the alternative prototype.
It submits fewer statements but causes more FTS deletion work as publication grows.
Private clones preserve the existing domain isolation and avoid that reset protocol.
This choice addresses repeated schema and catalog construction. It does not establish shared source publication across distinct domains.

**Compiler inference:** batch independent cold inference with one bounded SQLite CTAS per dependency frontier.
Use the production schema adapter, authorizer, and typed schema-cache keys.
Temporary output names identify each stage and column; SQLite still derives every column type.
Flush pending inference before a consumer reads one of its outputs.
Limit batches by SQL bytes, column count, and join count. Preserve a bounded single-query fallback.
Do not execute authored functions during inference.

## Regression findings

The first candidate heuristic treated required indexes as optional cuts.
The full Rust suite caught lost interval bounds, excessive growth with nesting, and a missing explicitly owned table.
The narrowed heuristic preserves those boundaries instead of weakening the tests.

The first eponymous prototype omitted empty syntax bindings.
A regression with empty, repeated-empty, and populated domains caught that omission.
Using compiler schemas for runtime emptiness also failed, as designed. Executable empty sources resolve both cases.

The selector test now crosses three output packets with both selectors.
Its previous fixture left one selector private to one packet, which the new planner correctly keeps inline.
The stronger fixture passes on both C and the corrected prototype.

## Integration boundary

The runtime patch combines native bindings, capture provenance, and the candidate cut rule.
The isolated policy copy adds admission and the four private flow pipelines.
Their complete-job capture is the combined runtime measurement.

Candidate rewrites, storage lifetimes, and compiler inference also have standalone component comparisons.
Their counts cannot be added to the combined job count without integration and another complete capture.
Storage uses Python's SQLite build; the runtime and inference host use the project's Rust SQLite build.
Keep that difference visible when interpreting CPU results.

Integrate the selected policy changes through the generator, then regenerate and check the pinned corpus.
Integrate compiler batching into the existing compiler pass without a second execution mode.
Give the storage seed an explicit grammar/version identity, bounded lifetime, and private mutable domains.
Test concurrent readers, edits, removals, cancellation, and failure before promoting storage ownership changes.
Measure source publication separately before claiming that repeated facts are constructed once across domains.

The final acceptance limit remains 100 engine-submitted statements for the complete cold job.
SQLite internal executions remain separately reported. None of these experiments changes that limit.

## Measurements

The [checked evidence](../fixtures/host/data-layer-port/profiling/post-c-spikes-20261024.json) retains trials, artifact hashes, boundaries, and validation.

| Complete fixed guard | C | Selected runtime and policy prototype |
| --- | ---: | ---: |
| Engine submissions | 975 | 718 |
| SQLite internal executions | 106 | 100 |
| Median CPU | 585.845 ms | 579.437 ms |
| CPU range, five fresh processes | 573.313–597.824 ms | 554.644–606.254 ms |
| Median process peak RSS | 299,696 KiB | 297,616 KiB |
| RSS range | 292,832–300,528 KiB | 287,232–299,824 KiB |

CPU and RSS ranges overlap. The combined prototype removes 257 submissions without a demonstrated CPU or memory gain.
Its 718 submissions remain 618 above the mandatory final limit.
The complete count includes all setup, transactions, publication, preparation, evaluation, and retirement.

| Separate component | Before | Selected prototype | Interpretation |
| --- | ---: | ---: | --- |
| Original cold inference programs | 53 | 31 | Equal column/type maps; CPU ranges overlap. |
| Cold inference after private flow fusion | 56 | 35 | Fusion changes schema-cache reuse; use this count for subsequent integration. |
| Storage and publication | 220 | 127 | Includes four common verification queries and seed/clone maintenance. |
| Frozen candidate set folding | 272 | 252 | Equal output multisets, higher stepping heap; do not promote the string rewriter. |

Storage stress repeats each captured source 128 times at distinct paths.
Fresh and seeded stores use median CPU of 63.542 and 63.242 ms, with overlapping ranges.
Cleared-store reuse needs 71.157 ms and generates 29,791 internal executions, versus 10,892 for seeded stores.
Choose seeds for simpler isolation and fewer submissions than fresh stores.
This component uses Python SQLite and excludes parsing, tape construction, and linter execution.

The corrected runtime passes 1,604 Rust tests and 324 native-facing tests.
It also passes 77 state observations and 120 positive/clean observations across 60 rules.
These are bounded spike checks. They do not establish full-repository recovery.
