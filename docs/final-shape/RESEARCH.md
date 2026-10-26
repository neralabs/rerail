# Research tracker for the final shape

Purpose: read every source in full and record findings with citations in `notes/`, so the design can be regenerated from notes instead of from memory. A design document is written only after every track below is `done`.

Rules for notes:
- Record mechanisms, layouts, numbers and decisions with `file:line` citations.
- Write "not found" rather than infer.
- Mark each number measured (M), derived (D) or assumed (A), as the source does.
- Record contradictions between sources explicitly.

Status values: `todo`, `reading`, `done`.

## Tracks

| # | Track | Sources | Reader | Notes file | Status |
|---|---|---|---|---|---|
| T1 | PIAT normative spec | `spec.md` (597), `spec-transducer.md` (722), `spec-indexes.md` (390), `spec-fusion.md` (326), `spec-tooling.md` (313) | lead | `notes/t1-piat-spec.md` | done |
| T2 | PIAT tape, residency, facts, transducer research | `research/{tape-regularity,tape-v8-measurements,tempo-residency,derived-facts,transducer-ir,sql-vs-transducer,metal-recursion,metal-enqueue,fusion,m2-m7,regex}.md`, `experiments/tape-v8/results/tables.md` | lead | `notes/t2-piat-research-core.md` | done |
| T3 | PIAT cost and profile research, ledger, contracts | `research/{heavy-rule-profiles,fusion-model,heavy-inputs,m1-baseline,ssc-fragments,workstream-scan-2026-10-25,shape-fixes}.md`, `lessons-ledger.md`, `findings-contract.md`, `authored-sql.md` | agent | `notes/t3-piat-research-costs.md` | done (agent; spot-check pending) |
| T4 | ast-index-spike prior work | `prior-work/ast-index-spike-design-index.md` (1484), `prior-work/ast-index-spike-history-index.md` (1319) | agent | `notes/t4-ast-index-spike.md` | done (agent; spot-check pending) |
| T5 | Tempo: tape caching, residency, pinned uncompressed tapes, workspace store | nera-landing-2 `tempo/docs/`: `phase-a-anchor-indexing`, `phase-b-tape-residency`, `content-addressed-workspace-store`, `workspace-memory-implementation-plan`, `workspace-store-runtime-journal`, `sha-indexed-transducer-execution`, `shared-arena-direct-publication-design`, `shared-sparse-bump-arena-design`, `sparse-rule-file-grid-design`, `cpython-335-seconds-workplan`, `distribution-model` | agent | `notes/t5-tempo-residency.md` | done (agent; spot-check pending) |
| T6 | Tempo: transducer, JIT, value programs, Metal execution designs | nera-landing-2 `tempo/docs/`: `transducer-*`, `metal-*` (design, decision and proposal docs; the cutover logs only for decisions), `packed-*`, `unified-packed-runtime-*`, `scm-*`, `tree-sitter-query-quantifier-design`, `native-regex-lua-lowering-design`, `sql-driven-transducer-execution` | agent | `notes/t6-tempo-transducer-metal.md` | reading |
| T7 | Main: shipped store, invalidation, tape, transducer docs | wolfeel `docs/content-addressed-workspace-store.md` (892), `content-store-bulk-publication-design.md`, `transducer-design.md`, `sql-driven-transducer-execution.md`, `rule-scoping.md`, `writing-rules.md`; the code behind them (`native/src/.../content_store`, tape, workspace) | lead | `notes/t7-main-store-invalidation.md` | todo |
| T8 | PIAT branch docs outside rule-engine-direction | PIAT `docs/`: `relational-execution-design`, `native-relations-*`, `shared-producer-*`, `shared-syntax-*`, `gpu-producer-execution-contract`, `reachability-spikes`, `sql-*`, `data-layer-engine-gaps`, `data-layer-rule-port`, `data-layer-performance-journal` (3701) | agent | `notes/t8-piat-branch-docs.md` | reading |
| T9 | Typing-support in full | `spikes/datalog-engines/**/RESULTS.md`, `FINDINGS.md`, `README.md`, `rdl/*.rdl`, `rdl/rdl.py` (structure only), `sqlite-ext/src/setfix.rs` (layout), `lattice/src/main.rs` (strategies) | agent | `notes/t9-typing-support.md` | done (agent; spot-check pending) |
| T10 | Moti facts: declared facts, memo, cross-file decision | `origin/Moti-Velz/facts-engine:docs/facts/**`, `facts_memo.rs`, `spikes/facts-b/src/{ygraph,y2,y3}`; `origin/Moti-Velz/facts-plan:docs/facts/**` | agent | `notes/t10-moti-facts.md` | reading |
| T11 | DLRE, N1562, SSC since PIAT snapshots | DLRE `docs/{cold-cpu-*,sql-schema-lifecycle-plan,sql-execution-owner,performance-decisions/*}`; N1562 `docs/sql-scope-and-operational-semantics-design.md`; SSC `docs/sql-structural-execution-plan.md` | agent | `notes/t11-streams-since.md` | done (agent; spot-check pending) |

## Questions the research must answer

The final shape has to answer each of these from the notes, with citations:

1. **Transducers.** What is the IR (encoding, machine model, operations E1–E17), what lowers to it, what was decided about the set stage, fusion, the verifier, and the backends? What did Tempo learn about running it on Metal, and why?
2. **Tape and disk.** v7 against v9np: bytes per node on disk and in memory, compression, regularity, and what is stored against what is derived. Why a layout expansion is or is not acceptable.
3. **Residency and caching.** Tempo's tape caching and pinned-memory uncompressed tapes; spec §20 forms, slabs, pinning and device views; per-edit residency.
4. **Fact storage.** Base and derived facts: schema, width, interning, per-document ownership, caching by digest, SQL against native storage.
5. **Lattices and inference.** Where lattices live, which evaluation strategies, the cross-file layering and its cutoff, and the Metal form.
6. **Invalidation.** File and content invalidation on main (content-addressed store), dependency cones (PR 2539, 0005), Moti's read-set memo, Typing's interface digests. How each document's facts and the derived results are invalidated on an edit.
7. **Indexes and statistics.** The exact index DDL, the anchor index, statistics, and the plan cache.
8. **The vtab boundary and outer SQL.**
9. **Tooling.** Memory limits, debugging, testing and profiling (spec-tooling).

## Log

- 2026-10-26: tracker created. `01-design.md` withdrawn (written without most of these sources).
- 2026-10-26: T1 done (spec, transducer, indexes, fusion, tooling). Open conflict recorded in t1: positional anchor postings (0006 vs spec-indexes). Agents for T3–T6, T8–T11 running. T2 started.
- 2026-10-26: T2 done (added metal-enqueue, fusion, m2-m7, which no other track covered). T3, T4, T5, T9, T11 agent notes delivered; not yet spot-checked. Note: commit 710a76f6 swept in partial agent files; later commits carry the finished versions.
