# Facts engine: scratchpad

Decider-maintained. Newest entries first within each section. Numbers here are either copied from a cited run directory or marked `[reported, unverified]`.

## Status

| Item | State | Owner | Branch | Notes |
|---|---|---|---|---|
| Phase 1 plan | written | decider | `Moti-Velz/facts-plan` | `docs/facts/{PLAN,PHASE-1,SCRATCHPAD}.md` |
| Spike A: facts once + in-memory evaluation | launching | SpikeA (Opus 5.5) | `Moti-Velz/facts-spike-a` | batch 6, gates A1–A5 |
| Spike B: base + delta dataflow | launching | SpikeB (Opus 5.5) | `Moti-Velz/facts-spike-b` | R1–R3 on 40 worktrees, gates B1–B5 |
| Phase 2 compiler | not started | | | blocked on Phase 1 go/no-go |

## Gate ledger

| Gate | Target | Measured | Verified by decider | Decision |
|---|---|---|---|---|
| A1 parity | 0/0/0 | | | |
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

- Which batch-6 rules depend on facts outside the syntactic schema (needs the census).
- Whether `ascent`'s compile-time rules bias the A2 number optimistically versus a runtime evaluator; Phase 2 must re-measure.
- Divergence of the 47 worktrees from a common base (B reports it; it sets per-worktree state).

## Log

- Phase 1 docs written; spike worktrees created from the plan commit; SpikeA and SpikeB launched.
