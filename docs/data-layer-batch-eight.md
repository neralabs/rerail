# Data-layer batch eight

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
Batch eight is complete. The inventory now contains 80 of 142 ports.
The ten batch-eight documents are in `packs/data-layer/rules/`; seventy earlier ports remain candidates.
The default engine executes these rules. A companion addon is no longer required.

| Check | Main contract |
| --- | --- |
| `eager_resolve` | Source patterns, eager helpers, phase propagation and external boundary definitions |
| `lazy_architecture` | All 18 diagnostic families, scoped aliases and ordered function reachability |
| `pure_lazy_runtime_purity` | Function coloring, imported helpers, runtime handles, globals and exports |
| `cached_pure_lazy_allowlist` | Cache contracts, memoization policies, exact fingerprints and protected runtime signatures |
| `lazy_sql_surface` | Terminal calls, compile context access, execution authority and result annotations |
| `plan_helpers_allowlist` | Signed callsite names, imports, ordered aliases and forbidden helper access |
| `legacy_callback_quarantine` | Callback shapes, owner exemptions and production path exclusions |
| `readthrough_authority_architecture` | Authority ownership and transaction entry paths |
| `authentication_plan_purity` | Authentication state, reachable helpers and compatibility forwarding |
| `lazy_adapter_ownership` | Shared lazy bodies, direct adapters and legacy ownership |

## Correctness evidence

All 456 upstream cases run under the harness in snapshot and resident-workspace modes.
The upstream source files remain unchanged.
The harness rejects original Python detector or analyzer execution.
Each shard verifies loaded upstream source against the pinned Git blobs.
The collector rejects skipped tests, duplicate coverage, missing test identities and incomplete results.

| Gate, per native API | Result |
| --- | ---: |
| Original upstream cases | 456 passed; zero skipped |
| Cases with native calls | 336 |
| Native API calls | 523 |
| Diagnostic calls containing findings | 201 of 326 |
| Complete API results | 305; zero differences |
| Complete analyzer inputs | 85; zero field differences |
| Graph-helper inputs | 12; exact return values, memo contents, insertion order and visiting state |
| Supplied-fact batches without source files | 17 |
| Original semantic fallbacks | 0 |

The other 120 original cases test host constants, DTOs, discovery, signing or CLI orchestration.
They are host coverage, not additional native detector coverage.
Four instrumentation cases replace Python implementation hooks with actual native counters.
Their behavior assertions remain intact.

The recorded scanner corpus contains 558 observations: 351 positive and 207 clean.
Both native APIs match it without missing findings, extra findings or engine failures.
Independent probes cover every LA-001 through LA-018 diagnostic.
The source pin has no dedicated lazy-SQL-surface pytest module; independent probes cover its positive and clean behavior.

The original suites reject disabled source and supplied-fact detectors.
Those controls produce 173 failed assertions and ten positive-fixture setup errors across all nine upstream pytest suites.
The native replay also rejects disabled detectors for all ten rules.
Changed-fingerprint and missing-declaration controls test the cached-discovery comparison itself.

The [assertion receipt](../fixtures/host/data-layer-port/batch-08-native-assertions.json) binds the results to the programs, adapters, oracles and addon.
The [scanner receipt](../fixtures/host/data-layer-port/batch-08-native-parity.json) records individual positive and clean comparisons.
The [engine receipt](../fixtures/host/data-layer-port/batch-08-engine.json) identifies the default build.

## Defects closed after the audit

LA-011 now chooses the first matching hydration call in visitation order.
It previously chose the smallest source location.
The supplied-fact regression orders calls at lines 3 and 2; the correct diagnostic is line 3.
This is a supplied-fact API reproduction, not a source-discovery counterexample.

The standalone transitive helper now uses the production ordered-graph producer.
It consumes existing positive and negative memo entries and preserves every memo update in order.
Its tests cover cached roots and children, cycles, visiting roots, target precedence and missing callable facts.

Production integration exposed a separate result-contract defect.
The enforcer and API accepted only scalar variables and rejected the SQL `row` object.
They now accept the existing recursive JSON value type.
Tests run shipped authentication and purity rules through the real enforcer, with default options and scopes.
They check exact messages and lines, delivery, retraction, excluded test paths and unrelated paths.

## Engine implementation

The default engine now includes canonical Python facts, ordered first-match graph traversal and exact SQL diagnostic messages.
Existing SQL fusion, binding reuse and typed producer contracts remain in place.
Compiler inference retains native output schemas and validates graph-state inputs.
The per-file source-byte limit applies consistently to both execution paths.

