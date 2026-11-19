# M1: per-rule baseline on the current engine

Scripts: `../experiments/m1/` (recordings not checked in). Stopped early by direction; the per-rule table is complete. The per-rule table below is complete: all 8 rules ran twice. The stage breakdown is complete for serverplan-steps only. The 60-rule run was skipped because it isn't cheap: serverplan-quality alone needs 10 GiB and about 350–410 s CPU, and DLRE's own first-50 run took 1,990 s wall before the all-60 run was cancelled.

## Setup
- **Timing addon (runs -1 and -2):** the Pulling-it-all-together build, `/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/native/rerail_engine.darwin-arm64.node` (sha256 `c5a261c0…`).
  - It was built at 09:38, after every native source file was last changed. The tree is the staged DLRE merge at `d3470da2`, which is exactly the head DLRE measured its gate at.
  - It is a production release build: thin LTO, one codegen unit, embedded jemalloc, and no `code-rule-profile` feature, so it records no stage spans.
- **Profile addon (-prof runs):** DLRE's `.rerail/profiles/full-scale-direction-20261025/matched-addon/engine.node` (`0b748ebd…`). It comes from the same `d3470da2` engine, but with opt-level 1, `code-rule-profile` and the system-malloc shim. Its absolute times are inflated; use it for proportions only.
- **Driver:** unchanged `conformance/data-layer/suite.ts`. The recording is `all-60-recording.json` (pinned commit `2b1504ad`, 9,021 files, 137.9 MB), filtered to one rule each. I used the Pulling-it-all-together policies, 4 native workers, and no `--cold-only`.
- **Command (from `scratchpad/m1/run_one.sh`):**
  `RERAIL_ENGINE_PATH=<addon> RERAIL_NATIVE_WORKERS=4 /usr/bin/time -lp -o resource.txt node --import tsx conformance/data-layer/suite.ts --recording recordings/<rule>.json --output parity.json --policies packs/data-layer/candidates/rules --progress`
- **Columns:** "Cold" is the driver's measurement for a fresh process. "Ingest" is the per-file applies (parse, SQL anchor selection, AST matching). "Global" is the repository-wide relational apply. "Edit" is the warm one-file edit of `impl/common/l2200_sql_dialect/sql_connection.py`.

## Per-rule results (run -1 / run -2)
Load average at the start of each run: run -1 was 35–62 on 18 cores; run -2 was 8–48, and mostly 8–17.

| Rule | Process wall s | user / sys s | Peak RSS GiB | Cold CPU s | Ingest / Global CPU s | Warm edit CPU s (wall) | Findings (actual/expected) |
|---|---|---|---|---|---|---|---|
| serverplan-state-tripwires | 304 / 186 | 148/44 · 133/46 | 1.90 / 1.90 | 190 / 177 | 149/41 · 142/35 | 0.4 / 0.3 | 0/0, **1 failure** |
| serverplan-steps | 213 / 97 | 52/8 · 54/5 | 1.36 / 1.33 | 59 / 57 | 25/34 · 23/34 | ~0 (out of scope) | 0/0 |
| serverplan-quality | 1145 / 448 | 250/186 · 242/125 | **10.0 / 10.5** | 413 / 346 | 341/72 · 279/67 | 19 (41 s) / 18 (18 s) | 0/0 |
| internal-dead-code | 418 / 192 | 181/48 · 147/40 | 3.25 / 3.85 | 195 / 161 | 160/36 · 133/28 | 32 (76 s) / 25 (25 s) | 4931/4931 |
| public-api-triples | 234 / 126 | 125/42 · 87/32 | 2.06 / 2.05 | 158 / 113 | 151/7 · 109/4 | 6.8 / 4.3 | 0/0 |
| async-sync-delegation | 133 / 67 | 48/41 · 30/31 | 1.28 / 1.39 | 86 / 60 | 86/0 · 60/0 | 0.16 / 0.12 | 0/0 |
| transaction-patterns | 87 / 48 | 23/36 · 14/29 | 1.40 / 1.40 | 58 / 42 | 58/0 · 42/0 | 0.14 / 0.12 | 0/0 |
| async-sync-boundary | 91 / 47 | 20/43 · 12/28 | 1.28 / 1.45 | 61 / 39 | 61/0 · 38/0 | 0.16 / 0.07 | 0/0 |
| **Sum of 8 (cold CPU)** | | | | **1,220 / 994** | | | |

