# Data-layer batch nine

Status, 2026-11-05: the audited defects are fixed, and the expanded acceptance suite passes.
The [completion audit](data-layer-batch-nine-audit.md) retains the original failures and their resolutions.
The earlier completion claim was incorrect. The results here describe the repaired implementation and its measured coverage.

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
The inventory contains 90 authored ports out of 142. Batch nine passes the gates described here; fifty-two further checks remain unauthored.
The private pack ships twenty documents. Seventy earlier ports remain candidates.

| Check | Preserved contract | Recorded API results with findings |
| --- | --- | ---: |
| `connection_mode_split` | Public names, imports, handle modes, method kinds and explicit file scope | 9 |
| `connection_structural_boundaries` | File limits, class bases, lifecycle ownership, supplied trees and definition order | 10 |
| `connection_service_architecture` | Delegation, schema authority, facades, ownership across files and legacy wrappers | 22 |
| `schema_architecture` | Standalone planning checks and the configured lifecycle plugin, SAP001–SAP005 | 8 |
| `schema_bootstrap_contracts` | Exact annotation names, parameter kinds, lazy signatures and authority use | 11 |
| `bootstrap_engine_invariants` | Function checks, raw-source checks, nested traversal, primitive installation and diagnostic multiplicity | 65 |
| `bootstrap_lazy_sql` | Eager materialization, query chains, row loops and helper calls | 1 |
| `bootstrap_eager_resolve` | The same detector, exposed through its separate configured entry point | 1 |
| `bootstrap_cache_health` | Bootstrap frequency, fixture ownership, table creation, exemptions and parser diagnostics | 10 |
| `plan_composition_architecture` | Source boundaries, mutable results, exact reference remapping and live builder observations | 10 |

The table counts API results, not independent tests or individual findings.
Every check has positive and clean results.
The eager-resolution entry point has no dedicated upstream test module at this pin.
Its upstream script delegates to the lazy-SQL CLI. Both ports receive independent oracle-evaluated controls.

## Assertion and result coverage

The corpus contains 212 pytest cases:

- 137 upstream linter and supplied-tree framework cases.
- 13 adjacent upstream codemod cases, retained as host coverage.
- 62 additional cases, evaluated first by the original Python implementation.

The 137 upstream cases include seven lifecycle-plugin cases outside `scripts/tests`.
They also include the service-rule repository-clean test, which reads 100 files and 3,425,634 source bytes.
That complete test domain remains intact.
Four MCP composition cases now run through source-based discovery.
Independent pytest collections preserve upstream contract ownership when modules declare the same contract markers.
The additional cases include two harness checks for discovery and rejection of a changed diagnostic.

The original implementation records 254 complete API results, including 147 results with findings.
One instrumentation case measures the original Python subtree walk.
Its native replacement checks one actual module walk and one actual parse.
The remaining 253 results compare complete return values through both native APIs.
Comparison includes ordering, multiplicity, messages, paths, line numbers, exception details and fact identifiers.
Absolute paths inside fact identifiers receive the same root normalization as their path fields.

The harness runs the original behavior assertions against native results.
It rejects original detector execution through a Python call guard.
It also rejects input drift, missing calls, skipped tests and incomplete case coverage.
The aggregate verifier compares complete API values as well as result counts.
A regression changes a diagnostic and requires rejection.
Each run verifies loaded upstream source files against the pinned Git blobs.
Host orchestration, DTO construction, signing and codemod assertions remain Python code.
These host cases do not count as native detector observations.

Detector-disabled runs must make the original positive assertions fail for every check.
The independent TypeScript replay also disables each detector against a recorded positive.
Twenty-two production regressions exercise shipped defaults, scopes, exclusion paths and original positive results through both APIs.
Two additional runtime tests exercise the production runner, fresh observations, edits, stale snapshots and application failures.
They also cover revision-zero snapshots and timestamp-valid stale Python bytecode.
The [assertion receipt](../fixtures/host/data-layer-port/batch-09-native-assertions.json) records the final results and file identities.
The [API corpus](../fixtures/host/data-layer-port/batch-09-complete-api.json) retains the complete source inputs and expected values.

## Engine changes driven by these ports

