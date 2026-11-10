# 0090: Cross-file maintenance costs what the delta reaches

Status: accepted (2026-11-08). The direction, and its order of work. Step 1 (the profile) is built and measured;
§1–§5 are not built yet. Aggregates by changed group (0077 §3) and Typing-support's three O125 fixes are in.
Its order of work is revised by 0092 §6: steps 6 and 4 (definition node ids, per-file base deltas) come before
steps 2, 3 and 5, because a delta wider than the edit (Intro, cause 2) multiplies every other cost.

## Intro

An editor-speed run must cost what an edit changes, not what the repository holds. The target architecture says
so twice: "only changed facts are updated and propagate" (03 §2), and a warm run after a small change takes
milliseconds (03 §10, answer 5).

Decision 0077 gave a session a kept cross-file part, maintained by delete-and-rederive (DRed). It is correct: in
the O125 spike, `calls.rr` over 84 files has the same findings maintained as derived. But it is slower than
deriving again:

| Edit (84 files, `calls.rr`) | Cross part, maintained | Cross part, cold |
|---|---:|---:|
| a comment | 11.4 s | 4.5 s |
| a function added | 17.7 s | 4.2 s |
| a `return` changed | 16.4 s | 4.5 s |

The delta is not the problem. An edit over-deletes 3–88 rows of the 1.14 M held. The cost has three sources:

1. **Work linear in the stored fixpoint, outside any stratum** (over 4 s). Every run:
   - digests every input whole;
   - binds and measures every kept relation;
   - makes `r__old` sources by copying the kept rows in;
   - copies every survivor (`h :- h__old, !h__del`);
   - copies every derived relation out again (`held_rows`) to keep it;
   - plans the rewritten program from scratch.
2. **A delta wider than the edit.** A comment adds a tree node, which shifts node ids, so `node` changes and 183
   of 218 strata read it. Nothing that matters changed.
3. **The rewrite's size in large recursive components.** Points-to's insert and overdelete rules are 1,697 and
   1,147, against 464 in the program. The component takes 3.6 s maintained against 1.7 s cold.

Every sibling that met the cross-file part met one of these:

- **Bend-language-investigation (0076):** memo replay by import component copies 2.18 M rows, and "the edit needs
  early cutoff".
- **Typing-support (0065):** a coarse stored derivation of the whole part gives an edit nothing.
- **Two-pass-roots (0083):** names the unit of change (the definition) and the model (one stored fixpoint,
  maintained by deltas).

This record fixes how this branch's maintenance meets that model.

## Decision

### 1. The maintained fixpoint stays in the engine's own relations, read in place

A session keeps the native engine's relations themselves (`setfix::InRel`: columns and hash indexes), not rows
copied out of them. The next run binds each one as its `r__old` input, with the indexes it has, and its new rows
are the old ones with the run's deltas applied in place:

```text
                      today (0077)                          this record
keep a relation       copy rows out (held_rows)             keep the InRel (an Arc), indexes and all
next run's r__old     copy rows in (columns_source), index  bind the kept InRel; probe its indexes
survivors             h :- h__old, !h__del   (O(|h|))       h := h__old ∖ gone ∪ new, in place (O(delta))
r__minus, r__plus     r__old, !r  /  r, !r__old  (O(|r|))   h__del ∖ h  /  h__ins ∖ h__old   (O(delta))
statistics            measured from every kept row           kept with the relation, moved by the delta's counts
```

This is the overlay of 03 §4: an access path that sees overlay ∪ store, with the delta as the overlay. Writing the
fixpoint back to SQLite stays eventual (03 §2, step 4), off the fast path. The SQL executor's session keeps its
relations the same way: retained plans, result indexes and fact bindings, with signed insert and delete batches.
It is ported here into the native engine, as 0077 ported its DRed.

In-place survivors need a stratum to run in two halves: the overdelete fixpoint over the old rows, then the
deletes applied, then the rederive and insert fixpoint, semi-naive, over the new rows. That is DRed as Gupta,
Mumick and Subrahmanian state it. 0077 wrote it as one program so that no engine changed. Only this step changes
the engine.

### 2. Base deltas come from what changed, never from a whole-relation diff

An input's delta is made from the files that changed, not by comparing its rows:

- **Per file, from the memo.** Each file's file-local rows are memoised under its content (0052). An edited file's
  delta is its old entry against its new one. An input changed exactly when a file that gives it rows changed.
  No input is digested whole.
- **Per definition**, when 0083 §3's node ids, `(definition, index)`, land. A comment then changes no exported
  row, and the 183 strata see an empty delta and do nothing.

