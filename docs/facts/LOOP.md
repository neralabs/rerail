# The evaluation loop

Binding from 2026-10-23. Goal: an engine change is checked for correctness in under a minute, measured in about two, and proven at merge. Everything below is enforced by `just` recipes so that an agent cannot do it the slow way by accident.

## Tiers

| Tier | Command | When | Budget | What it proves |
|---|---|---|---|---|
| T0 | `just facts-t0` | before every commit | ≤ 60 s incl. incremental build | unit tests, random differentials against bundled SQLite 3.53.2 and the reference, and the mutants whose sites are in the files changed (git diff → mutant table) |
| T1 | `just facts-t1` | before every milestone, and for every A/B | ≤ 2 min | `sub200` parity (200 heaviest files, findings and failures identical to the committed reference), engine-vs-reference on 50 files, the edit-replay 100-event prefix for facts-b changes, and the archetype bench (§ Bench) reported as work counts |
| T2 | `just facts-gate --mutants` | at merge, by the integrator | ≤ 10 min | the full gate, unchanged in strength |

Rules: T0 and T1 build once, in the `loop` profile, into one target dir, with the `mutants` feature always on (hooks are env-gated and free when `FACTS_MUTANT` is unset). T2 builds in `release` once, into the same tree's `target/release`, no separate gate dir. No executor runs T2; the integrator does, on the merged tree.

## Bench

`just facts-bench [archetype…]`: one representative rule per archetype on `sub200`, single-threaded, reported as **work counts** (§ Counts), with wall-clock as a secondary column. Archetypes and their rules are fixed in `benchmark/facts/archetypes.json`: pattern-only (`impl-direct-execution`), relational per-file (`state-flow-1`), closure-heavy (`quality-contracts-1`), selection-heavy (`quality-conditional-1`), repository (`required-contracts`, through `greplay`). An A/B is two `facts-bench` runs; no bench-lock, no quiet machine, no repetition: counts do not move with load. Wall-clock is calibrated once per merge by the integrator (three locked runs on the batch-6 corpus, as today) and recorded in the scratchpad next to the counts.

## Counts

Work is counted deterministically at two levels, both additive over files and plans, both printed by `EXPLAIN ANALYZE`-style output and written to `results.json`:

- **Algebraic** (engine-independent; the reference evaluator and the engine report the same numbers): rows read per leaf, rows emitted per logical operator, derivations per fixpoint round, matcher node visits and predicate evaluations per input. Budgets (`maxVisitedNodes`, work allowances, closure steps) are defined over these units; a budget failure is a fact about the plan on that content, reproducible on every engine.
- **Physical** (engine-specific, deterministic for a given plan): index builds by rows inserted, probes, hash lookups, rows materialised, bytes copied. Regressions in constant factors show here.

Counts are memoised per `(content hash, plan digest, schema version)` with the results, so a corpus-level count after a change is the sum of stored counts for unchanged plans plus fresh counts for the plans whose digest changed. The gate adds a check: no clause's algebraic or physical count grows by more than 5% at merge without a registry row explaining it.

## Parity on changed plans only

`just facts-t1` and the gate's corpus parity re-evaluate a file only for clauses whose plan digest differs from the stored one; other clauses' findings and counts come from the memo. A rewrite that changes three plans re-runs three plans. The memo lives in the run directory for now (`~/rerail-bench/facts/memo/`), keyed as above; it is the content-store key of Phase 3 and moves there unchanged.

## Process

- Executors run T0 before every commit and T1 before every milestone; they never run T2 and never wait for a quiet machine.
- Every number in a report is a count, with the wall-clock column marked `calibration` or omitted.
- The integrator runs T2 once per merge and the three locked calibration runs once per merge.
- A slice that cannot keep T1 under 2 minutes for its archetype says so in its first milestone; that is a measurement, not a failure.
