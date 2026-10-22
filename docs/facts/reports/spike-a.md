# Spike A report: facts once + in-memory relational evaluation (batch 6)

Status: **incomplete** (milestones: census, evaluator, recording parity). Gates A2–A5 are not measured yet; the run stopped at the request budget. § 7 lists exactly what remains. This file is updated at every milestone; § 7 says what is done and what is next.

## 1. Setup

| Item | Value |
|---|---|
| Branch | `Moti-Velz/facts-spike-a` (worktree `~/orca/workspaces/rerail/facts-spike-a`), base `fba987925` |
| Crate | `spikes/facts-a` (workspace member; depends on `rerail-engine` as an rlib for the tape parser only) |
| Production change | visibility only: `native/Cargo.toml` adds `rlib`; `code_rule_dsl::{eval,lines,model,tape,language,package_manifest}` and the tape/grammar/file-input items the spike reads are `pub` (commit `baca38d83`). No behaviour change. |
| Corpus | `~/rerail-bench/nera-data-layer-2b1504ad` (9,010 files) and `fixtures/host/data-layer-port/batch-06-{recording,probes}.json` |
| Policies | `packs/data-layer/candidates/rules/data-layer-*.json` for the 10 batch-6 checks, unmodified |
| Build | `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=spikes/facts-a/target cargo build --release -p facts-a` |

## 2. Results

| Gate | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| A1 parity, source corpus (1,259 recorded calls) | 0 FP, 0 FN, 0 failures | 0 FP, 2 FN, 6 failures. 1,253/1,259 calls match; every mismatch is `serverplan_quality` / `quality-contracts-1` refusing the `annotation` sub-grammar | **fail** (9/10 checks pass) | `~/rerail-bench/facts/spike-a/runs/20261022T120000-replay` |
| A1 parity, repository (173 findings, 9,010 files) | 0/0/0 | not run: the `repo` command is a stub | not measured | — |
| A2 repository CPU | ≤ 40 s | not measured | not measured | — |
| A3 per (clause, file) | mean ≤ 0.5 ms, p95 ≤ 5 ms | not measured (the timers exist in `eval::Output.clause_samples`) | not measured | — |
| A4 extraction | p50 ≤ 10 ms, p99 ≤ 100 ms | not measured | not measured | — |
| A5 peak RSS | ≤ 2 GiB | not measured | not measured | — |

Per check (source corpus): `execution_authority_bypass` 5/5, `impl_direct_execution` 68/68, `legacy_execution_surfaces` 80/80, `raw_execute_regression` 4/4, `serverplan_bypass` 6/6, `serverplan_dispatch_architecture` 59/59, `serverplan_state_tripwires` 79/79, `serverplan_steps` 746/746, `serverplan_target_architecture` 12/12. `serverplan_quality` matches 194/200 calls.

Untimed observation, not a gate number: one process replaying all 1,259 calls (parse + facts + evaluation, `loop` profile, one thread) took about 37 s wall in `~/rerail-bench/facts/spike-a/runs/20261022T120000-replay/stdout.txt`. This is not comparable to the native 393 s repository figure: the corpus and the build profile differ, and no `bench-lock` was used.

Kill criteria: none can be evaluated yet. A2 and A4 are unmeasured. The A1 gap is a schema gap, not a wrong v0 relation: see § 6.

## 3. How measured

- Build: `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=spikes/facts-a/target cargo build --profile loop -p facts-a`. `loop` is the workspace's release profile without LTO. Gate timings must use `--release`.
- A1 source corpus: `~/rerail-bench/facts/spike-a/runs/20261022T120000-replay/command.txt` (`facts-a replay --recording .../batch-06-recording.json --recording .../batch-06-probes.json --output results.json`). The comparator is `main.rs::replay`, and it follows `replay.ts`:
  - The config per event is `oracleAnalyzer = !production_scope`, merged with `oracle_config.options`.
  - The event scope replaces the policy scope unless `production_scope` is set.
  - Events with a `retained_reason`, `upstream_passed=false` or no `files` are skipped.
  - Findings are compared as multisets of `[path, line, code]`.

## 4. Parity artifacts

- `~/rerail-bench/facts/spike-a/runs/20261022T120000-replay/results.json` holds the per-check counts and every mismatching call with its FN, FP and failures.

## 5. Design notes

### 5.1 Stage census (step 1)

