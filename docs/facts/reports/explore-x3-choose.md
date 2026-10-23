# Exploration X3: per-clause maintain-vs-recompute, chosen by measurement

## Decision-table row

| Design | Memory, load / after 1,000 edits | Per-edit p50 / p95 / p99 | Exactness | Breaks | Kill point | Commits | Report |
|---|---|---|---|---|---|---|---|
| (c) per clause, maintain or recompute (from scratch per altering edit), chosen by measurement | Chosen mix = maintain all 5 = today: 29–31 GB RSS at load, 42 GB at edit 1,000 (S3 § 15.3). Memory-floor mix (recompute `exported-families` and dead code, maintain the 3 light clauses): **1,744 MB footprint at load** (7,618 MB RSS), 1,786 MB after 10 events, **peak 5,780 MB footprint / 9,095 MB RSS** (one worktree's from-scratch dataflow). After 1,000 edits: not run (killed first) | Memory-floor mix, 10 events: **38.2 / 43.2 / 43.2 s** (6 edits recomputed both clauses; 4 `whitespace` edits skipped them, 2–3 ms). Per recompute in that run: `exported-families` 4.3 s, dead code 37.1 s (p50). Single-clause 300-event runs: `exported-families` 7.6 / 12.2 / 14.7 s | 0 differences in 123 checks of held recomputed findings against a fresh recompute (30 per light clause, 30 `exported-families`, 1 dead code, 2 in the mix run); 93 of them fell on events that skipped the recompute; 0 runtime refusals | I8 (recomputed clauses get no generated delta plans); I3 if the choice is a clause list rather than a plan property; the gate's incremental ≡ from-scratch check is vacuous for recomputed clauses | **Killed.** No mix reaches < 2 GB with p95 < 500 ms. The 2 clauses holding 99.9 % of the state alter their inputs on 928 of 1,050 single-file edits (88 %). Recomputing them costs 9–25× (`exported-families`) and 75–90× (dead code) the 500 ms budget. The memory-floor mix also peaks at 5.8 GB footprint | `109107c2b` (harness), plus this report | `docs/facts/reports/explore-x3-choose.md` |

## 1. What was built

`facts-b greplay --recompute P1,P2,…` (commit `109107c2b`, `spikes/facts-b/src/gen/drive.rs`) is a disposable harness:

- The named real plans are left out of the maintained dataflow. Their exported rows stay resident, as the maintained inputs.
- At load, each recomputed plan is evaluated once per worktree with S3's from-scratch dataflow (`drive::scratch`, that plan alone, the worktree's members tagged base, run to completion and dropped). The harness keeps only the findings and the live refusals.
- On each event, a recomputed plan is re-evaluated for the event's worktree only if the event alters one of the plan's exported relations. The test compares the changed path's rows as multisets, old content against new. New worktrees always recompute.
- Skipping is exact: equal inputs give equal outputs. The check runs every 10th event and compares the held view with a fresh recompute. Of those checks, 93 fell on events whose recompute had been skipped. 0 differences.
- Event latency = maintained step + the alter test + the recomputes. Maintained plans run as before.

**Recompute engine.** The assignment names facts-a's executor (`eval::finish`). It cannot serve here:

- Dead code is refused at compile time in facts-a (`compile: unreachable-internal-symbols/raw_modules: projection file.module`).
- `exported-families` takes 49.9 s in `finish` on worktree 01 (5,725 files), 48.9 s of it in `base_edges` (`phase-3/p3c/runs/20261023T141318-repo01-all60`).
- facts-a per worktree 01: `required-contracts` 22 ms, `state-flow-project-1` 0.5 ms, `quality-project-taxonomy` 0.1 ms.

The table therefore uses the faster engine available: S3's from-scratch differential dataflow over the same lowered plans.

## 2. Per-clause table (300-event prefix; dead code: 10 events, see § 4)

Machine: 18 cores, 128 GB, shared, load average 38–63 throughout, no bench-lock. Counts are deterministic; times are inflated by contention. S3 measured from scratch unloaded at 3.3 s for the 4 light plans together and 32 s for dead code on a 9,010-file worktree. Memory is the physical footprint (`proc_pid_rusage`). RSS is in the run files. It disagrees under memory pressure: the maintained dead-code run was at 6.6 GB RSS against an 18.9 GB footprint.