An input whose rows name no document (0052's unlocated rows) is diffed whole, as now, and counted in the profile.

### 3. A stratum's delta is its net change; an empty one stops propagation

Readers after a stratum see its net change:

- `h__minus`: overdeleted and not put back;
- `h__plus`: inserted and not there before.

They never see its overdeletion. A stratum whose inputs' net deltas are all empty runs only rules over empty
relations, which cost a lookup each. That is 03 §2's early cutoff at the grain of a relation (03 §10, answer 4: a
recomputed partition equal to the last stops evaluation). It is also 03 §7's waves: each stratum consumes the
deltas before it.

It records no reads. 0064 measured +60% for recording reads inside the eager engine and was withdrawn. The cutoff
here comes from the deltas the rules already compute.

### 4. Maintain or derive again, by measured size

Per stratum, the engine derives again instead of maintaining when either holds:

- the input delta is past a share θ of the stored rows;
- the rewrite's estimated cost, its rules times the delta, passes the cold plan's.

This is spec §21.8's threshold. With it, a maintained run never costs much more than a cold one, which O125
showed it can today. θ is set by measurement: points-to on `calls.rr`, and the data layer's 9,010 files.

### 5. Rewritten programs and their plans are memoised

The rewrite is a function of the program and of which relations changed. It is kept under that key, with its
plans, as the plan cache keeps a program's (0038, 0073). A session edits the same files again and again, so the
same rewrite comes back.

### 6. What is kept as it is

- **DRed with no witness.** 0077 stores no support counts and no levels. O125's over-deletion (3–88 rows) does not
  justify them. 0083 §2 names support counts as the fallback if a large recursive component over-deletes.
  - **For the architecture text:** 03 §2 says "No DRed and no level witness. An invalidated fact is recomputed from
    its inputs." DRed's rederive is exactly that recomputation, of the over-deleted rows. What 03 rules out, a
    stored witness, is not used. 03's wording predates 0077, and nothing reconciles the two yet. That is the
    architecture owner's call, and this record does not make it.
- **Aggregates by changed group** (0077 §3). The counting reducers of the SQL executor (counts, exact sums, min and
  max multiplicities) come in only where a group's size makes a group's recomputation the cost.
- **Stored derivations with name→hash keys** (03 §14, 0065) remain for facts shared beyond one store: a library's
  rows reused by content across workspaces (0083 §2). The session's own fixpoint is maintained, not looked up.

## Order of work

Each step is measured before the next. Each is measured on the O125 spike (`calls.rr`, 84 files: CPU, RSS,
findings equal to a cold run) and on a pack's cross-file rules over the data layer.

1. **Profile the session's linear costs** (built): spans `session/digest inputs`, `rewrite and old sources`,
   `bind and measure kept`, `plan and evaluate`, `copy derived out` and `retain`, and the `session_costs`
   measurement above.
2. **Kept relations stay engine relations (§1, first half).** `Retained` holds the run's `InRel`s. `r__old` binds
   them with their indexes, and `held_rows` and `columns_source` go. The target is the copy-in, copy-out and
   re-indexing time, gone.
3. **In-place survivors and net deltas from del/ins (§1, second half; §3).** The two-half stratum in `setfix`. The
   target is that work outside strata is O(delta).
4. **Per-file base deltas from memo entries (§2).**
5. **The threshold (§4) and memoised rewrites (§5).**
6. **Per-definition node ids** with 0083 step 5a. The target is that a comment edit has an empty exported delta.

The end target is O125's own: a one-function edit under 5% of a cold cross part, in 1 GB.

## Measured: step 1 on this branch

`session::tests::session_costs` (ignored; `RERAIL_SESSION_COST_FILES`): generated modules, each importing the next
and one of a seventh of them. The program has `dep`, a recursive `reach`, `fan` as a count, and two findings. Each
edit changes one file. All 11 strata are maintained every time, and none is derived again. CPU seconds, debug
build, load 20–60:

| 1,500 files (`reach`: about 1.1 M rows) | cold | edit 0 | edit 1 | edit 2 |
|---|---:|---:|---:|---:|
| cross part | 4.8–5.8 | 11.5 | 13.6 | 12.2 |
| `CYC__del`: loading `reach__old` and indexing it | | 2.16 | 2.00 | 2.89 |
| `reach__minus`, `reach__plus`: whole-relation diffs | | 2.02 | 1.94 | 2.50 |
| `reach__del`: loading old rows | | 0.79 | 0.63 | 0.67 |
| binding and measuring kept relations | | 1.19 | 1.13 | 1.13 |

At 3,000 files (`reach` about 4.5 M rows), the cold part's evaluation is 22.1 s, and an edit's is 51–70 s, with
5.5–6.2 s binding and measuring.

