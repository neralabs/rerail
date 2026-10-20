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
- **Mutations:** native against SQL on the recorded mutations, after the rounds (`data-layer/mutations.ts`). The
  subset only has findings for the clauses that fire on the clean tree, so for most clauses its parity covers only
  the clean side. Each recorded mutation (`<policies>/<linter>/mutations/`) makes some rule fire. For each rule, one
  child process loads the base on both engines. Each mutation is then an incremental apply of its changed files,
  compared line by line like the subset parity (native is the oracle, not the linter's record), after which the base
  is restored. `--mutations selected` (the default) runs the mutations listed in `data-layer/loop-mutations.json`
  over the subset. That is the greedy smallest set giving every clause without a subset hit its first true positive:
  159 of 437 mutations, about 50 s. `--mutations all` runs every mutation over the whole tree (over 3 minutes,
  mostly serverplan_state_tripwires). `--mutations off` skips the stage. A differing case fails the run, as a
  differing clause does. A child that fails or passes 540 s, or a selected clause that no longer gets its true
  positive (the selection is stale), is a PROBLEM.

A run is untrusted when:

- the 1-minute load average before it is above 6;
- it ran one round, or its rounds differ by more than 10% in CPU time;
- any clause timed out or failed.

`--clauses a,b` keeps only the clauses whose `linter/clause` name contains one of the filters.

Native's lines over the subset (the oracle) and its whole-tree time are cached per native source hash in
`$RERAIL_BENCH_HOME/data-layer/loop-cache/`. Each run is recorded in `$RERAIL_BENCH_HOME/data-layer/loop-runs/`. The
home defaults to `~/rerail-bench` (see below). To prefix the timed run with a command, set
`RERAIL_BENCH_LOCK=<command>`, for example `benchmark/data-layer/setup/bench-lock`, a lock that serializes
measurements on one machine.

`just rerail-sql-loop-tool <command>` runs the loop's other commands:

- **`select --from <subset.json>`** writes the manifest from a distilled subset: each file, the reason it is there,
  and each clause's native coverage over the subset.
- **`calibrate`** runs the latest run's units over the full corpus on SQL and native. It compares the full-corpus
  SQL/native ratio with the subset's, then writes the correction and its error into the manifest. Run it again after
  every `select`.
- **`costs`** times every subset file on SQL with every clause in one program, one file per apply, each capped by
  `--cap`. The profile gives each (clause, file) its statement time. It writes `$RERAIL_BENCH_HOME/data-layer/loop-costs.json`.
- **`mutations`** runs the mutation stage alone, by default `--mutations all` (every mutation over the whole tree).
  It records every case in `$RERAIL_BENCH_HOME/data-layer/mutation-runs/`. With `--select`, it runs every mutation over
  the subset and rewrites `loop-mutations.json` from what native found. Re-select after a policy, mutation or subset
  change. The clauses that already have a finding file in the subset are those with a `finding` in the manifest;
  `--cover <cover.json>` takes them from a distillation's per-clause coverage (`hit`) instead.

### Running the loop on a fresh machine

The loop reads its corpus and policies from a benchmark home, `RERAIL_BENCH_HOME` (default `~/rerail-bench`), and
records its runs there. Both are internal, so neither is in the repository. `just rerail-bench-setup` lays the home
out from a nera-data-layer clone and a nera-landing clone you already have:

```sh
just rerail-bench-setup --data-layer ~/repos/nera-data-layer --nera-landing ~/repos/nera-landing [--home <dir>]
```

It fetches from each clone's `origin` only the commit it lacks, then:

- **Corpus:** adds `nera-data-layer-fce4dc72/`, a detached worktree of the nera-data-layer clone at
  `loop-subset.json`'s corpus commit. The clone's own checkout is not touched. An existing one must be at that commit
  and clean, since the loop reads the working tree.
- **Addon:** runs `npm ci` when `node_modules/` is missing, and builds the `loop` addon into `target-loop/` when it is
  missing, as `just rerail-sql-loop` does. `--addon <path>` uses another build for the conversion instead.
- **Policies:** `data-layer/setup/convert.mts` exports `tempo-oracles/data-layer/` from nera-landing's
  `Moti-Velz/tempo-data-layer-benchmark` at `aa63f36b5`, the commit the loop's policies were converted from, into
  `data-layer/policies/<linter>/`. It copies each linter's oracle files verbatim. It stores each `policy.json` as
  Rerail's own check, `validateCodeRule`, returns it, compiled on the addon, and writes
  `data-layer/conversion-report.json`.
- **Check:** `data-layer/setup/verify.mts` requires every oracle file to have the commit's git blob id, the report to
  convert all 12 linters, and each `policy.json`'s sha256 to equal its `policies` entry in `loop-subset.json` (the
  hash `select` records, over the file's bytes). A policy hash differs when this checkout stores a rule differently
  from the checkout the subset was selected with; the subset's coverage is then stale, so re-run `select` and
  `calibrate`.

A home that already passes the check is left as it is, so running the recipe again only checks. It ends by printing
the loop and gate commands to run next, with `export RERAIL_BENCH_HOME=<dir>` first when the home is not the default.
The first loop run in a home measures native over the subset and the whole tree once; later runs read that from
`loop-cache/`.

The loop does not need the distillation that chose the subset. Its tools are kept for re-selecting:
`data-layer/setup/cover.py` writes `$RERAIL_BENCH_HOME/distill/cover.json`, each clause's finding and clean cover files,
from the home's corpus and policies and a whole-tree and profile run (`data-layer/out/baseline/`,
`data-layer/out/profile/`). Its helper, `data-layer/setup/lits/`, extracts regex literals as native does; build it with
`cargo build --release --manifest-path benchmark/data-layer/setup/lits/Cargo.toml`. `data-layer/setup/merge-subset.py`
merges `cover.json` with the construct census (`$RERAIL_BENCH_HOME/distill/constructs.json`: each hard construct's
hit and clean files) into `distill/subset.json` and `subset.txt`, and `select --from <subset.json>` then writes the
manifest. The script that produced the construct census was not kept; re-selecting without it uses `cover.json` alone.

