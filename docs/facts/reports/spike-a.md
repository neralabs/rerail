# Spike A report: facts once + in-memory relational evaluation (batch 6)

Status: **complete; gates measured.**

- A1 fails on quality-contracts-1 only. Its `annotation` refusal is excluded by decision; every other check has 0 false positives, 0 false negatives and 0 failures, on both corpora.
- A4 passes.
- A2, A3 (mean) and A5 fail.
- The kill criteria are evaluated in § 2.

## 1. Setup

| Item | Value |
|---|---|
| Branch | `Moti-Velz/facts-spike-a` (worktree `~/orca/workspaces/rerail/facts-spike-a`), base `fba987925` |
| Commit of the gate runs | `96168e00a`. Later commits change only the fact-file writer and reader, not `repo`. |
| Crate | `spikes/facts-a` (workspace member; depends on `rerail-engine` as an rlib, for the tape parser and `LineTable` only) |
| Production change | Visibility only: `native/Cargo.toml` adds `rlib`; the spike reads `code_rule_dsl::{eval,lines,model,tape,language,package_manifest}` and the tape and grammar items, made `pub` (`baca38d83`); `lines::{Terminators, LineTable::new, start, end}` are `pub` (`fb46a8bd1`). No behaviour change. |
| Corpus | `~/rerail-bench/nera-data-layer-2b1504ad`: 9,010 `.py` files, 137.9 MB, the oracle's exact file set (`repo-compare` checks it), and `fixtures/host/data-layer-port/batch-06-{recording,probes}.json` |
| Policies | `packs/data-layer/candidates/rules/data-layer-*.json` for the 10 batch-6 checks, unmodified |
| Build | `export CARGO_BUILD_JOBS=4; CARGO_TARGET_DIR=spikes/facts-a/target cargo build --release -p facts-a` |
| Machine | darwin 25.6 arm64. 1-minute load at the start and end of the gate runs: 3.81–4.45, under 8, so every gate run is trusted. 4 workers. |
| Excluded | `quality-contracts-1`'s `annotation` sub-grammar, by decision: it records a failure and is excluded from A2 (ii) |

## 2. Results

| Gate | Target | Measured | Pass/fail | Run dir (`~/rerail-bench/facts/spike-a/runs/…`) |
|---|---|---|---|---|
| A1 parity, source corpus (1,259 recorded calls) | 0 FP, 0 FN, 0 failures | 0 FP, 2 FN, 6 failures. 1,253/1,259 calls match. All 6 mismatching calls are `serverplan_quality` / `quality-contracts-1` refusing `annotation`. | **fail**: 9/10 checks pass | `20261022T131243-replay` |
| A1 parity, repository (173 findings, 9,010 files) | 0/0/0 | 0 FP, 0 FN, 38 failures. 173/173 findings match. All 38 failures are `quality-contracts-1` refusing `annotation`; no other check has a miss. | **fail**: 9/10 checks pass; quality-contracts-1 is the only miss | `20261022T130531-repo/parity.json` (identical in `…130638-repo`, `…130749-repo`) |
| A2 (i), all 10 rules as executed | ≤ 10% of native: ≤ 39.3 s of 393 s | 212.9 s CPU (user + sys), median of 211.5 / 212.9 / 222.7 s; 54.2% of native. Wall 67.6 s (66.9–70.3). | **fail** | `20261022T130531-repo` (median), `…130638-repo`, `…130749-repo`; `a2.json` in `…130749-repo` |
| A2 (ii), quality-contracts-1 excluded on both sides | ≤ 10% | 188.2 s facts vs 364.7 s native: 51.6%. Apportioning in § 3. With the full-probe sensitivity: 52.0%. | **fail** | same |
| A3 per (clause, file) | mean ≤ 0.5 ms, p95 ≤ 5 ms | mean 0.79 ms, p95 1.30 ms, p50 0.012 ms, p99 12.1 ms, max 1.57 s (228,685 samples) | **fail** (mean); p95 passes | `20261022T130531-repo` (other runs: mean 0.83 / 0.79 ms, p95 1.36 / 1.30 ms) |
| A4 extraction | p50 ≤ 10 ms, p99 ≤ 100 ms; bytes per file reported | Parse + extract: p50 0.53 ms, p99 10.5 ms, max 74.9 ms. Extraction from the tape alone: p50 0.045 ms, p99 1.06 ms. Fact-file bytes per file: p50 51.0 KB, p95 407 KB, p99 1.02 MB, mean 116 KB, 1.05 GB in total (8.3× the source at p50). In memory: p50 57 KB. Fact-file round trip: 0 mismatches over 9,010 files. | **pass** | `20261022T130531-repo`, `20261022T131227-store` |
| A5 peak RSS, 4 workers | ≤ 2 GiB | 8.46 GB (7.88 GiB); 8.44–8.46 GB across runs. 3.57 GB at the end of the file phase; the repository partitions (`eval::finish`) raise it to the peak. | **fail** | `20261022T130531-repo` (`max_rss_bytes` in `results.json`, `/usr/bin/time -l` in `stderr.txt`) |