Every unit of the ten policies (`tools/census.py`, options resolved to their defaults) is classified as:

- **(a)** AST-schema relation that becomes an extracted fact: the node, edge, ownership, descendant and ordinal tables the policies rebuild in SQL (`syntax_rows`, `nodes`, `edges`, `ownership`, `descendants`), the Python-derived relations built on them (`functions`, `classes`, `methods`, `calls`, `arguments`, `keywords`, `attributes`, `assignments`, `parameters`, `returns`, `decorators`, ...), the `syntax` spec tables (`Facts::build`), and AST inputs that only dump node rows (kind/regex/gate filter plus field captures).
- **(b)** rule logic derivable from facts: joins, antijoins, recursion, aggregates, string predicates, and AST inputs that are structural predicates (`has`/`inside`/`field`/`kind`/`regex`).
- **(c)** bespoke, needs a fact not in v0: decoded Python string values (`value`, the `DECODED` transform), the `pythonFString` and `annotation` sub-grammars (`parse`), and regex clauses over raw text with Python `splitlines` numbering.

| rule | clauses | relational | forbid | budget | AST inputs | SQL stages | (a) schema | (b) logic | (c) bespoke |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| `serverplan-bypass` | 3 | 0 | 0 | 3 | 0 | 0 | 0 | 3 | 0 |
| `serverplan-steps` | 21 | 7 | 14 | 0 | 22 | 132 | 82 | 82 | 11 |
| `serverplan-state-tripwires` | 32 | 32 | 0 | 0 | 139 | 97 | 154 | 98 | 16 |
| `serverplan-dispatch-architecture` | 9 | 1 | 8 | 0 | 25 | 22 | 40 | 13 | 3 |
| `serverplan-quality` | 7 | 6 | 1 | 0 | 31 | 106 | 77 | 64 | 6 |
| `serverplan-target-architecture` | 16 | 1 | 15 | 0 | 10 | 14 | 18 | 21 | 1 |
| `execution-authority-bypass` | 1 | 1 | 0 | 0 | 5 | 7 | 4 | 8 | 1 |
| `impl-direct-execution` | 1 | 1 | 0 | 0 | 4 | 3 | 5 | 3 | 0 |
| `raw-execute-regression` | 7 | 0 | 7 | 0 | 0 | 0 | 0 | 7 | 0 |
| `legacy-execution-surfaces` | 8 | 1 | 7 | 0 | 27 | 23 | 38 | 17 | 3 |
| **total** | 105 | 50 | 52 | 3 | 263 | 404 | 418 | 316 | 41 |

Bespoke (c) units:

- `serverplan-steps` / `side-effect-tags` / forbid `side-effect-tags`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `functions`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `calls`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `positional_edges`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `keyword_edges`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `container_edges`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `dictionary_edges`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `spread_edges`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `returns`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `bindings`: needs value
- `serverplan-steps` / `runtime-reference-summaries` / input `postops`: needs value
- `serverplan-state-tripwires` / `queue-udf-symbols` / input `events`: needs value
- `serverplan-state-tripwires` / `serverplan-ddl` / input `events`: needs value
- `serverplan-state-tripwires` / `private-lazy-ref` / input `events`: needs value
- `serverplan-state-tripwires` / `state-flow-1` / input `raw_strings_0`: needs transform/value
- `serverplan-state-tripwires` / `state-flow-1` / input `raw_strings_1`: needs transform/value
- `serverplan-state-tripwires` / `state-flow-1` / input `raw_strings_2`: needs transform/value
- `serverplan-state-tripwires` / `state-flow-1` / stage `string_values`: decoded string values / module text
- `serverplan-state-tripwires` / `state-projection-recovery-2` / input `raw_strings_0`: needs transform/value
- `serverplan-state-tripwires` / `state-projection-recovery-2` / input `raw_strings_1`: needs transform/value
- `serverplan-state-tripwires` / `state-projection-recovery-2` / input `raw_strings_2`: needs transform/value
- `serverplan-state-tripwires` / `state-projection-recovery-2` / stage `string_values`: decoded string values / module text
- `serverplan-state-tripwires` / `state-flow-project-1` / input `raw_strings_0`: needs transform/value
- `serverplan-state-tripwires` / `state-flow-project-1` / input `raw_strings_1`: needs transform/value
- `serverplan-state-tripwires` / `state-flow-project-1` / input `raw_strings_2`: needs transform/value
- `serverplan-state-tripwires` / `state-flow-project-1` / stage `string_values`: decoded string values / module text
- `serverplan-state-tripwires` / `state-flow-project-1` / stage `builder_tokens`: decoded string values / module text
- `serverplan-dispatch-architecture` / `backend-identity` / forbid `backend-identity`: needs value
- `serverplan-dispatch-architecture` / `step-name-compare` / forbid `step-name-compare`: needs value
- `serverplan-dispatch-architecture` / `dialect-compare` / forbid `dialect-compare`: needs value
- `serverplan-quality` / `quality-contracts-1` / input `raw_strings`: needs transform/value
- `serverplan-quality` / `quality-contracts-1` / input `raw_fstring_parts`: needs parse
- `serverplan-quality` / `quality-contracts-1` / input `quoted_atoms`: needs parse
- `serverplan-quality` / `quality-contracts-1` / stage `module_string_values`: decoded string values / module text
- `serverplan-quality` / `quality-local-taxonomy` / input `candidates`: needs parse/value
- `serverplan-quality` / `quality-project-taxonomy` / input `candidates`: needs parse/value
- `serverplan-target-architecture` / `strategy-getattr` / forbid `strategy-getattr`: needs value
- `execution-authority-bypass` / `authority-sites` / input `monkeypatches`: needs value
- `legacy-execution-surfaces` / `post-fetch-filter_query_results` / forbid `post-fetch-filter_query_results`: regex over raw text with Python splitlines
- `legacy-execution-surfaces` / `post-fetch-filter_query_results_async` / forbid `post-fetch-filter_query_results_async`: regex over raw text with Python splitlines
- `legacy-execution-surfaces` / `graph-fallback-text` / forbid `graph-fallback-text`: regex over raw text with Python splitlines