| Clause | Maintained: footprint at load / peak | Maintained: arranged records (edit 300) | Maintained: per edit p50 / p95 / p99 | Edits altering its inputs (of 1,050 single-file) | Recompute: one worktree p50 / p95 / max (40 worktrees, 5,493–5,725 members) | Recomputed: footprint at load / peak | Recomputed: per edit p50 / p95 / p99 | Choice |
|---|---|---|---|---|---|---|---|---|
| `required-contracts` | 57 / 62 MB | 26.0K | 0.05 / 0.11 / 0.19 ms | 66 (6.3 %) | 3.9 / 5.0 / 5.2 ms | 60 / 67 MB | 0.00 / 3.7 / 5.7 ms | maintain (state < 50 MB) |
| `quality-project-taxonomy` | 81 / 96 MB | 5 | 0.04 / 0.08 / 0.16 ms | 0 | 0.5 / 0.8 / 9.5 ms | 77 / 96 MB | 0.00 / 0.00 / 0.00 ms | maintain (state < 50 MB) |
| `state-flow-project-1` | 119 / 120 MB | 3 | 1.4 / 2.0 / 2.4 ms | 0 | 33 / 66 / 74 ms | 142 / 188 MB | 0.01 / 0.02 / 0.06 ms | maintain (state < 50 MB) |
| `exported-families` | 8,127 / 8,520 MB (S3 unloaded: 9,697 MB RSS) | 31.6M | 7.0 / 32.5 / 83.5 ms | 928 (88 %) | 8.8 / 11.5 / 12.6 s | 797 / 1,550 MB | 7.6 / 12.2 / 14.7 s | maintain (recompute p95 > 500 ms) |
| `unreachable-internal-symbols` (dead code) | 18,926 / 22,019 MB (S3 unloaded: 23,578 MB RSS) | 139.5M | 38.7 / 153 / 435 ms | 928 (88 %) | 71.0 / 81.1 / 90.9 s | 1,704 / 7,844 MB | 38.6 / 45.8 / 45.8 s (10 events, 6 recomputed) | maintain (recompute p95 > 500 ms) |

- Each footprint includes the store and the process baseline, 47–110 MB (`load.rss_store_mb`).
- Recomputed state at load is the inputs plus the held findings:
  - Dead code: 535 MB of exported rows and the 600 MB interner. The interner is filled by the recomputes, and never freed.
  - `exported-families`: 75 MB of rows and 43 MB interned.
  - The rest of the footprint is allocator retention after the dropped dataflows.
- The peak is the from-scratch dataflow of one worktree.
- "Edits altering its inputs" is counted offline over the full replay: the exports of (path, old content) against (path, new content), per clause, from `exports/w6/replay-*.jsonl`. On the prefix the harness counted 16/298 (`required-contracts`), 255/298 (`exported-families`) and 6/10 (dead code).

**Rule.** Recompute a clause iff two conditions hold:

1. Its maintained state is ≥ 50 MB (worth removing).
2. Its recompute p95 per altering edit is ≤ 500 ms (inside the latency budget).

Otherwise maintain it. Recompute cost depends on the worktree, not the edit: it ranges over 5,493–5,725 members, within ±15 %. The 88 % altering rate means a clause's p95 per edit is its p95 recompute time.

The rule gives **maintain all 5**: the 3 light clauses fail (1), and the 2 heavy ones fail (2). That mix is today's dataflow: 29.3 GB at load, unlocked, S3 § 15.2. It fails the < 2 GB kill criterion.

## 3. Every mix (32 = 2^5), against the kill criterion

The 3 light clauses contribute < 100 MB and < 6 ms in either mode, so the mix reduces to the 2 heavy clauses:

| `exported-families` | dead code | Memory (footprint) | Per-edit p95 | < 2 GB and p95 < 500 ms |
|---|---|---|---|---|
| maintain | maintain | 29.3 GB RSS at load (S3) | 60–64 ms (S3 protocol) | no (memory) |
| recompute | maintain | ≈ 18.9 GB + 0.8 GB | ≈ 12 s | no (both) |
| maintain | recompute | ≈ 8.1 GB + 1.7 GB, peak ≥ 7.8 GB | ≈ 46 s | no (both) |
| recompute | recompute | measured jointly: 1,744 MB footprint at load (7,618 MB RSS), peak 5,780 MB footprint / 9,095 MB RSS | 43.2 s (p50 38.2 s, n = 10) | no: latency 86× over; memory passes only on steady footprint, and fails on peak and RSS |

