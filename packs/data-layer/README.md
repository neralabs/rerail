# Data-layer candidate rules

This private pack contains forty authored candidates from the pinned data-layer revision.
All four batches pass their recorded source-detection gates; retained integration contracts remain documented.
The policies are in `candidates/rules/` and are not activated by the pack manifest.

Run the original tests with `scripts/dsl/data_layer_record.py`.
Replay their recorded inputs with `conformance/data-layer/replay.ts`.
Use `data_layer_suite_record.py` and `conformance/data-layer/suite.ts` for a combined repository scan.
The [execution report](../../docs/data-layer-port-execution-plan.md) gives commands, results, and remaining gaps.

The [second-batch report](../../docs/data-layer-batch-two.md) covers annotation, alias, async-context, and source-line state behavior.

The source reference is in [UPSTREAM.md](UPSTREAM.md).

The [third-batch report](../../docs/data-layer-batch-three.md) covers ownership, callbacks, exception detection, and caches.

The [fourth-batch report](../../docs/data-layer-batch-four.md) records SQL grammar candidates, Rust coverage, upstream ambiguities, completed fixes, and performance measurements.