SQL constructs (number of stages + final queries using each):

| construct | statements |
|---|--:|
| join | 273 |
| subquery (scalar or derived table) | 176 |
| string ops (|| substr instr trim lower upper replace length char like) | 173 |
| exists / not exists (semi/anti join) | 99 |
| window row_number | 83 |
| union (set) | 81 |
| union all | 71 |
| recursive cte | 58 |
| regex fn (rx_*) | 58 |
| left join | 55 |
| distinct | 38 |
| case | 35 |
| group by / aggregate | 24 |
| json_each / json_extract | 15 |
| cast | 8 |
| order by ... limit | 8 |
| not in / in (subquery) | 4 |

Reading the census:

- 418 of 775 units (54%) re-derive the syntax tree. The policies carry their own AST database in SQL because the native engine has no shared one; in the facts engine these are extracted once per content.
- 41 units (5%) need facts outside v0. All are per-node pure functions of the source (string decoding, f-string and annotation parsing, line splitting), so they become derived per-node facts, not cross-file state.
- Phase 2's compiler needs the constructs in the last table: joins and left joins, correlated `EXISTS`/`NOT EXISTS` and scalar subqueries (with `ORDER BY ... LIMIT 1`), set and bag unions, recursive CTEs (both `UNION` and `UNION ALL`), `row_number()` windows, `GROUP BY` with `count`/`min`/`max`/`group_concat`, `DISTINCT`, `CASE`, `CAST`, string functions, `json_each`/`json_extract`, and Rust-regex functions.

### 5.2 Evaluator choice

The evaluator is not `ascent`. The batch-6 rules are 544 KB of SQL in 404 stages plus 263 AST queries. The SQL uses 58 recursive CTEs, 83 `row_number()` windows, 99 correlated `EXISTS` and 176 subqueries. Hand-translating that to compile-time Datalog could not be verified exactly in the time available. It would also bias the A2 number in exactly the way the scratchpad's open question worries about. The spike therefore implements Phase 2's front end directly, and runs it at run time:

- **Facts** (`facts.rs`): built from the native `TapeDoc` (visibility-only reuse). Node ids, kinds, fields and spans are identical to the engine's: preorder over every node, named and anonymous. The relations are:
  - `node(kind, parent, start, end, start_line, end_line, subtree_end, flags, field)`
  - a children CSR
  - kind postings
  - the line table

  Persistence in the agreed fact-file format (agreed with SpikeB: magic `RRFACTS\0`, schema version, blake3 hash, string table, columnar relations; SpikeB writes `.xfacts` siblings) is **not written yet**.
