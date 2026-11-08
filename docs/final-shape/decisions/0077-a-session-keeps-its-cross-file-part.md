# 0077: A session keeps its cross-file part; an edit runs the strata it changed, then maintains their rows

Status: accepted (2026-11-08). Stages 1 and 2 built.

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

Decided with the user: the native set engine maintains rows itself, porting the SQL executor's algorithm
(delete-and-rederive, `sql_executor/maintenance.rs`), rather than running changed strata through the SQL executor.
Built as a program rewrite (`corpus::maintain`): the running strata's rules become maintenance rules, and the native
engine runs them as it runs any program. One evaluation path, one engine call; the engine is unchanged.

For a changed relation `r`, an input whose digest differs or a relation of a stratum that runs, the program has
its new rows under its own name, its old rows as an input `r__old` (from what was kept), and its deltas:

```text
r__minus(x) :- r__old(x), !r(x).        // rows gone
r__plus(x)  :- r(x), !r__old(x).        // rows new
```

A stratum `S` is maintained when its rules are joins, negations and comparisons, and every changed relation it
reads had its old rows kept. For each rule `h :- B`, and each position `k` of `B` that can change (an atom of `S`,
or of a changed relation, positive or negated):

```text
overdelete   h__del(..) :- B over old rows, k as its loss      // q__del, r__minus; a negated !n as n__plus
survivors    h(..)      :- h__old(..), !h__del(..).
rederive     h(..)      :- h__del(..), B over new rows.
insert       h__ins(..) :- B over new rows, k as its gain      // q__ins, r__plus; a negated !n as n__minus
             h(..)      :- h__ins(..).
```

Overdeleted rows cover every row that lost all its derivations, so a survivor holds. A new row has a derivation
that uses a gained row at some position, by induction on the derivation, so the insert rules reach it. Within a
recursive stratum, `h__del` and `h__ins` are recursive too; the engine evaluates them semi-naively, as any stratum.

An aggregate over a changed relation `a` is a position by its groups. Its correlated variables `g` (the atom's
variables but the aggregated ones) over `a`'s rows gone and new are the groups whose value may have changed:

```text
grp(g)       :- a__minus(..).      grp(g) :- a__plus(..).
overdelete   h__del(..) :- grp(g), B over old rows       // the aggregate over a__old
insert       h__ins(..) :- grp(g), B over new rows
```

A head row that read an unchanged group read the same value before and after, so the other positions' rules
cover it, with the aggregate over old rows in their overdeletes. A group that loses its last row (`count … = 0`)
is in `a__minus`. This is the SQL executor's partition-scoped rederivation at group grain, not its counting: a
changed group's value is computed again over its rows, which costs the group's rows, not the delta's. Aggregation
is never recursive (`front::Error::AggregationInRecursion`), so the aggregated relation is an earlier stratum's or an input.

A stratum with a native operator, a lattice, an `ast` or `nearest` literal, or a changed read whose old rows were
not kept is derived again from its inputs, its rules unchanged. Its rows then give the deltas to the
strata after it, as a maintained stratum's do. An empty delta derives nothing after it.

Kept for this: each input's rows, as columns, after the derived relations in the same budget (§1). An unchanged
input's rows are carried over, not copied again.

Measured:

- Test `a_session_maintains_a_recursive_cross_file_stratum`: import reachability, recursive, over four files.
  Four edits break a cycle, close another, break it, and close it again. Every changed stratum is maintained,
  the recursive one included, and the findings equal a full run's each time.
- Test `a_session_runs_the_cross_file_strata_an_edit_changed` (pack rules B301 and B307): every stratum that ran was
  maintained.
- Test `a_session_maintains_a_cross_file_aggregate`: a module's importers counted (`n = count`) and modules that
  import nothing (`count … = 0`), over four edits that raise and lower the counts to none. All 11 strata are
  maintained each time, none derived again, and the findings equal a full run's.

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

- Inputs are digested whole each run, and a maintained stratum's deltas, survivors and copies scan whole relations
  (O(rows)). Per-file deltas (ledger #14) would make both O(changed). Today the saving is in joins: a changed stratum
  joins only its deltas, where it would otherwise derive everything again.
- Aggregates are maintained by changed groups, not counted: a group with many rows costs them all on any change.
  Incremental reducers (the SQL executor's counts, sums and min/max multiplicities) would make it O(delta). The
  spec's threshold (§21.8: a delta past a share of the relation derives again) is not applied.
- The run still makes the part's views, tables and plans each time, and replays memoised exports into `x`; a session
  could keep the connection and `x` too.
- Programs with gates, and typing programs with bounded rounds, run whole.
