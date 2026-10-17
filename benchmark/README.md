# Benchmarks

Development tools that compare native query engines. Nothing here ships, and nothing here runs in CI.

- `query-parity.ts` (`just rerail-query-parity`) runs the same code-rule workloads through two or more native engine
  builds, each in its own process. It reports every section where their findings, failures or coverage differ.
- `query-parity-semantic.ts` names the semantic-resolution workloads (`resolves`, `usage`) that `query-parity.ts` runs.

The workloads are host test data under `fixtures/host/benchmark/<kind>/`. Benchmark results are never committed. Keep
them out of the repository, and put headline numbers in prose (with corpus commits and tool versions) where a change
needs them. For the engine's own micro-benchmark, see `npm run bench:code-rule-dsl` (`scripts/dsl/`).

## nera-data-layer

`data-layer/` (`just rerail-data-layer <command>`) runs twelve ported nera-data-layer linters over that repository's
5,725 tracked Python files. The rules, the linters' recorded oracles and the corpus are internal, so they stay outside
the repository: `--policies <dir>` holds one `<linter>/` directory each (`policy.json`, `manifest.json`,
`tree.expected.json`, optional `unsupported.json`, `mutations/`), and `--corpus <dir>` is a clean nera-data-layer
checkout at the oracles' commit. `--addon <path>` chooses the native build, so two builds can be A/B'd.

- `whole-tree --output <dir> [--runs N]` evaluates every rule over the whole corpus in one `applyAsync`, in a fresh
  process per run. It prints the apply's wall and process CPU time (all four engine workers), and counts findings,
  failures and blind files. It writes the sorted violation and failure lines to `whole-tree-<run>.lines.txt` and their
  sha256 is the run's digest. It also writes the blind files to `whole-tree-<run>.blind.txt` and everything to
  `whole-tree.json`. It exits 1 when the runs' digests differ.
  `--engine sql [--sql-batch N]` matches the AST clauses on the set-based SQL engine
  (`docs/sql-set-composition-design.md`), N files per batch (default 64), through the workspace option
  `new CodeRuleDslWorkspace({ engine: "sql", sqlBatchSize })`. A clause the engine refuses fails on every file it reads
  with `sql-unsupported`; the run prints each refused clause and its reason, and how many clauses compiled and ran. The
  lines of the clauses that ran go to `whole-tree-<run>.ran.txt`, with their own digest, to compare with native's lines
  for the same clauses.
- `profile --output <dir> [--jobs N] [--repeat N]` times single-file applies for a parse and a text baseline, all rules,
  and each rule alone. `profile.md` ranks rules by their own cost and lists the slowest files and those over 500 ms.
- `conformance` compares each rule with its linter's recorded findings over the tree and each mutation. Codes the
  rule omits or matches only partly are declared in `unsupported.json`.

Production builds bound each file's shared work and each clause's work (`CLAUSE_WORK_BUDGET`,
`native/src/code_rule_dsl/workspace/mod.rs`), so a slow file reports `clause work budget exceeded` instead of its
cost. `npm run build:native:unbounded-work` builds `target/unbounded-work/rerail_engine.<platform>.node` with the
`unbounded-work` cargo feature. That feature lifts those allowances, and `health().kernelVersion` then ends in
`+unbounded-work`. The harness refuses any other addon unless given `--allow-limits`. The capability limits
(`maxVisitedNodes` and the rest) still apply; `--capabilities <json>` compiles the rules against other ones.
