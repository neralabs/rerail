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

The plan's whole-program choices are which relations are inlined or stored (0097) and which negations are demanded
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
- θ and the cost estimate (§4) are unmeasured.
- Interface summaries for libraries (03 §14's derivations) and this session store meet at the library boundary;
  0083 §5–§6 places it, and this record does not.
