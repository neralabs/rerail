# Workstream scan, 2026-10-25 (since the survey)

The scan was read-only. Each section is measured against `docs/rule-engine-direction/` (README, architecture, findings-contract, authored-sql, prior-work). "HI" refers to `prior-work/ast-index-spike-history-index.md`.

## Summary table

| Stream | New since survey | Heading | Verdict vs direction |
|---|---|---|---|
| DLRE (uncommitted, 68 files, +2,952/−398) | A full-repo measurement reset; a shared SQL execution owner (`CodeRuleDslExecution`); SQL producer identity; materialization boundaries; candidate retirement | Making authored-SQL relational execution shareable and bounded | Measurement converges (it adopted the gate and read the direction README). The execution work deepens authored SQL, which conflicts with decision 0002 |
| N1562 (15 commits `16343282..8b6576f0`, clean tree) | `docs/sql-scope-and-operational-semantics-design.md` (776 lines); `query/plan/{identity,scope,grouping,utility,semantics}.rs` | Exact-scope grouping, semantic plan ids, utility dedup, SQL statistics, no SQL continuation state | Strong convergence on planning. Diverges on operational semantics: an EffectSummary/replay ledger and ordered regions conflict with decision 0001 |
| SSC worktree (branch `jeremy/sql-capture-demand-spikes`, uncommitted: −3,753 lines plus new `fragments.rs`) | `docs/sql-matcher-fragment-transition.md`: "generic SQL state machine is a rejected compiler architecture"; native matcher fragments behind one batched vtab filter | SQL composes typed relations; native fragments own sequential semantics | Converges (README §4, principle 1; HI D73). Still holds "exact work charges" open |
| Moti `facts-plan` (7 commits past `facts-engine`, tip `9a901915`, 10-23) | `docs/facts/DECISION-CROSSFILE.md`; Y1 maintained reference graph; Y2 local queries | Not "which engine" but a maintained compact reference graph plus rules rewritten as local queries | Converges with authored-sql.md (engine-owned shared relations). New, decision-relevant evidence for per-edit cross-file rules |
| Moti `facts-engine`, `sql-dl60`, `sql-set-composition` | Nothing new | — | — |
| Tempo Metal (`nera-landing-2/ner-1562-scale-test-the-re-railer`, 6 commits `a091aa063..e95613625`) | Fixes to terminal ownership, retirement and containment | The full-corpus run still fails | Still on the old `policy_dsl` transducer. Full run 701.9 s, about 2 GB tracked, 577 dispatches against a 600 limit, failing on containment memory |
| ast-index-spike | No new commits (origin tip `9449260`) | — | — |
| PR 2539 | OPEN, last update 2026-08-12 | — | — |
| Other branches | `tbelanger/rule-pack-generators` (4 files), plus one-line touches in `jeffreysnell/eslint-test-set` and `ggiangi/ner-1854` | Pack generation | Not engine execution |

## DLRE

**New numbers.** From `docs/data-layer-performance-journal.md` ("Full-repository measurement reset"), first-50 rules over 9,021 files:

| | Passing pre-SQL reference | Last SQL run | Current |
|---|---|---|---|
| Cold CPU | 125.8 s | 2,507.5 s | 958.6 s |
| Ingestion CPU | 73.4 s | | 875.3 s |
| Wall | | | 1,978 s |
| Full-process RSS | | | 5.83 GiB |
| Findings | | | 24,820 (match) |
| Execution failures | | | 3 |

- The failures are a clause work budget on one generated file, name-closure incompleteness, and an interrupted relational query.
- Exclusive counters: anchor selection 537 s, index preparation 137 s, AST matching 58 s, parse 11 s.
- The current run is still 7.6x worse than pre-SQL.
- The 60-rule run was in progress.

**Self-diagnosis, which matches the direction.**
- `Lower::pattern` builds every possible posting and runs `count(*)` to price each arm at run time. This is HI T18: seeds located only to be priced.
- The producer range path estimates a constant 32 rows.
- Physical anchor planning must move after index publication so it can read SQL statistics.
- Statement counts are demoted to diagnostics.
- It explicitly read the direction README and the ledger.

**Execution owner** (`docs/sql-execution-owner.md`, `execution/mod.rs`, `relational/{shared,pipeline,schema/inference}.rs`, `anchors/sql/corpus/owner.rs`):
- one FTS store shared across consumers;
- SQL producer identity via the SQLite authorizer;
- materialization boundaries: shared producers materialize once, and inlining everything hit SQLite's 65,535-reference limit;
- CTAS type inference;
- candidate retirement at 131,072 rows.

