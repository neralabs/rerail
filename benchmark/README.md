# Benchmarks

Development tools that compare native query engines. Nothing here ships, and nothing here runs in CI.

- `query-parity.ts` (`just rerail-query-parity`) runs the same code-rule workloads through two or more native engine
  builds, each in its own process. It reports every section where their findings, failures or coverage differ.
- `query-parity-semantic.ts` names the semantic-resolution workloads (`resolves`, `usage`) that `query-parity.ts` runs.
- `history/ner-1562-historical-sql-reproduction-2026-10-16.md` records the reproduced historical SQLite slowdown.
- `history/ner-1562-sql-batching-milestones-2026-10-16.md` records the batching milestones and the new bottlenecks.
- `history/ner-1562-component-compiler-and-real-catalog-2026-10-18.md` records the component compiler and the complete data-layer run.

The workloads are host test data under `fixtures/host/benchmark/<kind>/`. Benchmark results are never committed. Keep
them out of the repository, and put headline numbers in prose (with corpus commits and tool versions) where a change
needs them. For the engine's own micro-benchmark, see `npm run bench:code-rule-dsl` (`scripts/dsl/`).
