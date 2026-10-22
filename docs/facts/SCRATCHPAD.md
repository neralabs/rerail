# Facts engine: scratchpad

Decider-maintained. Newest entries first within each section. Numbers here are either copied from a cited run directory or marked `[reported, unverified]`.

## Status

| Item | State | Owner | Branch | Notes |
|---|---|---|---|---|
| Phase 1 plan | written | decider | `Moti-Velz/facts-plan` | `docs/facts/{PLAN,PHASE-1,SCRATCHPAD}.md` |
| Spike A: facts once + in-memory evaluation | run 1 out of budget; run 2 (SpikeA2) launched | SpikeA2 (Opus 5.5) | `Moti-Velz/facts-spike-a` @ `56aff3818` | census + evaluator + recording parity done; A1-repo, A2–A5 and fact-file writer open |
| Spike B: base + delta dataflow | **done, verified** | SpikeB (Opus 5.5) | `Moti-Velz/facts-spike-b` @ `23d89d3e6` (gate binary `3019c8adf`) | B1, B2, B4, B5 pass; B3 255 MB under the strict-majority base, 195–198 MB with per-path or cluster bases |
| Phase 2 compiler | not started | | | blocked on Phase 1 go/no-go |

## Gate ledger

| Gate | Target | Measured | Verified by decider | Decision |
|---|---|---|---|---|
| A1 parity | 0/0/0 | recordings: 1,253/1,259 calls, 0 FP, 2 FN, 6 failures, 9 of 10 checks exact; all 8 misses are `quality-contracts-1` `quoted_atoms` (`parse: {grammar: annotation}`), refused as a missing fact. Repository: not run | artifact read: `~/rerail-bench/facts/spike-a/runs/20261022T120000-replay/results.json` | continue; classifier is Phase 2 schema backlog, not a v0 relation error |
| A2 CPU ≤ 10% native | ≤ 40 s (393 s) | | | |
| A3 per (clause, file) | mean ≤ 0.5 ms, p95 ≤ 5 ms | | | |
| A4 extraction | p50 ≤ 10 ms, p99 ≤ 100 ms | | | |
| A5 peak RSS | ≤ 2 GiB | | | |
| B1 exactness | 0 differences | 0 differences in 1,576 worktree checks (1,112 checkpoints), R1–R3 as sets, multiplicity 1 enforced | re-run by decider: `20261022-122941-verify-exactness-flat`, 0 differences | **pass** |
| B2 latency | p50 ≤ 10, p95 ≤ 50, p99 ≤ 200 ms | reported p50 0.170 / p95 0.388 / p99 0.939 / max 4.0 ms over 1,050 edits | re-run by decider: `20261022-…-verify-timing-flat`: 0.185 / 0.425 / 1.016 / 4.4 ms, load 2.3 | **pass** (50× under target) |
| B3 resident | ≤ 200 MB | 255.0 MB after 1,000 edits with the spec's strict (path, hash)-majority base; 88 MB of it is process + fact store + interner before any dataflow. Variants: cluster bases 195.1 MB (exact, 0/1,576); per-path plurality 198.2 MB (not oracle-checked) | re-run by decider: 255.9 MB at edit 1,000 | **fail as specified, pass with per-path or cluster base**; under the 500 MB kill line. Phase 2/3: base chosen per path, hierarchical bases; eviction or mmap for the fact store |
| B4 per-worktree state | ∝ divergence | 2.0–2.7 MB per worktree at 104–818 delta paths; a clone holds nothing until edited, 1–4 KB after 10 edits; the 8,209-path fresh worktree 14.1 MB; correlation 0.9987 with net changed fact rows | artifact read: `20261022-121844-track-flat/results.json` (8 paths → ~0 MB; 8,209 → 14.1 MB) | **pass** |
| B5 new worktree / bulk | ≤ 1 s / ≤ 2 s | 0.110 ms / 45.8 ms max; real commits p50 11 ms, max 122 ms (1,418 files); whole tree 7,818 files in 425 ms | re-run by decider: 0.115 ms / 50.6 ms max | **pass** |

## Baselines (measured earlier, for comparison)