This is what makes the red test `test/engine/native/recovery/execution-owner.test.ts`, already merged, go green.

**Perspective.**
- The measurement reset and the anchor diagnosis are exactly right. Stats-chosen anchors from `fts5vocab` (README, Statistics) are the fix for the largest cost, 537 s of 958 s.
- Most of the rest is infrastructure for sharing and bounding authored SQL stages: producer identity over SQL text, deferred expressions, CTAS inference, retirement. Under decisions 0001 and 0002 most of it loses its purpose.
- It is also a temp-table and materialization path, HI D50 territory.
- Worth keeping: the shared source owner and version isolation (independent of authored SQL), and the `relational_facts.py` capture diff.
- Worth keeping: the `scripts/profiling/full_repository.py` runner. It is the measurement gate implemented, and it is the most valuable new artifact anywhere.
- Note: its matched controls use `JEMALLOC_OVERRIDE` with a system-malloc shim. Allocator choice confounds comparisons.

**For the integration.**
- Re-merge DLRE when this is committed. It touches files phase 1 resolved: `relational.rs`, `workspace/global.rs`, `anchors/sql/corpus.rs`, `relational/syntax.rs`. Expect conflicts again around `file_tape` returning `Option` and `FileImage`.
- Take `full_repository.py` and the execution owner early, in phase 1b.
- Treat the producer-identity, materialization and retirement machinery as frozen with `relational`.

## N1562

**New.** Design commit `ae305827` and seven TDD pairs:
- architecture boundary checks (`check-boundaries.mjs`, `query-plan-boundaries.mjs`: no SQL continuation state in plans);
- deterministic semantic plan identities, with no `Arc::as_ptr` memo keys;
- exact candidate-scope identities (`ScopeRecipe` digest of typed fields, not SQL text);
- clause grouping only by exact candidate scope;
- utility dedup by semantic demand identity;
- "assign ordered work to typed operators" (`semantics.rs`: `DslBehavior` → declarative contract, or a pushed operator such as `OrderedRegion` or `EffectLedger`).

The design also specifies:
- statistics in SQL-visible tables, including `fts5vocab`;
- vtab `estimatedRows` from those statistics;
- one statement per exact-scope group, split only on measured limits;
- the 8 MiB limit as a safety check only.

**Perspective.**
- This is the most direction-aligned planning code anywhere: principles 3, 4 and 5, README §3 and §5. It is a better source for `engine/plan/` than SSC's `physical.rs`/`job.rs`. Scope identity, grouping and semantic utility dedup map one-to-one onto "batches, with sharing only across identical scope" and "compile-time utility dedup".
- It diverges on operational semantics. `EffectSummary { replay_work, relative_peak_depth, fills }`, the `LogicalEffectLedger`, `OrderedRegion`, `FirstWitnessRelation` and the "budget contract" exist to keep memo replay, allowances and first-decisive order exact across sharing. `findings-contract.md` §3 removes those as requirements; budgets become `Unknown` per (rule, file).
- Under the loose contract, `semantics.rs` collapses: most `Pushed(…)` owners become declarative or fragment-owned, and `EffectLedger` disappears.

**For the integration.**
- Merge N1562 at `8b6576f0` (not `16343282`) in phase 3. `query/plan/` is new and disjoint, so expect few conflicts.
- In phase 4, move `query/plan/{identity,scope,grouping,utility}.rs` to `engine/plan/`.
- Mutate `semantics.rs` and the effect ledger out in phase 5.

## SSC

