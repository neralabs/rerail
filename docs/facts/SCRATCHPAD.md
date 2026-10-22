# Facts engine: scratchpad

Decider-maintained. Newest entries first within each section. Numbers here are either copied from a cited run directory or marked `[reported, unverified]`.

## Status

| Item | State | Owner | Branch | Notes |
|---|---|---|---|---|
| Phase 1 plan | written | decider | `Moti-Velz/facts-plan` | `docs/facts/{PLAN,PHASE-1,SCRATCHPAD}.md` |
| Spike A: facts once + in-memory evaluation | run 1 out of budget; run 2 (SpikeA2) launched | SpikeA2 (Opus 5.5) | `Moti-Velz/facts-spike-a` @ `56aff3818` | census + evaluator + recording parity done; A1-repo, A2–A5 and fact-file writer open |
| Spike B: base + delta dataflow | launching | SpikeB (Opus 5.5) | `Moti-Velz/facts-spike-b` | R1–R3 on 40 worktrees, gates B1–B5 |
| Phase 2 compiler | not started | | | blocked on Phase 1 go/no-go |

## Gate ledger

| Gate | Target | Measured | Verified by decider | Decision |
|---|---|---|---|---|
| A1 parity | 0/0/0 | recordings: 1,253/1,259 calls, 0 FP, 2 FN, 6 failures, 9 of 10 checks exact; all 8 misses are `quality-contracts-1` `quoted_atoms` (`parse: {grammar: annotation}`), refused as a missing fact. Repository: not run | artifact read: `~/rerail-bench/facts/spike-a/runs/20261022T120000-replay/results.json` | continue; classifier is Phase 2 schema backlog, not a v0 relation error |
| A2 CPU ≤ 10% native | ≤ 40 s (393 s) | | | |
| A3 per (clause, file) | mean ≤ 0.5 ms, p95 ≤ 5 ms | | | |
| A4 extraction | p50 ≤ 10 ms, p99 ≤ 100 ms | | | |
| A5 peak RSS | ≤ 2 GiB | | | |
| B1 exactness | 0 differences | | | |
| B2 latency | p50 ≤ 10, p95 ≤ 50, p99 ≤ 200 ms | | | |
| B3 resident | ≤ 200 MB | | | |
| B4 per-worktree state | ∝ divergence | | | |
| B5 new worktree / bulk | ≤ 1 s / ≤ 2 s | | | |

## Baselines (measured earlier, for comparison)

- Batch 6 native: 323 s wall, 393 s CPU, 28.6 GiB; global phase 267 s (198 stage SQL, 45 re-extraction, 13.5 setup). Python oracle 23.4 s. (`~/rerail-bench/dl60-split/`)
- Native per-file matching on his rules: 0.4–1.4 ms per rule per file.
- 47 worktrees: 7,091 distinct contents of 262,567 files.
- Store compaction merged: native 8 worktrees cold 93 s, warm 14 s.

## Decisions

- Phase 1 = the two spikes. Base `e6d2ac73c` (`Moti-Velz/sql-dl60`).
- Executors are `task` subagents; the `task` model role resolves to `anthropic/claude-opus-5-5:high` (cfg `modelRoles`). Mechanical work may go to `sonic` subagents.

## Open questions

- Which batch-6 rules depend on facts outside the syntactic schema: answered by the census, 41 of 775 units (decoded strings, f-string and annotation sub-grammars, line splitting); the annotation classifier is the only one that blocks a recorded call.
- Divergence of the 47 worktrees from a common base (B reports it; it sets per-worktree state).

## Log

- Phase 1 docs written; spike worktrees created from the plan commit; SpikeA and SpikeB launched.
- Spike A run 1 (53 min, request budget): census 775 units over 105 clauses: 418 (54%) rebuild the syntax tree in SQL → facts; 316 rule logic over facts; 41 need facts outside v0 (decoded strings, f-string and annotation sub-grammars, line splitting). Evaluator is not `ascent`: an own SQLite-subset parser (`sqlparser` fails on 15 of 452 statements) and an in-memory executor whose join order follows the SQL text with lazy hash indexes; recursive CTEs level by level; no SQLite, no planner. This is closer to Phase 2's runtime evaluator than Ascent would have been, so the Ascent-optimism open question is moot. Batch-6 SQL: 544 KB, 404 stages, 58 recursive CTEs, 83 `row_number()`, 99 correlated `EXISTS`. Production change: visibility only (`baca38d83`, engine also builds as rlib).
- Decision: SpikeA2 resumes on the same branch. Order: fix the two allocation hotspots → `repo` command, 4 threads, streaming → repository parity (173) → A2/A5 under bench-lock with `/usr/bin/time -l`, 3 runs → A3/A4 from the existing timers → fact-file writer and bytes per file. A2 reported twice: all 10 rules as executed, and excluding `quality-contracts-1` on both sides using the dl60-split apportioning. No Ruff annotation-classifier port in Phase 1.
- Risk noted by Spike A: row-order-sensitive SQL (`row_number()` ties, `LIMIT 1` with partial `ORDER BY`, unordered `group_concat`) is resolved by input order; no replayed call diverged, but the repository run must confirm.
