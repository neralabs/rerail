# Batch-nine completion audit — 2026-11-05

The previous completion claim is withdrawn.
The ten documents exist, but batch nine has confirmed parity and production-integration defects.
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
These cases remain failing regressions. This audit does not fix them.

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