- Batch 6 native: 323 s wall, 393 s CPU, 28.6 GiB; global phase 267 s (198 stage SQL, 45 re-extraction, 13.5 setup). Python oracle 23.4 s. (`~/rerail-bench/dl60-split/`)
- Native per-file matching on his rules: 0.4–1.4 ms per rule per file.
- 47 worktrees: 7,091 distinct contents of 262,567 files.
- Store compaction merged: native 8 worktrees cold 93 s, warm 14 s.

## Decisions

- Phase 1 = the two spikes. Base `e6d2ac73c` (`Moti-Velz/sql-dl60`).
- Executors are `task` subagents; the `task` model role resolves to `anthropic/claude-opus-5-5:high` (cfg `modelRoles`). Mechanical work may go to `sonic` subagents.
- **Spike B: go.** The base + delta dataflow is exact and 50× under the latency target. Memory passes only with a per-path or hierarchical base, so Phase 2's membership model is: base chosen per path (most common content), cluster bases for runs of similar worktrees, worktree deltas relative to their cluster. The tag algebra in B's § 5.2 generalizes to that tree.
- Caveat carried into Phase 2: B's R1–R3 are precisely defined stand-ins over B's extractor, not ports of Jeremy's rules (§ 5.4 of its report). B proves the dataflow machinery and the memory model, not rule fidelity. Phase 2's gate runs the real rule semantics through the compiler against the oracle.
- Open for Phase 3: the 88 MB fixed floor (process + fact store + interner, no eviction; 15,717 contents accumulated by the end of the replay) needs eviction by membership or memory-mapped fact files.

## Open questions

- Which batch-6 rules depend on facts outside the syntactic schema: answered by the census, 41 of 775 units (decoded strings, f-string and annotation sub-grammars, line splitting); the annotation classifier is the only one that blocks a recorded call.
- Divergence of the 47 worktrees from a common base (B reports it; it sets per-worktree state).

## Log

- Phase 1 docs written; spike worktrees created from the plan commit; SpikeA and SpikeB launched.
- Spike A run 1 (53 min, request budget): census 775 units over 105 clauses: 418 (54%) rebuild the syntax tree in SQL → facts; 316 rule logic over facts; 41 need facts outside v0 (decoded strings, f-string and annotation sub-grammars, line splitting). Evaluator is not `ascent`: an own SQLite-subset parser (`sqlparser` fails on 15 of 452 statements) and an in-memory executor whose join order follows the SQL text with lazy hash indexes; recursive CTEs level by level; no SQLite, no planner. This is closer to Phase 2's runtime evaluator than Ascent would have been, so the Ascent-optimism open question is moot. Batch-6 SQL: 544 KB, 404 stages, 58 recursive CTEs, 83 `row_number()`, 99 correlated `EXISTS`. Production change: visibility only (`baca38d83`, engine also builds as rlib).
- Decision: SpikeA2 resumes on the same branch. Order: fix the two allocation hotspots → `repo` command, 4 threads, streaming → repository parity (173) → A2/A5 under bench-lock with `/usr/bin/time -l`, 3 runs → A3/A4 from the existing timers → fact-file writer and bytes per file. A2 reported twice: all 10 rules as executed, and excluding `quality-contracts-1` on both sides using the dl60-split apportioning. No Ruff annotation-classifier port in Phase 1.
- Risk noted by Spike A: row-order-sensitive SQL (`row_number()` ties, `LIMIT 1` with partial `ORDER BY`, unordered `group_concat`) is resolved by input order; no replayed call diverged, but the repository run must confirm.
- Spike B done in 1 h 21 min, 9 commits. Decider re-ran exactness (0 differences, 1,576 checks) and one timing row (p50 0.185 ms, p95 0.425 ms, RSS 255.9 MB at edit 1,000, load 2.3). Step 7 measured: base + delta holds 5.7× fewer arranged records than naive worktree keying for R1 (161 vs 403 MB) and a new worktree costs 0.012 ms instead of 314 ms. Extraction of 7,091 contents: p50 0.98 ms, p99 36 ms per file, 6.1 s wall on 4 workers. Divergence: 40 worktrees differ from the strict base in 104–818 paths (median 285); 104 paths exist everywhere but no version reaches a majority, which is what pushed B3 over 200 MB.
