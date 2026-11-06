# Benchmarks

Development tools that measure Rerail: native query engines against each other, the dashboard's history reads, and
code-rule steer strategies on real coding agents. Nothing here ships, and nothing here runs in CI.

- `query-parity.ts` (`just rerail-query-parity`) runs the same code-rule workloads through two or more native engine
  builds, each in its own process. It reports every section where their findings, failures or coverage differ.
- `query-parity-semantic.ts` names the semantic-resolution workloads (`resolves`, `usage`) that `query-parity.ts` runs.
- `dashboard-history.ts` (`just rerail-dashboard-history`) seeds a throwaway database with a long audit history (`--audits`,
  `--rules`, `--backlog`, `--sessions`), serves the daemon's API over it, and times what the dashboard reads: each history
  read computed and repeated, one poll while a session keeps committing, recording one repository audit, and how long
  `/healthz` waits while dashboard tabs poll (`--tabs`, `--seconds`).
- `steer-strategy.ts` (`just rerail-steer-strategy`) runs a real harness on one task per host × strategy × trial and
  compares the code-rule steer strategies (`message`, `rewind`, `subagent`). See [Steer strategies](#steer-strategies).
- `merge-assist.ts` (`node --import tsx benchmark/merge-assist.ts [--suites capture,transfer,compare,lookup] [--trials N]
  [--compare-trials N] [--concurrency N] [--cases id,id] [--pairs id,id] [--json out.json] [--verbose] [--keep]`)
  evaluates Merge Assist on `fixtures/host/benchmark/merge-assist/cases.json` through `createMergeAssistService` and
  the person's own agent CLI as judge. Each trial gets a fresh temporary Git repository (worktrees `jeremy` and
  `tim`), database and `RERAIL_HOME`, removed afterwards unless `--keep`. **capture** turns each correction session
  into a decision; **transfer** commits each saved code rule on Jeremy's branch and scans Tim's worktree with teammate
  files written independently; **scope** matches each decision's scope against paths it should and should not cover;
  **compare** scans Tim's decision against Jeremy's and reads the verdict from the finding's tier; **lookup** times
  1,000 generated scopes over 10,000 paths, with no model. It prints each metric against its pre-registered threshold
  (per-case rows with `--verbose`). Capture, transfer and compare spend the person's model budget on every judge call;
  results are never committed.
