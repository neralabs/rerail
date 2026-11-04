# Data-layer batch eight

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
Status: the recorded suites pass, but a follow-up audit found two correctness defects outside that corpus. Batch eight is not yet exact. The repository has 80 candidate ports. The historical repository benchmark predates these changes.

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

## Open correctness defects

The supplied-fact lazy-architecture consumer selects the lowest source location for the LA-011 hydration diagnostic.
The upstream consumer selects the first call in visitation order.
A batch with matching calls ordered at lines 3 and 2 returns line 3 upstream and line 2 in Rerail.

The standalone `_transitive_line_to_target_symbols` adapter ignores supplied memo entries.
With `memo={'root': 99}` and a target call at line 10, upstream returns 99 and the adapter returns 10.
The production ordered-graph producer is separate from this helper adapter.
These defects need regression cases and fixes. The passing corpus below does not cover them.

## Complete native comparisons

The previous completion claim was premature. Partial assertions missed complete-result differences.
The adapters also rescanned source when callers supplied facts.
This delivery replaces those paths and removes the `oracleAnalyzer` option from all ten rules.

All 456 upstream cases pass through both native APIs, with no skips or original semantic fallback.
The harness checks upstream source files against their Git blob identities.
Four instrumentation cases replace Python implementation hooks with actual native counters.
Their behavior assertions remain intact.

| Evidence, per native API | Count |
| --- | ---: |
| Original cases passed | 456 |
| Cases that directly invoke native execution | 336 |
| Native API calls | 523 |
| Diagnostic calls | 326 |
| Positive diagnostic calls | 201 |
| Helper and fact calls | 197 |
| Original semantic fallbacks | 0 |

The other 120 cases check host constants, DTOs, file handling, signing or CLI orchestration.
These are host assertions, not additional native detector coverage.

The [complete API oracle](../fixtures/host/data-layer-port/batch-08-complete-api.json) records 305 results from the unchanged upstream implementations.
Comparison checks every DTO field, list order, count and exception detail.
The [complete analyzer oracle](../fixtures/host/data-layer-port/batch-08-complete-facts.json) checks every field for 85 distinct upstream inputs.
Both comparisons have zero differences through both APIs.
The first complete analyzer comparison found differences in 25 inputs.

The [supplied-fact oracle](../fixtures/host/data-layer-port/batch-08-fact-api.json) adds 16 batches, each tested through both APIs without source files.
These tests check fact consumption, declared flags, event order, graph cycles, policy ordering and annotation formatting.
Source and fact consumers use the same comparison and graph definitions.
The adapters no longer reconstruct supplied facts from source.

All 558 recorded scanner observations match both native APIs: 351 positive and 207 clean.
This includes complete coverage of the 18 lazy-architecture diagnostic families.
The [assertion receipt](../fixtures/host/data-layer-port/batch-08-native-assertions.json) binds the results to the rules, adapters and engine.

Always-clean mutations reject positives for all ten rules.
The original suites also reject mutations of the supplied-fact consumers.
That control run has 173 failed assertions and ten positive-fixture setup errors, with no semantic fallback.
The source pin has no dedicated lazy-SQL-surface pytest module; independent oracle probes cover that rule.

## Shared implementation

The fixes preserve CPython visitation order, including conditional tests, dictionary keys and call arguments.
Raw deleted-surface checks retain their own text coordinates.
Canonical unparsed text supplies eager diagnostic expressions.
Policy comparisons preserve error order and keep declaration-specific errors separate.
Recursive global-value classification checks every nested tuple and `frozenset` argument.
The default eager allowlist verifies the pinned signatures.

Independent reachability did not preserve the linter's memoized traversal through cycles.
The generic native first-match producer now consumes three typed SQL queries:

| Input | Columns |
| --- | --- |
| Roots | `__path`, `id`, `ordinal` |
| Edges | `__path`, `src`, `dst`, `ordinal`, `location` |
| Sinks | `__path`, `id`, `location` |

`selection: "memoized-first"` preserves root order, edge order and cached negative results within each scope.
`match: "node"` includes the root; `match: "edge"` checks called targets.
A null sink scope shares a sink without copying it across file scopes.
The output contains `__path`, `root`, `node` and `location` as indexed virtual-table columns.
The algorithm contains no data-layer symbols or Python detection logic.
It performs no per-node SQL submission or scalar callback.

## Reproduce native assertions

Build the companion engine from the [engine receipt](../fixtures/host/data-layer-port/batch-08-engine.json).
Set `RERAIL_ENGINE_PATH` to that addon and use CPython 3.13 with the upstream test dependencies.

```sh
python3 scripts/dsl/data_layer_audit/native_contracts.py \
  --source .rerail/data-layer-testsuite-positives/upstream-git \
  --output .rerail/data-layer-batch-eight/native-contracts.json
export RERAIL_NATIVE_CONTRACTS="$PWD/.rerail/data-layer-batch-eight/native-contracts.json"
env -u GIT_PAGER PYTHONHASHSEED=0 /path/to/python3.13 \
  scripts/dsl/data_layer_audit/api_parity.py compare \
  --source .rerail/data-layer-testsuite-positives/upstream-git \
  --repo /path/to/nera-data-layer-git-repository \
  --oracle fixtures/host/data-layer-port/batch-08-complete-api.json \
  --mode snapshot \
  --output .rerail/data-layer-batch-eight/full-api-final-snapshot.json
```

