# Data-layer candidate rules

This private pack contains seventy authored candidates from the pinned data-layer revision.
All seven batches pass their recorded source-detection gates. The complete 70-rule repository gate matches all findings without engine failures.
The [seventh-batch report](../../docs/data-layer-batch-seven.md) records compatibility, retained contracts, and the unresolved memory cost.
The [restart plan](../../docs/data-layer-port-restart.md) identifies the remaining 72 checks.
The policies are in `candidates/rules/` and are not activated by the pack manifest.

Run the original tests with `scripts/dsl/data_layer_record.py`.
Replay their recorded inputs with `conformance/data-layer/replay.ts`.
Use `data_layer_suite_record.py` and `conformance/data-layer/suite.ts` for a combined repository scan.
The [execution report](../../docs/data-layer-port-execution-plan.md) gives commands, results, and remaining gaps.

The [second-batch report](../../docs/data-layer-batch-two.md) covers annotation, alias, async-context, and source-line state behavior.

The source reference is in [UPSTREAM.md](UPSTREAM.md).

The [third-batch report](../../docs/data-layer-batch-three.md) covers ownership, callbacks, exception detection, and caches.

The [fourth-batch report](../../docs/data-layer-batch-four.md) records SQL grammar candidates, Rust coverage, upstream ambiguities, completed fixes, and performance measurements.

The [fifth-batch report](../../docs/data-layer-batch-five.md) covers declaration graphs, family contracts, and project reachability.

The [sixth-batch report](../../docs/data-layer-batch-six.md) covers ServerPlan structure, execution authority, state propagation, and relational scope analysis.
