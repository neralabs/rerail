# Batch-nine completion audit — 2026-11-05

The previous completion claim was withdrawn.
The repair section records the subsequent fixes and expanded acceptance results.
The findings before the repair section describe the audited commit.
Its ten documents contained confirmed parity and production-integration defects.
The passing receipt measures its recorded corpus and adapter behavior. It does not prove complete rule semantics.

The audit uses the same source pin, `2b1504adc4291daaf080b2e932962b134da1a69a`.
The audited commit is `1c2c3be73`.
The [audit evidence](../fixtures/host/data-layer-port/batch-09-audit-20261105.json) retains sources, expected results, native results and the addon hash.

## Confirmed semantic defects

Three new cases pass against the original linters.
All three fail against the native implementation in both snapshot and workspace modes.

| Rule | Source shape | Original result | Native result |
| --- | --- | --- | --- |
| `schema_bootstrap_contracts` | A typed function contains `lambda untyped: untyped` | Clean | False SBC002 for `outer(untyped)` |
| `schema_bootstrap_contracts` | A typed parameter default contains that lambda | Clean | The same false SBC002 |
| `bootstrap_engine_invariants` | A bootstrap tail lacks its required parameter but contains `lambda produced_ref: produced_ref` | BEI024 | No finding |

The parameter queries associate arguments with an enclosing function through `fn`.
They fail to require ownership by that function's actual `arguments` node.
Lambda parameters therefore leak into function-signature checks.
The same error creates false positives and hides a required positive.

The reduced cases are in `scripts/dsl/data_layer_nine/audit_cases.py`.
Run them through `suite.py --oracle`, then use that report as the native `--baseline`.
Use each native mode and pass the reduced case file as the explicit pytest target.
These cases failed at the audited commit. They now remain as passing regressions in the acceptance corpus.

## Confirmed production-path gaps

The following checks use the shipped scopes, default options and direct `rule.evaluate()` output.
They bypass the replay adapter.

| Case | Original behavior | Shipped native behavior |
| --- | --- | --- |
| Malformed schema source containing `SchemaTransaction` | The configured reporter emits SAP000 | No finding and no failure |
| Malformed cache-health source containing `create_tables` | BCH000 with the parser error | Generic NATIVE-SYNTAX |
| Plan composition without manually supplied observations | The original linter runs its live builder probes | Execution fails with `composition observations required` |

`NineReplay.run()` replaces the shipped scope with `**/*`.
It also supplies per-call options and translates native output into the upstream return types.
For cache-health parser errors, it explicitly manufactures BCH000 from NATIVE-SYNTAX.
That conversion exists in the test adapter. Direct policy execution does not perform it.
The existing production tests cover three of the ten checks and omit these failure paths.

The live composition producer exists in `scripts/dsl/data_layer_nine/live.py`.
Its only caller is the pytest harness.
The production execution path has no wiring that supplies fresh observations.
A valid producer contract and tests with injected observations do not complete that integration.
The missing producer must remain visible as an execution failure until its integration is implemented.

## Missing upstream coverage

The previous suite omits four composition tests in `tests/l9875_mcp/test_mcp_architecture_contracts.py`:

- `test_mcp_semantic_plans_reject_runtime_imports`
- `test_mcp_semantic_plans_reject_runtime_handle_fields`
- `test_only_the_provider_executor_can_execute_mcp_plan_scopes`
- `test_mcp_repository_plan_composition_probes_pass`

All four pass when explicitly run against the original implementation and the adapted native harness.
Their omission is a discovery gap, rather than a newly failing detector case.
The default test list must include them.
The other tests in that module concern different linters and are outside this batch.

Neither bootstrap-lazy-SQL entry point has a dedicated upstream pytest module at this pin.
Their five recorded API calls come entirely from added controls.
The earlier restart document incorrectly said that lazy-SQL tests had been recovered.

## Required completion work

1. Correct argument ownership and retain the three reduced cases in the acceptance corpus.
2. Add the omitted upstream composition cases through source-based test discovery.
3. Preserve SAP000 and BCH000 on the shipped production path, including messages and locations.
4. Integrate the live composition producer with the production host boundary and test its fresh observations.
5. Compare production defaults and file scopes for all ten checks against original behavior.
6. Re-run complete positive, clean, parser-error and incremental acceptance cases before issuing another completion claim.

The old receipt remains historical evidence of its actual runs.
Its case count must not serve as a declaration that batch nine is complete.
This audit is not exhaustive. No Linear tickets were created.

## Repair and acceptance — 2026-11-05

The historical audit JSON remains unchanged. The current implementation fixes each confirmed defect.