### The SQL engine's gate

`just rerail-sql-gate` is the correctness check to run after an edit under `native/src/code_rule_dsl/sql/`. It
builds the `loop` profile's test binary into `target-gate/`, then runs every `sql::` test. Two of them carry the
gate:

- `replayed_cases_agree` replays `native/src/code_rule_dsl/sql/tests/replay.jsonl`. Each case kills at least one
  seeded mutant, and every case must be compared and agree.
- The differential runs at the gate's case count (seed `0x5e7c0de`, 1239 cases), the smallest count whose cases
  kill every seeded mutant.

The seeded mutants are plausible regressions of the engine's hard constructs, a few lines each, listed with
`sql::mutant`. `RERAIL_SQL_MUTANT=<n>` selects one in test builds and under the never-shipped `sql-mutants`
feature. `just rerail-sql-gate --mutants` runs the replay and the differential on the clean build and under each
mutant. It fails unless the clean build passes and both checks kill every mutant.

A generator change moves the cases. To re-measure the count, run the differential under each mutant with
`RERAIL_SQL_DIFF_KEEP=<file>`, which logs every differing case with its index. The count is the largest first
index plus one. A differing case's line is a replay case.

## Many workspaces: peak memory

`workspaces/` (`just rerail-workspaces <command>`) measures one process serving many repositories in turn, on the
native or the SQL engine (`--engine native|sql`): its peak resident set above all, which G3's memory condition bounds
(`docs/sql-set-composition-design.md`). It ports Tempo's `benchmark:rerailer` (nera-landing #2622), which measured
native's 564 MiB peak over 47 nera-data-layer workspaces in about 26 minutes.

The 47 workspaces are nera-data-layer checkouts at 47 commits. `SUBSET` in `workspaces/workspaces.ts` fixes eight of
them, numbers 1 + round(k × 46 / 7), from the largest (01, 5,726 Python files) to the smallest (47, 5,491), with their
commits; each engine runs them in a minute and a half with the #2622 rules. `setup --data-layer <nera-data-layer
clone> --workspaces <dir>` adds them as worktrees of the clone under `<dir>`; `run` checks each is clean at its commit
and `--only 01,47` runs some of them.

`run --engine native|sql --rules <dir> [--rules <dir> …] --workspaces <dir> --output <file.json>` compiles every
`rerail.policy/v1` document under the rule directories (recursively; other JSON is skipped) once, then for each
workspace in turn, in one process:

1. a fresh workspace on the engine registers every rule (on SQL, this compiles the program);
2. the full scan, timed: the enforcer's stored repository audit. A content store in a temporary directory attaches,
   the Git checkout is indexed chunk by chunk (`indexRepositoryAsync` with the enforcer's bounds), the final apply runs
   quietly, the revision is recorded and every result page is read. `--store off` scans as the enforcer does where no
   store attaches instead: batches of 32 files, closed at 1 MiB, each with the workspace-wide pass deferred;
3. `--edits` edits (default 20), each appending a comment to the Python file at the minimum, median, p95 or maximum
   size in turn, applied as the enforcer applies one (inline through `tryApply` below 4,096 characters, else
   `applyBufferAsync`), each followed by `evictInactive`;
4. `evictInactive` and `--idle-ms` of idle time (default 5,000), and the workspace closes.

A worker thread samples the process's resident set every 10 ms, so a peak inside a long synchronous native call
(a rule's compile, `addCodeRule`) is seen; `getrusage`'s high-water mark is reported beside it. Per workspace the
output has the full scan's wall and CPU time, findings and failure codes; the edits' wall and CPU p50 and p95; the
peak by phase and the resident set after the scan, after the edits (final) and after evicting and idling; at each
point the Node heap, the addon's allocator (`nativeMemoryProfile`) and the workspace's `memoryProfile`; and the engine
report, whose SQL `program` and `sessions` (setup, statements prepared, and the heap the pooled sessions hold:
`schemaBytes`, `statementBytes` and `cacheBytes` of their SQLite connections, `frameBytes` of their frames) attribute
the SQL engine's share. SQLite allocates through the system allocator, so its heap is in the resident set but not in
the addon's allocator.

`compare <native.json> <sql.json>` prints the two runs side by side: the run's peak, the resident set after the
compile and at the end, edit percentiles over every workspace, and one row per workspace.

The rules #2622 measured are internal, so they stay outside the repository: the 166 bundled Tempo rules and the three
cross-file workloads E1–E3 of nera-landing `697bd2bd7`. Extract them once into a directory to pass as `--rules`:

```sh
git -C <nera-landing> archive 697bd2bd7 tempo/policies/executable-policy-dsl/rules | tar -x -C <tmp>
cp -R <tmp>/tempo/policies/executable-policy-dsl/rules <dir>/catalog
for e in e1-component-story e2-restricted-api e3-usage-budget; do
  git -C <nera-landing> show 697bd2bd7:tempo/benchmark/crossfile/$e.json > <dir>/crossfile/$e.json
done
```

Most of those rules scope `nera-data-layer/…` paths of a landing checkout and so read nothing in a data-layer
checkout, as in #2622. The twelve ported data-layer linters (`--rules $RERAIL_BENCH_HOME/data-layer/policies`) are
the SQL engine's own workload; they need `--store off` while the content store refuses their publications (more
than 10 relationship statements per batch).