- `merge-assist-context.ts` (`node --import tsx benchmark/merge-assist-context.ts --build-index out.json
  [--distractors N]`, then `--index index.json --variants none,guard-signals,...
  --split dev|heldout|size|confirm|heldout-large|heldout-large2|all [--tasks id,id] [--trials N] [--concurrency N]
  [--distractors N] [--seed N] [--no-control] [--json out.json] [--keep] [--verbose] [--model m] [-c key=value]`,
  `--sweep control.json`, `--list-variants`, `--validate-oracles`, `--index index.json --amend out.json`) asks
  whether injecting teammates' decisions into a real Codex loop, when it touches files they cover, makes its code
  follow them, and which protocol does so with the least chatter. The dataset,
  `fixtures/host/benchmark/merge-assist/context.json`, reuses cases.json's project and
  names its ten lasting decisions (team or peer, with the hand-written text and scope the oracles grade through), the
  distractor pool and 85 tasks: per split (`dev` for tuning, `heldout` to confirm), nine that tempt one decision each,
  two neutral ones and two where this branch's decision contradicts a teammate's; then the `size`, `confirm`,
  `heldout-large` and `heldout-large2` splits of `--arms` below. Nothing hand-written about a real
  decision reaches the agent: `--build-index` runs the product's `draftDecision` with the person's agent CLI judge on
  each decision's capture case (correction, before and after) and keeps the decision text, scope and validated signals
  it produces, leaving out any the judge calls one-off or does not answer; distractors and the conflict tasks'
  teammate decisions get their signals from `deriveSignals`. The index stores each entry's provenance, every judge
  prompt and each decision's embedding (the bundled model), and is never rebuilt over an existing file. A decision
  changes the way the product changes one: `--index index.json --amend out.json` runs `draftDecision` on each
  amendment case in cases.json's `amend` list (the decision's author's correction, in the person's words, with the
  code before and after it and the decisions on that decision's branch as `ownDecisions`) and writes a new index in
  which that decision's entry is the candidate the product drafted, only when its `supersedes` names the decision
  (provenance `case` the amendment case, `amends` the candidate it replaced; a fresh embedding), and nothing when the
  product calls the correction one-off, a duplicate or a new decision. Every run reads it through `--index`, after an
  integrity check that refuses an entry with a field the product does not produce or an oracle's regex in it, and
  reports the oracles' code text the product's own output holds. Each run gets a
  temporary repository and `CODEX_HOME` (`auth.json` a symlink to `~/.codex/auth.json`, an empty `config.toml`, and
  `hooks.json` running `merge-assist-context-hook.ts` on PreToolUse and PostToolUse), then `codex exec --json
  --dangerously-bypass-hook-trust --sandbox workspace-write` for at most six minutes. The hook injects the variant's
  decisions as `additionalContext`; variants are presets in the script (`--list-variants`), `none` the control
  (`--no-control` skips it, for smoke runs). `--distractors N` adds N of the index's distractors to every run (the
  same ones, in the same places, every run), as noise a protocol has to filter. The `guard-*` presets orient on
  conflict pairs at a file's first touch and guard each patch: `signals` fires a decision in scope whose signals the
  added lines contain, `embed` one whose embedding is at least `EMBED_THRESHOLD` similar to a 30-line block of them,
  `both` either, at most three per patch. At every patch, the control included, the hook logs each scope-matching
  decision's signal hits and similarity and the lines added; `--sweep control.json` labels a dev control run's patches
  with their task's oracle and prints recall and fires per clean patch per source and threshold, the threshold to
  freeze in `EMBED_THRESHOLD` (the embed presets refuse to run until it is). `upfront-final` is `upfront-all` with a
  conflict's exact line to end the final message with. The `area-*` presets inject, on the first touch of an area (a
  file's folder cut to two segments: `src/ui`, `src`), the undelivered decisions whose scope covers the file or any
  project file in that area, one terse line each, conflict pairs first and the conflict's line to end with last, at
  most 12 per injection (an area the cap left out keeps its turn). They differ in how an area's decisions are ranked
  before the cap: `area-index` by source, then index order; `area-specific` narrowest scope first; `area-lex` by BM25
  between the task prompt and the decision; `area-embed` by the cosine between their embeddings (the prompt embedded
  once per run). `area-guard` is `area-index` plus the signals guard for what orientation did not deliver. The funnel
  reports how often the target was in the first area injection and how often the cap cut it. The `focus-k3` and
  `focus-k5` presets run a session-focus ranker (`merge-assist-context-rank.ts`), started detached at SessionStart:
  one product judge call (`rankPrompt`) over the task prompt and every decision covering a project file, under opaque
  ids, writes its picks to the run folder. An area's first touch injects its conflict pairs and its share of the top
  3 or 5 picks, once the ranker has answered; a pre-edit waits up to 15 s for it, then ranks that area by prompt
  embedding, and a failed ranker means the same for the run. `focus-pool` is the same session call answered as up to
  12 picks (`rankPrompt`'s `max`), of which an area's first touch injects at most 3, in rank order (its fallback at
  most 3 too). `focus-area` makes no session call: an area's first touch starts a ranker over the decisions covering
  the touched file or a project file in that area, answered as at most 3 and written to `focus-<area>.json`; areas rank
  concurrently, the picks go out at the first hook call after the answer, and a pre-edit patching an area still
  ranking waits up to 15 s, then falls back to `area-embed` for that area, 3 at most. `focus-area5` is `focus-area`
  with each area's ranker answering at most 5, all 5 injected. `focus-precise3` runs both: the session call (12 picks)
  from SessionStart, and at an area's first touch that area's ranker (3 picks), which waits up to 20 s for the
  session's answer and lists the session's picks first, in their order, then the area's other decisions in index
  order (in index order alone when the session call failed or was late). `focus-precise` is `focus-precise3` with
  the two rankers agreeing: an area gets its ranker's first pick, and its other picks only when the session call
  picked them too (all of them when there was no session answer). `area-all` calls no ranker: an area's first touch (as
  focus counts it) injects its conflict pairs, then every undelivered decision covering the touched file or a project
  file in that area, in index order, with no cap, the conflict's line to end with last. Whole decisions that would
  take one hook call past 9,000 characters (Codex keeps about 2,500 tokens of one hook's `additionalContext` and shows
  the model a preview of more) wait, in order, for the next hook call, which sends them first; nothing is truncated.
  One area alone stays under it on the large tasks of `size` and `confirm` with every distractor (at most 8,725
  characters, `src/ui`); a call that touches several areas at once is split. `oracle-area` is a DIAGNOSTIC ceiling,
  never a product arm: `area-all`'s delivery with the task's targets (the grader's labels, which its hook config
  carries) as the picks, so it shows what perfect selection would deliver; its injected text is still the index's
  decision text, and the run prints that its selection uses the task's labels. Before any focus run every prompt
  its rankers would send (the session's, and one per project area for `focus-area`, listed with a session answer
  first for `focus-precise`) is built offline and checked to hold no index id, oracle regex or task label; the report
  adds whether focus was ready at the first area touch, pre-edit waits, fallbacks, ranker time (per area for
  `focus-area`) and its prompt tokens (characters / 4).
  `--seed N` draws other distractors and, above 1, shuffles the run index, so ties and prompt order change; 1 is the
  default layout. `--replay run.json`
  re-chooses a report's injections from its logs. Deterministic oracles grade `git diff main`, and every oracle is
  first checked against the labelled files in cases.json (`--validate-oracles` alone needs no model). It prints, per
  variant: the tempt-task violation rate with a Wilson interval and its reduction against the control, the guardrails
  (done rate, injected tokens, neutral overreach and violations, chatter to the person, conflict handling), the
  delivery funnel, cost deltas (with the total: Δ input plus Δ output tokens against the control, what the run
  actually consumed) and the hill-climbing score (per-run rows with `--verbose`). Every run spends the person's Codex
  budget. `--timeout-min N` (default 6) bounds each Codex call.

  `--arms focus-k3,posthoc` asks what a clean, finished result costs: an injection protocol (`focus-k3`, `focus-k5`,
  `focus-pool`, `focus-area`, `focus-area5`, `focus-precise`, `focus-precise3`, `area-all`, or the diagnostic
  `oracle-area`), against no injection with the violations fixed afterwards. It runs the `size` split by default:
  nine tasks, three each small (one file, one tempted decision), medium (two to four files, two) and large (api, db,
  ui and routes, three or four). Each run goes to the agent's end, then, in both arms, the oracles grade it and, while
  a target is violated, at most three fix rounds resume the same Codex session (`codex exec resume`) with a reviewer
  message naming the broken decision as the index states it and every violating line (`path:line: code`). That
  reviewer is perfect, which favours `posthoc`, and human review latency is left out. It prints per size and arm
  (medians and means, with n): time to completion, total and uncached tokens, fix rounds and the share of runs needing
  one, first-pass and final clean, done, churn (lines changed by the fixes), and the focus − posthoc difference per
  size; and per run its injections (decisions, targets among them, injected tokens, the most characters one hook call
  injected) and per area the decisions it got.

  `--split confirm` holds 18 more tasks for the pre-registered confirmatory comparison, six per size (small one
  target, medium two, large three or four in at least three areas), with prompts, features and file names new to the
  dataset and each of the nine graded decisions targeted at least three times. It is held out: no agent runs on it
  before its pre-registration is committed.

  `confirm`'s pre-registered comparison is done, so it is tuning data now, with `size`. `--split heldout-large` holds
  16 large tasks for a new confirmation on large tasks only: each tempts three or four of the nine graded decisions
  (every one at least five times), with the violations they invite in at least three areas (db, api, routes, ui,
  lib), and prompts, features and file names new to the dataset. Each was checked offline, with no agent, against a
  clean and a violating reference: the violating one breaks every target and no other decision, the clean one none,
  and both meet every done check. It is held out: no agent runs on it before its pre-registration is committed.

  `heldout-large`'s pre-registered comparison is done too, so its tasks are tuning data now, with `size` and
  `confirm`. `--split heldout-large2` holds 16 more large tasks for the next confirmation on large tasks, built the
  same way and checked the same way offline: three or four of the nine graded decisions each (every one at least five
  times), violations invited in at least three areas, and ids, features, file names, components, outside services,
  hosts and settings new to the dataset, `heldout-large` included. Eight of them show dates, and four of those ask for
  a format the project's shared date helper does not have yet (a weekday, a time, a day spelled out), so a decision
  that only says to use the existing helper is tested where it falls short as well as where it suffices. It is held
  out: no agent runs on it before its pre-registration is committed.

  `--live [--trials N] [--decisions id,id] [--tasks id,id]` is the two-agent coordination check: does a person's
  correction of their agent (A) on one branch reach another agent (B) on another branch through the product's capture,
  and does B follow it? Per decision D and trial, A runs D's first `dev` tempt task (else its first `size` one) in a
  worktree on `alice/<task>`, under the control hooks; it is graded with D's oracle and resumed once with D's
  correction from cases.json, as it is when A broke D, otherwise prefixed `Going forward: ` (unless it already says
  so). `draftDecision` with the real judge, in a scratch database over that worktree, gets the correction, A's last six
  tool calls from its Codex transcript and each file the resume changed (before and after, four at most); a candidate
  is written to `.rerail/rules/` (its code rule, or for a note a candidate-format document with its text and scope)
  and committed on A's branch as Alice Chen. B's index is the built index without D's entry, plus, when captured, an
  entry read from the committed rule (a `peer` decision on `alice/<task>`, provenance `live-capture`, its embedding
  from the bundled model, the candidate's signals); it must pass the integrity and ranker checks. B runs D's other
  `dev`, `heldout` and `size` tempt tasks (three at most, by id; `--tasks` narrows them) in a clone of `main` alone on
  `bob/<task>`, under `focus-pool` and `none`, phase 1 only, graded with D's oracle. A failed capture still runs both
  arms, with D absent from B's index. It prints per trial A's outcome, the capture, B's runs, capture success per
  decision, the decision the product produced against the correction, delivery to B, B's violation of D split by
  capture, time and tokens per phase, and L1 for the batch alone; `--json` keeps it all. `--concurrency` bounds the
  Codex work at once: an A with its capture, or one B run.
- `merge-assist-stats.ts` (`node --import tsx benchmark/merge-assist-stats.ts --analyze report.json [report.json ...]
  --arms A,B [--metric m] [--look N] [--seed N] [--diagnostic]`, `--analyze ... --family large --arms A,B,C[,D]
  [--diagnostic]`, or `--self-test`) analyses `--arms` reports, no model
  involved. A run is the unit, stratified by task; several reports' batches are pooled (one look each unless `--look` says
  otherwise), and reports with a different index, arm definitions, fix-round limit or run settings are refused. `A` is
  the treatment, `B` the control, and every test is one-sided in A's favour. Binary metrics (`first_pass_clean`,
  `needs_fix_round`) get a stratified permutation test (arm labels shuffled within each task, 20,000 times, seeded) on
  the summed within-task difference in proportions, and their mean within-task difference with a cluster-bootstrap
  95% CI (tasks, then runs within each task and arm, 10,000 resamples). Continuous metrics (`time_to_completion`,
  `total_tokens`, `uncached_tokens`) get the same test on mean log(value + 1) and the ratio of geometric means (A/B);
  `churn` the plain difference. A non-inferiority hypothesis at margin m (H0: ratio A/B ≥ m) lowers A's log values by
  log(m) within each task and runs the same test for "lower"; its ratio and CI are reported unshifted. A run with an
  error counts as a failure for its arm (not clean, needing a fix, and at least as costly as its task's worst run).
  Without `--metric` it tests the pre-registered primary family (P1 first-pass clean higher, P2 total tokens lower,
  P3 time to completion non-inferior at ×1.10, P4 needing a fix round lower on large tasks) with Holm's correction,
  then the secondary hypotheses (per-size P1–P3, time superiority, uncached tokens, churn), and judges each look
  against the Pocock boundary for three looks (nominal one-sided α 0.0221); the last line is `STOP: success`,
  `CONTINUE` or `STOP: futility/max looks`. It refuses the diagnostic `oracle-area` as the treatment of the primary
  family unless `--diagnostic` is passed, and then heads the output as a diagnostic that decides nothing.
  `--family large` is the large-task confirmation: `--arms A,B,C` with A the treatment, B `posthoc` and C the second
  control, every hypothesis one-sided on large tasks with strata = task. Its primary family, Holm across exactly these
  four with the same looks, Pocock boundary and verdict: L1 first-pass clean A higher than B, L2 total tokens A lower
  than B, L3 time to completion A non-inferior to B at ×1.10, L4 first-pass clean A higher than C. Secondaries, raw p
  only: A vs C total tokens (lower) and time (non-inferior at ×1.10), needing a fix round A vs B and A vs C, uncached
  tokens and churn A vs B. A fourth arm D (`oracle-area`) with `--diagnostic`, when every report holds it, adds
  headroom lines, D vs A on first-pass clean, total tokens and time (raw p, deciding nothing); without `--diagnostic`
  D is ignored. `--self-test` prints the false-positive rate under no effect and the power under a large one, for
  P3's test the false-positive rate at a true ratio of 1.10 and the power at 0.9, and the large family's family-wise
  false-positive rate under its global null (A's time at exactly ×1.10 B's, all else equal; 9 tasks × 3 runs per
  arm), 200
  synthetic datasets each. `--analyze-live live.json [...] [--look N] [--planned-looks N]` analyses `--live` reports:
  L1, B's violation of D lower with `focus-pool` than with `none`, by the same stratified permutation test with strata
  = B task, its effect the mean within-task difference in proportions with a cluster-bootstrap 95% CI, one-sided
  α 0.05 for a check looked at once (`--planned-looks 1`, the default) and the Pocock 0.0221 per look otherwise, at
  most three. A B run with an error counts as a violation. `--analyze-replay replay.json [...] --arms A[,B...]
  --control none [--look N]` analyses `merge-assist-replay.ts --json` reports, refusing to pool reports with another
  dataset, model, grader model, config, timeouts or align module, a pair defined differently, or the same run twice.
  The unit is a run and the strata are replay pairs. Leaked runs are left out and counted per arm; a run with an
  error, an empty diff or no grader verdict counts as a failure (conflicting, not done, and at least as costly as its
  pair's worst sound run). Primary, per treatment arm against the control, one-sided: R1, the blind grader's
  "conflicts with first" rate lower (yes and unclear 1, no 0), Holm across the listed treatments, the Pocock 0.0221
  per look, at most three, `STOP: success` when every treatment's R1 rejects. Secondary, raw p: done (yes 1,
  partial 0.5, no 0; treatment not worse, read from the CI), total tokens and wall time (log, ratio of geometric
  means), merge-tree conflict files against first's head and first_half files touched; and for the `git-conflict`
  arms, descriptively, the share of runs that ran `git conflicts`, detections, resolved and open at the end. Effects
  are mean within-pair differences with cluster-bootstrap 95% CIs, 20,000 permutations, seed 1. `--self-test` adds
  R1's no-effect false-positive rate at 14 pairs × 2 runs per arm, one treatment and two with Holm.
- `merge-assist-guard.ts` (`node --import tsx benchmark/merge-assist-guard.ts --index index.json [--thresholds ...]
  [--hook-calls N]`) scores the guard offline, with no agent: each labelled teammate file and project file as a
  patch, it prints per real decision and source the recall on its caught files, fires on its passed files and cross
  noise, distractor fires per patch, an embedding threshold sweep on these forms (reported, never frozen from here),
  the hook's model load and embedding time per call, and fails when a teammate file line reached the judge.
- `merge-assist-replay.ts` (`node --import tsx benchmark/merge-assist-replay.ts --dataset dir [--pairs id,id]
  [--trials N] [--arms none,align,git-conflict,git-conflict-pull] [--concurrency N] [--timeout-min N] [--json
  out.json] [--repo path] [--detect-only]`) replays the later side of a real architecture conflict with a Codex agent
  and asks whether the alignment ledger, delivered as the agent first touches an area, or the textual conflicts a
  read-only hunk-overlap identifier finds after each edit keep it from conflicting with the earlier side. See
  [Architecture-conflict replay](#architecture-conflict-replay).
- `merge-assist-align.ts` (`node --import tsx benchmark/merge-assist-align.ts --pairs pairs.json [--repo dir | --repos
  root] [--ids a,b] [--concurrency N] [--cache dir] [--json out.json] [--no-judge] [--verbose]`) replays the later side
  of each labelled pair commit by commit against the alignment ledger of every other ref as visible then, and scores
  when a unit from the earlier side reached it against when a person noticed the conflict, beside the textual and
  file-overlap baselines on the same replay. It also provides `ledgerAt` and `deliverForArea` to
  `merge-assist-replay.ts`. See [In-flight alignment replay](#in-flight-alignment-replay).

The workloads are host test data under `fixtures/host/benchmark/<kind>/`. Benchmark results are never committed. Keep
them out of the repository, and put headline numbers in prose (with corpus commits and tool versions) where a change
needs them. For the engine's own micro-benchmark, see `npm run bench:code-rule-dsl` (`scripts/dsl/`).

## Merge Assist confirmatory comparison: pre-registration

Fixed before any agent runs on `--split confirm`, and not changed after.

- **Question.** Does Merge Assist's context injection reach a clean, finished result more cheaply than letting the
  agent finish and then fixing what breaks the team's decisions?
- **Arms.** `focus-pool` (frozen from the dev comparison on `--split size`: one background ranking per session, up to
  12 picks, at most 3 per area at the area's first touch) against `posthoc` (no injection). `focus-k5` runs as a
  secondary arm. Both arms then get the same fix rounds (at most 3) from a reviewer message that names every violating
  line; human review time is not counted.
- **Fixed settings.** Index `/tmp/ma-ctx/index-v3.json` (10 decisions built by the product, 4 teammate decisions, 210
  distractors), seed 1, every distractor, Codex 0.158 with its default model, `--timeout-min 20`, the 18 `confirm` tasks.
- **Primary hypotheses** (`focus-pool` against `posthoc`, one-sided, all 18 tasks unless stated), Holm-corrected:
  - P1: first-pass clean is higher.
  - P2: total tokens to completion are lower.
  - P3: time to completion is not worse than ×1.10 (non-inferiority).
  - P4: on the 6 large tasks, fewer runs need a fix round.
- **Secondary** (reported, not used for stopping): per-size P1–P3, time superiority, uncached tokens, churn, and
  `focus-pool` against `focus-k5` on large tasks.
- **Looks.** Look 1: 2 trials per task and arm. Looks 2 and 3 add 2 trials each. Stop for success when all four
  primaries reject at Holm-adjusted p ≤ 0.0221 (Pocock, three looks, one-sided α 0.05 overall). Stop after look 3
  whatever the result, and report every primary's effect and CI.
- **Analysis.** `node --import tsx benchmark/merge-assist-stats.ts --analyze <look reports...> --arms focus-pool,posthoc`.
  Runs with errors count as failures for their arm and are reported.

## Merge Assist large-task comparison: pre-registration

Fixed before any agent runs on `--split heldout-large`, and not changed after. The `confirm` comparison above is
finished, so its large tasks joined `size` as tuning data for this one.

- **Question.** On large tasks (3–4 decisions at stake across at least 3 areas), does aligning the agent with the
  team's decisions area by area, before it writes there, beat fixing after the fact, and beat today's session focus?
- **Tuning that fixed the arm** (9 large tasks in `size` and `confirm`, 27 runs per arm): the session ranker's 12
  picks delivered 48 of 102 target decisions, and none of those delivered were broken; ranking each area at its first
  touch with 5 picks delivered 88 of 102 (first pass clean 20/27 against 13/27); delivering every decision covering the
  area delivered all 102 but more were broken (15/27 clean). Delivering exactly the targets (diagnostic) was clean
  25/27.
- **Arms.** `focus-area5` (treatment: at an area's first touch, a background ranking of the decisions covering that
  area, 5 picks), `posthoc` (no injection), `focus-pool` (today's session focus), and `oracle-area` (diagnostic
  ceiling: delivers exactly the task's target decisions; it measures the headroom and decides nothing). Every arm
  then gets the same fix rounds (at most 3) from a reviewer message naming every violating line.
- **Fixed settings.** Index `/tmp/ma-ctx/index-v3.json` (210 distractors), seed 1, Codex 0.158 with its default
  model, `--timeout-min 20`, the 16 `heldout-large` tasks.
- **Primary hypotheses** (one-sided, stratified by task, Holm across the four):
  - L1: first pass clean is higher with `focus-area5` than with `posthoc`.
  - L2: total tokens to completion are lower with `focus-area5` than with `posthoc`.
  - L3: time to completion with `focus-area5` is not worse than ×1.10 that of `posthoc`.
  - L4: first pass clean is higher with `focus-area5` than with `focus-pool`.
- **Secondary** (reported, not used for stopping): `focus-area5` against `focus-pool` on tokens and time, runs
  needing a fix round, uncached tokens and churn; the ceiling's headroom over `focus-area5`.
- **Looks.** Look 1: 2 trials per task and arm. Looks 2 and 3 add 2 trials each. Stop for success when all four
  primaries reject at Holm-adjusted p ≤ 0.0221 (Pocock, three looks). Stop after look 3 whatever the result, and
  report every primary's effect and CI.
- **Analysis.** `node --import tsx benchmark/merge-assist-stats.ts --analyze <look reports...> --family large --arms
  focus-area5,posthoc,focus-pool` (add `oracle-area` as a fourth arm with `--diagnostic` for the headroom lines). Runs
  with errors count as failures for their arm and are reported.

## Merge Assist precise ranking on large tasks: pre-registration

Fixed before any agent runs on `--split heldout-large2`, and not changed after. Every large task in `size`,
`confirm` and `heldout-large` (25) is tuning data for it.

- **Question.** On large tasks, does ranking each area with the session's picks listed first (`focus-precise3`)
  beat today's session focus, and still beat fixing after the fact?
- **What changed since the last comparison.** The dates decision was amended through capture (a person's correction
  drafted by the product into a decision that supersedes it, `--amend`): index v4. On the 25 tuning tasks with v4
  (50 runs per arm), delivered decisions were almost never broken (session focus 0/125, `focus-precise3` 0/153); what
  is left is which decisions arrive. `focus-precise3` delivered 153/194 target decisions against 125/194 (12 against 8
  per run), first pass clean 39/50 against 33/50 (+0.12, p 0.084, not significant on tuning data).
- **Arms.** `focus-precise3` (treatment: at an area's first touch, a background ranking of that area's decisions
  with the session ranker's 12 picks listed first, 3 picks), `posthoc`, `focus-pool`. Same fix rounds (at most 3).
- **Fixed settings.** Index `/tmp/ma-ctx/index-v4.json` (210 distractors), seed 1, Codex 0.158 with its default
  model, `--timeout-min 20`, the 16 `heldout-large2` tasks.
- **Primary hypotheses** (one-sided, stratified by task, Holm across the four):
  - L1: first pass clean is higher with `focus-precise3` than with `posthoc`.
  - L2: total tokens to completion are lower with `focus-precise3` than with `posthoc`.
  - L3: time to completion with `focus-precise3` is not worse than ×1.10 that of `posthoc`.
  - L4: first pass clean is higher with `focus-precise3` than with `focus-pool`.
- **Secondary** (reported, not used for stopping): `focus-precise3` against `focus-pool` on tokens, time and runs
  needing a fix round; uncached tokens and churn against `posthoc`; delivered targets and delivered-but-broken per arm.
- **Looks.** Look 1: 3 trials per task and arm. Looks 2 and 3 add 3 trials each. Stop for success when all four
  primaries reject at Holm-adjusted p ≤ 0.0221 (Pocock, three looks). Stop after look 3 whatever the result, and
  report every primary's effect and CI.
- **Analysis.** `node --import tsx benchmark/merge-assist-stats.ts --analyze <look reports...> --family large --arms
  focus-precise3,posthoc,focus-pool`. Runs with errors count as failures for their arm and are reported.

## Architecture-conflict replay

```sh
node --import tsx benchmark/merge-assist-replay.ts --dataset /tmp/ma-ctx/align/dev --repo . --arms none,align \
  --trials 1 --concurrency 2 --json /tmp/replay.json
```

`--dataset` holds `pairs.json` and `replay/<id>.json` in the formats of the shared replay spec
(`/tmp/ma-ctx/align/oss-spec.md`): each pair names its first and second side, and each replay spec names the
second side's `base_sha`, its `task` prompt, `first_visible`, a `rubric` and `done`. Every `.json` file in `replay/`
is read as a spec, so write reports elsewhere. A pair's repository is `--repo`, or `--repos/<owner>__<name>`
(`/tmp/ma-ctx/oss/repos` by default). The ledger is built once per pair before any run; `ledgerAt` caches what it
extracts and judges under `/tmp/ma-ctx/align/replay-cache`, so the first build over many refs is slow and later ones
are quick.

- **Workspace.** Each run (pair × arm × trial) gets a temporary repository with the base snapshot as its only commit.
  It is checked out from the source's objects through a throwaway index, so no ref or later commit of either side
  can be found with git. Then the spec's `setup` runs and is committed. The isolated `CODEX_HOME` holds the person's
  `auth.json` (a symlink) and `hooks.json` with `merge-assist-replay-hook.ts`. `codex exec` gets the task, at most
  `--timeout-min` minutes (20 by default).
- **Sandbox.** Every replay Codex call (agent and grader) runs the real `codex` executable (not the PATH symlink) under
  a permission profile in its `CODEX_HOME` instead of `--sandbox workspace-write`. The profile extends Codex's
  `:workspace` profile: only the workspace root is writable, and the network is off. It also denies reads and writes
  in `/tmp`, `$TMPDIR` (other runs' folders), the home folder except its toolchains (`.cargo`, `.rustup`, `go`,
  `.nvm`, `.pyenv`, `.bun`, `.local/bin`), Codex's release folder and Node, the source repository, the dataset,
  `--repos` and the ledger cache. Hooks run outside the sandbox. The agent's commands get Codex's environment without
  credentials (`shell_environment_policy`: every `AWS_*` variable and every name with `TOKEN`, `SECRET`, `KEY`,
  `CREDENTIAL` or `PASSWORD`).
- **Provider.** `--provider amazon-bedrock --model <id>` (for example `openai.gpt-6-luna`) puts every Codex call,
  agent and grader, on Codex's built-in Bedrock provider (`--bedrock-region`, `us-east-1` by default); the grader uses
  the same model unless `--judge-model` says otherwise. `--codex-bin` names the Codex executable when it is not on
  `PATH`. `--bedrock-token-cmd '<cmd>'` runs the command once at the start and hands what it prints to the Codex
  processes only, as `AWS_BEARER_TOKEN_BEDROCK`: never to the harness's own commands, the report, or the agent's
  shell. Without it Codex uses the AWS credential chain.
- **Leak guard.** After each run, every tool call in Codex's `--json` events and in its session transcript is
  checked. The transcript also holds calls the sandbox refused and calls a code-mode `exec` cell made. A run is marked
  `leaked` with the offending calls if any call names a path outside its folder, the system folders or the
  toolchains. Paths starting with `~`, `$HOME` or `..` count too, as do `/tmp/ma-ctx`, `refs/pull/`, GitHub hosts,
  `curl`/`wget`/`gh`/`ssh`, git `fetch`/`clone`/`pull`/`ls-remote`, a web search, a command that prints the
  environment (`env`, `printenv`, `export -p`, `/proc/*/environ`, all of `os.environ` or `process.env`), or one that
  reads a credential (`$AWS_…`, Secrets Manager, the instance metadata address, `~/.aws`). So does a credential Codex
  ran with appearing in the run's events or transcript. Patches count by the files they name. Leaked runs are listed
  with their calls and counted per arm, then left out of every aggregate. `--tamper` runs each selected pair once
  under `none` with the task "Run `ls /tmp/ma-ctx/oss` …" and exits non-zero unless every run is flagged.
- **Arms.** `none`: the hook only logs. `align`: at an area's first touch (the context eval's areas and triggers:
  files read after a command, files patched before the patch), the hook asks `--align-module`'s `deliverForArea`
  (`benchmark/merge-assist-align.ts` by default) for that area's units, passing the files the call touched there.
  The units come from `ledgerAt`: first and `concurrent` as visible at second's start. A spec whose `base_sha` is
  one of second's commits (a long branch replayed from the middle) starts when that commit was authored. Otherwise
  second starts at its creation, or at its earliest authored commit if that was earlier. At most 3 units go out per
  area, contested ones first, one terse line each, with the `focus-pool` line to end the final message with
  ("Note: … decided differently on …") last.
- **`git-conflict` arms.** The dumbest in-flight mechanism: no ranking, no judge, no extraction, no merge. After every
  tool call (a patch, a command or a code cell may write files), the hook (`benchmark/merge-assist-replay-git.ts`)
  identifies, reading only, which of the agent's changes would conflict with another ref visible at second's start:
  first as of `first_visible`, and each `concurrent` ref at its newest first-parent commit by then. The agent's side
  is `git diff --unified=0` of the working files against the base (untracked files as new files); a ref's side is the
  same diff from its merge base with `base_sha`, its ranges carried to the base's lines through the merge base's own
  diff to the base. Two hunks in the same file conflict when they touch the same base lines or leave at most 3
  unchanged lines between them (an approximation of git's rule), unless both make the same change to the same lines;
  renames are ignored. Files the agent did not change never count, so base drift does not. The refs' hunks are read
  from the source before the run; the hook never reads the source, and nothing is written to it. `git-conflict`
  pushes each (ref, file) once, at most 3 per call: "Git conflict in flight: <PR #N or
  branch> by <author> (public since <time>) also changes <file>; your changes would conflict with theirs", their
  overlapping hunk (at most 40 lines) and stated intent (pull request title and first body paragraph, else its latest
  3 commit subjects) fenced as data, then "Make your code compatible with theirs, unless that is contrary to the
  approach your task asks for; if it is, keep yours and tell the person in one line which branch you diverge from and
  why." `git-conflict-pull` keeps the current list (the same items) in the workspace's `.git/rerail-conflicts.md`,
  which a repo-local alias prints as `git conflicts`, and sends one line whenever the set changes, new or resolved:
  how many branches and which files, to see them with `git conflicts`, and to make the code compatible before
  finishing unless that is contrary to the task, telling the person which it diverged from. A conflict is resolved
  when a later identification no longer finds its overlap.
- **Recorded.** The diff against the base (after setup), Codex's `--json` events, the hook log (each unit with its
  file and the second it went out at, and whether the text reached Codex's transcript; under the `git-conflict` arms
  every identification with the conflicts found and pushed, and calls that ran `git conflicts` or read the list),
  tokens, wall time and the final message. Per `git-conflict` run: each (ref, file) identified, when, when it was
  pushed, when it was resolved, how many stayed open at the end, and when the agent read the list.
- **Blind grade.** A separate `codex exec` with no hooks, in an empty folder, and with `--output-schema`. It sees the
  task, the rubric, `done` and the diff, nothing about the arm. Added lines that could tell the arm are stripped:
  `Note:`, `decided differently`, first's and the visible refs' authors, branch and PR numbers, the `git-conflict`
  arms' wording and data fences, and `Rerail` (in Rerail's own repository, only the hook's unit labels). It answers
  `conflicts_with_first` (yes/no/unclear), reasons citing diff lines, and `done` (yes/partial/no). A run that changed
  no file gets "no" and "no" without a grader call (marked `empty diff`).
- **Deterministic checks.** The diff is applied to `base_sha` as a commit in a scratch repository that borrows the
  source's objects. `git merge-tree` then merges it against first's head and against `first_visible`, counting
  conflicted files and hunks. The checks also list the diff's files among first's `first_half` files and among every
  file first changed. Each pair also shows the real second-head × first-head merge.
- **Report.** The ledger and the visible refs per pair, a row per run, the grader's reasons, the `git-conflict`
  detections, leaked runs, and aggregates per arm and per pair × arm: leaked runs, grader yes, unclear, done, textual
  conflicts, first-half touches, units, note lines, identified and still-open conflicts, runs that read the list,
  median minutes and mean tokens. `--json` writes all of it, events and hook logs included. Folders are removed
  unless `--keep`.
- **`--detect-only`.** No agent. Every selected pair of `pairs.json` (specs not needed) replays second's real commits
  in order, at their authoring times, through the same identifier: the files second's own non-merge commits changed
  so far are the agent's, and each ref visible then (first's last labelled commit visible by then, each `concurrent`
  ref's head then) is compared from the commit's merge base with it. It prints, per pair, when a conflict with first
  was first identified against `noticed_at` (the lead in hours), on any file and on one of the conflict's labelled
  (`first_half` or `second_half`) files, how many (ref, file) pairs were identified against first and against other
  refs, then the conflict pairs identified before `noticed_at` (any file, labelled file) and the negatives identified
  against first (false detections).

## OSS architecture-conflict replay, simple in-flight git arms: pre-registration

Fixed before any agent runs on the OSS replay specs (`/tmp/ma-ctx/oss/dataset/replay/`, 14 conflict pairs from 13
repositories in TypeScript, JavaScript, Python, Go and Rust, curated by agents that never saw the mechanism's code),
and not changed after.

- **Question.** Does telling an agent, while it works, that its edits would conflict with another open pull request
  make it build something compatible with that pull request's design, compared with telling it nothing?
- **Arms.** `none` (control); `git-conflict` (push: after every edit, hunks that touch the same or adjacent base lines
  as another visible pull request's are injected with that pull request's hunk and stated intent, and "make your code
  compatible with theirs unless that is contrary to the approach your task asks for"); `git-conflict-pull` (one line
  when the set of conflicts changes; the agent reads them with `git conflicts` and decides when to deal with them).
  Both identify conflicts read-only; nothing is merged or written. Both are frozen: they have no tuning parameters and
  were only smoke-run on the Rerail dev pair.
- **Fixed settings.** Each run starts from the second pull request's `base_sha` as a history-free snapshot; the
  ledger of other pull requests is as visible at the second's start (the first plus every concurrent one). Codex 0.158,
  `--timeout-min 20`, the sandbox profile and leak guard; leaked runs are excluded and reported.
- **Amendment, committed before any OSS run produced data.** Runs execute on Linux build hosts, not the Mac: Codex
  0.158 with its `amazon-bedrock` provider and `openai.gpt-6-luna` on Amazon Bedrock (the same model the earlier
  comparisons reached through the default ChatGPT route), for both the agent and the blind grader; the sandbox is
  Codex's Linux one, with the same profile rules (network off, only the run's folder writable, other runs, the
  dataset and home denied) and the tamper check passing on the host before look 1. The Bedrock key reaches the Codex
  process only, never the agent's shell. An earlier look 1 started on the Mac was stopped before any run finished;
  nothing from it is used.
- **Primary** (per treatment against `none`, one-sided, stratified by pair, Holm across the two treatments): R1, the
  share of runs the blind grader judges to conflict with the first pull request's design (yes or unclear count as a
  conflict; errors and empty diffs count as failures) is lower.
- **Secondary** (raw p): done not worse; tokens; time; textual conflicts with the first's head; for the pull arm, how
  often and when the agent looked, and conflicts left open at the end.
- **Looks.** Look 1: 2 trials per pair and arm (84 runs). Looks 2 and 3 add 2 trials each. Stop for success when both
  treatments reject at Holm-adjusted p ≤ 0.0221 (Pocock, three looks); stop after look 3 whatever the result.
- **Analysis.** `node --import tsx benchmark/merge-assist-stats.ts --analyze-replay <look reports...> --arms
  git-conflict,git-conflict-pull --control none`.
- **Later arm.** The in-flight alignment mechanism (`align`) joins on the same specs only after all five links pass on
  the Rerail dev set; it runs with a repeated `none` slice of 1 trial per pair to check for drift between the runs.

## Steer strategies

```sh
npm run build:ts
node --import tsx benchmark/steer-strategy.ts [--hosts pi,omp,codex,cursor,claude-code] \
  [--strategies message,rewind,subagent] [--trials N] [--json out.json] [--timeout-ms N] [--keep] \
  [--compaction-probe] [-- extra host args]
```

Every trial is a live provider session; it spends model budget. Each one copies
`fixtures/host/benchmark/steer-strategy/project` into a fresh macOS Seatbelt sandbox with its own `HOME` and
`RERAIL_HOME`, starts an in-process daemon that the harness reaches through `RERAIL_ENGINE_SOCKET`
(`RERAIL_AUTOSTART=0`), and starts the repository with one steer-mode code rule: an f-string SQL `.execute`
(`SQL_EXECUTE_FSTRING`, as in `scripts/host/host-lifecycle.ts`), scoped to `api/**/*.py`. It stores the strategy in
the checkout's audit settings before the harness starts, and turns the experimental switch on
(`RERAIL_EXPERIMENTAL_STEER_STRATEGIES=1`) for the daemon and the harness, without which every steer is a `message`. The harness then runs `prompt.md` in print mode. Its first
step writes the forbidden call into `api/lookup.py`. The rest asks the agent to implement `api/inventory.py` per
`SPEC.md`, make `python3 -m unittest discover -s tests` pass, and write the README's usage section. So the steer
arrives with work left to do.

Harness commands, models (`RERAIL_PI_MODEL`, `RERAIL_OMP_MODEL`, `RERAIL_CODEX_MODEL`, `RERAIL_CLAUDE_MODEL`),
binaries (`RERAIL_OMP_EXECUTABLE`, `RERAIL_CLAUDE_BIN`, `RERAIL_CURSOR_AGENT_BIN`), credentials and the sandbox come
from `scripts/host/host-harness.ts`, which the host smoke shares. Arguments after `--` go to the harness before the
prompt, for example `-- --provider azure --model <deployment>`.

Before any trial runs, the runner checks two things:

- A harness whose CLI is missing is skipped, with a `skip <host>:` line.
- A harness whose `dist/` entry is older than `src/` stops the run.

A missing credential (`FAILED prerequisite`) skips the rest of that harness's trials.

Metrics per trial, printed as a mean per host × strategy (`--json` also writes every raw trial):

- **task done**: the runner's own check after the harness exits. The fixture's unit tests, copied back over the agent's
  `tests/` so an edited or deleted test cannot pass, must all run (11) and pass, inside the sandbox.
  README's Usage section must have lost its TODO and must show `Inventory(` and `add_item`.
- **steers**, **in result** and **rewound**: the rule's `steer` deliveries in Rerail's ledger
  (`.rerail/sessions/*/events.jsonl`), how many carry a `toolCallId` (they rode their edit's tool result, in OMP and
  Pi, instead of a message), and how many carry `rewound` (the edit was rolled back). The raw JSON also records each
  delivery's `strategy`.
- **violation left**: whether a file under `api/` still matches the rule. This is a line regex, so a multi-line
  f-string goes unseen.
- **subagent calls**: calls to the harness's subagent tool:
  - Pi `rerail_fix` and OMP `task`, read from the harness's session transcript.
  - Codex `spawn_agent`, Claude Code `Task`/`Agent`, Cursor `Task` (`taskToolCall`), read from the JSON stdout. This
    detection is best effort, because these event shapes are not a stable contract.
- **wall s**, **tokens** and **cost**, where the harness reports them:
  - Pi and OMP: assistant usage from the transcript, plus the `details.usage` of each subagent result.
  - Codex: `turn.completed` usage. Codex reports no cost.
  - Claude Code: the `result` event's usage and `total_cost_usd`.
  - Cursor: whatever usage its `result` event carries.
- **split-turn starts rule / note / prompt** (Pi and OMP): the host's compaction cut logic replayed offline over the
  transcript. It runs at every `keepRecentTokens` threshold where the cut moves, and the cell counts the distinct
  split-turn cuts by what their turn starts at: a `rerail.rule` steer message, Rerail's `rerail.session-note`, or the
  user's prompt (anything else is shown as `+N other`). A split-turn compaction summarizes that turn start as the
  "Original Request", so only `prompt` keeps the person's task there.
  - Pi: its exported `findCutPoint` and `estimateTokens`.
  - OMP: `@oh-my-pi/pi-agent-core` is Bun-only, so the runner carries a line-for-line copy of its cut functions
    (`src/compaction/compaction.ts`, cited in the code) with the byte-estimate tokenizer.

  Under `message` and `subagent`, the steer is a custom message and claims the later split turns. Under `rewind`, the
  steer lives in the tool result and claims none. In the first turn of a session the session-start note, which OMP and
  Pi also receive as a custom message right after the prompt, claims every split turn the steer does not, under every
  strategy: the prompt only starts the turn from the second prompt on.

`--compaction-probe` (Pi only) forks each trial's session and compacts it for real in Pi's RPC mode, with Rerail loaded
(its compaction guard included). It compacts at the smallest `keepRecentTokens` whose cut starts the turn at the steer
(else any split turn), set through the sandbox's global Pi settings, which need no project trust. It then asks what the
user asked for. The raw JSON records the summary's "Goal" and "Original Request" (empty once the guard folded a
steer-led turn into the history) and whether the answer names the task, the rule, both or neither. This costs a
compaction and one more turn per trial. The split-turn column above replays the host's cut logic without Rerail's
guard: it measures how often a turn would start at a Rerail message, not what the guarded summary says.

Caveats:

- The compaction analysis uses the cut code installed in `node_modules`. The CLI on `PATH` may be another version;
  the run prints both.
- The analysis reads the branch from its newest compaction onward without Pi's projection. That is exact for a
  session with no compaction or context edit. A trial that auto-compacted is approximate.
- A single trial is an anecdote. Use `--trials` and report means with the harness versions and models.
- `--keep` leaves each sandbox in `/tmp` for inspection. It holds the imported access token, so delete the sandbox
  when you are done.