Per check, source corpus: `execution_authority_bypass` 5/5, `impl_direct_execution` 68/68, `legacy_execution_surfaces` 80/80, `raw_execute_regression` 4/4, `serverplan_bypass` 6/6, `serverplan_dispatch_architecture` 59/59, `serverplan_state_tripwires` 79/79, `serverplan_steps` 746/746, `serverplan_target_architecture` 12/12. `serverplan_quality` matches 194/200 calls.

Per check, repository: `impl_direct_execution` 173/173. The other nine checks expect 0 findings and report 0. The only failures are `serverplan_quality` / `quality-contracts-1` (38).

Per rule, native (apportioned, § 3) vs facts. Facts here is evaluation thread time per rule from the median run. Parse and extraction (10.7 s), rule compilation (0.33 s per worker, 1.3 s in all, plus 0.6 s on the merging thread) and the process overhead are shared and not apportioned.

| rule | native global (s) | native file-phase share (s) | native total (s) | facts evaluation (s) |
|---|--:|--:|--:|--:|
| `serverplan-state-tripwires` | 148.5 | 64.9 (139 inputs) | 213.4 | 84.9 |
| `serverplan-quality` | 38.7 | 14.5 (31 inputs) | 53.2 | 83.4 |
| `legacy-execution-surfaces` | 27.0 | 12.6 (27 inputs) | 39.6 | 15.6 |
| `serverplan-steps` | 23.6 | 10.3 (22 inputs) | 33.9 | 12.3 |
| `serverplan-dispatch-architecture` | 0.6 | 11.7 (25 inputs) | 12.2 | 0.9 |
| `execution-authority-bypass` | 4.0 | 2.3 (5 inputs) | 6.4 | 0.9 |
| `serverplan-target-architecture` | 0.1 | 4.7 (10 inputs) | 4.7 | 1.0 |
| `impl-direct-execution` | 0.1 | 1.9 (4 inputs) | 1.9 | 0.1 |
| `serverplan-bypass`, `raw-execute-regression` | 0 | 0 | 0 | 0.1 |
| **total** | 242.5 | 122.8 | 365.3 | 199.2 |

Where the facts CPU goes in the median run (thread time):

- Parse: 9.7 s. Fact extraction: 1.0 s. File reads: 0.6 s.
- Evaluation: 181.9 s on the workers plus 18.0 s of repository partitions on the main thread.
  - AST inputs: 78.0 s.
  - SQL stages: 115.2 s. Of this, `descendants` takes 11.2 s, `nodes` 11.1 s, `ownership` 8.5 s, `direct_events` 6.5 s, and final queries 5.3 s.

Progression, all with the same parity:

