# Data-layer rules

This private pack contains the twenty batch-eight and batch-nine architecture rules for nera-data-layer.
A second audit exposed public-contract gaps after the earlier batch-nine repair claim.
The current report records the repairs, measured coverage and remaining certification limits.
The [audit](../../docs/data-layer-batch-nine-audit.md) retains the original failures and their resolutions.
Load it explicitly with `readBundledCatalog([packDirectory])` for repository integration.
It is not part of the default core catalog.
The other seventy ports remain in `candidates/rules/`.

The [batch-nine report](../../docs/data-layer-batch-nine.md) records the next ten ports and their complete API evidence.
The production CLI and `runDataLayerAudit()` API collect fresh plan-composition observations from the application.
The report defines the low-level observation contract.
Generate batch nine with `scripts/generate/packs/data_layer_batch_nine.py`.

The [batch-eight report](../../docs/data-layer-batch-eight.md) records the upstream tests, positive controls and runtime requirements.
The [restart plan](../../docs/data-layer-port-restart.md) identifies the remaining 52 checks.
The source pin and attribution are in [UPSTREAM.md](UPSTREAM.md).

Generate the batch-eight documents with `scripts/dsl/data_layer_generate_batch_eight.py`.
Use the pinned upstream checkout as `--source`.
Canonical Python facts require CPython 3.13 on the engine process's PATH.
The worker parses source in isolation. It does not import or execute that source.

Run the unchanged upstream assertions with `scripts/dsl/data_layer_audit/api_parity.py`.
The CI workflow runs both native APIs and checks complete results, analyzer facts and graph state.
It also requires upstream positives to reject disabled detectors.
Use `conformance/data-layer/replay.ts` to replay the recorded positive and clean corpus.

The earlier reports cover [batch two](../../docs/data-layer-batch-two.md), [three](../../docs/data-layer-batch-three.md), [four](../../docs/data-layer-batch-four.md), [five](../../docs/data-layer-batch-five.md), [six](../../docs/data-layer-batch-six.md) and [seven](../../docs/data-layer-batch-seven.md).
Historical repository benchmarks used earlier engine builds. They do not measure the current batch-eight or batch-nine kernels.

Run the production integration with a repository snapshot:

```sh
rerail data-layer --repo /path/to/data-layer --pack packs/data-layer \
  --snapshot /path/to/snapshot.json
```

The explicit command imports application code for live composition probes.
It uses the application's `.venv/bin/python` when present, or `python3.13`.
Use `--python` to select the application's interpreter and dependencies.
Use `--rules` to select comma-separated rule IDs.
`runtime/observations.py` collects fresh typed observations. SQL evaluates the composition predicates.
Probe failures remain failures. The runner rejects snapshot source that differs from application source.
