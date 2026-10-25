# Unified rule engine: architecture and integration plan

Status: integration in progress on `jeremybarnes/Pulling-it-all-together`. The direction and its evidence are in `README.md`; the lessons are in `lessons-ledger.md`.

## Principles

1. **SQL drives execution.** Every rule evaluation is a SQLite statement, or a short sequence of statements, over named relations. Native code runs only behind relations the planner can see.
2. **One relation, two bindings.** Every logical relation has a stable contract: name, typed columns, key, declared ordering. It is bound either to a pure-SQL definition over the tape or to a batch-level virtual table that fronts native code. Both bindings are differentially tested.
3. **Shape from statistics in SQL.** The compiler chooses lowering shapes at plan time from statistics held in SQLite: `fts5vocab` over the kind index, plus stored edge and value counts. Rust reads statistics; it does not own them.
4. **Batches, with sharing only across identical scope.** One `xFilter` per (operator, batch). Rules are grouped only when their candidate scope is identical.
5. **Compiled plans are cached.** Plans are keyed by (rule digest, compiler version), together with the statistic values their decisions used. Compile time is measured separately from execution.
6. **Executors are swappable.** The CPU matcher, the packed CPU/JIT executor and Metal sit behind the same relation contracts.
7. **Integrate by merge, then mutate.** Code arrives by `git merge` from its source branch, so history is kept and live branches can be re-merged. It is then moved and changed to fit this layout; it is not rewritten.

## Layers and module map

All paths are under `native/src/code_rule_dsl/` unless noted. "Source" says where the code comes from.

| Layer | Target module | Responsibility | Source |
|---|---|---|---|
| Substrate | `tape/` (v7), `tape/index.rs`, `tape/wire.rs` | Immutable preorder tape, wire form, per-tape index | main, Data-layer-rule-expressivity (DLRE) |
| Storage | `native/src/content_store/` | Content-addressed store, FTS kind index, publication, results | main; DLRE corpus publication; `facts_memo.rs` from Moti's facts-engine (later) |
| Statistics | `engine/stats/` | SQL views over `fts5vocab(search_kind_fts)`, stored edge and value counts, `ANALYZE`/`PRAGMA optimize` policy, read API for the compiler and `xBestIndex` | new; `planner_stats.rs` from sql-capture-demand-spikes (SSC), changed to read SQL |
| Relation contracts | `engine/relations/` | Catalog of typed narrow relations (name, columns, key, ordering, completion) and their bindings | DLRE `producers/` (contract catalog); ner-1562 (N1562) revival-design schema |
| Virtual tables | `engine/vtab/` | Batch-level virtual tables: Arc-relation table with data-derived estimates, node table (ranges, ordering), batch terminals, match output | DLRE `relational/table.rs`; SSC `sql/nodes.rs`, `sql/terminals/`; N1562 `transducer/sql_backend.rs` |
| Lowering | `engine/lower/anchors/` | Candidate selection lowered to SQL (anchor plans to SQL), with the anchor chosen from statistics | DLRE `matcher/anchors/sql/` |
| Lowering | `engine/lower/pattern/` | Structural pattern compiler: grammar to typed relations to SQL fragments, demand, producers, recursion | SSC `sql/relational/` |
| Lowering | `relational/` (authored clauses) | `relational` clause kind: authored SQLite stages over typed evidence | DLRE `relational/` |
| Planning | `engine/plan/` | Plan cache, shape choice, batch grouping by scope digest, statement assembly | SSC `physical.rs`, `job.rs`, `shared_job.rs` |
| Execution | `engine/exec/packed/` | Packed ABI, CPU interpreter and arm64 JIT, Metal backend behind the vtab boundary | N1562 `packed/`; later, the newer Tempo Metal executor |
| Execution | `matcher/` | Native ast-grep-compatible matcher (per-file binding, parity oracle) | main |
| Orchestration | `workspace/`, `eval/` | Workspace evaluation and the stateless evaluator, routed through the engine | main, DLRE, SSC |

## What is deliberately not carried forward as an executor

These are kept in history and mined for tests and shapes, but not wired into production:

- **SSC frame/stage set-composition engine** (`sql/frames*`, `sql/emit*`, `sql/fused`, `sql/large`). Its own author retired it as an executor at about 3x native; it encodes an interpreter in SQL. Parts that are used by the pattern compiler stay until they are replaced.
- **N1562 whole-catalog relational IR** (`query/relational*.rs`, `query/compact_output_sql.rs`). The unit of one statement per workspace × catalog is wrong (8 MiB rail). Its 108 contract goldens and plain-table substitution tests are kept as tests.
- **Moti's facts-engine executor** (`facts/`, `spikes/facts-*`). It replaces SQLite and forbids stats-chosen plans. Its assets are taken: the content-hash memo, the gate discipline, and the declared-facts design.

## Integration phases

| # | Phase | Method | Exit check |
|---|---|---|---|
| 1 | DLRE onto current main | `git merge` of DLRE committed tip | native build, cargo tests for touched modules, vitest engine tests |
| 2 | SSC relational compiler (and Moti's SQL base it depends on) | `git merge` of `jeremy/sql-capture-demand-spikes`; Moti's `sql-dl60` resolution used as reference | builds; SSC catalog parity tests pass |
| 3 | N1562 packed executors, match vtab, contract goldens | `git merge` of `jbarnes/ner-1562-sql-rerailer` | builds; packed and contract tests pass |
| 4 | Restructure into `engine/` | `git mv` plus path fixes only | builds; same tests pass |
| 5 | Mutate toward the principles | statistics from SQL; relation catalog as the single contract; vtabs report stats-based estimates; plan cache; remove executors not carried forward | builds; parity suites; end-to-end gate recorded |
| 6 | Newer Tempo Metal executor | port from the `nera-landing-2` NER-1562 branch onto the v7 tape under `engine/exec/` | Metal parity tests on macOS |
| 7 | facts-engine assets | merge or port `facts_memo.rs`; adopt the gate and mutant discipline | memo tests |

## Source snapshots

| Source | Ref | SHA |
|---|---|---|
| main | `origin/main` | `5cf4246e` |
| DLRE | `jeremybarnes/Data-layer-rule-expressivity` | `d3470da2` (staged, uncommitted work in its worktree is excluded) |
| SSC | `jeremy/sql-capture-demand-spikes` | `b87eba3b` |
| N1562 | `jbarnes/ner-1562-sql-rerailer` | `16343282` (uncommitted work in its worktree is excluded) |
| Moti SQL+DLRE merge (reference) | `origin/Moti-Velz/sql-dl60` | `e6d2ac73` |
| Moti facts | `origin/Moti-Velz/facts-engine` | `685db3f7` |
| Tempo Metal | `nera-landing-2` `jbarnes/ner-1562-scale-test-the-re-railer` | `a091aa063` (approximate; re-pin at phase 6) |
