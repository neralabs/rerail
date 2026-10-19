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

`just rerail-sql-loop [args]` takes an edit under `native/src/code_rule_dsl/sql/` to a SQL/native CPU ratio, with
real-policy parity as a second check, in under five minutes. (Correctness strictness comes from the replay and
differential gate.) It builds the `loop` cargo profile (release optimisation without LTO, many codegen units for the
engine crate, incremental, into `target-loop/`), then runs `data-layer/loop.ts run` over the files in
`data-layer/loop-subset.json`.

The reporting clauses run in units. A unit is one workspace over the subset (one parse of each file it accepts),
holding some of one policy's clauses with that policy's `define`s. The units are decided before the run from the
costs the previous runs on the subset recorded:

- A clause whose SQL CPU reached 5 s, or that timed out, runs alone.
- Each policy's other clauses are packed, in order, into chunks of up to `--chunk` seconds of estimated SQL CPU
  (default 10). A clause no run measured counts 0.25 s.
- The units then stay those of the previous run: each unit parses the files once on each engine, which dominates
  native's time, so the SQL/native CPU ratio depends on the units, and keeping them makes runs comparable. Only a
  chunk that did not finish (TIMEOUT or ERROR) splits into its clauses, each alone. A policy whose clauses changed is
  packed afresh; `--replan` packs every policy afresh from the latest costs.

Every unit runs on SQL and on native, both engines' units in one queue, longest estimate first, over `--jobs`
child processes (default 14). A child reads the subset once and serves units until one kills it; after a unit it
takes the same unit on the other engine, so the unit's rule compiles once and both engines run it under the same
load. The running units' peak resident sets, as the previous runs measured them (4 GiB for a unit never measured),
stay within `--memory` GiB (default half the machine's memory); a child samples its resident set every 20 ms.
`--rounds` repeats the queue (default 1); later rounds leave out the units SQL did not finish.

Caps are CPU seconds (user and system, every thread of the child), so the machine's load does not move them:

- A clause alone past `--cap` (default 60 s), or a chunk past `--group-cap` (default 90 s), is killed and reported
  as a TIMEOUT row with its CPU and wall time. A unit is never split and retried within a run; the clauses of a chunk
  that timed out run alone in the next run.
- A unit the engine fails (its apply throws, for example `Unsupported(…)`) is an ERROR row with the message; the
  child goes on with the next unit. A child whose wall time passes six times its cap, plus a minute, is killed as
  stuck, and one that exits is lost: ERROR rows too. A clause whose unit failed runs alone in the next run.

The SQL program compiles, and each worker's session opens, inside the timed apply, as they do in the whole-tree run.

The verdict reports:

- **Parity:** each clause's SQL lines against native's over the subset, ignoring files where native ran out of its
  work budget.
- **Speed:** SQL and native CPU and wall time over the units both engines finished, and their ratio. It lists the
  TIMEOUT and ERROR rows, the costliest statements, clauses (by the statement time the profile gives each clause) and
  (clause, file)s. It also shows the change since the previous run on the same subset and clause set.
- **Projection:** native's whole-tree wall time times the subset's SQL/native CPU ratio, times the calibration's
  correction.
- **Program:** the whole-program SQL setup, capped by `--program-cap` (default 60 s CPU), in the same queue. Every
  clause is compiled into one program, and each worker's session opens. The whole-tree run pays this before it reads
  any file.

A run is untrusted when:

- the 1-minute load average before it is above 6;
- it ran one round, or its rounds differ by more than 10% in CPU time;
- any clause timed out or failed.

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

### The SQL engine's gate

`just rerail-sql-gate` is the correctness check to run after an edit under `native/src/code_rule_dsl/sql/`. It
builds the `loop` profile's test binary into `target-gate/`, then runs every `sql::` test. Two of them carry the
gate:

- `replayed_cases_agree` replays `native/src/code_rule_dsl/sql/tests/replay.jsonl`. Each case kills at least one
  seeded mutant, and every case must be compared and agree.
- The differential runs at the gate's case count (seed `0x5e7c0de`, 1001 cases), the smallest count whose cases
  kill every seeded mutant.

The seeded mutants are plausible regressions of the engine's hard constructs, a few lines each, listed with
`sql::mutant`. `RERAIL_SQL_MUTANT=<n>` selects one in test builds and under the never-shipped `sql-mutants`
feature. `just rerail-sql-gate --mutants` runs the replay and the differential on the clean build and under each
mutant. It fails unless the clean build passes and both checks kill every mutant.

A generator change moves the cases. To re-measure the count, run the differential under each mutant with
`RERAIL_SQL_DIFF_KEEP=<file>`, which logs every differing case with its index. The count is the largest first
index plus one. A differing case's line is a replay case.
