# Rerail rule pack: Data-layer architecture rules

This private pack contains sixty authored candidates from the pinned data-layer revision.
All six batches pass their recorded source-detection gates; retained integration contracts remain documented.
The policies are in `candidates/rules/` and are not activated by the pack manifest.

Run the original tests with `scripts/dsl/data_layer_record.py`.
Replay their recorded inputs with `conformance/data-layer/replay.ts`.
Use `data_layer_suite_record.py` and `conformance/data-layer/suite.ts` for a combined repository scan.

The source reference is in [UPSTREAM.md](UPSTREAM.md).