That is O125's ratio, 2.3–3.2x a cold part, on a program with no typing at all. Half of an edit's CPU is the
copies and diffs that §1 removes: copying old rows in and indexing them, diffing whole relations, and measuring
kept rows. The joins over the deltas cost almost nothing (`reach__del`'s joins: 0.000 s).

## Measured: step 5's memoised rewrites and plans

Measured 2026-11-08 on the tree with every branch merged. O125's replay, `calls.rr` over the data layer:
`O125_STEPS=5-6 O125_SPANS=1`, which runs cold at step 04 and then steps 05 and 06 in one session and one process.
Debug build, load 10–22. Spans are each step's own, in CPU seconds:

| span | 05 (14 files, plans new) | 06 (2 files, plans kept) |
|---|---:|---:|
| session part | 60.8 | 42.6 |
| `rewrite and old sources` | 0.04 | 0.04 |
| `bind and measure kept` | 2.71 | 2.76 |
| `spec` | 6.16 | 2.17 |
| `plan #` (all rules) | 5.85 | 1.93 |
| `search` | 4.24 | under 1.4 (not in the top 14) |
| `statistics and plan` | 1.67 | 1.65 |
| `logical key and scalar analysis` | about 1.8 | 1.60 |
| engine run (joins) | 24.05 | 13.16 |
| the 174-relation over-delete stratum | 26.07 | 17.07 |
| engine open (inputs, indexes) | 4.46 | 3.42 |

- **The rewrite is not worth memoising.** It costs 0.04 s a step.
- **Plans are already kept across a session's runs.** `KeepPlans::start(session::generation())` keeps the session's
  plans. At step 06 they save about 10 s of 43: `spec`, `plan #` and `search` together go from 16.3 s to about 5 s.
- **What is left each step does not depend on the plans:**
  - statistics, 1.6 s;
  - key and scalar analysis, 1.6 s;
  - binding kept relations, 2.8 s;
  - opening the engine, 3.4 s.
  Under §4 these are linear costs. The next step is to make them O(delta), not to memoise more.
- **Evaluation dominates:** joins and the over-delete stratum are 30 s of 43 at step 06. That is §3's work and
  O137's stratum, not step 5's.

## Measured: a step's work in the delta's size, not the program's (2026-11-09)

This was measured after merging the rollup and its siblings, including kept prices and replans (d74dbdba4), kept
relations measured once (1c0aa91ea) and definition ids in T's shape (a3dc7edfd). It is the same replay, step 06
(2 files) after a cold run at 05, with `RERAIL_NODE_IDS=definition`, on a debug build at load 70–150. Each change
is compared against the same binary with the change switched off.

**Most of DRed's rules have nothing to do on a given step.** Of 11,757 rules run, 6,571 found no rows at their
first step. In 657 of 1,794 strata no rule found any.

**Joins over empty relations are left out** (`RERAIL_KEEP_EMPTY_JOINS` switches this off):
- A rule with a positive atom over a lower stratum's relation that has no rows is not planned, keyed, specified or
  opened.
- A stratum left with no rules holds its relations empty and opens no component.
- An aggregate, a negation, the stratum's own relations, and a relation a request may fill are never treated as
  empty.

| span | joins over empty kept | left out |
|---|---:|---:|
| session | 46.2 s | 31.2 s |
| cross part | 37.7 s | 23.9 s |
| `search` | 11.8 s (24,479 calls) | 3.2 s (13,473) |
| `spec` | 8.8 s | 3.0 s |
| `plan #` | 8.2 s (15,873) | 2.7 s (7,816) |
| replans | 5.3 s | 1.3 s |
| `key` | 1.2 s (49,374) | 0.5 s (25,003) |
| cold run at 05, its cross part | 9.6 s | 7.4 s |

The outputs equal a full run's. The gate holds `3988 303 298 0`.

**Engine open is not overhead per stratum.** It opens 943 components instead of 1,434, but still costs about
5.3 s. The spans under `open:` split it:

| part | CPU | calls |
|---|---:|---:|
| index of a base input | 2.7 s | 696 |
| index of a derived input | 1.2 s | 1,703 |
| survivors (`h__old` less `h__del`, in place) | 0.7 s | 277 |
| input loads, other indexes | 0.1 s | |

Nearly all of the base-input index cost is in the input diffs, the `r__minus` and `r__plus` strata
(`r__minus :- r__old, !r`), and not in hash tables. Their probes read a column's sorted order (`Stored::index`). A
run binds its base relations as new in-memory columns, so that order is sorted again: 11 sorts of about 181k rows
(`node`, `rr_nearest_*fscope`) and many smaller ones, about 1.9 s.

Keeping the inputs whose rows did not change, with their indexes, was tried and measured no gain (5.22 s both
ways). The 154 kept inputs were cheap; the cost is in the 105 that changed. It was removed.

The two diff strata cost 6.7 s of the 21 s evaluate, all of it in the repository's size: each anti-joins a whole
relation against its old rows. The digests of every input read the same rows in 0.14 s.

**An input's delta is read from the documents that changed** (`corpus/input_delta.rs`). The session's memo
already holds each document's rows per content (decision 0052). So a run records each document's content, and the
next run takes the old and new entries of the documents whose content changed, came or went:

```text
r__minus = ∪ over changed d: old(d) \ new(d)     r__plus = ∪ over changed d: new(d) \ old(d)
```

These are bound as inputs in place of the diff rules (`maintain::rewrite`'s `given`). The work is the changed
documents' rows.

A relation qualifies only when its rows are each one document's: every row of `d`'s entry names a node of `d` or
of one of its guests. The file-local part sees one document, so no other entry holds that row. A guest's key is a
hash of its host's path, so its rows are in that host's entry alone. Two guards send a relation back to the diff
rules:
- a row that names no such node (a text test's strings, a loaded document's rows, which go to every file that
  asked, or `doc(d, path)`, keyed by the document key alone);
- row counts that do not add up (`old − gone + new ≠ rows`), meaning another source fed the relation.

The empty-join pruning above also covers these bound deltas: they are bound once, before the run. Other inputs
can be bound when their first reader runs, so emptiness is not trusted for them. `RERAIL_DIFF_INPUTS` makes every
delta by the rules again (below, "One path for an input's delta").

On the replay every changed input qualifies at steps 06 and 07 (105 and 135). At step 08, 185 do, and a few
small ones keyed by the document key alone (`doc`, `exists_0`, `py_docmod`) go to the rules. On the same binary, with the outputs equal to a full
run's at every step:

| step | deltas by rules | from documents |
|---|---:|---:|
| 06 (2 files) | 24.0 s | 17.6 s |
| 07 (6 files) | 26.6 s | 20.3 s |
| 08 (6 files) | 53.9 s | 43.1 s |

Session CPU, load 45–70. At step 06 (load 90–130) the diff strata went from 5.3 s to 0.15 s, and engine open
from 4.1 s to 2.7 s. Reading the deltas costs 0.55 s, nearly all of it hashing the two changed files' rows:
the per-file unit 0083 §0 names as the step before per-definition.

**Binding** costs 3.4 s: 2.2 s for 103 sources that are not kept, and 1.1 s for 282 kept ones.
- The 103 are inputs' old rows (`X__old`), which were measured from their rows every step. They are now given the
  statistics their run planned `X` with (`retain::InputStats`, `Stats::of_relation`): their rows are those rows.
- A kept relation now also keeps its constant counts (`Source::value_counts`), as it keeps its statistics.

Binding the kept relations then costs 1.25 s.

**One path for an input's delta** (merged with serverplan-quality's `c70c82be9`). `maintain::rewrite` asks for an
input's delta only when a maintained stratum reads it, and the answer is taken in this order:

1. an earlier phase's delta (`given`, `phases::Versioned`);
2. the changed documents' memo entries (`input_delta`), where every row is one document's: no unchanged
   document's row is read;
3. the documents hashed (`retain::document_delta`): each document's rows of the new source are hashed, and only
   the documents whose hashes differ are compared. This is exact for any input, but reads every row of the input;
4. else, the diff rules.

Step 2 is the one in the edit's size; step 3 covers the relations it leaves (`doc`, `exists_0`, a text test's
strings).