| commit | change | CPU (user + sys) | wall | run dir |
|---|---|--:|--:|---|
| `9e7dba591` | before the util-memo fix | 1,910 s (one run) | 510 s | `20261022T121700-repo-probe` (untimed probe) |
| `fb46a8bd1` | util memoisation | 362.6 s (361.4 / 362.6 / 372.9) | 104.9 s | `20261022T125330-repo`, `…125532-repo`, `…125716-repo` |
| `96168e00a` | OR-union probes | 212.9 s (211.5 / 212.9 / 222.7) | 67.6 s | gate runs above |

The `fb46a8bd1` runs `…124952-repo` and `…125144-repo` are untrusted: the load reached 14.4.

Kill criteria:

- **"A2 between 10% and 25% with A1 passing → the evaluator replaces only the SQLite stages":** not triggered as written. A2 is 54% (i) and 52% (ii), above the band, and A1 is not passing. PLAN.md's coarser form, "< 4× win → the in-memory evaluator replaces only the SQLite stages and pattern matching stays native", is triggered: the win is 1.85× (i) and 1.94× (ii).
  - The measurement does not support its premise that native pattern matching is the cheaper half. The spike's AST inputs take 78.0 s against native's 122.8 s file phase plus 44.8 s of re-extraction. Its SQL takes 115.2 s against native's 197.7 s of stage SQL.
  - Neither half is 10× cheaper while the policies' SQL re-derives the tree (§ 6). The decision belongs to the decider.
- **"A1 unreachable for an AST-schema relation → fix the schema":** not triggered. The only A1 gap is quality-contracts-1's `annotation` parse, a (c) bespoke cross-node fact (Ruff's string-type-definition classifier). No (a) relation is wrong: every other check is exact on both corpora.
- **"A4 p50 > 30 ms → split core and derived facts":** not triggered. The A4 p50 is 0.53 ms including the parse.

## 3. How measured

- **Build:** as in § 1.
- **A1, source corpus:** `./spikes/facts-a/target/release/facts-a replay --recording fixtures/host/data-layer-port/batch-06-recording.json --recording fixtures/host/data-layer-port/batch-06-probes.json --output <run>/results.json` (`command.txt` in `20261022T131243-replay`).
  - The comparator is `main.rs::replay`, following `replay.ts`.
  - The configuration per event comes from `call_config`: `oracleAnalyzer = !production_scope` where the policy declares it, merged with `oracle_config.options`; the event scope replaces the policy scope unless `production_scope` is set.
  - Events with a `retained_reason`, with `upstream_passed=false` or without `files` are skipped.
  - Findings are compared as multisets of `[path, line, code]`.
- **A1 repository, and A2 to A5:** `spikes/facts-a/tools/run-repo.sh`, once per run. It runs `facts-a repo --threads 4 --out-dir <run>` under `~/rerail-bench/bench-lock bash -c 'uptime; /usr/bin/time -l perl -e alarm(900) …; uptime'`, then `facts-a repo-compare --oracle ~/rerail-bench/facts/spike-a/work/suite-b6.json --results <run>/results.json --output <run>/parity.json`. Each run directory holds:
  - `command.txt`;
  - `stdout.txt`;
  - `stderr.txt` (`/usr/bin/time -l`);
  - `uptime-{before,after}.txt`;
  - `results.json`: findings, failures, per-rule, per-clause, per-input and per-SQL-stage nanoseconds, A3 and A4 summaries, `max_rss_bytes` checkpoints;
  - `per-file.tsv`: per file, bytes, nodes, read, parse, extract and evaluation nanoseconds;
  - `parity.json`.
