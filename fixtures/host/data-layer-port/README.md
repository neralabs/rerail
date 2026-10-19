# Data-layer migration corpus

`batches.json` assigns all 142 pinned source checks to migration batches.
`batch-01-recording.json`, `batch-02-recording.json`, and `batch-03-recording.json` contain inputs and oracle results from each batch’s ten original pytest modules.
Their corresponding `batch-XX-evidence.json` files record provenance, coverage, and repository measurements.
`batch-02-probes.json` contains 58 additional source cases, each evaluated through the original analyzer and plugin reporter.
Its 116 calls are supplemental oracle probes, not original pytest cases.

The source commit is recorded in each JSON file.
Regenerate original-test expectations with `scripts/dsl/data_layer_record.py` against that revision, with `--batch 1`, `--batch 2`, or `--batch 3`.
Regenerate supplemental probe expectations with `scripts/dsl/data_layer_probe_record.py`.
Do not edit expected findings to match a candidate rule.
The original Python test modules and complete repository snapshot are not bundled here.
This corpus is excluded from published package files.

The native tests replay positive and clean cases and reject deliberately incorrect detectors.
See [the execution report](../../../docs/data-layer-port-execution-plan.md) for commands and remaining coverage gaps.

The [second-batch report](../../../docs/data-layer-batch-two.md) explains production reporter filters and additional native capabilities.

`batch-03-probes.json` contains 88 additional source cases and their original analyzer/reporter results.
The native gate compares 167 calls and retains nine signed aggregate reporter calls separately.
`batch-03-evidence.json` distinguishes source detection parity from retained integration contracts.
See [the third-batch report](../../../docs/data-layer-batch-three.md).

`batch-04-recording.json` contains the fourth batch’s original analyzer calls and mechanically translated classifier inputs.
`batch-04-probes.json` contains 171 supplemental cases, including positive and clean Rust cases and aggregate escape-hatch reporters.
The combined native gate compares 573 calls with positive and clean coverage for all ten checks.
Only two upstream duplicate-owner ambiguities and three original I/O observations remain separate.
`batch-04-evidence.json` records parity counts and repository measurements.
`escape-hatch-allowlist-signature.json` contains the pinned public verification material for the host adapter.
Use `data_layer_record.py --batch 4` and `data_layer_probe_four.py` to regenerate their oracle results.
See [the fourth-batch report](../../../docs/data-layer-batch-four.md).