**New** (uncommitted, in the worktree named `…Moti-Velz-sql-set-composition`, which is on SSC's branch):
- `docs/sql-matcher-fragment-transition.md`, a 2026-10-25 decision: the generic SQL state machine is rejected. SQL composes typed relations; coherent matching stays in batched native matcher fragments behind one vtab filter, with IN-batch demand, typed captures, explicit unknown outcomes and file-isolated failure.
- It removes about 3.7k lines of finite and recursive SQL interpreter emitters and adds `fragments.rs` and its tests.
- A composition control passes (a join of two fragment relations, plus exclusion and aggregation).
- It targets the 11 catalog refusals (B615, SIM905, B020, F405 and others).
- It says production planning reads statistics from SQL, and compiled output is cached by digest and compiler version.

**Perspective.**
- This is direct convergence with README §2–§4 and principle 1, and it retires what architecture.md already marked not-carried (the frame/emit engine).
- HI D73 (query-as-data interpreter: 12x slower) predicted it.
- Its "open semantics" section still preserves exact work charges, cache-fill accounting and depth "within a complete execution region". Under decision 0001 that constraint goes away, which frees fragment boundaries.
- The fragment contract (demand → completion, outcome, typed captures; one filter per batch) is the right shape for the `terminal_match` relation and should become its native binding.

**For the integration.**
- Phase 2 should merge SSC at its next commit, after this lands. Merging `b87eba3b` would bring in the interpreter emitters it is about to delete.
- If the work stays uncommitted, merge `b87eba3b` and expect to delete the same files in phase 5.

## Moti: facts-plan

New to this survey (only `facts-engine` was pinned). `DECISION-CROSSFILE.md` reports results on 40 worktrees, 1,111 events and the 5 repository plans:

| Design | Memory | Latency | Outcome |
|---|---|---|---|
| Full differential-dataflow maintenance | 28–42 GB | | Cannot reach 250 MB; floor about 50 B × 120M records |
| Recompute | | 5–70 s per edit for dead code and exported-families | Inputs change on 88% of edits |
| Shared declared-facts layer (X6) | | 0.3–0.7 ms per rule per file | Exact, but does not fix the cross-file arithmetic |
| **Y1:** maintained reference graph (keyed tables, cached per-key queries with read sets, base + overlays + worktree deltas) | 72 MB base | 0.73 / 2.4 ms p50/p95 per edit | Exact against 145,431 edges |
| **Y2:** dead code and exported-families as local queries over the graph | 24 MB across 41 worktrees | 0.07 / 1.5 ms | 0 diffs |

Its conclusion: the target is unreachable "over these rules as authored". It takes rule-shape change, with local queries instead of global fixpoints, plus one maintained compact reference graph.

**Perspective.**
- This is the strongest evidence yet for authored-sql.md's recommendation that generic analyses be engine relation contracts, computed once and shared, with rules as local declarative queries. It quantifies why DLRE's rule-authored global fixpoints (categories E and G) cannot meet per-edit.
- It also confirms that cross-file per-edit needs maintained state: one structure, the reference graph. The direction docs don't yet cover incremental maintenance; this fills that gap.
- It conflicts with nothing except the facts-engine executor itself, which is already not carried. Y1's base + overlay + delta model is independent of that executor.

**For the integration.**
- Phase 7 should take the Y1 design (and `DECLARED-FACTS.md`) as the specification for the `reference_graph` / `symbol_ref` relation contract.
- Phase 5 should re-express the dead-code and exported-families rules as local queries.
- Treat "one maintained reference graph" as a decision record candidate.

## Tempo Metal

- Six commits fix terminal ownership and retirement, and containment with maximal anchors.
- Handover: frozen full run on `441c218c3`, 577 dispatches complete, failing at containment scratch (22.6 MB request with 1.995 GB reserved), 701.9 s, process-group RSS 3.24 GB, 600-dispatch limit.
- No full-corpus parity yet.

**Perspective.**
- It is making progress on correctness and memory ownership, but it is still a full-corpus batch executor on the pre-v7 `policy_dsl` transducer in another repo.
- The direction needs the GPU for per-edit, where the README's limiting factor is launch count per file. The current trajectory (more dispatches, about 2 GB) is the opposite regime.
- Its exactness work (ordering, complete-value grouping) partly serves exact output order, which findings-contract §3 drops.
- Keep the affordances (typed output pages, IN-demand). Phase 6 should port onto v7 behind the `terminal_match` contract rather than chase full-corpus parity in Tempo.
- Re-pin at `e95613625`.

## Changes to the integration plan

1. N1562: merge at `8b6576f0`, and use `query/plan/` as the `engine/plan/` source instead of SSC `physical.rs`/`job.rs`.
2. SSC: prefer merging after the fragment transition is committed; otherwise expect to delete the interpreter emitters in phase 5.
3. DLRE: plan a phase-1b re-merge of its execution-owner work and `full_repository.py`. The same files will conflict again (`file_tape` Option, `FileImage`).
4. Add `facts-plan` (Y1/Y2) as a phase-7 input and a decision-record candidate.
5. Tempo: re-pin at `e95613625`.
6. Three streams (N1562 `semantics.rs`/EffectSummary, SSC "open semantics", Tempo ordering) still encode exact operational semantics. They need to hear decision 0001 before investing further.
