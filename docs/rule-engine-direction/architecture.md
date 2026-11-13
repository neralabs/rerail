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
8. **Findings are a set.** Every executor answers to `findings-contract.md`: finding keys plus a per-(rule, file) `Complete`/`Unknown` outcome. No executor reproduces order, witnesses, spans, messages or operational behaviour. One shared consolidation step normalises, collapses, rehydrates payloads and derives ids.
9. **Rules carry no SQL or code.** Rules compose declarative operators over named relations. Analyses shared by many rules are engine relation contracts (`authored-sql.md`).
10. **Decisions cite prior work.** Every design decision is recorded in `decisions/` with citations into the ast-index-spike indexes and the lessons ledger (`prior-work/README.md`).

## Starting point, and why

**Trunk: current `main` (`5cf4246e`).** Main is the production engine. It has the v7 tape, multi-document `FileImage` (embedded programs), Forte/Blade, pack test data in S3, and the newest native matcher. Every stream branched from an older main. Integrating onto main means each stream's conflicts are resolved once, against the code that ships.

**First stream merged: Data-layer-rule-expressivity (DLRE).** Merging DLRE into main gives the same content as starting from DLRE and merging main into it. Merging into main keeps main on the first-parent line. DLRE goes first for three reasons:

1. **It overlaps most with main's production modules** (`eval/`, `workspace/`, `matcher/`, `tape`, `content_store`). Its conflicts are the hardest, and they are best resolved while the rest of the tree is still main. Seventeen files conflicted.
   - The main collision was main's `FileImage` (a primary tape plus derived documents) against DLRE's single-tape code paths. The resolution keeps main's model and adapts DLRE's callers to it.
2. **It carries the assets the target needs first:**
   - candidate selection lowered to SQL over the FTS kind index (`matcher/anchors/sql/`), the seed of statistics-chosen anchors;
   - the producer contract catalog, the seed of the relation catalog;
   - 60 ported data-layer rules;
   - the data-layer parity harness (3,210 observations), which feeds the measurement gate.
3. **The other streams are mostly additive.** SSC adds `sql/`; N1562 adds `packed/`, `query/relational*` and `transducer/sql_backend.rs`. Both merge more cleanly on top.

**What DLRE brings that is not wanted:**

- The authored-SQL `relational` clause. It arrives with the merge and is frozen by decision 0002.
- The exact-oracle parity bar, replaced by decision 0001.

DLRE's value here is its harness, anchors, contracts and ports, not its relational executor.

**Not chosen as the base:**

- **Moti's `facts-engine`.** It replaces SQLite and forbids statistics-chosen plans, which contradicts principles 1 and 3. Its assets are taken in phase 7.
- **`sql-dl60`.** It is Moti's SQL base merged with an older DLRE. It is used as the reference resolution for phase 4.
- **N1562.** Its whole-catalog IR has the wrong unit (one statement per workspace × catalog). Its packed IR, executors, lowering and vtab boundary are the key merge (phase 2).
- **SSC.** Its author retired the frame engine as an executor. Its emitter, statistics reader and cross-file pieces are merged in phase 4.

## Layers and module map

The normative module map is `spec.md` §16. It is transducer-centric: `engine/transducer/{ir,lower,exec,facts}` carries everything decidable within one document, `engine/vtab/` is the boundary, and `engine/compose/` lowers cross-file operators to SQL above it. Code arriving from each stream lands as follows:

