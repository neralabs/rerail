# 0328: Maintenance is how the evaluator runs a rule on deltas, not a program written per stratum

Status: draft (2026-11-16), for the user's choice. It would retire the maintained forms (decisions 0117 step 3, 0241,
0298) and the support relations (`corpus/supports.rs`, counting as 0090 made it the default). Nothing is built.

## Intro

When one file changes, a session does not run every rule again. It keeps the last run's rows and works out what
changed: which rows of each relation are gone and which are new. Then it updates each relation by those changes
alone. That is *maintenance*.

Today the engine maintains a stratum by **writing a new program for it**: a set of rules over new relations
(`__del`, `__ins`, `__minus`, `__plus`, …) that compute the changes. That program is the stratum's *maintained
form*. Each form is written, keyed, stored, read back, opened, and then planned and placed like any program, on
every edit and for every stratum the edit reaches. On dl10 that is about 250 strata for a comment added to one file.

This decision asks to stop writing those programs. The evaluator already runs a rule with one of its atoms bound
to "the rows new since the last round": that is how its semi-naive fixpoint works (`native/setfix.rs`). An edit's
changes are the same thing, one level up: the first round's new and gone rows. So a stratum is maintained by running
its own rules with a changed atom bound to that atom's changes, not by a program written for it.

## The relations a maintained stratum uses

Take one rule, `h(x) :- p(x, y), q(y)`, and an edit that changed `p`. Every name below is a relation the engine makes
or binds today, in `plan/corpus/maintain.rs`.

```text
p__old(x, y)      the rows of p the last run kept           bound from the kept part, not derived
p(x, y)           p's rows now                              the input as the edit left it
p__minus(x, y)    rows gone:  p__minus :- p__old, !p.       a diff of old and new
p__plus(x, y)     rows new:   p__plus  :- p, !p__old.

h__del(x)         rows of h that may have lost a derivation ("overdelete")
                    h__del(x) :- p__minus(x, y), q__old(y).
h__redo(x)        of those, rows that still have one        ("rederive")
                    h__redo(x) :- h__del(x), p(x, y), q(y).
h__ins(x)         rows a new p row derives                  ("insert")
                    h__ins(x) :- p__plus(x, y), q(y).
h(x)              h's rows now: the old ones not deleted, plus the redone and the inserted
                    h(x) :- h__old(x), !h__del(x).   h(x) :- h__redo(x).   h(x) :- h__ins(x).

h__minus(x)       h's net loss, for the strata that read h:  h__del, not redone, not inserted
h__plus(x)        h's net gain:                               h__ins, not in h__old
h__ins__ready(0)  a flag that h__ins has run, so h__minus may read !h__ins
```

This is delete-and-rederive (DRed): delete everything that *might* have lost its support, then win back what still
has one.

### Counting, and `__sup`

DRed's "might" costs something: `h__del` can hold rows that never really went, and `h__redo` derives them again.
*Counting* avoids that by keeping, for each row of `h`, how many ways it is derived: a row goes when its count reaches
zero, so no row is deleted and rederived. Counting does not hold across recursion, because rows in a cycle keep
each other's counts above zero.

The engine counts by making each rule of a non-recursive stratum go through a *support relation*, one row per
derivation (the `counting` pass, `corpus/supports.rs`):

```text
h(x) :- p(x, y), q(y, _).      becomes      h__sup0(x, y, w0) :- p(x, y), q(y, w0).
                                            h(x) :- h__sup0(x, _, _).
```

A row of `h__sup0` has exactly one derivation, so its deletes are exact. A row of `h` goes when its last support row
does (`h__lost`, `h__held`). The count is never stored as a number: it is the support rows themselves.

### Why counting is the default

Decision 0090 measured both on the O125 replay (2026-11-09). DRed over-deleted 1.3–1.6× what was gone in
non-recursive strata, and 6.7–22× in recursive ones. But counting does not apply to recursive strata, which 0090
gave to *levels* instead. In the non-recursive ones, counting would save their redo, "about 0.3–0.5 s a step… Small
either way".

Counting was made the default on 2026-11-09 all the same, because it deletes only what is gone and derives nothing
again. The measurement in the same section shows its cost: "a support per rule makes about 3.7× the strata, and each
pays a fixed cost". On O125 steps 07–08, counting took 14.4–19.7 s for the cross part against DRed's 9.7–10.9 s:
"until 0101 removes the per-stratum fixed cost, counting is the slower default on edits like these". That fixed cost
was not removed. It is the form per stratum: each support relation is a stratum, so each gets a form, a key, a
stored source, a selection, a placement and a spec.

## What was measured (2026-11-16)

### One form, end to end

dl10, the 39 policies, one comment added to one `.py` file. The rule `kind(n, k) :- node(_, n, k, _, _)`
(`typing/programs/calls.rr:33`):

1. **Counting** splits it: `kind__sup0(n, k, w1, w2, w3) :- node(w1, n, k, w2, w3)` and
   `kind(n, k) :- kind__sup0(n, k, _, _, _)`. `kind__sup0` is `node` with its columns in another order, kept between
   runs as a relation of its own: a second copy of the corpus's node table.
