# Spike A report: facts once + in-memory relational evaluation (batch 6)

Status: **in progress** (milestone 1: census). This file is updated at every milestone; § 7 says what is done and what is next.

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

Not measured yet.

## 3. How measured

Not measured yet.

## 4. Parity artifacts

None yet.

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

Pending (see § 7).

## 6. Risks found

- The replay harness evaluates every recorded call with `oracleAnalyzer=true`, `entryPoint=<event entry>` and per-event extra options, and replaces the policy scope; the repository suite uses the defaults and production scopes. The spike resolves options per call exactly as `replay.ts` does.

## 7. Resume notes

- Done: census (this section), crate skeleton with the tape-based fact builder (`spikes/facts-a/src/facts.rs`).
- Next: evaluator over facts (AST matcher + relational engine), recording replay, repository run.
- Native semantics specs used by the port (read from source by read-only scouts): `~/rerail-bench/facts/spike-a/work/spec-{ast,rel,clause}.md`.