| Source | Lands in |
|---|---|
| main | `tape/`, `content_store/`, `matcher/` (parity oracle and rehydration), `workspace/`, `eval/` |
| DLRE | anchor SQL → `engine/plan/` scope; producer contracts → relation catalog; reachability/forest → `engine/vtab/graph.rs`; fact extraction → `engine/transducer/facts/`; `relational/` frozen (decision 0002); parity harness → gate |
| N1562 | `packed/` → `engine/transducer/ir/` and `exec/{cpu,jit}`; lowering → `engine/transducer/lower/`; `sql_backend` → `engine/vtab/transducer.rs`; `semantic_facts` → `facts/`; `query/plan/` → `engine/plan/`; stats tables → `engine/stats/` |
| Tempo (`nera-landing-2`) | IR word layout → `engine/transducer/ir/`; Metal executor → `exec/metal/` |
| SSC (`c17d2e47`) | `sql/fragments/` → `engine/vtab/` (boundary, demand handle, typed cells, outcome codes); `sql/engine/` session and compile cache → `engine/plan/`; `queries()` publication → `engine/compose/`; `statistics.rs` → `engine/stats/`; `structural.rs`, `candidates.rs` → `engine/fallback/`; checker → gate tooling; crossfile/select_when → `engine/compose/` |
| facts-engine | `facts_memo.rs` → `content_store/`; gate and mutant discipline |

## What is deliberately not carried forward as an executor

These are kept in history and mined for tests and shapes, but not wired into production:

- **SSC frame/stage set-composition engine** (`sql/frames*`, `sql/emit*`, `sql/fused`, `sql/large`). Its own author retired it as an executor at about 3x native; it encodes an interpreter in SQL. Parts that are used by the pattern compiler stay until they are replaced.
- **N1562 whole-catalog relational IR** (`query/relational*.rs`, `query/compact_output_sql.rs`). The unit of one statement per workspace × catalog is wrong (8 MiB rail). Its 108 contract goldens and plain-table substitution tests are kept as tests.
- **Moti's facts-engine executor** (`facts/`, `spikes/facts-*`). It replaces SQLite and forbids stats-chosen plans. Its assets are taken: the content-hash memo, the gate discipline, and the declared-facts design.

## Integration phases

**Measure before merging.** Each phase begins with the measurements that justify it, run on the gate corpora and recorded as a decision record. A merge whose motivating measurement has not been run does not start. Within a phase, tests follow `test-budget-governance`: targeted tests in the inner loop, full suites once at the phase gate, in the background.

The order is transducer-first (decision 0004). N1562 is the key merge, because it carries the packed IR, executors, lowering and vtab boundary. Tempo's IR and Metal executor come next. SSC contributes only the pure-SQL emitter, statistics reader and cross-file pieces.

### Pre-merge measurements

From `research/heavy-rule-profiles.md` §7 and `research/fusion-model.md`. "Gates" names the phase that may not start until the measurement is recorded.

| # | Measurement | Pass condition | Gates | Result |
|---|---|---|---|---|
| M1 | Each of the 8 heaviest rules alone on the existing native matcher: CPU and RSS per rule | Recorded baseline (none exists today) | 2 | **Done (partial, by direction):** 8 rules × 2 runs; cold CPU 39–346 s per rule (994 s summed, double-counting fixed ingest); quality 10.5 GiB peak; AST-only rules are SQL-anchor-dominated; state-tripwires fails a repository input budget (`research/m1-baseline.md`) |
| M2 | Fused transaction-patterns and async-sync-delegation (interned regexes, masks) on the fusion prototype | Same finding set as the native matcher; AST cost ≤ 1 ns per program-node | 2 | **PASS (2026-10-25):** identical sets (1,244 keys with all 58 TP clauses; 97/97 fixture events); 0.26 ns/pn all clauses, 0.40 ns/pn defaults; ASD alone 0.86–1.02 ns/pn (`research/m2-m7.md`) |
| M3 | Fact tables served through a vtab over native data, rerunning resolution, reachability, dead-code and taint queries | Load plus joins ≤ 5 s for the gate | 4 | — |
| M4 | internal-dead-code and public-api-triples over fact tables, after reproducing DLRE's "interrupted" failure | Same findings; recorded query plans | 6 | — |
| M5 | `state-flow-1` data flow as a native relation | Matches the oracle on all 1,259 observations; ≤ 2 s | 6 | — |
| M6 | Rows actually consumed per `syntax_*` input of `quality-conditional-1` | No input copies every node | 6 | — |
| M7 | `generated_facades.py` and the 1,000-sync-function case under mask lowering | Completes with no budget failure | 2 | **PASS:** 20 ms and 3.6 ms with keyed sets; native fails its budget on both and from ~100 function pairs up |
| M8 | One-file change through facts, resolution and reachability | ≤ 1 s at p95 | 6 | — |
| M9 | v9np vs v7 on the gate: SQL span queries, random access, end-to-end | The `tape-v8-measurements.md` gains hold end to end | 6 (tape change) | — |
| M10 | L5 (candidate exports + indexed SQL semi-join) against keyed sets on M7's inputs and step-contracts | Total CPU within 1.5×; per-edit p99 unchanged | 6 | — |
| M11 | Cold materialization of all derived facts in `research/derived-facts.md` on the gate corpus | ≤ 2.5 s on one core, plan-recorded good forms | 6 | — |

