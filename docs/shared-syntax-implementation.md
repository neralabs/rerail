# Shared syntax facts and direct tree execution

The [requirement](shared-syntax-requirement.md) puts preparation before faster execution algorithms.
SQLite remains the rule execution engine.
The new syntax source supplies structural facts from the existing parser tape.

## Preparation

A document supplies shared immutable node metadata, field links, kind postings, parent postings, and owner indexes.
The document cache identifies its exact source version, file context, and grammar.
An evaluation pool shares those facts across clauses and repository consumers.
Both caches have residency limits.
Failed or incomplete preparation never enters either cache.

The producer declares its file eligibility input, fields, and columns.
The input gate preserves each source group and the rule scope.
Column demand precedes extraction, so unused field text never enters SQL rows.
Requested source text remains lazy.
The syntax grammar comes from the eligibility input.

Node identities are integer parser ordinals within a file version.
Fields return those same identities.
SQL consumers retain file qualification in every join.
No textual identities or runtime table fingerprints mediate sharing.

Node, edge, and owner virtual tables share the metadata.
Equality and `IN` use postings or direct identity lookup.
Start ranges use binary search.
An upper end bound also narrows the candidate start range.
SQLite rechecks all predicates, including the original end bound, affinity conversions, nulls, and collations.
Owner indexes use the authored kind and nearest or outermost selection.
The native engine contains no data-layer detector decisions.

## Execution

The quality policy retains its three source groups.
Each group uses the shared syntax source instead of matcher captures and reconstructed node, edge, and ownership tables.
Frequently reused derived relations retain ordinary SQLite materialization and indexes.
Name propagation and annotation resolution remain in SQL.

The direct descendant adapter accepts roots over the syntax relation.
It enumerates visible nodes inside the parser subtree and writes bounded result batches.
It retains SQLite indexes for SQL consumers.
It avoids graph-key normalization, temporary edge conversion, and reconstruction of another forest.
The existing directed and strict forest adapters remain available for authored graphs.

SQL normalization also retains indexed access for typed identities.
Unary plus removes declared affinity from temporary identity columns without changing their values.
Without this change, integer inputs force numeric comparisons against the untyped key catalog.
SQLite then uses only the file prefix of the identity index and scans keys repeatedly.
The normalization control exposed this problem under the existing work allowance.
Public syntax metadata and descendant identities retain the evidence backend's neutral affinity.
Their values remain integers.
This contract also preserves full identity-index access in downstream materialized SQL facts.
Constrained native tests and a bounded SQLite test cover both join directions.

Native traversal and lookup use bounded work units.
SQL execution, regular expressions, copied source bytes, and native work share the execution allowance.
Cancellation remains active throughout preparation and execution.

## Controls

The generator defaults to shared syntax and direct descendants.
The `--evidence-syntax` control reproduces the preceding policy.
The `--normalized-syntax-graph` control uses shared facts but retains SQL graph normalization.
This control separates preparation savings from the direct descendant adapter.
The `--baseline-relations` control retains the older eager-text recursive SQL policy.

The bounded profiler records syntax preparation, owner preparation, SQL stages, and relation release.
SQL stage traces also record the remaining work allowance before and after execution.
Failed pilots are excluded from performance comparisons.

## Contracts checked during the port

Root selection retains field demand until all descendant requests are declared.
JSON object keys explicitly convert integer node identities to text.
Function-body scans apply both start and end bounds before enumeration.
Evidence line numbers remain zero-based.
Suppression uses the existing directive reader and removes findings without removing syntax facts.
Metadata differential tests compare native fields with the preceding evidence projections.

## Measurements

The [recorded evidence](../fixtures/host/data-layer-port/profiling/shared-syntax-metrics.json) contains three matched rounds on five pinned files, totaling 1,026,452 bytes.
All variants use the same native binary, workload, allocator, and measurement settings.
Tracing and external sampling remain off during these rounds.

| Median | Previous implementation | Shared preparation, normalized graph | Shared preparation, direct descendants |
| --- | ---: | ---: | ---: |
| Cold process CPU | 12.619 s | 3.642 s | 3.192 s |
| Edit process CPU | 3.268 s | 1.672 s | 1.284 s |
| Peak process RSS | 936 MiB | 485 MiB | 537 MiB |
| Policy compilation CPU | 431 ms | 77 ms | 83 ms |