The graph producer reads whole SQL batches:

| Input | Columns |
| --- | --- |
| Roots | `__path`, `id`, `ordinal` |
| Edges | `__path`, `src`, `dst`, `ordinal`, `location` |
| Sinks | `__path`, `id`, `location` |
| Initial memo, when supplied | `__path`, `id`, nullable `node`, nullable `location`, `ordinal` |
| Initial visiting set, when supplied | `__path`, `id` |

`selection: "memoized-first"` preserves root order, edge order and negative memo entries.
`match: "edge"` checks called targets; `match: "node"` also checks the root.
`emit: "memo"` returns complete ordered memo state, including negative entries.
These fields express semantics; they do not select optional execution backends.
The result uses typed column storage and indexed postings.
The producer contains no data-layer symbols and submits no SQL per graph node.

Canonical facts use one isolated CPython 3.13 process per nonempty demand wave.
The worker runs with `-I -S` and never imports or executes the supplied source.
Missing CPython causes an explicit failure.
The worker bounds source at 8 MiB and output at 64 MiB per batch; it rejects excess work rather than dropping rows.
Text-only identifier and byte-column requests need no CPython parse.
The independent canonical oracle covers 30 documents and 160 fingerprints, including protected runtime files.

## Admission and performance

The real upstream discovery test reads 3,095 production files, totaling 62,921,799 source bytes.
Profiling exposed full SQL index publication for files already rejected by their selectors.
Preparation now publishes facts only for admitted demands in both execution paths.

Complex Unicode trigger expressions also lost text admission when literal extraction exceeded its bound.
A conservative regex requirement now handles that case.
It removes zero-width assertions before searching a containing source, so node anchors cannot become file anchors.
Regression tests cover Unicode, node boundaries and absent-source publication.
The work-budget test now supplies real matching roots, preventing admission from removing its intended workload.

The [kernel measurements](performance-decisions/data-layer-batch-eight-default-kernels-20261104.json) separate compile CPU, evaluation CPU, process-family CPU and RSS.
They cover only the new graph and canonical-Python kernels.
They do not establish a repository-wide speedup or memory improvement.
The historical repository benchmark used another engine build and remains unchanged.

## Reproduction and CI

Build this checkout normally with `npm run build:native`.
Alternatively, use the profiling build described in the engine receipt.
Leave `RERAIL_ENGINE_PATH` unset to exercise the default addon.
Put CPython 3.13 on PATH.

```sh
python3 scripts/dsl/data_layer_audit/native_contracts.py \
  --source /path/to/pinned/data-layer \
  --output .rerail/data-layer-batch-eight/contracts.json
export RERAIL_NATIVE_CONTRACTS="$PWD/.rerail/data-layer-batch-eight/contracts.json"
```

Run `api_parity.py compare` with `PYTHONHASHSEED=0`, `--parts 4` and each `--part` from 0 through 3.
Use `batch-08-complete-api.json` as `--oracle` and `api-<mode>-<part>.json` as the output name.
Run `collect.py --reports <directory> --mode <mode>` afterward.
Repeat for `snapshot` and `workspace`.
The [CI workflow](../.github/workflows/ci.yml) contains the complete commands and dependency setup.
Each assertion subprocess has a 115-second limit.

`facts_parity.py compare` checks `batch-08-complete-facts.json`.
`graph_api.py compare` checks `batch-08-graph-api.json`.
`native_mutation.py` creates disabled detectors while retaining real helper producers.
`native_suite.py` runs the unchanged positives against those mutations; `collect.py --controls` rejects ineffective controls.

Trusted CI jobs check out the pinned private upstream repository using the existing CI app with read-only contents permission.
Forks still run the recorded native corpus without upstream credentials.
Remote CI execution and the app's installation access are not claimed as locally verified.

All 80 policies compile on the default engine.
The Rust DSL suite passes 1,552 tests, with 22 existing ignores.
Test compilation also passes after the fixture relocation.
Typechecking, boundary checks and Windows path checks pass.
The structure gate retains only its three existing directory-size findings.

The private pack has one document per rule id.
Load it explicitly with `readBundledCatalog([packDirectory])`; core does not enable it automatically.
The original CLI, signing and ratchet orchestration remains host code; it has not been rewritten as Rerail commands.
Installing rules in nera-data-layer and wave/GPU lowering remain separate integration work.
The remaining port inventory is 62 rules.