CPython 3.13 supplies canonical syntax facts in one batch. It does not import or execute inspected source.
SQLite executes the detection predicates over typed scalar columns.
Canonical facts preserve Python preorder, breadth-first order, parent fields, argument kinds and annotation spelling.
Annotation-name relations retain the original handling of forward references.
The producer now publishes `arguments` containers. Signature relations require ownership by the actual function argument container.
Lambda parameters therefore cannot satisfy or violate an enclosing function signature.
A generic source-mode parser stage returns batched `syntax_error` facts for malformed source.
The schema and cache policies produce SAP000 and BCH000 directly, including the original messages and file-start locations.
The replay adapter no longer manufactures BCH000.

The module producer now transfers demanded columns instead of complete row dictionaries.
The generator uses SQLite column-demand analysis to omit unused fields.
Breadth-first ordering and annotation rendering run only when consumers require them.
The native adapter publishes column vectors directly.
Bulk transfer accounting includes row setup, text bytes and string headers.
The existing capacity and statement limits remain unchanged.

Canonical argument spans can omit the stars in variadic parameters.
The engine accepts these spans only after the canonical producer validates them against the source.
Fabricated SQL ranges still fail anchor validation.
A bounded SQL-owner cache retains these proofs for the exact grammar and source digest.
The proofs therefore survive reuse of SQL results by a fresh workspace document.
The existing binding-memory limit accounts for their storage and evicts them with the binding identities.

Real test failures also exposed an uncorrelated service-method descendant query.
It falsely reported 351 clean methods. Explicit owner aliases now preserve correlation.
A regression puts a bad method beside a clean method in the same file.
Repeated delete targets and keyword arguments now preserve repeated findings.
Legacy lifecycle helpers preserve their first-definition semantics and fixed method order.

## Runtime contracts

Canonical facts require CPython 3.13 on the engine process's PATH.
The production `schema_architecture` default runs the configured lifecycle plugin.
The standalone script remains available through its separate entry point.
Source entry points preserve their original supplied-tree and parser-error behavior.

Plan composition also checks live `SemanticPlanBuilder` behavior.
The production CLI and `runDataLayerAudit()` host API collect fresh observations automatically.
The public runner lives in `src/cli/data-layer-audit.ts`.
It runs `packs/data-layer/runtime/observations.py` in a new process against the actual application.
The harness calls that same producer in-process to preserve upstream monkeypatches.

The producer returns typed observations:

- `fingerprints`: two strings from equivalent plan constructions.
- `firstRefs`: serialized destination references from the first fragment import.
- `secondRefs`: serialized destination references from the second fragment import.

SQL compares the fingerprints and reference sets.
The runner binds observations before schema normalization resolves option expressions.
It rejects mismatches between snapshot source and application source.
A fresh bytecode-cache path prevents reuse of stale Python bytecode, including timestamp-valid caches.
Application import or probe failures remain execution failures.
The producer requires the application's Python dependencies and imports application code only for this explicit command.

Low-level compiled rules still require the observation batch for `all` and `run_behavior_probes`.
They fail closed without it. Recorded observations support replay, and cannot replace fresh observations in repository integration.
Source-only entry points retain the original source API.

Run the production command with a repository snapshot:

```sh
rerail data-layer --repo /path/to/data-layer --pack packs/data-layer \
  --snapshot /path/to/snapshot.json --rules data-layer-plan-composition-architecture
```

The command selects all shipped rules if `--rules` is absent.
The command uses `.venv/bin/python` when present, or `python3.13`. Use `--python` for another executable.
Its exit codes are zero for clean results, one for findings and two for engine failures.

## Reproduction

Generate the documents with `scripts/generate/packs/data_layer_batch_nine.py --source <pinned-checkout>`.
Run `scripts/dsl/data_layer_nine/suite.py` with `--oracle` to record original results.
Then run it with `--baseline fixtures/host/data-layer-port/batch-09-complete-api.json` and each native `--mode`.
Use `--parts 4` and each `--part` from zero through three for bounded shards.
The shards divide test cases; each case retains its complete file domain.

`scripts/dsl/data_layer_nine/evidence.py` checks aggregate coverage and detector-disabled controls.
`scripts/dsl/data_layer_nine/receipt.py` binds both execution paths, controls and implementation hashes.
The `data-layer-parity` CI job runs both native APIs and the disabled controls.
`conformance/data-layer/batch-nine-kernels.ts` measures only the new canonical-column kernel.
Compile CPU and evaluation CPU remain separate.
No cumulative cold repository benchmark forms part of this delivery.
Historical repository performance remains an open concern.
No Linear issues were created.