| Defect | Repair | Permanent regression |
| --- | --- | --- |
| Lambda arguments leak into function signatures | Publish actual `arguments` containers and join signature parameters through their owning function | `audit_cases.py`; `parser-source.test.ts`; shipped-policy regressions |
| Schema parser diagnostic disappears | Batched source-parser facts feed the production SAP000 relation | `batch-nine-production-regressions.test.ts` |
| Cache parser diagnostic exists only in the adapter | The production BCH000 relation preserves the parser message and file-start location; adapter fabrication removed | `parser-source.test.ts`; `batch-nine-production-regressions.test.ts` |
| Production probes require manual observations | The public CLI and host API run the actual producer freshly before rule normalization | `production-runtime.test.ts` |
| Four upstream MCP cases are omitted | Source-based discovery includes all four; separate collections preserve upstream contract markers | `audit_cases.py`; assertion receipt |

The runtime regressions also found stale Python bytecode reuse and rejection of revision-zero snapshots.
The producer now uses an empty bytecode-cache path for each process.
The fresh workspace maps an initial revision of zero to its first application revision.
The aggregate verifier also requires complete API-value equality. A changed diagnostic must fail verification.

The original oracle and both native APIs pass 212 cases.
Each native mode matches 253 complete API results, including 147 results with findings.
One additional original instrumentation record maps to actual native parse and walk counters.
All ten detectors fail their disabled-detector controls.
Twenty-two production regressions preserve shipped defaults and scopes; two runtime tests exercise the public production boundary.
The native DSL suite passes 1,576 tests, with 23 existing ignores.

The [current report](data-layer-batch-nine.md) describes runtime requirements and reproduction.
The [current receipt](../fixtures/host/data-layer-port/batch-09-native-assertions.json) binds the expanded suite to the implementation.
These results establish the stated acceptance coverage. They do not turn the original incomplete claim into valid historical evidence.

## Second audit of the repair claim — 2026-11-05

Commit `8e4cede5f` fixed the first audit defects but still did not establish completion.
The [public audit evidence](../fixtures/host/data-layer-port/batch-09-public-audit-20261105.json) preserves new reproductions against that commit.

| Confirmed gap | Why prior tests passed | Repair and permanent gate |
| --- | --- | --- |
| BEI035 points to line one instead of line three | DTO adapters use a SQL line field rather than the public span | A bounded `relational.line` anchor narrows the proven witness; both replay paths compare public diagnostics |
| Leading blank lines shift raw-source findings | The captured module text omits initial blank lines | Read full snapshot source; retain blank-line and Unicode regressions |
| Cache-health misses `conform/helpers.py` | Shipped scope only includes `test_*.py` | Use the configured plugin's full Python scope |
| Ignored directory scope differs | Single positive and unrelated-path tests miss directory exclusions | Compare original `FactEngine.plan()` selections for cache-health and mode-split |
| Report-only checks become blocking | Shared policy generator assigns `block` to both aliases | Use `log` and retain a successful command exit with visible findings |
| Source parser behavior replaces configured defaults | BCH000 and exception tests call source APIs | Observe actual plugin reporters; compare default malformed-input results in both native modes |
| Partial bootstrap findings survive failed project derivation | No invalid coverage dependency case | Preserve the original discarded project result for malformed coordinator or enum dependencies |

The configured observation recorder verifies 121 loaded upstream files against the source pin.
Nine malformed-input and dependency scenarios cover configured behavior.
Two scope inventories cover original directory discovery.
The real production composition command also ran against the pinned application and matched its clean original result.
The positive runtime tests still use deliberately defective builder behavior to exercise the detector.

The earlier repair section records historical acceptance results. Its numbers do not certify complete production semantics.
The current [batch report](data-layer-batch-nine.md) states the expanded evidence and its limits.

## Third audit — numeric equality and file-presence semantics

The audit targets commit `e5d9cc2a7` and retains its [six failing evaluations](../fixtures/host/data-layer-port/batch-09-third-audit-20261105.json).

| Defect | Reduced original behavior | Failing port behavior | Repair |
| --- | --- | --- | --- |
| BEI007 numeric coercion | Four overflowing real or imaginary values are not zero | SQLite casts canonical infinity text to zero and emits BEI007 | Compare canonical zero representations without numeric casts |
| CSB030 lifecycle fallback | An existing malformed lifecycle file prevents fallback | Only valid-file evidence controls presence, so fallback emits CSB030 | Include invalid-file evidence in the presence decision |

The original reduced suite passes 23 cases.
Before the repair, both native modes fail three cases and pass twenty.
The permanent shape gate initially fails ten TypeScript tests, including direct shipped-policy comparisons.
After the repair, both reduced suites and the shape gate pass.

The shape recorder retains 88 original observations across all ten checks.
Twelve additional pytest cases cover nine numeric variants and three lifecycle states.
The complete oracle now contains 228 cases and 270 API records.
The additional matrix records are separate observations, rather than recovered upstream tests.
The fixes change SQL policies and preserve existing engine limits. No native implementation changed in this round.
