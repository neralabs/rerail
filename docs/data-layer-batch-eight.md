# Data-layer batch eight

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
Status: all ten candidates pass native-backed upstream assertions and positive corpus replay. The repository now has 80 candidate ports. The historical repository benchmark predates these changes.

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

## Native assertion completion

The earlier audit left semantic helper assertions on the original Python implementation. That was incomplete validation.
The [native assertion receipt](../fixtures/host/data-layer-port/batch-08-native-assertions.json) supersedes that audit.

All 456 original upstream cases now pass through both native execution APIs. No case is skipped.
The harness leaves upstream files unchanged and verifies their Git blob identities.
It substitutes native scanner and helper APIs before pytest imports the test modules.
A guard rejects original detector, analyzer, collector and graph-helper execution.

| Evidence, per execution API | Count |
| --- | ---: |
| Original cases passed | 456 |
| Cases that directly invoke native execution | 336 |
| Native API calls | 480 |
| Diagnostic calls | 283 |
| Positive diagnostic calls | 199 |
| Native helper and fact calls | 197 |
| Original semantic fallbacks | 0 |

The other 120 cases check constants, DTOs, file handling, signing orchestration or CLI scaffolding.
These host assertions remain in Python. They do not run an original detector.
Native helper calls now cover function facts, graph targets, eager exemptions, wrapper signatures, cached discovery and runtime fingerprints.
Plugin tests materialize the complete source demand through native SQL before invoking the original host reporter.

Three instrumentation functions contain four parametrized cases.
Their mechanical translations replace Python implementation hooks with actual native work counters.
They check one candidate module walk, one syntax load, and one shared legacy relation for two consumers.
Their behavior assertions remain intact. The receipt records both source hashes for each translation.

Disabling the nine detectors with original pytest modules causes 173 assertion failures and ten positive-fixture setup failures.
There are no semantic fallbacks in that control run.
The source pin has no dedicated lazy-SQL-surface pytest module.
Its native coverage uses independent positive and clean oracle probes, including an always-clean mutation control.

The original and supplemental recordings provide 534 observations: 335 positive and 199 clean.
Another 18 [exact-message probes](../fixtures/host/data-layer-port/batch-08-exact-probes.json) provide 12 positive and six clean observations.
All 552 observations match both APIs without finding differences or engine failures.
The new probes compare diagnostic prose too.
They cover ordered eager targets, cycles, first matching connection-owner calls, nested classes, compatibility imports and deferred delegates.

The work fixes a real graph-selection error: alphabetical target selection differed from Python's call order beneath a shared first hop.
SQL now orders demanded walks by source call order, stops at matching sinks and guards cycles.
It limits these walks to roots with diagnostic demand.
The shared producers also retain class method aliases, pseudo-function fields, exact diagnostic details and stable purity scopes.

The engine now accepts a relational message column and rejects missing or nontext payloads.
Its profile reports actual canonical parses, module walks, syntax loads and relation materializations.
Snapshot merging and workspace apply reporting retain those counters.
Text-only canonical identifier and byte-column requests avoid parsing and walking an AST.
Unused module literals no longer trigger immutable-container classification.

## Reproduce native assertions

Use the [selected engine receipt](../fixtures/host/data-layer-port/batch-08-engine.json) to build the companion branch.
Set `RERAIL_ENGINE_PATH` to that addon.
Generate shared helper contracts, then run each execution API:

```sh
python3 scripts/dsl/data_layer_audit/native_contracts.py \
  --source .rerail/data-layer-testsuite-positives/upstream-git \
  --output .rerail/data-layer-batch-eight/native-contracts.json
export RERAIL_NATIVE_CONTRACTS="$PWD/.rerail/data-layer-batch-eight/native-contracts.json"
env -u GIT_PAGER /path/to/python3.13 scripts/dsl/data_layer_audit/native_suite.py \
  --source .rerail/data-layer-testsuite-positives/upstream-git \
  --repo /path/to/nera-data-layer-git-repository \
  --mode snapshot \
  --output .rerail/data-layer-batch-eight/native-pytest-final-snapshot.json
```

Repeat with `--mode workspace` and its corresponding output name.
The `--policies` argument selects a test policy directory for detector mutation controls.
It does not select a production execution backend.

The final focused TypeScript run passes 35 tests. Canonical producer tests pass nine; relational tests pass 94.
The integrated compiler accepts all 80 current candidates.
Typechecking, boundary checks and Windows path checks pass.
The structure check retains its three previously recorded directory-size failures.
No complete repository cold execution ran for this delivery.

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
A self-contained serializer prototype remains outside Git and is not a production path.

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
git worktree add --detach .rerail/data-layer-batch-eight/reproduce-engine ebcfefa0a
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
  test/engine/native/data-layer-batch-eight-capabilities.test.ts
node --import tsx conformance/data-layer/compile-integrated.ts \
  --source .rerail/data-layer-batch-eight/reproduce-engine --through 8 \
  --output .rerail/data-layer-batch-eight/reproduce-compatibility.json
node --import tsx conformance/data-layer/replay.ts \
  --recording fixtures/host/data-layer-port/batch-08-recording.json \
  --recording fixtures/host/data-layer-port/batch-08-probes.json \
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
