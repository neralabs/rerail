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

### The SQL engine's loop

`just rerail-sql-loop [args]` takes an edit under `native/src/code_rule_dsl/sql/` to a verdict in minutes. It
builds the `loop` cargo profile (release optimisation without LTO, incremental, into `target-loop/`), then runs
`data-layer/loop.ts run`. Every reporting clause runs alone, with its policy's `define`s, as
`sql::tests::census` runs it, over the files in `data-layer/loop-subset.json`. Clauses run on native and on SQL in
alternating rounds (`--rounds`, default 2), spread over `--jobs` child processes (default 3). A clause that passes
its cap (`--cap`, default 30 s) is killed and reported as a TIMEOUT row with its time, and it stays out of later rounds.
The verdict reports:

- **Parity:** each clause's SQL lines against native's, ignoring files where native ran out of its work budget.
- **Speed:** SQL and native wall and CPU time, summed per clause, and their ratio. It lists the costliest
  statements, clauses, and (clause, file)s from the `unbounded-work` build's profile, and the change since the
  previous run on the same subset and clause set.
- **Projection:** each clause's subset SQL time is scaled to the full corpus by native's full ÷ subset time for that
  clause, then multiplied by the calibration's correction.
- **Program:** the whole-program SQL setup, capped by `--program-cap` (default 30 s). This compiles every clause into
  one program and opens a first session, which the whole-tree run pays before it reads any file.

A run is untrusted when the 1-minute load average before it is above 6, when its rounds differ by more than 10% in
CPU time, or when any clause timed out. `--clauses a,b` keeps only the clauses whose `linter/clause` name contains
one of the filters. Native's per-clause lines and times are cached per native source hash in
`~/rerail-bench/data-layer/loop-cache/`. Each run is recorded in `~/rerail-bench/data-layer/loop-runs/`. Set
`RERAIL_BENCH_LOCK=<command>` to prefix the timed run, for example with a lock that serializes measurements.

`just rerail-sql-loop-tool <command>` runs the loop's other commands:

- **`costs`** times every (clause, file) on SQL, one file per apply, each capped by `--cap`. It merges the times into
  `~/rerail-bench/data-layer/loop-costs.json`.
- **`select`** writes the subset. It takes a finding file and a clean file per clause, the cheapest first, and a
  fixed-seed sample stratified by top directory and size quartile. For each hot clause it adds at most one file that
  costs more than `--file-limit`, as long as the subset's summed SQL stays within `--sql-budget`.
- **`calibrate`** runs each clause alone on SQL over the full corpus. It compares those times with the latest run's
  projection, then writes the correction factor and its error into the manifest. Run it again after every `select`.
