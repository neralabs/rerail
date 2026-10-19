# Benchmarks

Development tools that measure Rerail: native query engines against each other, and the dashboard's history reads.
Nothing here ships, and nothing here runs in CI.

- `query-parity.ts` (`just rerail-query-parity`) runs the same code-rule workloads through two or more native engine
  builds, each in its own process. It reports every section where their findings, failures or coverage differ.
- `query-parity-semantic.ts` names the semantic-resolution workloads (`resolves`, `usage`) that `query-parity.ts` runs.
- `dashboard-history.ts` (`just rerail-dashboard-history`) seeds a throwaway database with a long audit history (`--audits`,
  `--rules`, `--backlog`, `--sessions`), serves the daemon's API over it, and times what the dashboard reads: each history
  read computed and repeated, one poll while a session keeps committing, recording one repository audit, and how long
  `/healthz` waits while dashboard tabs poll (`--tabs`, `--seconds`).

The workloads are host test data under `fixtures/host/benchmark/<kind>/`. Benchmark results are never committed. Keep
them out of the repository, and put headline numbers in prose (with corpus commits and tool versions) where a change
needs them. For the engine's own micro-benchmark, see `npm run bench:code-rule-dsl` (`scripts/dsl/`).