The ≈ rows add the single-clause measurements of § 2. The last row is a joint run (§ 4).

## 4. Runs

All runs are in `~/rerail-bench/facts/phase-3/x3-choose/runs/`, using the w6 exports and the specs in `x3-choose/specs/`. Those are S3's `w6/spec-replay-*.json` with the 2 rule files of the deleted `s3-crossfile` worktree replaced by copies at `703e5b93`, in `x3-choose/policies/`; both are unchanged in git since then. Scripts: `maint300.sh` and `recompute.sh`.

- Maintained, one clause alone, 300 events, no checks, binary `/tmp/s3w6/facts-b-r48` (S3's protocol binary): `20261023-155709-maint300-required-contracts`, `…155725-…-quality-project-taxonomy`, `…155734-…-state-flow-project-1`, `…155746-…-exported-families`, `…160034-…-unreachable-internal-symbols`.
- Recomputed, one clause alone, 300 events, check every 10th: `20261023-160227-recompute300-quality-project-taxonomy`, `…160236-…-required-contracts`, `…160245-…-state-flow-project-1`, `…160302-…-exported-families` (47 min).
- Dead code recomputed, 10 events, 1 check: `20261023-170739-recompute10-unreachable-internal-symbols` (55 min: 45 min for the 40 load recomputes). A first attempt with 20 events hit the job's 60 min deadline after its load (3,147 s for 40 recomputes; RSS 5.9 GB at load). It was killed before writing results, and its directory was removed.
- Memory-floor mix, all 5 plans with `--recompute exported-families,unreachable-internal-symbols`, 10 events, check every 10th: `20261023-180245-mix-floor10-all5` (37 min). The machine was less loaded than in the single-clause runs: per worktree, `exported-families` took 4.6 s and dead code 40.1 s (p50 over 40).
- Full replay: not run. The design is killed on the prefix: at ≈ 50 s per recompute and 88 % altering edits, the 1,050 edits alone would take ≈ 13 h.

Ceremony: `cargo test --release -p facts-b`, 14/14. `just facts-t0` and T1 were not run; the harness touches only `greplay`'s driver.

## 5. What the design breaks

- **I8.** A recomputed clause gets no generated delta plans; I8 assigns delta plans to every membership-dependent plan. The harness keeps I8's other half: per-worktree state is the findings only.
- **I3.** The choice made here is a list of clause names, i.e. a per-rule execution path. A shippable version would have to derive it from the plan, e.g. estimated state from I2's cardinality bounds against estimated recompute cost.
- **I9 holds.** Recompute is from scratch, and the skip is by input equality. 0 differences, 0 runtime refusals.
- **I6 is unaffected.**
- **Gate.** The incremental ≡ from-scratch check is vacuous for a recomputed clause. The skip rule would need its own differential test and mutant (I10).

## 6. Why no mix can work, and what would have to change

- The memory sits in 2 clauses: `exported-families` and dead code, 99.9 % of the arranged records.
- Their inputs change on 88 % of edits. The breakdown by edit kind is identical for both clauses:
  - every `call` (211), `delete_fn` (271) and `rename_fn` (212) edit changes them;
  - so do 151 of 152 `import` edits and 83 of 94 `revert` edits;
  - the only unchanged ones are all 110 `whitespace` edits, plus 11 reverts and 1 import.

  Skipping unchanged inputs therefore removes only 12 % of recomputes.
- Recompute is O(worktree) per edit: 5,500–5,700 files, 5.0M stage rows for dead code (S3 § 15.1).
- To fit the budget, a from-scratch evaluation of dead code would need ≤ 0.5 s, i.e. ≤ 100 ns per stage row including the linker joins and the reachability fixpoint. That is 64× faster than S3's unloaded 32 s, and 80–140× faster than measured here (40–71 s p50 per worktree, depending on machine load). It would have to come from a columnar engine that does not run this clause today: facts-a refuses it at compile time.
- Even at that speed, recomputing dead code peaks at 7.8 GB (one worktree's dataflow), against the 2 GB criterion.
- Recompute-vs-maintain is a valid lever only for clauses that are cheap both ways. Here those are the 3 light clauses, which hold < 0.1 % of the state. The decision for the heavy clauses must come from a design that shrinks maintained state: pruning, a shared linker, columnar arrangements. It cannot come from choosing between the two existing implementations.
