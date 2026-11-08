# 0077: A session keeps its cross-file part; an edit runs the strata it changed, then maintains their rows

Status: accepted (2026-11-08). Stage 1 built; stage 2 (native DRed) decided, being built.

(Numbered past 0076, which Bend-language-investigation uses for its per-component cross memo, not yet in this
tip.)

## Intro

A corpus run has two parts. The file-local part runs per chunk of files, and a session (decision 0063) already
keeps each file's rows: an unchanged file is neither parsed nor evaluated. The cross-file part runs once over every
file's exported rows. Until now it ran in full on every run: every stratum derived again from empty, its views,
tables and plans made again.

That part is the cost most branches now hit:

- Typing-support's one fixpoint (30ee40e7b): 24.9 s after a one-file edit, against 8.0 s warm, because any edit
  derives the whole fixpoint again.
- Final-shape-quality's full data-layer tree: the cross part's held bases and peak memory dominate (829 MB).
- The explorer: the cross part is 20–30 ms of a 60–110 ms keystroke.

The target architecture (03-architecture.md, point 3, "push path") says an event replaces a batch of facts and only
changed facts are updated and propagate; a recomputed fact identical to the previous one stops there. The engine
spec says how, for repository facts (`docs/rule-engine-direction/spec.md` §21.8): "Repository facts in the cone are
updated by semi-naive delta over the changed join keys; recursive components use delete-and-rederive, or a full
recompute when the delta exceeds the planner threshold. Only rules whose recorded read set intersects a changed
(fact, key) pair re-run." This record builds that for the cross-file part, in two stages.

```text
edit to a.py
  file-local part: a.py evaluated, the other files' rows from the memo (0063)
  cross-file part:
    stage 1: inputs whose rows changed  ──►  strata that read them, and what reads those, run
             the other strata's relations are bound as inputs, from what the last run kept
    stage 2: a stratum that runs maintains its rows from its inputs' deltas (DRed),
             and an empty delta out stops propagation
```

## Decision

### 1. What a session keeps of the cross-file part

`corpus::retain::Retained`, in the session (`Session::cross`), for one cross-file program (keyed by the prepared
program's key and the part's lowered program):

- a digest of each input the part read: base relations held for it, exports, repository relations. The digest is
  order-independent over the rows (they are sets);
- each derived relation's rows, as columns, and an output's decoded rows and node columns.

Kept rows are bounded by a byte budget, 256 MB (`RERAIL_CROSS_RETAIN_MB`), in stratum order. A relation past the
budget is not kept, and its stratum runs again next time (decided with the user: budgeted retention, not all in
memory, not spilled to disk). A relation of no columns (a flag) is never kept: its rows' count is its value.

### 2. Stage 1: only the strata an edit reaches run

`retain::cut`, over the part's strata (`front::strata`) in order. A stratum runs when

- it reads an input whose digest changed, or a relation of a stratum that runs; or
- one of its relations was not kept; or
- the program has gates (they decide on relations kept to the program's end), or nothing was kept.

The other strata are clean. The run evaluates the part without the clean strata's rules (`Cut::program`): a clean
relation a running rule reads becomes an input, bound from its kept rows with its statistics measured, as exports
are. A clean output's rows are the kept ones. After the run, what ran is read back and kept (`retain::after`).

Measured (test `a_session_runs_the_cross_file_strata_an_edit_changed`, two pack rules over four files): unchanged
files run 0 of 5 strata; an edit to the imported module runs 2 of 5; each run's findings equal a full run's.

Stage 1 is coarse: an input's digest covers every file's rows, so an edit that changes any file's rows of an input
runs every stratum that reads it.

### 3. Stage 2: a stratum's rows are maintained, natively

Decided with the user: the native set engine maintains rows itself, porting the SQL executor's algorithms
(`sql_executor/maintenance.rs`, `reductions/`, `recursive/`), rather than running changed strata through the SQL
executor. One evaluation path, at the native engine's speed.

- **Deltas in.** A changed file's rows of each input, as the last run kept them, against this run's: `minus` and
  `plus` per input relation (ledger #14: each file owns its rows; an edit deletes and rewrites only that file's).
- **Monotone strata** (no negation or aggregate over a relation that changed): delete-and-rederive. Over-delete with
  the `minus` deltas against the old contents, rederive what has another derivation in the new contents, then
  insert semi-naively from `plus`. The engine's relations gain delete; its delta windows are what semi-naive already
  uses within a recursive stratum.
- **Non-monotone strata** (negation, aggregates): counting per group where the executor has a reducer for it
  (count, sum, extrema); otherwise the stratum is derived again from its inputs and diffed against the kept rows.
- **Out.** A stratum's `minus` and `plus` are the next strata's input deltas. An empty pair stops there: a recomputed
  stratum identical to the kept one runs nothing after it.
- **Threshold.** A delta larger than a share of the relation derives the stratum again (spec §21.8).

## What the prior work says

- **Spec §21.8** (rule-engine-direction): semi-naive deltas over changed keys, delete-and-rederive for recursive
  components, full recompute past a threshold, and only rules whose read set intersects a change re-run. *Agrees*:
  stage 1 is the read-set cut at stratum grain, stage 2 the deltas.
- **Lessons ledger #14:** each file owns its rows; an edit deletes and rewrites only that file's; whole-graph
  artifacts must not force whole-repository rebuilds. DLRE paid 121.6 s CPU for a one-file edit. *Agrees*; stage 2's
  deltas are per file.
- **History index §2.20(d):** write paths O(inserted), not O(index); one-shot must equal incremental. *Agrees*: every
  session run's findings are checked against a full run's (`session::full`, `logic --full`).
- **History index §2.16(d) (S238):** key memoised analysis by what it is a function of. The kept part is keyed by its
  program, and each input by a digest of its rows; nothing by run.
- **Design index §10 (S124):** incremental indexing allows no corpus-wide statistic. Clean relations bound as inputs
  are measured from their kept rows each run: work in proportion to what is bound.

Sibling branches (surveyed 2026-11-08):

- **Bend-language-investigation, 0076:** cross rows memoised per import component (Merkle keys), replayed by copy.
  Correct, but no faster (19.8 s replayed against 16.2 s derived, 1,038 MB): lookups are global, and replay copies
  1.55 M rows. *Departs*: this record keeps rows in place and maintains them, and does not assume rows depend on
  imports only. Its component graph is a candidate cone selector later.
- **Typing-support 0060/0065:** one fixpoint, no rounds (30ee40e7b); a coarse stored derivation of the whole part.
  *Agrees* with one fixpoint, which maintenance needs; the coarse key gives an edit nothing.
- **Rerail-rule-expressivity:** the SQL executor's retained state, DRed and counting (2–6 ms per edit on 50k edges).
  *Reused as the algorithm*, ported to the native engine (§3).
- **Final-shape-quality:** exports bound before their first reader and dropped after their last; bases held only for
  exported nodes. In tension with keeping rows; the budget (§1) is the bound.

## Open

- Stage 2 (§3) is being built.
- Inputs are digested whole each run (O(rows)); per-file deltas (§3) replace the digests.
- The run still makes the part's views, tables and plans each time, and replays memoised exports into `x`; a session
  could keep the connection and `x` too.
- Programs with gates, and typing programs with bounded rounds, run whole.
