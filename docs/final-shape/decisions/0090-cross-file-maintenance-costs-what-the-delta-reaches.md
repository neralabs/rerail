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

## Measured: whole-program choices kept by the program's digest (2026-11-09)

The plan's whole-program choices are which relations are inlined or stored (0099) and which negations are demanded
(0048). They are made once per program and scale, before any stratum runs. A corpus run now keeps them
(`plan::choices`). It keeps them in the process, and in the tape cache under the program's digest (the engine build,
the switches that change them, and the program). With them it keeps the node count they were chosen at. A run of
the same program at that scale or less takes them, and the program as they rewrote it, without choosing again. Such
runs are an edit's one-file chunk, a later chunk, and the cross part again. A run past twice that scale chooses
again. The tape cache's run manifest names the kept choices, so collection keeps them.

Pack one-edit run, debug build, load 90–155 (the same runs as 0077 §4):

| span | before | after |
|---|---:|---:|
| local `statistics and plan` | 4.12 s | 0.73 s |
| local `demand` | 4.05 s | 0.66 s |
| cross `statistics and plan` | 2.30 s | 0.64 s |
| cross `demand` | 2.26 s | 0.60 s, 0.57 s of it reading the choices back |
| local `memo replay` | 5.20 s | 2.41 s |

Reading the choices back is mostly parsing the rewritten program. The memo replay is a separate change. A file's
own rows of an export are appended to the export unchecked: no other file's entry can hold them. Only rows that
name another document are checked against the export's set (`exports::append_own`). The findings are identical.

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
  more than deriving again, which is §4's threshold. The threshold is now built (next section), but it saves less
  than expected: the toggles still cost about twice the full derive, because of fixed costs, not row work.

## Built: counting the default (2026-11-09)

A session now maintains its cross-file part by counting. `RERAIL_MAINTAIN=dred` keeps delete-and-rederive, to
compare with.

- **Supports** (`corpus/supports.rs`, unchanged): each rule of a non-recursive stratum has its own relation of
  derivations, one row per derivation, so its deletes are exact.