2. **The edit** changes `node`, so the run reaches `kind__sup0`'s stratum, then `kind`'s.
3. **The general form** (0298) of `kind__sup0`: 8 rules (`__del`, two `__ins`, the survivors, the inserted, `__minus`,
   `__plus`, `__ins__ready`), and 3 more of maintenance (the rule itself again, and `__minus` and `__plus` as a diff).
   All of it follows from the one rule, the read that changed and which old rows are kept.
4. **Stored** as one source of 7,302 bytes, with an 89-byte record. The source rule is 40 characters.
5. **Every edit:** the form is found by its key, decompressed, copied into aligned memory, checked, opened, selected,
   and its 8 rules are added to the run's program, split into pieces, priced to choose whether to derive the stratum
   again (0090 §4), and specified, placed and lowered like any rules.

Of the 249 strata this edit reached, 201 are support relations (`__sup`). 100 of the reached strata have one rule in
the program and 14 in the form the run plans.

### DRed against counting, today

The engine still has the switch (`RERAIL_MAINTAIN=dred`). dl10, debug build, each mode on its own store, warmed, then
the same three edits and reverts on both. Each edit's comment text was new to both stores. Instructions retired:

| Step | DRed | Counting (default) |
|---|---|---|
| edit 1 | 75.37 G | 75.29 G |
| revert 1 | 53.91 G | 59.52 G |
| edit 2 | 75.73 G | 80.83 G |
| revert 2 | 53.41 G | 88.43 G |
| edit 3 | 68.31 G | 73.67 G |
| revert 3 | 53.03 G | 58.79 G |

DRed is as cheap or cheaper on every step.

Both still write a form per stratum. What the forms and their planning cost was profiled under 0298 on the same
edit: the reached strata's forms 0.054 s, finding and opening them 0.082 s, and placement and spec "about 0.46 s", of
a cross-file part of 1.26 s. The joins themselves took 0.017 s.

## Decision

1. **A stratum is maintained by running its own rules on deltas.** For a rule `h :- B1, …, Bn` and a changed atom
   `Bk`, the evaluator runs the rule's join with `Bk` bound to its delta (gone rows to delete, new rows to insert),
   the atoms before `k` over the new rows and those after over the old. This is the semi-naive step `setfix`
   already runs for a recursive atom each wave, made the first step of every stratum an edit reaches. No `__del`,
   `__ins`, `__redo` or `__minus` relation is declared, and no program is written, stored, read or selected.
2. **A relation's old rows and its delta are one relation, versioned** (0266: a base and its delta, read as one).
   `p__old` is the base, `p` the base read with its delta, `p__minus` and `p__plus` the delta's kills and adds. They
   are not relations made by rules.
3. **A rule's delta plans are made with its program**, once per program, one per atom that can change: the same
   rule, joined from a different atom. A run picks the plans of the atoms that changed; it prices and places no
   new rules.
4. **Delete-and-rederive is the default; support relations go.** The rederive step is the rule's own join, over
   the rows `__del` named. Measured, DRed is as cheap or cheaper on every step above, and 0090 found its
   over-deletion small in non-recursive strata. Where a stratum's over-deletion is measured to cost more than a
   count would, it gets a count per row, held as a column of the relation (counting as Gupta, Mumick and
   Subrahmanian write it), not one relation of derivations per rule. A rule with one positive atom never needs a
   support relation: its derivations are its input's rows.
5. **Recursive strata keep levels** (0090), run the same way: their rules on deltas, in rounds, inside the fixpoint
   that already runs them.

## What goes

- `maintain::form`, `maintain::general`, `General`, `StoredForms` and the forms' keyed steps (0298 steps 2–4).
- The counting pass (`corpus/supports.rs`), the `__sup` relations, and the kept copies they hold.
- Per run: adding a form to the run's versions, splitting it into pieces, and specifying and placing its rules.

## Path

Each step is measured in instructions retired on dl10 against today's default, and each step's rows are compared
with a cold run's.

1. **Measure the split.** Of an edit's ~75 G, how much is forms, selection, pricing, spec and placement (formats
   8–15 of the walk above), and how much is the joins.
2. **DRed the default** (`RERAIL_MAINTAIN=dred`'s path), and the support relations removed. The `kind` example
   becomes one stratum with no copy of `node`.
3. **A rule's delta plans at preparation** (3), beside its plan, for non-recursive rules of joins and negations.
4. **The evaluator runs a reached stratum from them** (1), with the delta read from the versioned relation (2);
   `maintain::form` stays only for the shapes not yet covered (aggregates by group, `nearest`, text tests).
5. Those shapes, then recursive strata (5); then the forms' code and storage go.

## Open

- Aggregates by changed group, `nearest` over a changed tree, and natively made text tests: today each has its own
  form. Each needs its delta step in the evaluator.
- Whether any stratum's over-deletion is large enough for a count column (4): measured per stratum after step 2.
- Step 2's DRed path still writes forms; the gain from dropping supports alone is measured before step 3.