Repeat with `--mode workspace` and its corresponding output name.
This command runs the original assertions and the additional complete-result comparison.
The fixed hash seed preserves iteration order in original tests that use sets.
Use `facts_parity.py compare` with `batch-08-complete-facts.json` to check the complete analyzer fields.

`native_mutation.py --output <directory>` creates always-clean rule and fact consumers.
Run `native_suite.py --policies <directory>/rules` to check that upstream positives reject those mutations.
The helper producers remain active, so the control tests receive real input facts.

The integrated compiler accepts all 80 candidates.
Typechecking, boundary checks and Windows path checks pass.
The structure check retains its three existing directory-size findings.
No complete-repository cold benchmark ran for this delivery.

## Full repository result

The prior 80-rule run matched the fixed 9,021-file repository: 25,262 expected findings and 25,262 actual findings.
The run reports no extra findings, missing findings or engine failures.
The new ten rules contribute 269 findings: 34 eager findings and 235 purity findings.
The complete result also validates the first 70 rules on the selected engine.

| Measurement | Result |
| --- | ---: |
| Cold CPU, parent process | 695.641 s |
| Cold wall | 607.831 s |
| Compile CPU | 1.836 s |
| Compile wall | 1.875 s |
| Full process-family CPU, including Python children and close | 701.98 s |
| Full process wall | 613.94 s |
| Maximum RSS | 20.451 GB |

The [measurement receipt](performance-decisions/data-layer-batch-eight-20261104.json) records the binary, policy digests and workload identity.
The expected findings combine the unchanged first-70 and batch-eight oracle recordings on an identical verified source domain.
There is no newly timed cumulative Python oracle run.
The run enables no instrumentation and overlaps no agent-owned builds or tests.
Its policy digests identify the versions before the diagnostic metadata additions.
The historical measurement remains unchanged.

Memory remains an open optimization problem.
Workspace close reduces RSS from 13.765 GB to 2.604 GB.
The historical 70-rule run used 395.519 seconds cold CPU and 15.399 GB maximum RSS.
The rule set and engine differ, so that result supplies context rather than a speed comparison.

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

The producer returns typed columns through the existing indexed virtual-table interface.
SQL callbacks read completed columns; they do not parse source or invoke Python per node.
The producer supports canonical identifiers, byte columns, dumps, literal metadata, unparsed expressions, Python annotation representations and SHA-256 fingerprints.
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

The earlier broader TypeScript gate passed 117 tests, with two existing instrumentation tests skipped.
The earlier broader native run passed 2,525 tests, with 67 existing ignores.
It excludes the previously documented independent-oracle snapshot-hash failure.
This exclusion is inherited from the batch-seven baseline; it is not a new pass.

Recorded detector parity does not certify every reporter, signing workflow, ratchet or CLI path.
The repository recorder retains signed aggregate reporter results separately from source diagnostic comparisons.
The pack remains private and inactive.

CPU phase counters in Node exclude child processes.
Measurements of the Python-backed producer need an external process-family CPU receipt.
Compilation time remains separate. Focused replay duration is not a cold repository benchmark.


## Reproduce the selected engine

The engine receipt pins the source commit and the normal addon hash.
The profiling build profile supplies debug symbols. The receipt enables no extra instrumentation features.
The following commands reproduce the macOS build shape in a separate checkout:

```sh
git worktree add --detach .rerail/data-layer-batch-eight/reproduce-engine 29cbf425b
CARGO_TARGET_DIR="$PWD/.rerail/data-layer-batch-eight/reproduce-target" \
  cargo build --profile profiling --locked --offline \
  --manifest-path .rerail/data-layer-batch-eight/reproduce-engine/Cargo.toml \
  -p rerail-engine --lib
cp .rerail/data-layer-batch-eight/reproduce-target/profiling/librerail_engine.dylib \
  .rerail/data-layer-batch-eight/reproduce-engine.node
```

Set `RERAIL_ENGINE_PATH` to that addon before each command:

```sh
npx vitest run test/conformance/data-layer/batch-eight.test.ts \
  test/conformance/data-layer/batch-eight-fact-api.test.ts \
  test/engine/native/data-layer-batch-eight-capabilities.test.ts
node --import tsx conformance/data-layer/compile-integrated.ts \
  --source .rerail/data-layer-batch-eight/reproduce-engine --through 8 \
  --output .rerail/data-layer-batch-eight/reproduce-compatibility.json
node --import tsx conformance/data-layer/replay.ts \
  --recording fixtures/host/data-layer-port/batch-08-recording.json \
  --recording fixtures/host/data-layer-port/batch-08-probes.json \
  --recording fixtures/host/data-layer-port/batch-08-exact-probes.json \
  --recording fixtures/host/data-layer-port/batch-08-edge-probes.json \
  --output .rerail/data-layer-batch-eight/reproduce-parity.json
```

The compatibility receipt records file hashes and compiled policy digests.
These identities bind the recorded observations and repository results to the selected policies.


For the repository gate, record batches 1–7 and batch 8 separately with `data_layer_suite_record.py`.
Use the same clean source checkout at the pinned revision.
Combine their expected events and keep the verified 9,021-file domain from the first recording.
Reject duplicate checks, source mismatches and conflicting file contents before the native run.
The cumulative receipt leaves the Python timing unset because it combines two oracle executions.
Run `suite.ts --cold-only` with that combined recording and an external `/usr/bin/time -l` receipt.
Keep the addon and policy identities from the compatibility receipt.
