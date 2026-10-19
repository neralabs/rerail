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
`data-layer/loop.ts run` over the files in `data-layer/loop-subset.json`.

The reporting clauses run in units. A unit is one workspace over the subset, holding a chunk of one policy's clauses
(`--group`, default 40) with that policy's `define`s. Units run on SQL, then on native, for `--rounds` rounds
(default 2), spread over `--jobs` child processes (default 4).

Caps:

- A chunk that passes its cap (`--group-cap`, default 40 s) is killed and split, and each of its clauses runs alone.
- A clause alone that passes its cap (`--cap`, default 20 s) is reported as a TIMEOUT row with its time. It stays
  out of later rounds.
- A clause that timed out, or that took 5 s or more in the previous run, starts alone.

The SQL program compiles, and each worker's session opens, inside the timed apply, as they do in the whole-tree run.

The verdict reports:

- **Parity:** each clause's SQL lines against native's over the subset, ignoring files where native ran out of its
  work budget.
- **Speed:** SQL and native CPU and wall time over the units both engines finished, and their ratio. It lists the
  costliest statements, clauses (by the statement time the profile gives each clause) and (clause, file)s. It also
  shows the change since the previous run on the same subset and clause set.
- **Projection:** native's whole-tree wall time times the subset's SQL/native CPU ratio, times the calibration's
  correction.
- **Program:** the whole-program SQL setup, capped by `--program-cap` (default 30 s). Every clause is compiled into
  one program, and each worker's session opens. The whole-tree run pays this before it reads any file.

A run is untrusted when:

- the 1-minute load average before it is above 6;
- its comparable rounds differ by more than 10% in CPU time;
- any clause timed out.

`--clauses a,b` keeps only the clauses whose `linter/clause` name contains one of the filters.

Native's lines over the subset (the oracle) and its whole-tree time are cached per native source hash in
`~/rerail-bench/data-layer/loop-cache/`. Each run is recorded in `~/rerail-bench/data-layer/loop-runs/`. To prefix
the timed run with a command, set `RERAIL_BENCH_LOCK=<command>`, for example a lock that serializes measurements.

`just rerail-sql-loop-tool <command>` runs the loop's other commands:

- **`select --from <subset.json>`** writes the manifest from a distilled subset: each file, the reason it is there,
  and each clause's native coverage over the subset.
- **`calibrate`** runs the latest run's units over the full corpus on SQL and native. It compares the full-corpus
  SQL/native ratio with the subset's, then writes the correction and its error into the manifest. Run it again after
  every `select`.
- **`costs`** times every subset file on SQL with every clause in one program, one file per apply, each capped by
  `--cap`. The profile gives each (clause, file) its statement time. It writes `~/rerail-bench/data-layer/loop-costs.json`.