Shared preparation and integer identities reduce cold CPU by 71.1 percent before the direct adapter.
The direct adapter saves another 12.4 percent relative to that control.
The complete change reduces cold CPU by 74.7 percent, edit CPU by 60.7 percent, and peak RSS by 42.7 percent.
Direct descendants use 10.8 percent more peak RSS than the normalized control.
Their path-qualified result rows and two SQL indexes cost more memory than compact graph pairs.
The direct adapter remains the default because it reduces cold and edit CPU.
The generator retains both alternatives for future comparisons.

A separate diagnostic trace records exactly five syntax preparations and fifteen owner-index preparations.
It records no node, edge, or ownership table reconstruction and no graph-key normalization in the candidate.
Syntax preparation consumes 27 ms of worker CPU; owner preparation consumes 8 ms.
Input extraction falls from 1,865 ms to 59 ms in these diagnostic traces.
These traces measure different work from the unsampled timing rounds and remain separate evidence.

Two one-second macOS `sample` captures supply the external bottom-up profile.
SQLite execution stacks account for 94.9 percent of active native samples, including callbacks into native sources.
Record comparison, memory movement, virtual-table columns, and SQL VM execution remain prominent self costs.
The next substantial costs are SQL materialization and descendant-result indexing.
The candidate trace attributes 1.536 s of worker CPU to SQL materialization and 397 ms to descendant indexes.
`self_stores`, `scoped_nodes`, `arguments`, and `py_names` remain useful targets for query-plan analysis.
The evidence does not support replacing those detector decisions with native implementations.

The host is shared and has variable scheduling load.
CPU is the primary comparison; wall time is not a dedicated-host throughput claim.
The native engine uses optimization level 1, 256 code-generation units, and thin LTO; dependencies use optimization level 3.
All comparisons use the same system-malloc shim.
Full-repository scalability and the production allocator require separate measurements.
Metadata cache sizes use conservative reservations rather than allocation measurements.

## Validation

All 286 focused native tests and 52 Python tests pass.
The pinned upstream corpus passes all 200 cases: 135 positive and 65 clean.
Production comparisons pass 219 cases: 130 positive and 89 clean.
All 57 edit, restoration, and removal transitions pass.
Diagnostic comparisons retain path, span, code, message, and multiplicity.
Workspace comparisons also retain complete results under the same policy.

Type checking, repository gates, reference validation, formatting, and Clippy pass.
Clippy covers both production and profiling configurations.
Two constrained regressions cover indexed identity normalization and downstream descendant joins.
Both failures were reproduced before their respective fixes without increasing work allowances.

## Reproduction

Use the pinned data-layer checkout at `2b1504adc4291daaf080b2e932962b134da1a69a`.
Generate separate controls with these commands:

```sh
python3 scripts/dsl/data_layer_generate_batch_six.py --source "$DATA_LAYER" --repo . --output .rerail/shared-control --evidence-syntax
python3 scripts/dsl/data_layer_generate_batch_six.py --source "$DATA_LAYER" --repo . --output .rerail/normalized-control --normalized-syntax-graph
```

Run each variant three times with the same compiled binary and recording:

```sh
RERAIL_ENGINE_PATH="$NATIVE_BINARY" python3 scripts/profiling/run.py \
  --trace off --deadline 115 --allocator system-malloc-shim \
  --recording .rerail/profiles/batched-sql-integration/profile-five/selected-recording.json \
  --policies packs/data-layer/candidates/rules --output .rerail/profile-shared-1
```

Replace the policy directory with each control directory for the other variants.
The evidence records the recording hash, native binary hash, policy digests, build settings, and artifact locations.
For diagnostics, enable `--trace on --cold-only`.
For external sampling, add `--external sample --sample-seconds 1 --sample-count 3` and the binary's `--sample-module` name.
Exclude sampled and traced runs from timing medians.
Every build, test, and benchmark uses a deadline below two minutes.