- **A head is exact too** (`maintain`). A head whose rules each project one support (`h(x) :- h__sup0(x, _)`) is
  a count of its supports:
  - a row that lost a support goes to `h__lost`;
  - `h__held` keeps the lost rows that still have a support in the new rows;
  - `h__del` is lost and not held, with no rederive.

  A row that keeps a support is no longer deleted and appended again. So the survivors start from the old rows
  with only the truly gone ones dead (step 4's in-place start), and compaction sees only real garbage.
- **Recursive strata** have no supports. They are maintained by *levels, checked* (above), or by
  delete-and-rederive under `RERAIL_RECURSIVE=dred`, where the first comparison derived them again.
  `RERAIL_COUNT_RECURSIVE=again` restores that. The measurements below predate levels: their recursive strata ran
  by delete-and-rederive.
- **A session stores its cross-file relations** (`no_inline`, decision 0099), except one the program declares
  `inline`. An inlined relation leaves no rows for the next run, so every stratum over it was derived again: 98–118
  strata a step. `RERAIL_SESSION_INLINE=on` inlines as a full run does, to compare.
- **An inlined relation is not copied out** of the cross part (`Outcome::inlined`). Before this, copying one out
  failed ("no such table: py_f"). Its stratum runs again next time. Over unchanged reads it leaves its readers
  clean (`retain::cut`).

**Measured.** O125 steps 01–04, paired and alternated twice on one binary, at load 15–30. Each figure is session
CPU, and every step's outputs equal a full run's.

| mode | 01 (1 file) | 02 (2) | 03 (1) | 04 (4) | strata maintained, derived again |
|---|---:|---:|---:|---:|---|
| counting, stored (default) | 1.5–1.7 s | 1.7–1.8 s | 1.0 s | 1.3–1.4 s | 715–783, 0 |
| delete-and-rederive, stored | 1.0–1.3 s | 1.2–1.8 s | 0.7–0.9 s | 1.0–1.3 s | 190–210, 0 |
| counting, inlined | 1.3 s | 1.2–1.4 s | 1.0–1.2 s | 1.2–1.3 s | 616–672, 99–118 |

Steps 01–08 under counting, stored: 8 of 8 equal a full run, as under delete-and-rederive.

**What it says.** Counting derives nothing again and deletes only what is gone. But today it costs 0.2–0.5 s a step
more than delete-and-rederive. That is not row work: a support per rule makes about 3.7× the strata, and each
pays a fixed cost. Decision 0101 is the work that removes it: skip a stratum whose deltas are empty, make the
whole-program passes once, and run a stratum's pieces as one component.

## Built: derive again by input delta, and what makes levels expensive (2026-11-10)

This section covers §4's threshold, a profile of levels on the mass-delete steps, and four fixes made with them.
All figures are CPU seconds (user + system) on a debug build. The machine's load was high (14–116) and moved
during runs, so pairs differ by up to 1.4× on identical work; read the figures as directions, not constants.

### §4's threshold, built

Each maintained stratum keeps its rules as given beside its maintained form (`front::Again`). The run picks one
form per stratum, at the stratum's first piece:

- **Derive again** when its input deltas are at least θ of its old rows, and at least a floor of rows. The input
  deltas are the `__minus` and `__plus` relations the maintained form reads. θ is 0.25 and the floor is 1,024 rows
  (`RERAIL_AGAIN_THETA`, `RERAIL_AGAIN_MIN`). Without the floor, tiny sessions derived everything again.
- **The form derived again runs where its inputs are ready**: after the strata that make its input deltas. It
  then makes its own `__minus` and `__plus` against its old rows, so the strata above it read deltas as before.
- `RERAIL_AGAIN=off` keeps every stratum maintained. `RERAIL_AGAIN_WHY=1` prints each choice. A session's report
  counts the strata derived again under "input deltas past θ".

**Measured.** The stress replay's steps 22–25, one process each, in order: θ = 0.25, then off.

| step | change | θ = 0.25 | off | strata derived again |
| --- | --- | ---: | ---: | ---: |
| 22 | restore a package `__init__` | 14.8 | 16.8 | — |
| 23 | delete 2 files | 13.9 | 18.9 | 441 |
| 24 | restore them | 15.8 | 20.3 | 442 |
| 25 | delete a 24-file package | 14.8 | 20.4 | — |

(—: that run did not count them; the count was added after it.)

Both equal the full derive (θ = 0.25 checked it at every step). The threshold saves 2–6 s a step. But the cold
derive's cross part is 5.8–7.9 s, so a maintained step still costs about twice a cold one. θ = 0, which derives
every maintained stratum again, costs 18.3–18.9 s on steps 23–24. So the gap is not row work.

**What the gap is** (step 23, θ = 0, the profile's own times):

| part | CPU s | note |
| --- | ---: | --- |
| the rewrite and old sources | 4.4 → **0.31** | fixed below |
| joins in the engine | 4.9 | the row work |
| `demand`: inline-or-store and the negation split | 2.8 | a whole-program pass, over ~3× the rules |
| opening relations and indexes | 1.1 | 12,234 opens |
| recording plans for the profile | 0.9 | profile only |
| statistics, binding kept relations, patching their statistics | 2.5 | |

Most of it is a fixed cost per stratum or per program, which decision 0101 is meant to remove: run the
whole-program passes once, and skip strata whose deltas are empty.

### The gap, profiled against a cold derive

The table above is the profile of one session step. Compared span by span with the cold derive before it, on
one binary (`RERAIL_PROFILE`: the replay keeps each run's `spans-<step>-<run>.tsv`), the gap is this:

- **Inline-or-store priced every rule for nothing: 3.6 s, fixed.** It plans every rule of the program before
  looking for a relation it may inline. A maintained program has about ten times the rules: 19,777 estimates at
  step 23, against 2,052 cold. A session stores every cross-file relation, so there is never a choice to make.
  It now estimates only when some relation may be inlined: 0.2 s, and the step's cross part goes from 14.0 to
  12.1 s.
- **The first maintained step measures what it binds: 2.0 s, once.** Each kept relation and each input's
  `__plus` is measured when bound, and each maintained relation is counted for its patched statistics. From the
  second step on, the patched statistics are carried. What's left is about 0.7 s a step, mostly the `__plus`
  deltas (226 of them, 0.38 s).
- **Joins: 8.2 s at step 24 against 6.1 s cold.** Three causes:
  - *A stratum derived again also makes its deltas.* `h__plus :- h, !h__old` and `h__minus :- h__old, !h` are
    full anti-joins: 188 K probes each for `kind` and `docof`, whatever the change. Over step 24 they cost 1.5 s,
    more than the rules they follow (the supports, 1.1 s).
  - *It runs in its support form:* `kind__sup0` from `node`, then `kind` from that, where the cold cross part
    reads `kind__sup0` from the file-local part. That's two strata for each head, each with its deltas.
  - *More engine calls:* 3,531 against 1,286. Engine time outside the plan pieces rises 2.5 → 3.5 s, about
    0.4 ms an extra call.
- **Per-stratum bookkeeping: about 1 s.** Each stratum's own time, the run's own time, spec, re-pricing and
  binding, over 2,478 strata against 826.
- **Smaller:** input deltas from the changed documents 0.26 s, the rewrite 0.15 s, what's left of inline-or-store
  0.2 s.

Step 24 after the fix: cross part 13.0 s against 8.5 s cold, at load 55–70. On a mass delete, maintenance saves
no join work. The derived-again strata cost more than their cold form, because of the deltas and the support form.

**What would close it**, in order of size:
1. Make a derived-again stratum's deltas only when a maintained reader reads them. *Built: see below.*
2. Derive again in the cold form (the head's own rules, reading the support from the file-local part where the
   cold program does), and keep the support for the next run's counting only where it's maintained.
3. 0101's per-stratum fixed cost: fewer, larger strata and engine calls.

### Built: a stratum derived again makes its deltas only where they are read (2026-11-10)

Deriving a stratum again also made its deltas against its old rows, `h__plus` and `h__minus`. Each is a full
anti-join, whatever the change. Now they are deferred, and made where a stratum that runs first reads them.

**The reader's decision is the hard part.** A maintained group picks its form by measuring its input deltas. If
it measures a deferred delta, the delta must be made, and then nothing is saved: only 3 of the 441 groups derived
again at step 23 have deltas no group reads. So a group that reads a deferred delta takes its size from an
**estimate**: the input deltas of the stratum that makes it, at most that stratum's rows new and old. Every group
derived again carries its own estimate forward, so a large change propagates and a small one stops. A group
that stays maintained makes the deferred deltas it reads. What a deferred delta reads lives until its last reader.

`RERAIL_AGAIN_DEFER` picks the rule: `estimate` (the default), `propagate` (a reader of a deferred delta is
derived again without deciding), `measure` (the delta is made to be measured), or `off`.

**Measured.** CPU time alone could not separate the modes: identical work differed by 30% between runs. So each
comparison ran the modes at once, as separate processes under the same load, and counted the engine's work
(rows touched, deterministic) and its calls.

| | stress 23 | stress 24 | O125 06: work, calls | 07 | 08 |
| --- | ---: | ---: | --- | --- | --- |
| `estimate` | **9.2 s** | **9.5 s** | 1.47 M, 3,015 | 6.41 M, 3,079 | 7.15 M, 2,868 |
| `propagate` | 8.8 s | 9.4 s | 5.66 M, 2,200 | 6.14 M, 2,529 | 7.56 M, 2,362 |
| `off` | 10.9 s | 11.7 s | 1.47 M, 3,015 | 6.36 M, 3,744 | 6.88 M, 4,388 |

The cold derive's cross part takes 7.1–7.4 s and does 7.77 M work. `propagate` quadruples step 06's work: 4
groups crossed θ, and it pulled in 362 more. At about 0.4 µs a unit of work and 0.4 ms an engine call:
- `estimate` costs the same as `off` at step 06, and 0.2–0.5 s less at 07–08;
- on the toggles it saves 1.7–2.2 s a step, as `propagate` does.

Steps 08 and 24 equal the full derive, and the findings gate is unchanged.

### Investigated: deriving again without supports (2026-11-10)

The question was whether a stratum derived again should use the program's original form, without counting's
supports, and keep a support only where the next run maintains it. The data:

- **The cold program has supports too.** Of its 618 supports, 599 are relations it makes. Only 19 are SQL views
  of one input, `kind__sup0` and `docof__sup0` among them. So the cold form differs from the session's only in
  those 19.
- **Those 19 views cost real time in a session.** It makes them relations, keeps them and maintains them. At
  stress steps 23–24 they take 0.95 s and 0.65 s, where cold they cost nothing. At O125 steps 06–08 they cost
  nothing, since their changes there are 301–5,930 rows of about 181 K. They also hold 11.4 MB of kept rows in
  11 relations, against 16.4 MB for the other 444 supports and 19.9 MB for 158 heads. That's about a quarter of
  what a session keeps.
- **Folding the other supports into their heads saves little.** The support's join work stays (0.5–0.9 s a
  step, derived again). What goes is each head's projection of its support: 0.07–0.22 s a step, plus 167–333
  engine calls at about 0.4 ms each.
- **A support derived again is rarely maintained next step.** Under `estimate`, 4 of 166 go from derived again
  to maintained at 07→08, and 5 of 455 at 23→24. So not keeping it would rarely cost a rebuild.

**What this suggests** (built: next section). Keep a support that is a view of one input a view in a session as well. Its old rows are
the view over the input's old rows, and its delta is the view over the input's delta. That saves the 19 views'
time and about a quarter of the kept memory, with no third form in the rewrite. Folding the other supports into
their heads would save 0.15–0.35 s a step for a third form, and fewer, larger strata is 0101's work anyway.

### Built: a view of one input stays a view in a session (2026-11-10)

This follows the data above. A view of one input is a relation of one rule whose body is one positive atom of an
input, with every input column in its head. A row of it is then exactly one row of the input. The cold run plans
such a relation as a SQL view. A session now treats it the same way:
- its rule stays as given, so it plans as a view;
- its old rows are the view over the input's old rows, and its delta is the view over the input's delta;
- no rows of it are kept, and the cut neither runs it nor binds it when its input is unchanged.

`RERAIL_SESSION_VIEWS=off` gives the old behaviour.

**Measured**, both modes at once on one binary (cross-part CPU seconds and kept megabytes):

| | O125 06 | 07 | 08 | stress 23 | 24 |
| --- | ---: | ---: | ---: | ---: | ---: |
| views, CPU | 14.3 | 12.2 | 12.0 | 12.0 | 13.3 |
| relations, CPU | 16.1 | 12.4 | 12.1 | 13.6 | 14.1 |
| views, kept | 58.5 | 57.6 | 55.2 | 40.1 | 59.6 |
| relations, kept | 73.7 | 72.4 | 69.9 | 51.7 | 74.9 |

The engine's work is unchanged, as expected: the cost was in making, indexing and measuring the relations, not in
joins. There are 30–93 fewer engine calls a step, and the kept memory falls by 20–22%. Steps 08 and 24 equal the
full derive, and the findings gate is unchanged.

### What makes levels expensive

Steps 22–25 of the stress replay, profiled by stratum and by role:

- **On a mass delete, almost every candidate is truly gone.** At step 23, `pv` has 17,352 candidates and 17,324 of
  them are gone. DRed over-deletes 18,674 and puts back all but 17,319. `walk` has 18,803 in both. So the check
  that levels adds saves almost no deletes here, and its cost is all overhead.
- **The check runs in many small batches.** It runs one batch per level: `pv`'s check took 168 of them, where
  DRed's rederive took 27 waves. Each batch pays a plan and an engine call.
- **Some check plans are misordered.** One estimate is off by about 270×: a probe of `fv__live` by `(k, v)` is
  expected to return about 1 row and returns about 270, so the join order chosen is wrong.
- **Per role at step 25:** levels' candidates take 1.56 s (4.7 M rows), the check 0.77 s (2.6 M), insert 0.93 s.
  DRed's delete takes 1.69 s (4.8 M), insert 1.35 s, rederive 0.57 s.

Levels still wins on ordinary edits, where most candidates survive their check (steps 06–08 above). On mass deletes
the threshold now takes those strata out of maintenance. Two improvements remain open: batches per level, and the
skew estimate.

### Fixes made with it

- **Kept rows drifted under levels.** Two faults:
  - The driver showed again rows that were dead before it started.
  - When a deleted row had been pushed again, finding a row's live copy could return its old dead copy, with the
    old, lower level. The candidate then looked decided and was kept without a check.

  Over O125 steps 01–08, levels lost 3–281 `acls` rows and 2–8 `icall` rows against a derive over the old inputs.
  Now it matches DRed's kept rows at every step. `RERAIL_LEVELS_TRACE=<value>` traces the candidates holding a
  value.
- **A kept relation's bytes count its levels** (4 bytes a row) against the session's budget.
- **DRed's and counting's batch relations take no whole plans** (`__del`, `__lost`, `__ins`, `__redo`), as the
  levels relations already did. Their batches are small from the first row, and a whole plan scans the old rows
  first. O125 steps 07–08, everything under DRed, one pair on one binary (`RERAIL_SMALL_WHOLE=dred` gives the
  old behaviour back):

  | step | whole plans (before) | no whole plans |
  | --- | ---: | ---: |
  | 07 (6 files) | 13.7 | **9.7** |
  | 08 (6 files) | 12.1 | **10.9** |

- **The rewrite cost 4.4 s a step.** Building each stratum's derived-again form scanned every rule once per
  stratum, which is quadratic in the program. The delta rules are now found by relation, once: the whole rewrite
  takes 0.31 s.

### What this shows about the default

On O125 steps 07–08 the default (counting, levels, θ = 0.25) takes 14.4–19.7 s for the cross part. Everything under
DRed takes 9.7–10.9 s, and a cold derive takes 4.4–8.8 s. Counting runs 783 strata a step where DRed runs 209. The
"Built: counting the default" section measured a 0.2–0.5 s difference on steps 01–04, but here it is 4–10 s. So
until 0101 removes the per-stratum fixed cost, counting is the slower default on edits like these.

## Merged: levels the default in the integration branch, held only where read (2026-11-10)

Levels (above) is the default for a session's recursive strata on the integration branch too
(`RERAIL_RECURSIVE=dred` compares). Four fixes made it exact and cheaper there.

- **Rows dead before a run.** O125 step 08 kept 1 `type_of` and 2 `possible_type` rows a full run does not
  derive. A `live` row deleted in an earlier run came back to life when its level was shown, and `id_of` found
  that old dead copy, not the row's later live one. The driver now keeps the rows dead before the run
  (`was_dead`): never shown, never a candidate's row (517250487).
- **What is read back.** A relation is read back from the engine only if this run kept it there natively
  (`Outcome::native`), and only with levels. Reading back by `h__gone` left a 58-relation component on
  delete-and-rederive.
- **Levels only where read.** Every relation held a 4-byte level per row. Now only a relation on a cycle of its
  component's rules, a levels line's relation, or one started from a kept relation with levels gets them. At
  O125 the kept levels go from 13.1 MB to 2.9 MB.
- **The planner asks for them.** A component keeps levels only when its spec says `leveled`. The plan writes that
  line for a stratum whose relations carry the `levels` qualifier, and only a session's cross part sets it. A
  full run holds no levels. Its closures run on the device, since bit rows give reach, not distance. With
  levels, a closure is made by distance on the host (`by_distance`).

Result: O125 1–8 are the same as full with levels (with and without `RERAIL_AGAIN=off`).
Gate: 3988 303 298 0.

## Measured: θ, the smaller items, and counting against DRed again (2026-11-10)

O125 replay steps 06–08 (edits) and stress steps 23–24 (mass deletes), debug build, `RERAIL_NODE_IDS=definition`.
The modes of each table ran at the same time, at load 80–140. Work is the engine's work units (deterministic);
calls are engine runs.

**θ.** On edits it hardly matters. A lower θ derives more strata again: fewer calls, more work, the same CPU.

| θ | 07: cross CPU, work, calls | 08: cross CPU, work, calls |
| --- | --- | --- |
| 0.1 | 9.7 s, 6.67 M, 1,683 | 10.7 s, 7.14 M, 1,624 |
| 0.5 | 10.9 s, 6.39 M, 2,657 | 10.4 s, 7.38 M, 2,519 |
| 1 | 11.0 s, 6.18 M, 2,796 | 11.8 s, 6.38 M, 3,135 |
| off | 10.6 s, 4.95 M, 3,479 | 11.6 s, 4.92 M, 4,213 |

On mass deletes it matters, but anything from 0.1 to 0.25 does the same:

| θ | 23: cross CPU, work, calls | 24: cross CPU, work, calls |
| --- | --- | --- |
| 0.1 | 10.8 s, 4.60 M, 1,665 | 12.4 s, 7.35 M, 1,787 |
| 0.25 (default) | 11.4 s, 4.88 M, 1,679 | 12.3 s, 7.35 M, 1,787 |
| 1 | 14.7 s, 6.38 M, 2,109 | 12.2 s, 7.70 M, 1,909 |
| off | 26.4 s, 12.79 M, 3,660 | 20.7 s, 20.91 M, 3,729 |

So θ stays 0.25. The cost-estimate half of §4 is not built: a single θ from 0.1 to 1 moves an edit's cost by about
10%, so a better choice per group has little to gain on these replays.

**The probe off by 270×.** `fv__live` by `(k, v)` no longer runs at 23–24: θ derives those strata again. A joint
estimate for probes on several columns was tried. It prices a probe by the fanout of a native index on those
columns (Σ postings² / rows), not by the product of each column's own. Work moved by under 0.4% either way, with
derive-again on or off. One seed plan got worse (2,255 to 3,450 work units) and one check got slightly better,
so the change was not kept. The misordered joins left are of another kind (0101, "after the sort fix").

**Levels, one check batch per level.** No longer costly. The level waves take 0.06–0.17 s a step (167–220 runs).
Batching levels together is also not sound as it stands: a lower level's candidate could support a higher one's.

**The first step's statistics.** At step 06 a kept relation's statistics are counted from its postings, once,
so later steps patch them by delta. That is 798 relations and 1.55 s of the step, about 0.3 µs a row and column
in a debug build. Counting only for the strata that run would not help: nearly every stratum runs at the first
step (4,123 of them). It is the same cost as the measure it replaced, and it is paid once a session.

**Counting against DRed** (`RERAIL_MAINTAIN=dred RERAIL_RECURSIVE=dred`), cross CPU of steps 06 / 07 / 08:

| | counting | DRed |
| --- | --- | --- |
| before 0101's sort fix | 15.0 / 11.7 / 12.0 s | 10.9 / 8.7 / 8.7 s |
| after it | 14.2 / 8.2 / 7.7 s | 10.7 / 8.4 / 6.9 s |
| work | 1.47 / 6.38 / 7.45 M | 1.48 / 6.63 / 7.40 M |
| engine calls | 2,492 / 2,601 / 2,429 | 947 / 958 / 859 |

Before today's fixes counting cost 1.5–2× DRed. It is now even with it on edits 07–08, within this load's noise.
At the first step, 06, it costs about 3.5 s more: it counts its kept relations once (above), and it runs more
pieces. Its extra calls are its supports. Each one is cheap now: counting's 2,601 calls took 3.4 s of joins at
07 and DRed's 958 took 4.1 s, for the same work.

## Fixed: two ways recursive strata lost their levels (2026-11-12)

At O125 step 07, 82 strata were derived again past θ. Their work was 0.34 M of the step's 5.39 M, and for each of
them the planner priced deriving again below maintaining. The larger cost was one recursive component of 183
relations (`pv`, `fv`, `alook`, …), at 3.30 M. It ran on delete-and-rederive, not levels, and its deletes were
almost all false: `pv` overdeleted 3,816 rows and derived back 3,761 (98.6%).

`RERAIL_LEVELS_WHY` named the cause: `garg` and `gcall` had no levels. Their cycle closes only through a rule
over `tmap`, which has no rows. The run left that rule out, so the engine saw no cycle through them and kept no
levels, and a stratum is maintained by levels only when all its relations have them. The fix: the spec's
`leveled` line lists the relations that reach themselves through the stratum's rules as written, and those keep
levels.

A second cause, found while testing the first: a new process reads its kept relations from the store's frames,
which held rows only. So every recursive stratum a new process maintained ran on delete-and-rederive. The frames
now hold each relation's levels in a part of their own (`LEVELS_PART`, `i64::MIN + 1`: not −1, which became the first
delta frame's under 0117 step 4, and which a frame put whole removes), read where its digest and row count match.

After both fixes, at step 07, all 18 recursive strata are maintained by levels. The step's work fell from 5.39 M
to 4.05 M, and the component's from 3.30 M to 1.98 M (load 21–29). The test
`a_cycle_closed_by_an_empty_relation_still_keeps_levels` guards the frames' levels. The left-out cycle didn't
reproduce at test size, so it is checked at O125 only. O125's outputs after the change were not compared with a
full run (`O125_NO_FULL`).

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
- The cost-estimate half of §4 is not built. Measured, a single θ from 0.1 to 1 moves an edit's cost by about
  10% (θ, above), so it waits for a replay where groups differ more.
- Counting or DRed as the session default: even on edits after 0101's sort fix. Counting costs about 3.5 s more
  on a session's first step.
- Interface summaries for libraries (03 §14's derivations) and this session store meet at the library boundary;
  0083 §5–§6 places it, and this record does not.

## Measured: why a session's maintenance deletes so much (2026-11-09)

A session's maintained relation gives the rows it removed and added since the last run (`InRel::delta_since`,
decision 0096 step 4). A row in both was overdeleted and put back; a row only removed is truly gone. Over every
maintained relation of the O125 replay, steps 01–03, load 70–130:

| step | node ids | removed | put back (overdeleted) | truly gone | truly new | session CPU |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 01 (1 file) | preorder | 81,014 | 7,841 (10%) | 73,173 | 74,039 | 25.0 s |
| 02 (2 files) | preorder | 92,014 | 13,500 (15%) | 78,514 | 80,266 | 20.7 s |
| 01 | by definition | 70 | 22 | 48 | 903 | 12.8 s |
| 02 | by definition | 9,565 | 7,357 (77%) | 2,208 | 3,950 | 15.3 s |
| 03 | either | 2,386 | 810 (34%) | 1,576 | 1,666 | 7.8 s |

Two causes, in this order:

- **Node ids.** With preorder ids (the default), an edit renumbers every node after it in the file. Each row
  naming one is truly gone and comes back under a new id (`look`: 2,379 gone, 2,411 new). DRed is not at fault
  here: these are real deltas. Per-definition ids (`RERAIL_NODE_IDS=definition`, decision 0083) take step 01
  from 81,014 rows removed to 70, and the session from 25.0 s to 12.8 s.
- **DRed in recursive components.** Once the ids are stable, what is left is overdeletion, nearly all of it in
  recursive components. In the 58-relation one, step 02 removes 2,723 rows of `pv` and puts back 2,676 (47
  gone), and `found` removes 523 and puts back all 523. Non-recursive relations put back almost nothing (0–3% of
  what they remove): a row there rarely has a second derivation.

What the component's maintenance costs, by definition ids (the `acls/…` stratum and its `__del` stratum):

| step | the strata | joins | spec, planning, opening |
| --- | ---: | ---: | ---: |
| 01 (70 rows removed) | 3.4 s | 0.38 s | about 2.3 s: 4,363 join variants of the rewrite |
| 02 | 6.0 s | 3.1 s, mostly rows overdeleted and derived again | about 2.1 s |

So DRed is not needed:

- **Recursive components:** the levels scheme above deletes about what is truly gone (627 against DRed's 4,334
  at step 06; 13,280 against 37,686 at step 08). Its cost is 0.01–0.58 s on the 58-relation component, against
  DRed's 0.23–1.85 s, in the spike's evaluator. It replaces the overdelete and rederive rules with a count per
  row, 8 bytes a row.
- **Non-recursive strata:** DRed's overdelete is close to the true delta, so counting gains little on rows.
  Plain counting would still drop the rederive rules, which are part of the fixed cost of the rewrite's variants.
- **Order of work:** per-definition ids first (the larger win, and levels' deltas assume stable rows). Then levels
  for recursive components in the engine, its witness kept with the relation (0096's `Patched` keeps a session's
  per-relation state the same way). The rewrite's fixed cost of about 2 s for 4,363 variants needs its own fix:
  plan and spec only the variants whose delta is not empty.
