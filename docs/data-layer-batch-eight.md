# Data-layer batch eight

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
Status: ten authored candidates. Focused source parity passes. Repository acceptance is in progress.

| Check | Main contract |
| --- | --- |
| `eager_resolve` | Raw source patterns, eager helpers, phase propagation and external boundary definitions |
| `lazy_architecture` | All 18 diagnostic families, scoped aliases and function reachability |
| `pure_lazy_runtime_purity` | Pure function coloring, imported helpers, runtime handles, global state and exports |
| `cached_pure_lazy_allowlist` | Cache access contracts, memoization policies, exact fingerprints and protected runtime signatures |
| `lazy_sql_surface` | Lazy terminal calls, compile context access, execution authority and result annotations |
| `plan_helpers_allowlist` | Signed callsite names, imports, ordered aliases and forbidden helper access |
| `legacy_callback_quarantine` | Callback shapes, owner exemptions and production path exclusions |
| `readthrough_authority_architecture` | Read-through authority ownership and transaction entry paths |
| `authentication_plan_purity` | Authentication state, reachable helpers and compatibility forwarding |
| `lazy_adapter_ownership` | Shared lazy bodies, direct adapters and legacy ownership |

The unchanged upstream suite passes all 456 tests.
The [recording](../fixtures/host/data-layer-port/batch-08-recording.json) contains 308 eligible observations: 195 positive and 113 clean.
It also records 12 retained calls and 209 tests without a captured detector call.
These counts are distinct from translated pytest cases.

Supplemental expectations come from unchanged upstream analyzers.
They include original lazy-architecture fact-test source inputs and positive probes for every LA diagnostic family.
The source pin has no dedicated lazy-SQL-surface test module.
Its positive and clean evidence comes from these supplemental oracle probes.

The native gate requires positives and clean cases for every rule.
Always-clean controls must miss every expected finding on their selected positive input.
The edit harness checks clean, positive, restored, repeated-positive and removed-file states.
Removal results must match a fresh empty workspace.

## Canonical Python facts

The cached policy compares exact CPython AST dumps, rather than source text or structural-equality hashes.
The selected implementation uses a generic `pythonAst` producer stage in the integrated companion engine.
SQL selects a complete batch of source/node IDs.
One isolated CPython 3.13 process handles each nonempty producer wave.
A document is parsed once within that wave, even when several consumers request facts.

The worker parses source without importing or executing it.
It runs with `-I -S`, using only the standard library.
Missing CPython 3.13 causes an explicit engine failure.
There is no alternative backend or configuration switch.
A self-contained serializer prototype remains outside Git and is not a production path.

The producer returns typed columns through the existing indexed virtual-table interface.
SQL callbacks read completed columns; they do not parse source or invoke Python per node.
The producer supports canonical identifiers, dumps, literal metadata, unparsed expressions and SHA-256 fingerprints.
The cached rule uses these columns for policy comparisons and protected runtime verification.
JSON carries bounded process messages and literal signed metadata. AST properties remain native columns.

The independent oracle covers 30 documents and 160 function fingerprints.
It includes the three protected runtime files and cached-policy test inputs.
Native tests also cover isolated parsing, annotation normalization, literal metadata, invalid requests and empty batches.

The worker accepts at most 8 MiB of source and 64 MiB of output per batch.
These bounds reject excess work explicitly; they do not drop demanded rows.
The companion change preserves the producer callback guard and fixed producer-stage limit.

## Validation boundaries

The integrated engine changes are on `jeremy/data-layer-integrated-compat-20261103`.
The [engine receipt](../fixtures/host/data-layer-port/batch-08-engine.json) identifies the selected source and binary.
The port worktree keeps its earlier native engine for comparison.
Legacy relational execution still requires separate lowering into the wave/GPU model.

The broader native run passes 2,522 tests, with 67 existing ignores.
It excludes the previously documented independent-oracle snapshot-hash failure.
This exclusion is inherited from the batch-seven baseline; it is not a new pass.

Recorded detector parity does not certify every reporter, signing workflow, ratchet or CLI path.
The repository recorder retains signed aggregate reporter results separately from source diagnostic comparisons.
The pack remains private and inactive.

CPU phase counters in Node exclude child processes.
Measurements of the Python-backed producer need an external process-family CPU receipt.
Compilation time remains separate. Focused replay duration is not a cold repository benchmark.
