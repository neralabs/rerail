# Benchmarks

Development tools that compare native query engines. Nothing here ships, and nothing here runs in CI.

## NER-1562 full scan

`data-layer-full-scan.mjs` is the canonical NER-1562 benchmark. It reproduces the historical full scan that took more than 1,100 seconds. The script refuses a production-budget addon or any substitute corpus and catalog.

Build and run it with four workers:

```sh
npm run build:native:no-file-deadline
RAYON_NUM_THREADS=4 /usr/bin/time -l npm run benchmark:data-layer-full-scan -- \
  --repository /path/to/nera-data-layer-at-fce4dc7 \
  --policies /path/to/archived/data-layer-policies \
  --output /tmp/ner-1562-data-layer-full-scan.json
```

The required inputs are 12 policies, 829 clauses, 5,725 Python files, and 95,697,853 source bytes. Use this result in every NER-1562 performance report. A production-budget run is only a diagnostic and is not this benchmark.

- `query-parity.ts` (`just rerail-query-parity`) runs the same code-rule workloads through two or more native engine
  builds, each in its own process. It reports every section where their findings, failures or coverage differ.
- `query-parity-semantic.ts` names the semantic-resolution workloads (`resolves`, `usage`) that `query-parity.ts` runs.
- `history/ner-1562-historical-sql-reproduction-2026-10-16.md` records the reproduced historical SQLite slowdown.
- `history/ner-1562-sql-batching-milestones-2026-10-16.md` records the batching milestones and the new bottlenecks.
- `history/ner-1562-component-compiler-and-real-catalog-2026-10-18.md` records the component compiler and the complete data-layer run.

The workloads are host test data under `fixtures/host/benchmark/<kind>/`. Benchmark results are never committed. Keep
them out of the repository, and put headline numbers in prose (with corpus commits and tool versions) where a change
needs them. For the engine's own micro-benchmark, see `npm run bench:code-rule-dsl` (`scripts/dsl/`).