M1, M2 and M7 are cheap and settle most of the range; M4 and M5 carry the parity risk.

### Phases

| # | Phase | Method | Entry measurements | Exit check |
|---|---|---|---|---|
| 1 | DLRE onto current main | `git merge` of DLRE committed tip | — | native build, cargo tests for touched modules, vitest engine tests |
| 2 | N1562: packed IR, CPU interpreter/JIT, lowering, `sql_backend` vtab, `semantic_facts`, `query/plan/`, contract goldens | `git merge` of `jbarnes/ner-1562-sql-rerailer` | M1, M2 ✓, M7 ✓. Note: N1562's packed IR lowers none of the M2/M7 rules today (`research/m2-m7.md`); the merge brings infrastructure, and the M2 prototype's techniques must be ported into it | builds; packed and contract tests pass; differential test interpreter vs native matcher on the finding-key set |
| 3 | Tempo IR and Metal executor | merge or port from `nera-landing-2` onto the v7 tape; reconcile word layouts into one IR | metal-recursion and metal-enqueue results (recorded) | Metal parity on macOS; `metal_coverage` gate wired |
| 4 | SSC at `c17d2e47`: typed-fragment vtab boundary and demand handles (`sql/fragments/`), session, compile cache and execution budget (`sql/engine/`), resident partitioning, statistics reader, structural pure-SQL emitter, checker tooling | `git merge` of `jeremy/sql-capture-demand-spikes`; frame compiler and `retired_control.rs` not wired | M3 | builds; fallback differential tests |
| 5 | Restructure into `engine/` | `git mv` plus path fixes only | — | builds; same tests pass |
| 6 | Mutate toward the spec | findings consolidation and set-based parity first (0001); IR extensions E1–E14 and set stage; fusion; statistics from SQL; plan cache; re-express the 10 authored-SQL rules and remove `relational/` (0002); v9np tape (0003); remove executors not carried forward | M4, M5, M6, M8, M9 per item | set-based parity suites; end-to-end gate recorded against §19 of the spec |
| 7 | facts-engine assets | merge or port `facts_memo.rs`; adopt the gate and mutant discipline | — | memo tests |

## Source snapshots

| Source | Ref | SHA |
|---|---|---|
| main | `origin/main` | `5cf4246e` |
| DLRE | `jeremybarnes/Data-layer-rule-expressivity` | `d3470da2` (staged, uncommitted work in its worktree is excluded) |
| SSC | `jeremy/sql-capture-demand-spikes` | `c17d2e47` |
| N1562 | `jbarnes/ner-1562-sql-rerailer` | `16343282` (uncommitted work in its worktree is excluded) |
| Moti SQL+DLRE merge (reference) | `origin/Moti-Velz/sql-dl60` | `e6d2ac73` |
| Moti facts | `origin/Moti-Velz/facts-engine` | `685db3f7` |
| Tempo Metal | `nera-landing-2` `jbarnes/ner-1562-scale-test-the-re-railer` | `a091aa063` (approximate; re-pin at phase 3) |