- **`repo-compare`:** multisets of `[path, line, code]` per check, which is `replay.ts`'s rule keyed by `[check, path, line, code]`. It also verifies that every oracle event is `production_scope` with no `oracle_config`, retained reason or failed upstream, and that the walked file set equals the oracle's.
- **A2:** the process's user + sys from `/usr/bin/time -l`; the median of the three runs. Computed by `python3 spikes/facts-a/tools/a2.py <run>/results.json …`; its output for the gate runs is `20261022T130749-repo/a2.json`.
  - **Native baseline:** 393 s, the batch-6 native release run on the same corpus (`~/rerail-bench/dl60-split/`).
  - **Apportioning, written down:**
    - native global(rule) = stage-build seconds (`b6-stage-builds.csv`, `total_s`) + re-extraction seconds (`b6-extraction-per-input.csv`, `seconds`) of the rule's clauses;
    - native file share(rule) = 122.8 s × (the rule's AST matcher inputs ÷ 263), with counts from the census;
    - native(rule) = global + file share. These cover 365.3 s of the 393 s; the remainder (session setup, input loads, producer, result, teardown, process overhead) stays in the baseline.
    - quality-contracts-1: native = 25.01 s global + 122.8 × 7/263 = 3.27 s file share, 28.28 s in all. A2 (ii) native = 393 − 28.28 = 364.72 s. A2 (ii) facts = median-run CPU 212.92 s − quality-contracts-1's clause time 24.69 s = 188.23 s, giving 51.6%.
    - Sensitivity: counting every global-phase probe record of the clause (`b6.jsonl.gz`, summed as `tables.py` does: 27.86 s) gives 31.12 s and 52.0%.
    - quality-contracts-1's facts time is its time to the refusal, so (i) understates its full cost; (ii) removes it on both sides.
- **A3:** one sample per (clause, file) evaluation: the clause's thread time, including its first build of an input's rows. Repository partitions are not per file and are excluded.
- **A4:** per file, `parse` (`FileInput` + `eval::file_tape`: tree-sitter plus the engine's tape) and `extract` (the fact columns from the tape) nanoseconds. In-memory bytes come from `Facts::bytes`. Fact-file bytes come from `facts-a store --threads 4 --out-dir <run>` (`20261022T131227-store`), which writes every content to the store, reads it back and compares every column with the built facts: 0 mismatches over 9,010 files, 8,957 distinct contents.
- **Cross-spike readability:** `facts-a inspect` (`20261022T131231-inspect`) reads every `.facts` (8,957) and every SpikeB `.xfacts` (19,772) in `~/rerail-bench/facts/store` with the generic reader, with 0 failures.
- **A5:** `maximum resident set size` from `/usr/bin/time -l`, plus `getrusage` checkpoints after the workers, the merge and the finish.

## 4. Parity artifacts

- Source corpus: `~/rerail-bench/facts/spike-a/runs/20261022T131243-replay/results.json` holds the per-check counts and every mismatching call with its FN, FP and failures.
- Repository: `~/rerail-bench/facts/spike-a/runs/20261022T130531-repo/parity.json` holds, per check, the expected and actual counts, FP, FN, failures by clause and the FP/FN keys. It also records the configuration check and the file-set check.

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

  They are persisted in the fact-file format agreed with SpikeB (§ 5.3).
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

### 5.3 Fact schema v0 (final, Spike A's syntactic part) and fact files

In memory (`facts.rs`, per content; symbols are process-wide `u32`s):

| relation | columns | notes |
|---|---|---|
| `node` | `id:u32` (row), `kind:sym`, `parent:id?`, `start:u32`, `end:u32`, `start_line:u32`, `end_line:u32`, `subtree_end:u32`, `flags:u8`, `field:u16` | preorder over every tree-sitter node, named and anonymous, the engine's transducer ids; descendants of `id` are `(id, subtree_end)`; `flags` = named, extra<<1, missing<<2, error<<3; `field` = grammar field code under the parent, 0 = none |
| `child` | CSR over `parent` (`child_offsets`, `children`) | derived, rebuilt on read |
| kind postings | `kind_keys`, `kind_offsets`, `kind_nodes` | derived, rebuilt on read |
| `line` | the native `LineTable` (Ruff terminators `\r\n`, `\r`, `\n`) | |
| Python-derived, per node on demand | decoded string value (`pyvalue.rs`), f-string pieces (`fstring.rs`) | not persisted |

Batch 6 needs no further Python-derived relation as an extracted fact: the policies build `functions`, `calls`, `parameters`, ... in their own SQL from the inputs. The contract's `function`, `class`, `parameter`, `call`, `argument`, `attribute`, `assignment`, `decorator`, `import` and `string_lit` are what Phase 2 substitutes for those stages (§ 6).

On disk (`factfile.rs`), `~/rerail-bench/facts/store/<hash[0..2]>/<hash>.facts`, the layout agreed with SpikeB: magic `RRFACTS\0`, `u32` schema version (0), blake3-256 content hash (32 bytes), `u32` string count and strings as (`u32` length, UTF-8), `u32` relation count, then per relation `u32` name index, `u32` rows, `u32` columns, per column (`u32` name index, `u8` type: 0 `u32`, 1 `u16`, 2 `u8`, 3 `sym`, 4 `opt_u32`, 5 `opt_sym`), then each column at native width. After a `u16` or `u8` column only, zero bytes pad to the next 4-byte file offset. All little-endian; `opt` none is `u32::MAX`. Spike A writes:

| relation | columns |
|---|---|
| `node` | `kind:sym`, `parent:opt_u32`, `start:u32`, `end:u32`, `start_line:u32`, `end_line:u32`, `subtree_end:u32`, `flags:u8`, `field:opt_sym` |
| `identifier` | `id:u32`, `name:sym` (named `identifier` nodes) |
| `line` | `start:u32`, `end:u32` (before the terminator), `indent:u16` (leading spaces and tabs) |
| `syntax_error` | no columns; one row when the tree has errors |

Strings are per file. The reader re-interns kinds and maps field names back to grammar codes, rebuilds the derived columns, and checks the hash against the source.

### 5.4 Changes in this run (exactness kept; replay and repository parity re-checked after each)

- `facts::name` returns a shared `Rc<str>` from a per-thread copy of the interner, with no allocation. `ast::Eval::children` returns an iterator over the facts' CSR slice, with no allocation for tree nodes.
- `repo` (`repo.rs`): 4 workers pull files from a shared counter. Each worker has its own compiled rules and streams per file: read, parse, build facts, evaluate, drop. Workers return findings, timings and the repository-partition rows. The rows' `Rc` texts are converted to owned values to cross threads and re-shared on the merging thread. `eval::finish` then runs the repository partitions and budgets once. Rows are sorted by path, stably, so the merge order cannot change a result. The configuration is the one `replay.ts` resolves for a `production_scope` suite event (`main.rs::call_config`, shared with `replay`). `repo-compare` checks that the oracle's events really have that configuration and the same file set.
- **Util memoisation** (`ast.rs`). Verdicts of capture-free utilities are memoised per node, including utilities that read `value`, as the native matcher does (spec-ast: "verdicts memoised per (util, node, relevant bindings)"). Unknown verdicts are not memoised. Whether a verdict is unknown is now decided from that evaluation alone, not from flags the caller had already set. Before this fix, `state-signatures-1` re-ran `state-signature-source` (`has ... stopBy: end` over the whole module) for every candidate node: 1,616 s, with 188 s on one file. After it, the clause takes 15 s.
- **OR-union index probes** (`sql/engine.rs`). This is SQLite's OR optimisation as a fixed, text-determined rule, not a cost-based planner. A conjunct `b1 OR b2 ...` qualifies when every branch holds an equality between a column of the current FROM item and earlier items, outer rows or constants. Each branch probes its own hash index, extended by the item's plain equality probes. The candidates are the sorted, deduplicated union of those lookups, which is table order. The OR conjunct is re-checked, so the rows and their order are exactly the filtered scan's. This removed the quadratic join `nodes n ON n.__path=p.__path AND (n.id=p.id AND ... OR n.parent=p.id AND ...)` in the `parameters` stages: 149 s became 3.5 s.
- Instrumentation: per-input and per-SQL-stage time, `ru_maxrss` checkpoints, and per-file costs (`per-file.tsv`).

## 6. Risks found

- **A2 is bounded by the policies' own SQL, not by the evaluator.** The ten policies rebuild the syntax tree per clause in SQL (census: 418 of 775 units are (a)). The spike executes that SQL as written, as decided. In the median run the stages `nodes`, `descendants`, `ownership`, `edges`, `node_ordinals` and `syntax_rows_*` alone take 36.6 s. The 176 node-dumping AST inputs (`syntax_N`, `raw_nodes`) that only feed those stages take 70.5 s of the 78.0 s of AST inputs. The repository partition of `quality-conditional-1` rebuilds a `nodes` table, with each node's full text, over every file in scope. It takes 18.0 s and lifts peak RSS from 3.57 GB (end of the file phase) to 8.46 GB. "Facts once" only pays when Phase 2's compiler substitutes those (a) stages with the extracted relations and recovers text from the source on demand. Running the rule text as written cannot reach 10%. [INFERENCE] How much the substitution saves is not measured here.
- **`parse: { grammar: annotation }`** (quality-contracts-1 `quoted_atoms`) depends on Ruff 0.16.9's string-type-definition classification (`scope/python/typedefs.rs`). That classification needs the scope table's qualified-name resolution, a cross-node semantic fact outside v0. The spike refuses it with a failure rather than approximating it. That accounts for the whole A1 gap: 6 replay calls and 38 repository failures. Phase 2 needs either a `string_type_definition(host_id)` derived fact from a port of that classifier, or a narrowed scope for quality-contracts-1. A Phase 1 port is excluded by decision.
- **Row-order-sensitive SQL**: `row_number()` ties, `LIMIT 1` with a partial `ORDER BY`, and `group_concat` without an ordered subquery depend on SQLite's plan. The spike breaks ties by input order. No replayed call and no repository finding diverged.
- `json_each` over an object enumerates keys sorted (serde_json without `preserve_order`), whereas SQLite enumerates them in text order. The batch-6 rules use it only for set-valued lookups.
- The replay harness evaluates every recorded call with `oracleAnalyzer=true`, `entryPoint=<event entry>` and per-event extra options, and it replaces the policy scope. The repository suite uses the defaults and production scopes. The spike resolves options per call exactly as `replay.ts` does, for both.
- Fact files are about 8.3× the source at p50: 34 bytes per node, uncompressed, including the anonymous nodes. Phase 2 should decide whether anonymous nodes are stored or re-derived, and whether to compress.

## 7. Resume notes

- Commits on `Moti-Velz/facts-spike-a` (local, not pushed):
  - `baca38d83`: visibility only.
  - `0f3e1db49`: census.
  - `19d56c1c1`: evaluator and replay parity.
  - `56aff3818`: first report.
  - `9e6325db2`: allocation fixes.
  - `9e7dba591`: `repo` and `repo-compare`.
  - `fb46a8bd1`: util memoisation, fact files, `LineTable` visibility, A2 tool.
  - `96168e00a`: OR probes. This is the commit of the gate timing runs.
  - `5fa7ace5a`: fact-file padding per Spike B, and `inspect`. `repo` is unchanged.
- Production change in this run: visibility only. `native/src/code_rule_dsl/lines.rs` makes `Terminators`, `LineTable::new`, `start` and `end` `pub`, so the fact-file reader can rebuild a `LineTable`.
- Done: every gate measured (§ 2) and every kill criterion evaluated (§ 2).
- Not done, by decision: the annotation classifier for quality-contracts-1.
- Not done, out of scope: substituting the (a) SQL stages with fact relations. That is Phase 2's compiler, and § 6 explains why A2 needs it.
- Reproduce: build (§ 1), then follow § 3. The comparator outputs and `a2.json` are in the run directories.
- Native semantics specs used by the port: `~/rerail-bench/facts/spike-a/work/spec-{ast,rel,clause}.md`.
