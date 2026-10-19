# Behavior reference

Repository: https://github.com/neralabs/nera-data-layer

Commit: `2b1504adc4291daaf080b2e932962b134da1a69a`

The candidate policies implement behavior from `scripts/lint_*.py` at this revision.
The original test modules run in a separate checkout.
The recorder preserves their fixtures and assertions, then records analyzer inputs and results.
The generated replay corpora are in `fixtures/host/data-layer-port/batch-01-recording.json`, `batch-02-recording.json`, and `batch-03-recording.json`.
`batch-02-probes.json` adds source cases evaluated by the pinned analyzers and actual plugin reporters.
These probes are separate from the original pytest corpus.
They contain test-generated inputs and oracle results, with pinned provenance.
Generated recordings remain outside the published package.
No original Python implementation or test module is bundled here.

The strict replay gate compares finding multiplicity, relative path, line, and diagnostic code.
For backend string comparisons, it also compares the column.
For dynamic loading, admin purity, and type safety, it also compares the diagnostic category.
Message text, enforcement integration, and full-suite policy remain separate integration checks.
See [the execution report](../../docs/data-layer-port-execution-plan.md).

See [the second-batch report](../../docs/data-layer-batch-two.md) for its coverage and limits.

The third batch records fact boundaries and direct AST compatibility visitors.
It preserves original parsed source, caller configuration, and analyzer/reporter distinctions.
Its supplemental corpus is `batch-03-probes.json`.
Swallowed-exception source detections and signed aggregate reporter results remain separate contracts.
See [the third-batch report](../../docs/data-layer-batch-three.md).