**Outside the cross-file part, after 0096's keeper and its exports (2026-11-09, load 90–120).** Merging
Typing-support-final-shape brought the keeper of the base relations' statistics by file; it now keeps the exports'
too (keyed by entry key, an export's rows by their first node column, as the chunks' totals read them). At step 07:
measuring the held base rows 1.11 → 0.13 s, the memo replay 1.55 → 0.6 s (0.99 s of it was measuring every
unchanged file's exports again). All 278 exports' statistics come from the keeper; against measuring them, every
row count is equal, and 40 of 702 columns differ, by at most 1.5× in distinct values (sketched columns of a few
hundred values). Every step equals a full run.

**What is left, at step 07** (6 files, load about 10: session 14.3 s, cross-file part 9.2 s):
- the joins, 2.6 s. Most of this is DRed's over-estimate: `pv` over-deletes 3,660 rows and 70 are truly gone.
  Counting against DRed is the open maintenance decision.
- engine open, 1.9 s: indexes of the changed base relations, rebuilt over every row (0.64 s); derived inputs'
  indexes (0.52 s), the largest over `kind` and `docof`, which are maintained and change every step; survivors
  copied from the old relation (0.2 s). Making these the delta's size needs relations whose old and new versions
  share storage (a dead mark and an appended tail per version), not a copy.
- planning, spec and replans, about 2.6 s for 8,938 delta rules: the delta's size in rules, at a high cost per
  rule.
- outside the cross-file part, about 4 s that grows with the repository: the memo replay of every unchanged
  file (1.3 s), the chunks' totals and the measuring of the held base rows (1.2 s and 1.0 s), and exporting to the
  cross-file part (0.4 s). 0096's order of work is the plan for these: statistics patched from the same delta
  rows (its step 2), then the per-file cross store, so an unchanged file is neither replayed nor counted (its
  step 3).
- the key analysis is 0.28 s; its JSON `explain` (0.23 s) runs only while profiling.

## Measured: DRed against counting, the data for the open choice (2026-11-09)

O125 replay steps 06–08 (2, 6 and 6 files; every step equal to a full run), CPU time, load 25–45. Rows from
`RERAIL_CROSS_DELTA_COUNTS=each`; derivations (every row a rule emitted, new or already held) from
`RERAIL_SPIKE_DERIVATIONS` with `O125_DERIVATIONS=<dir>`; time from the strata's spans. A relation counts as
recursive when it never ran as a stratum of its own; 79 of the 280 maintained relations are, nearly all in one
component of 58 (`pv`, `bv`, `alook`, `cfn`, `found`, …).

**Rows deleted by DRed against rows truly gone**

| step | non-recursive: over-deleted / gone | recursive: over-deleted / gone |
| --- | --- | --- |
| 06 | 1,863 / 1,465 (1.3×) | 3,951 / 179 (22×) |
| 07 | 6,148 / 3,914 (1.6×) | 8,785 / 658 (13×) |
| 08 | 19,305 / 13,783 (1.4×) | 41,094 / 6,108 (6.7×) |

The worst relation is `pv`: at step 08 it over-deletes 14,711 rows, and 448 are gone.

**The cross-file part's strata, by kind (seconds)**

| step | cross | non-recursive del / redo / ins | recursive del | recursive ins + redo | derived again, other |
| --- | --- | --- | --- | --- | --- |
| 06 | 9.3 | 0.40 / 0.32 / 0.93 | 0.57 | 3.76 | 0.54 |
| 07 | 11.8 | 0.48 / 0.47 / 1.33 | 0.70 | 4.68 | 0.78 |
| 08 | 14.7 | 0.53 / 0.46 / 1.38 | 1.54 | 6.63 | 0.69 |

**Derivations emitted in the recursive component** (emitted / new rows)

| step | del | redo | ins | the relations themselves |
| --- | --- | --- | --- | --- |
| 06 | 13,024 / 3,951 | 12,795 / 3,772 | 14,447 / 4,335 | 8,107 / 4,335 |
| 07 | 24,100 / 8,785 | 23,160 / 8,127 | 60,135 / 17,624 | 25,751 / 18,047 |
| 08 | 162,646 / 41,094 | 126,276 / 34,986 | 153,884 / 39,096 | 74,082 / 42,894 |

A full run derives the recursive relations with 571,010 derivations, for 291,061 rows (1.96 a row); the other
cross-file relations take 1.18 a row.

What the data says:

- **Non-recursive relations.** DRed already deletes close to what is gone (1.3–1.6×). Counting would save their
  redo (about 0.3–0.5 s a step) and some of their del, and needs a count per row: about 1.1 M rows held, so 4–9 MB.
  Small either way.
- **Recursive relations.** This is where DRed costs: 13–22× over-deletion at small steps. Rederiving and
  over-deleting is about 1.7 s of step 07's 11.8 s and about 3.9 s of step 08's 14.7 s (del, plus the redo share of
  the ins + redo stratum by derivations). Plain counting is not exact here: rows in a cycle hold each other's counts
  above zero. The choices are DRed, a counting scheme that holds across cycles (derivation depth, or
  backward–forward), or deriving the component again.
- **Deriving the component again.** At step 08 DRed emits 517 k derivations in the recursive component; deriving it
  from scratch takes 571 k. At step 07 it is 133 k against 571 k. So by step 08's size, §4's threshold would derive
  this component again.

The caveats:
- Emitted rows approximate derivations: a semi-naive wave with two delta literals can find one derivation twice.
- Recursive versus not is taken from the strata's spans, not the program.
- This is one replay over one 84-file repository.

## Spiked: exact counting for recursive relations (2026-11-09)

DRed stays for the non-recursive relations. For the recursive ones, the spike compares DRed with two schemes that
are exact across cycles, and with deriving the component again. It is
`logic/plan/corpus/recursive_spike.rs`, behind `RERAIL_SPIKE_RECURSIVE`.

**How the spike runs.** It takes each recursive component from the replay: the kept rows, the inputs' old and new
rows, and the rules. It then maintains the component with each algorithm in its own small evaluator, which is the
same for all four, so the costs compare with each other but not with the engine's.

- **Reference.** The spike's own derive over the old inputs is the start, and its derive over the new inputs is
  what each result must equal. The engine's kept rows can't serve: `rr_in_annotation_0_gfn` lacks 160 rows its
  rules derive.
- **Verify.** `RERAIL_SPIKE_RECURSIVE_VERIFY` recounts every row's supports after the run.

The algorithms:

- **DRed.** Over-delete what a gone row derived, rederive one step from the survivors, then insert.
- **Levels.** Each row keeps the round it was first derived in, and a count of its derivations whose recursive
  premises are all at lower levels.
  - Those derivations are well-founded, so a count of zero means the row has lost its support. That holds inside
    cycles too, where plain counting is not exact.
  - Each lost or new derivation is found exactly once, from its first premise processed.
  - Kept state is 8 bytes a row: the level (u32) and the count (u32).
- **B/F** (backward/forward, after Motik et al. 2015). For each row a deletion touches, search backward for a
  proof that avoids the deleted rows. Delete the rows left unproved, then insert forward. Nothing is kept.

**Results.** All 39 recursive components, CPU seconds summed over them, load 20. Every algorithm equals the derive
from scratch at every step, and levels' counts verify with 0 mismatches.

| step | recompute | DRed | levels | B/F |
| --- | ---: | ---: | ---: | ---: |
| 06 (2 files) | 2.56 s, 765 k emitted | 0.25 s, 28.9 k, 4,334 deleted | 0.05 s, 3.0 k, 627 deleted | 0.08 s, 19.7 k, 584 deleted |
| 07 (6 files) | 2.77 s, 803 k | 0.63 s, 85.0 k, 8,926 | 0.41 s, 42.8 k, 1,612 | 0.39 s, 66.0 k, 1,363 |
| 08 (6 files) | 2.84 s, 816 k | 2.04 s, 277 k, 37,686 | 0.89 s, 76.5 k, 13,280 | 1.13 s, 187 k, 12,645 |

The 58-relation component (`pv`, `bv`, `acls`, …) dominates:

| step | recompute | DRed | levels | B/F |
| --- | ---: | ---: | ---: | ---: |
| 06 | 1.27 s | 0.23 s | 0.01 s | 0.05 s |
| 07 | 1.48 s | 0.55 s | 0.30 s | 0.29 s |
| 08 | 1.51 s | 1.85 s | 0.58 s | 0.90 s |

The witness for levels:
- **Kept state.** 3.7–3.8 MB for all the components (675 kB for the 58-relation one).
- **First build.** 6.4–7.0 s for all components: a derive plus a count of each row's supports, about 2.3× a
  derive. It runs once; each step after that maintains the witness, so it is not paid again.

What the data says:

- **Rows deleted.** DRed deletes 3–7× the rows truly gone (37,686 against levels' 13,280 at step 08); levels and
  B/F delete about what is gone.
- **Derivations emitted.**
  - DRed emits 3–10× what levels emits.
  - At step 08 DRed costs more than deriving the 58-relation component again (1.85 s against 1.51 s).
  - Levels stays at 40% of a derive.
- **Levels.** It is the cheapest at every step, at 8 bytes a row of kept state.
- **B/F.** It keeps nothing, at about 1.3–1.6× the CPU of levels at steps 07 and 08 (at step 06 every algorithm
  is under 0.1 s).
- **Recompute.** It costs 2.6–2.8 s a step whatever the change. It is the right choice only past the size where
  DRed now sits at step 08.

**Pitfalls the spike found**, which any implementation must handle:

- **A seeded negation must still be checked.** When a row of the negated relation seeds a firing, the rule must
  still test the negated literal against the relation's other rows: in `!opaque_dec($f, _)`, another row with the
  same `$f` still blocks it. Skipping that test left 5 stale rows at step 07 under all three algorithms.
- **The seed does not block itself.** At its own literal, the seed row must not count as blocking.
- **Rederive must not count a derivation made new by a negation.** In levels, a derivation whose negation became
  true because an input row is gone is new. Rederive must leave it to the insertion, or it is counted twice.
- **Join order needs measured rows per key.** Counting bound columns misleads:
  - `pv($o, 221, $k)` probed by the constant alone is most of `pv`;
  - `pv($a, $k, $v)` probed by `$k, $v` finds about 536 rows a key, where `cfn($c, _, $f)` probed by `$c` finds
    one.

  Planning by measured rows per key took the 58-relation component's step-08 maintenance from 9.6 s (DRed) and
  2.7 s (levels) to 1.85 s and 0.58 s, and its witness from 33 s to 2.9 s.

The caveats:
- One replay over one 84-file repository.
- The spike's own evaluator, not the engine's plans or the transducer path.
- Emitted rows stand in for derivations.
- The witness is built from scratch here; how it is kept across runs (with the kept rows, under 0096) is not built.

## Pressure-tested: levels over 26 steps, carried from run to run (2026-11-09)

The runs above build each algorithm's state from scratch at every step. These runs carry it from step to step, as
a real session would (`RERAIL_SPIKE_RECURSIVE_CARRY`). A level scheme can drift there: its levels inflate as edits
pile up, and fewer supports count as being from below.

**The replay.** Steps 1–20 are O125's recorded edits. Steps 21–26 are made up to be the worst case for any
counting scheme:

| steps | change |
| --- | --- |
| 21, 22 | delete, then restore, `l1400_types/__init__.py` (47 importers) |
| 23, 24 | delete, then restore, `purity.py` and `lazy_mixin.py` (51 importers each) |
| 25, 26 | delete, then restore, the whole `l2055_lazy_result` package (24 of 78 files) |

**A fourth algorithm.** *Levels, checked*: levels without counts.
- **What it keeps.** Each row keeps only its level, and every present row has a derivation from rows below it.
- **Deletion.** A row that lost a derivation is checked in level order. It stays when a derivation from rows below
  it still holds. Those rows are already decided, so the check is exact and one step deep, where B/F's search is
  open-ended.
- **Insertion.** New rows go above every level held (the maximum plus the wave). So no firing has to name its
  premises, and the first levels are the waves of an ordinary derive: there is no counting pass.
- **Cost.** 4 bytes a row.

All 39 components, CPU seconds summed over them. All five algorithms equal the derive from scratch at every step,
with every count and every below-support verified.

| steps | recompute | DRed | levels | B/F | levels, checked |
| --- | ---: | ---: | ---: | ---: | ---: |
| edits 1–20 | 83.8 | 36.3 | 18.2 | 21.6 | **12.9** |
| toggles 21–26 | 23.4 | 20.0 | 28.2 | 23.8 | 23.6 |
| toggles, recompute when cheaper | — | **14.8** | 17.5 | 15.9 | 15.6 |

What the data says:

- **Edits.** *Levels, checked* is the cheapest at nearly every step of 1–20, at 2.8× less than DRed and 1.4× less
  than counted levels. Its levels inflate as edits pile up (in the 58-relation component, a mean of 219 and a max
  of 421 by step 25, against 5 and 22 fresh), and that costs it nothing measurable. Counted levels barely drift
  (mean 5.44 against 5.13 fresh at step 25).
- **Mass deletes.** At steps 23 and 25 every maintained algorithm costs more than deriving again (5–8 s against
  2.6–3.0 s). So the §4 threshold is needed whichever algorithm is chosen. With it, DRed is about 5% ahead of
  *levels, checked* on the toggles.
- **Overall.** Over the 26 steps, *levels, checked* takes 28.5 s against DRed's 51.1 s (with the threshold), and it
  keeps 1.8–1.9 MB.

**Choice.** For recursive strata: *levels, checked*, with §4's derive-again threshold for large changes. DRed stays
for the non-recursive strata.

Pitfalls found on the way, beyond those above:
- **Carried state goes stale.** A component skipped one run (here, under the spike's size floor) moved without its
  state, and both level schemes then missed 297 rows. Carried state must be dropped whenever a run does not
  maintain it, and checked against the kept rows when it is read.
- **The engine's kept rows drift.** They include more and more rows a derive from scratch does not produce: 2 at
  step 3, 16 by step 7, with the per-step equality check off (`O125_NO_FULL`). This is open; it looks like a defect
  in the engine's DRed path.

## Built: levels, checked, in the engine (2026-11-09)

Recursive strata are now maintained by *levels, checked* by default (`RERAIL_RECURSIVE=dred` keeps DRed). It is the
same engine: the rewrite makes ordinary rules, and the component runs them in waves.

**The rules.** Per recursive relation `h`, the maintenance rewrite (`maintain.rs`) makes five relations:

| relation | rows | made by |
| --- | --- | --- |
| `h__cand` | heads of old derivations that lost a premise | DRed's delete rules, reading `q__gone` for the stratum's own `q` |
| `h__chk` | the candidates of the level being checked | the component's levels driver |
| `h__held` | the checked rows that still have a derivation | `h__held(H) :- h__chk(H), body` with the stratum's own atoms read as `q__live` |
| `h__gone` | the checked rows with none | the driver: `h__chk` less `h__held` |
| `h__live` | the kept rows already decided, below the level checked | `h__old` in place, rows hidden by the dead bitmap |

**The driver** (`setfix.rs`, `Levels`). At a quiet point of the component it does three things:
- it closes the level just checked: the rows of `h__chk` not in `h__held` go to `h__gone`;
- it shows in `h__live` the kept rows below the next candidate level that are not gone;
- it moves that level's candidates into `h__chk`.

A candidate at or below a level already closed was decided there. Rederive and insertion then run as in DRed. A
new row's level is the component's highest kept level plus its wave (`Rel::level`). Levels go with a kept
relation through `based`, compaction and hand-off, and with an output read back from the engine (`corpus.rs`).

**Two planning faults, fixed.** They made the first build slower than DRed outside the recursive strata:
- **`h__live` was read as a delta.** It starts holding all of `h__old`, so the first wave scanned 181 K rows.
  `h__live` is never a delta now.
- **Whole plans were used for small batches.** The levels relations get small batches from row 0, and the engine
  took those for a whole relation: it ran each batch with the plan for a whole relation, which scans `h__live` or
  `h__old` first. Levels relations take no whole plans now. `h__live` is also planned with `h__old`'s statistics.

**Measured** (O125 replay steps 06–08, CPU seconds in the component strata, from the profile):

| step | DRed, all strata | levels, all strata | DRed, recursive | levels, recursive |
| --- | ---: | ---: | ---: | ---: |
| 06 (2 files) | 1.76 | **1.39** | 0.72 | 0.53 |
| 07 (6 files) | 2.67 | **2.13** | 1.52 | 1.18 |
| 08 (6 files) | 6.59 | **2.83** | 3.51 | 1.13 |

- **Results.** Steps 06–08 equal the full derive (`same as full: true`), and the findings gate is unchanged:
  `total (all): 3988 303 298 0`.
- **The mass-delete steps.** The stress replay's steps 20–26, cross-file CPU seconds per step (the whole cross
  part, not only the strata). Both equal the full derive. The full derive's cross part takes 7.7–8.2 s.

  | step | change | DRed | levels |
  | --- | --- | ---: | ---: |
  | 20 | an O125 edit (2 files) | 12.2 | **9.1** |
  | 21 | delete `l1400_types/__init__.py` | 6.4 | 6.4 |
  | 22 | restore it | **6.1** | 7.0 |
  | 23 | delete `purity.py`, `lazy_mixin.py` | **9.9** | 11.5 |
  | 24 | restore them | **10.7** | 12.8 |
  | 25 | delete a 24-file package | **10.0** | 12.3 |
  | 26 | restore it | **12.4** | 13.9 |

  This is what the spike found: levels wins on edits and loses 15–20% on mass toggles. At steps 23–26 both cost
  more than deriving again, which is §4's threshold. The threshold is not built yet: it is the next step, and with
  it the toggles cost the full derive's 8 s whichever algorithm runs.

## What the prior work says

- **Target architecture (03):**
  - §2, the push path. *Agrees*: only changed facts propagate (§3), and write-back is eventual (§1).
  - §4, overlay ∪ store. *Agrees*: §1's delta is the overlay.
  - §7, waves on deltas. *Agrees* (§3).
  - §10 answer 4, coarse cutoff with a counter. *Agrees*: the cutoff is per relation, and the profile counts the
    strata run against those changed.
  - §2's "no DRed". *In tension in wording*; see §6.
- **Decision 0083 (Two-pass-roots):**
  - §0, the unit of change is the definition, never the file. *Agrees*: §2's per-file delta is the step before
    per-definition, and no semantics hang on it.
  - §2, maintenance of a stored fixpoint by deltas, read in place by index under 1 GB. *Agrees*: this record is
    how this branch's maintenance gets there.
  - §3, symbol keys and `(definition, index)` node ids. *Adopted* as step 6.
- **O125** (the spike): the costs in the Intro, and its stated fix: indexed store, deltas in place, old rows read
  where they are, plans kept. *Adopted* as §1 and §5.
- **O126:** definition deltas are 14× smaller than preorder ones and 25× smaller than a file's. This is why step 6
  matters more than any constant factor.
- **Typing-support:**
  - 77e10d104 measures old and delta sources with the constants rules test them by. *Taken* (cherry-picked), and
    kept with the relation's statistics in §1.
  - a62c34692 names kept sources by `sql_col` and reports why a stratum is derived again. *Taken*.
- **Bend-language-investigation 0076 §9:** replay copies 2.18 M rows, and a component key made of bytes has no
  cutoff. *Departs* on both: nothing is replayed, and the cutoff is the net delta.
- **Decision 0064:** recording reads in the eager engine cost +60%. *Respected*: §3 records no reads.
- **Rerail-rule-expressivity, the SQL executor:**
  - retained session state;
  - signed change batches;
  - DRed and counting reducers;
  - 0.3–6 ms per edit.

  *Reused as the model*. Its 1.1 GiB at 50k edges is the warning behind keeping indexes only where a rule probes
  them (the engine's `unindex` already drops unread ones).
- **Spec §21.8:** semi-naive deltas, DRed for recursive components, full recompute past a threshold. *Agrees*; §4
  applies the threshold.
- **Lessons ledger #14:** an edit rewrites only its files' rows. **History index §2.20(d):** write paths O(inserted);
  one-shot must equal incremental. *Agrees*: every step keeps `session::full`'s equality check.
- **Resource budgets:** 1 GB. The kept `InRel`s are counted against `RERAIL_CROSS_RETAIN_MB`, as today's copies are.

## Open

- Whether 03 §2's "no DRed" is to be read as "no stored witness" (§6). This needs the architecture owner.
- θ and the cost estimate (§4) are unmeasured. The mass-delete steps above (23–26) are the cases it must catch.
- Interface summaries for libraries (03 §14's derivations) and this session store meet at the library boundary;
  0083 §5–§6 places it, and this record does not.