- **Noise:** wall roughly halves between the two passes and cold CPU drops 3–35%, so CPU itself is load-sensitive on this host. Treat run -2 as the better figure.
- **Compile:** under 0.6 s for every rule.
- **Warm edit:** the edited file is out of scope for serverplan-steps, so its edit figure means nothing. For the rules where it is in scope, the edit cost is large: internal-dead-code 25–32 s CPU and serverplan-quality about 18 s.

## Why serverplan-state-tripwires exits 1
It is an engine error, not a findings mismatch. Findings match (0 expected, 0 actual, no false positives or negatives). The run fails because `parity.json` records one failure: `state-flow-project-1`, `source-incomplete`, "repository input budget exceeded". The same failure repeats in the unchanged and edit phases, which is the "2 incremental differences" in the output. It happens in both passes.

A likely factor: the merged tree's policy for this rule lacks the `requireNonemptyInput: "events"` guard on four clauses. DLRE added it later (commit `7222d7e8`), and the merged engine already supports it. I queued a run with DLRE's policy to test this, but it was cancelled before it started, so this is unverified.

## Which engine path each rule takes
- **Native matcher, through SQL anchor selection, no relational stage:** async-sync-delegation, transaction-patterns and async-sync-boundary. Their global CPU is 0.
- **Relational executor (SQLite) at the global stage:** state-tripwires (32 relational clauses), steps (7 relational + 14 forbid), quality (6 relational + 1 forbid), internal-dead-code (1) and public-api-triples (1).
- **Even AST-only rules are not pure native matching:** ingestion runs SQL candidate (anchor) selection first, so "the existing native matcher" in this tree is SQL-assisted matching.

## serverplan-steps stage breakdown (profile addon, cold CPU 81.4 s)
**Ingestion, 25.0 s** (native worker counters, exclusive):

| Stage | CPU s |
|---|---|
| Anchor selection (SQL candidate selection) | 20.7 |
| Anchor index | 2.8 |
| Parse (only in-scope files are parsed) | 0.56 |
| Manifest | 0.43 |
| Tape | 0.3 |
| AST match | 0 |

AST match is 0 because selection rules out every candidate. In the trace spans, the anchor SQL work is working-set 9.7, materialize 4.3, results 4.2 and prepare 0.9; fact preparation is 1.4.

**Global relational, 56.4 s** (exclusive span CPU, same-thread nesting removed):

| Group | Stages | CPU s | Share |
|---|---|---|---|
| SQLite | sql-result 20.4, sql-materialize 10.5, sql-index 5.5, sql-input 1.2, sql-analyze 0.75 | 38.4 | 68% |
| AST witness production | input-stream 9.1, producer-prepare 1.9, input-preparation 0.6 | 11.6 | 21% |
| Batch overhead | relational-batch | 6.4 | 11% |

In the release build the global stage is 34 s, so apply about the same proportions.

- **serverplan-quality-prof** finished but cannot be used as a breakdown: on the opt-1 profile addon it failed 858 times (856 "producer stage limit" on `quality-contracts-1`, plus "interrupted" and "budget exceeded" on `quality-conditional-1`). The release addon passes the same rule cleanly.
- **Partial or not started:** state-tripwires-prof and internal-dead-code-prof were killed partway. The empty-rule baseline and the tripwires/DLRE-policy run never started.

## High sys time
Sys is 30–70% of CPU for the scope-wide rules. For example, transaction-patterns is 14 s user / 29 s sys, and quality reaches 125–186 s sys. serverplan-steps, whose AST scope is only 264 files, shows only 5–8 s sys. So sys seems to scale with the number of in-scope files, and it fell between passes as load dropped.

I didn't profile it. Likely causes are the per-file storage writes and reads in the content store, page faults from the large SQLite anchor working sets, and contention on a heavily loaded host (DLRE recorded about 16.5 GiB swap in use during its gate run). A `sample` or `fs_usage` run would tell them apart.

## Caveats
- The machine was shared and noisy, and each figure comes from at most 2 runs.
- "Cold" means a fresh process; the OS file cache was not flushed.
- Release-build times and profile-build proportions come from different optimisation levels.
- The per-rule sum (994–1,220 s) double-counts the fixed per-run cost of ingesting the corpus, so it is not a 60-rule estimate. The empty-rule baseline that would measure that fixed cost was never run.

Everything is in `../experiments/m1/`:
- `runs/<rule>-{1,2,prof}/`: `resource.txt`, `parity.json`, `stderr.log`
- `run_one.sh` and `batch.sh`: the runners
- `make_recordings.py`: builds the one-rule recordings
- `summarize.py`: produces the table
- `analyze_prof.py`: produces the stage breakdown
- `recordings/`: one-rule recordings, about 144 MB each
