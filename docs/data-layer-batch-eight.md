# Data-layer batch eight

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
Status: ten candidates pass focused parity and the cumulative 80-rule repository gate.

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
The 214 supplements contain 134 positive and 80 clean observations.
All 522 original and supplemental observations match both native APIs, with zero finding or execution differences.
All ten always-clean controls and all 50 edit/removal transitions pass.
They include original lazy-architecture fact-test source inputs and positive probes for every LA diagnostic family.
The source pin has no dedicated lazy-SQL-surface test module.
Its positive and clean evidence comes from these supplemental oracle probes.

The native gate requires positives and clean cases for every rule.
Always-clean controls must miss every expected finding on their selected positive input.
The edit harness checks clean, positive, restored, repeated-positive and removed-file states.
Removal results must match a fresh empty workspace.

## Full repository result

All 80 rules match the fixed 9,021-file repository: 25,262 expected findings and 25,262 actual findings.
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

The broader TypeScript gate passes 117 tests, with two existing opt-in instrumentation tests skipped.
The broader native run passes 2,525 tests, with 67 existing ignores.
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
git worktree add --detach .rerail/data-layer-batch-eight/reproduce-engine 899c3e53d2cb43a509a47d94e0bdc4fd36bb3c21
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