- **AST queries** (`ast.rs`): a port of the native matcher's semantics over the facts. It covers first-success binding, relations with `field` and `stopBy`, three-valued `all`/`any`/`not`, `each` enumeration with exclusion, `nthChild`, `count`, `compare`, `value` and `parse: pythonFString`. Identical evidence queries are evaluated once per file per policy.
- **Derived (c) facts:**
  - `pyvalue.rs`: Python string decoding. Operators, calls and containers are refused.
  - `fstring.rs`: f-string pieces with host spans.
  - Both are ports of the native `value/python.rs` and `literal/{python_fstring,offsets}.rs`.
- **SQL** (`sql/`): an own lexer and parser for the SQLite subset. `sqlparser` 0.58 failed on 15 of 452 statements: `trim(x, chars)`, `AS MATERIALIZED`, `CROSS JOIN ... ON`, and others. The plan is fixed by the text:
  - FROM order, index nested loops;
  - equality conjuncts against earlier items, outer rows or constants probe hash indexes built lazily once per table;
  - other conjuncts are placed at the earliest bound level;
  - correlated subqueries probe the same indexes, and uncorrelated ones are cached per statement;
  - recursive CTEs run level by level (FIFO-equivalent);
  - SQLite affinity and comparison rules; CTAS affinity conversion.

  There is no SQLite and no per-statement planner at run time.
- **Relational clauses** (`rel.rs`):
  - input tables: `__path, __start, __end` plus projected fields in name order;
  - `syntax` spec tables built from facts;
  - deferred columns and all three reachability variants;
  - final-row anchoring (input anchors, then syntax spans, then `locations`);
  - duplicate-identity failure;
  - repository partitions run once per evaluation, including with zero files.

## 6. Risks found

- **`parse: { grammar: annotation }`** (quality-contracts-1 `quoted_atoms`) depends on Ruff 0.16.9's string-type-definition classification (`scope/python/typedefs.rs`), which needs the scope table's qualified-name resolution. That is a cross-node semantic fact outside v0. The spike refuses it with a failure rather than approximating it, which accounts for all of the A1 gap. Phase 2 needs either a `string_type_definition(host_id)` derived fact from a port of that classifier, or a narrowed scope for quality-contracts-1.
- **Row-order-sensitive SQL**: `row_number()` ties, `LIMIT 1` with partial `ORDER BY`, and `group_concat` without an ordered subquery depend on SQLite's plan. The spike breaks ties by input order. No replayed call diverged from this.
- `json_each` over an object enumerates keys sorted (serde_json without `preserve_order`), whereas SQLite enumerates them in text order. This is only used for set-valued lookups in the batch-6 rules.

- The replay harness evaluates every recorded call with `oracleAnalyzer=true`, `entryPoint=<event entry>` and per-event extra options, and replaces the policy scope; the repository suite uses the defaults and production scopes. The spike resolves options per call exactly as `replay.ts` does.

## 7. Resume notes

- Commits:
  - `baca38d83`: visibility only, cherry-pickable by SpikeB.
  - `0f3e1db49`: census.
  - `19d56c1c1`: evaluator and replay parity.
- Done: census; fact builder; the full evaluator; the source-corpus comparator with 1,253/1,259 calls matching.
- Next, in order:
  1. `repo` command (`main.rs::repo` is a stub). Walk `~/rerail-bench/nera-data-layer-2b1504ad` with 4 threads, each with its own compiled rules. Use the default config and production scopes. Merge `eval::Output` and `RepoAcc`, run `eval::finish`, and compare with `~/rerail-bench/facts/spike-a/work/suite-b6.json` (173 findings), keyed as `[check, path, line, code]`. Record the per-file extraction ns (A4) and `clause_samples` (A3). Wrap the run in `~/rerail-bench/bench-lock /usr/bin/time -l` with `--release` (A2, A5).
  2. Fact-file writer and reader (format above) plus bytes per file (A4).
  3. Annotation classifier (§ 6), or narrow quality-contracts-1's scope and recompute the ≥ 70% CPU apportioning from `~/rerail-bench/dl60-split/b6-*.csv`.
- Half-done: `facts::name` clones strings and `ast::Eval::children` allocates. Both are performance debt to fix before timing.
- Native semantics specs used by the port (read from source by read-only scouts): `~/rerail-bench/facts/spike-a/work/spec-{ast,rel,clause}.md`.
