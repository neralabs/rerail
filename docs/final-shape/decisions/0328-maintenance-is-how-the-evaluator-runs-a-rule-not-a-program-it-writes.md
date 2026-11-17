# 0328: A rule is stored once, small; maintenance is metadata and a mode of the evaluator, not more rules

Status: draft (2026-11-16; on blocks and spiked, 2026-11-17), not settled. The user agrees with the direction,
asked for the details on decision 0266's blocks to be explored before deciding, and then asked whether blocks and
deltas make counting and DRed's machinery unnecessary. A spike says they do for every stratum but a recursive one
(below, "Maintenance from the deltas"). How recursion is kept is open for the user. It would retire the maintained forms (decisions 0117 step 3, 0241,
0298), the support relations (`corpus/supports.rs`, 0090's counting), and the program image as the format of a
pass's output (0173, as 0266 step 3 also plans). Nothing is built.

## Intro

When one file changes, a session keeps the last run's rows and updates each relation by what changed. That is
*maintenance*. Today the engine does it by **adding rules**:

- a pass (`counting`) gives each rule a relation of its own derivations (`__sup`);
- for each stratum an edit reaches, a program of change rules (`__del`, `__ins`, `__minus`, `__plus`, …) is
  written, stored, read back, selected from, and planned and placed like any program: the stratum's *form*;
- the program passes that lead there (the split, the cross-file part, the supports) each store a whole program
  image.

The information all of this carries is small: for each rule, which of its atoms changed and in which order to join
from it; for each relation, its old rows and what was added and removed; and for a stratum that needs it, a count or
a level per row. This decision stores the rules once, in a compact form, and keeps that information as metadata
beside them. The evaluator runs a rule *from a changed atom* as a mode, the way its fixpoint already runs a
recursive rule from the last round's new rows (`native/setfix.rs`, semi-naive). No rule is written for it.

## The relations a maintained stratum uses today

Take `h(x) :- p(x, y), q(y)`, and an edit that changed `p`. Every name is one the engine makes or binds today
(`plan/corpus/maintain.rs`):

```text
p__old(x, y)      the rows of p the last run kept           bound from the kept part
p__minus(x, y)    rows gone:  p__minus :- p__old, !p.       a diff of old and new
p__plus(x, y)     rows new:   p__plus  :- p, !p__old.

h__del(x)         rows of h that may have lost a derivation ("overdelete")
                    h__del(x) :- p__minus(x, y), q__old(y).
h__redo(x)        of those, rows that still have one        ("rederive")
                    h__redo(x) :- h__del(x), p(x, y), q(y).
h__ins(x)         rows a new p row derives                  ("insert")
                    h__ins(x) :- p__plus(x, y), q(y).
h(x)              the old rows not deleted, plus the redone and the inserted

h__minus(x)       h's net loss, for the strata that read h:  h__del, not redone, not inserted
h__plus(x)        h's net gain:                               h__ins, not in h__old
h__ins__ready(0)  a flag that h__ins has run, so h__minus may read !h__ins
```

This is delete-and-rederive (DRed): delete what *might* have lost its support, then win back what still has one.

**Counting** avoids the over-deletion: a row whose derivations are counted goes when its count reaches zero. It does
not hold across recursion, because rows in a cycle keep each other's counts above zero. The engine counts by giving
each rule of a non-recursive stratum a relation of its derivations:

```text
h(x) :- p(x, y), q(y, _).      becomes      h__sup0(x, y, w0) :- p(x, y), q(y, w0).
                                            h(x) :- h__sup0(x, _, _).
```

The count is never a number: it is those rows. For a rule of one atom it is a copy of the atom's input:
`kind(n, k) :- node(_, n, k, _, _)` gets `kind__sup0`, the node table with its columns in another order, kept between
runs. **Levels** (0090) give a recursive stratum's rows the round they were derived in, which is what lets counting
hold inside recursion.

Decision 0090 made counting the default on 2026-11-09, and measured then that "a support per rule makes about 3.7×
the strata, and each pays a fixed cost", and that on O125's larger steps counting was 4–10 s slower than DRed "until
0101 removes the per-stratum fixed cost". That cost was not removed: it is the form per stratum.

## What was measured (2026-11-16)

Debug build, CPU binding, strict SQL, each mode on its own store; instructions retired, and CPU from profiles.

### dl10 and the Python pack, both modes

| Workload | Step | DRed | Counting (default) |
|---|---|---|---|
| dl10: 39 data-layer policies, 10 files | edit (3) | 68.3–75.7 G | 73.7–80.8 G |
| | revert (3) | 53.0–53.9 G | 58.8–88.4 G |
| Python pack: 317 rules (Benchmarks-and-parity's step benchmark, the Async-family's rules among them), 50 files | cold | 2,702 G | 2,749 G |
| | append a function | 459 G | 482 G |
| | change a body | 636 G | 610 G |
| | add a comment | 408 G | 444 G |
| | add an import | 699 G | 490 G |

Every step's findings are the same in both modes. Neither mode wins everywhere: an import, which moves the
cross-file module relations, costs DRed 43% more. So the choice is per stratum and per edit, not one switch.

### Typing-support's workloads (2026-11-17)

These ran once this branch had Typing-support's branch merged, with three fixes:
- Q21, theirs, which lets `calls.rr` read a node's text from the command line;
- the native join's lattice `top` and `@flat_bin`, which let `workload.rr` run under the strict mode;
- the planner's heavy-key and walk samples (0209, 0233, 0245), which read a kept relation's own row ids through a
  source that numbers its live rows. From step 05 on, every maintained run of `calls.rr` panicked there.

The O125 replay (`~/orca/rerail-shared/o125-replay`, 77 files, 20 commits) was run cold, then through steps
01–08, each mode on its own store. Instructions retired, in G:

| Step | Files changed | `calls.rr`, counting | `calls.rr`, DRed | `workload.rr`, counting | `workload.rr`, DRed |
|---|---|---|---|---|---|
| cold | 77 | 309.7 | 288.8 | 71.0 | 66.2 |
| 01 | 2 | 204.5 | 184.3 | 63.3 | 62.6 |
| 02 | 1 | 193.2 | 168.2 | 63.3 | 60.6 |
| 03 | 3 | 244.7 | 214.3 | 65.1 | 62.3 |
| 04 | 1 | 181.4 | 157.7 | 63.4 | 60.8 |
| 05 | 14 | 338.2 | 307.7 | 69.9 | 66.9 |
| 06 | 2 | 204.4 | 177.2 | 64.1 | 61.5 |
| 07 | 6 | 212.5 | 188.4 | 63.7 | 61.1 |
| 08 | 6 | 279.2 | 245.7 | 68.7 | 65.6 |

- **Findings.** Both modes give the same findings at every step (`calls.rr`: 10,475 to 10,939). Step 08's equal a
  run from scratch on a fresh store. `workload.rr`'s rules find nothing in these files; its `cv_const` (41 rows)
  and `type_of` (15,678) are its work.
- **DRed is cheaper at every step**, by 9–14% on `calls.rr` and 1–5% on `workload.rr`. Both modes maintain
  recursive strata by levels; the mode changes the others.
- **An edit costs 55–110% of a cold run**, 14 files at step 05. Profiled at step 06 (CPU, the cross-file part):

| Share of the cross-file part | `calls.rr` step 06, counting | `calls.rr` step 06, DRed |
|---|---|---|
| the cross-file part | 7.78 s | 6.04 s |
| kept rows moved to and from their frames | 29% | 30% |
| joins, and the jobs that run them | 28% | 25% |
| placement, spec and pricing of the strata | 18% | 17% |
| input deltas and digests | 15% | 17% |
| forms | 5% | 5% |

  Counting places 649 strata against DRed's 467, and 204 of its strata are recursive against 92 (2.0 s against
  1.6 s of their CPU). A support relation inside a component makes more of it recursive. Recursive strata are
  40–45% of the strata's CPU here, against 20–26% on dl10 and the Python pack. This is the workload levels are for.

### Where an edit's cross-file part goes

Own CPU of the cross-file part's spans, grouped:

| | dl10 edit, counting | Python import, counting | Python import, DRed |
|---|---|---|---|
| the cross-file part | 1.21 s | 19.26 s | 15.02 s |
| kept rows moved to and from their frames | 30% | 26% | 28% |
| placement, spec and pricing of the strata | 20% | 21% | 20% |
| input deltas and digests | 14% | 13% | 15% |
| forms: written, keyed, read, selected | 9% | 10% | 8% |
| joins, and the jobs that run them | 10% | 25% | 26% |

The joins are a tenth to a quarter. One import edit on 50 files places 3,105 strata under counting (2,855 under DRed),
and writes or reads 1,114 forms (646).

### Shapes

Strata placed on the same edits, by what they hold:

| | dl10 | Python pack |
|---|---|---|
| non-recursive | 839 (0.81 s) | 2,572 (7.11 s) |
| non-recursive, with a text test | 236 (0.08 s) | 197 (0.46 s) |
| non-recursive, with an aggregate | 17 (0.03 s) | 57 (0.14 s) |
| recursive | 84 (0.32 s) | 279 (1.96 s) |

Recursive strata are 7–9% of the strata and 20–26% of their CPU. Each shape needs its own way to maintain:
- joins and negations: DRed or counting;
- aggregates: their changed groups;
- recursion: levels, or deriving the component again;
- text tests: only the new strings.

Two more workloads did not run from the command line on 2026-11-16: `calls.rr` (O-cross-file-answers) and
`workload.rr` (a stratum the native engine declined). Both run now; see "Typing-support's workloads" above.

### What is stored

| | dl10 |
|---|---|
| the 39 programs' rules, as text | 3,416 rules, 481 KB; 3.3 body atoms a rule |
| the program images in the store | 110 images, 10.6 MB raw (four of 1.35–1.64 MB: a pass's output is a whole image) |
| general forms | 252 after one edit, 4.4 MB raw; one is 7.3 KB for a 40-character rule |
| rules counting adds | 952 through support relations (Python: 1,940); general forms 10,379 rules over 655 strata |

A program image is about 3.4× its rules' text. Its header is 976 bytes whatever a layer holds, and a pass's output
is stored whole.

## Decision

1. **A rule is a small record, stored once.** It holds:
   - the head relation, and each body literal as a relation id, a sign and its argument slots;
   - constants in a table of their own, referenced from the slots;
   - comparisons and functions as short ops over slots.

   At 3.3 atoms of about three arguments, a rule is about 30–40 bytes, against about 480 bytes in today's image and
   144 characters as text. Names are atoms of the program block (0266), so a rule holds no text.
2. **A pass's output is metadata over the rule table, not a new program.** The split into file-local and cross-file
   parts, a stratum's components, the parts and the cone of requests are sets of rule ids (bitsets) and small
   tables. A rewrite that does make a new rule (the split's exports, a request's rule) appends its record. No pass
   stores a whole program, and none copies one.
3. **Maintenance is a mode of evaluating a rule.** For a rule and one of its atoms that changed, the evaluator joins
   from that atom's delta: removed rows over the old state, added rows over the new. Removed rows give the rule's
   removal candidates, added rows its gains. A candidate is rechecked by the head's rules probed with the head
   bound, over the new state. None of `__del`, `__ins`, `__redo`, `__minus`, `__plus` or `__ready` is a declared
   relation or a rule, and no count or support is kept.
4. **A rule's delta plans are metadata, made with its program.** For each atom that can change, a join order from
   that atom, with each step's access path: a few bytes a step, about 25 bytes a rule. For dl10's 1,348 cross-file
   rules that is about 35 KB, against 4.4 MB of forms after one edit. A run picks the plans of the atoms that
   changed. It writes, stores, reads, selects and places no rules, so the forms' 8–10% goes, and most of the
   placement and spec (20–21%), because the plans were placed with the program.
5. **A relation's old rows and its delta are one relation, versioned** (0266: a base and its delta, read as one).
   The old rows are the base, the rows now are the base read with its delta, and the delta's kills and adds are
   what was removed and added. The kept rows are read in place, not moved to and from frames (26–30% today).
6. **How a stratum is maintained follows from its kind, and keeps nothing but its relations** (rewritten
   2026-11-17, from the spike below):
   - **a non-recursive stratum:** delta joins and a recheck (3). Nothing kept. A head row with several
     derivations needs no count: the recheck finds the one that remains.
   - **an aggregate:** its changed groups computed again through an index on the group columns.
   - **a recursive stratum:** open for the user: a level column (0090's *levels, checked*), or no state and the
     changed partitions derived again. See "Recursion" below.
   - **any stratum:** derived again when 0090 §4's threshold says an edit reaches too much of it.

   Counting and its support relations go, and so does the strategy byte: what a stratum does follows from whether
   it is recursive or an aggregate, which preparation knows.

## Maintenance from the deltas: the spike (2026-11-17)

The user's question: with immutable blocks and deltas (0266), the last run's state is a block read in place and
this run's change of every relation is its delta. Is anything more needed for maintenance?

`native/crates/blocks/examples/maintain.rs` answers it on the spike's blocks.
- **The evaluator:** a generic rule evaluator over `View`s (base and delta read as one).
- **The program:** five strata over documents' `def`, `imp` and `call` rows:
  - `edge`, a projection: a head row with many derivations;
  - `defined` and `unresolved`, a negation;
  - `resolved`, three atoms with a projection;
  - `reach`, recursive;
  - `tainted`, over the recursive stratum, and `uses`.
- **The checks:** each edit is applied five ways from the same kept block, each into a delta of its own. Every
  result is checked against a derive from scratch, every relation by digest.

### What a run reads and keeps

- **This run's change of a relation** is read off its delta: the rows past the carried ones (its adds), and the
  kills past the carried ones (its removals). That is a mark of two numbers per relation, set when the delta is
  carried: the `{rows, kills}` of the earlier draft, kept.
- **The old state** is the kept block itself, read in place: no `x__old` and no copy.
- **The new state** is the open delta over its base, each stratum frozen before the next reads it.

### A non-recursive stratum: delta joins and a recheck

For each rule and each literal whose relation changed:
- its removed rows (or, for a negated literal, its added ones) are joined over the **old** state: the heads found
  are removal candidates;
- its added rows (or, for a negated literal, its removed ones) are joined over the **new** state: the heads found
  are gains.

A candidate the old block holds, and that is not also a gain, is **rechecked**: the head's rules probed with the
head bound, over the new state. It is killed when no rule derives it. A gain the old block lacks is pushed.

That is all of it. A projection (`edge`, `resolved`) needs no count: its candidates are the heads a removed row took
part in, and the recheck finds a derivation that remains. Negation is the same join with the signs swapped.
Counting's only advantage is to skip the recheck probe, and it pays for that with a count kept on every row, a
freeze that counts instead of deduplicating, and plans that list every binding.

### Recursion

Rows in a cycle can support each other, so a recheck over the new state can find a derivation through a row that
is itself gone. Something beyond the deltas is needed. The spike tried four ways on `reach`:
- **derive again:** the stratum derived into scratch and diffed. No state.
- **DRed:** overdelete from the removed reads over the old state, rederive each overdeleted row once over what is
  left, then insert semi-naive. No state.
- **partitions derived again:** the rules are checked to be partitioned by the first column (every own atom's
  first argument is the head's). Then only the keys a changed read reaches are derived again, each with its key
  bound, and diffed against the old block's rows of those keys. No state.
- ***levels, checked*** (0090): each row's level, kept. Candidates are taken in level order. A candidate survives
  when a rule derives it from surviving rows of lower level, and a killed one makes the rows it supported
  candidates. Killed rows that are still derivable, and the added reads' heads, are inserted semi-naive, each at
  one more than its derivation's highest level.

### Measured (300 documents, instructions retired, profiling build)

The old block has 48,000 `reach` rows, 36,000 `call` rows and 3,681 `edge` rows. Each figure covers an edit's
derived strata. Edit 11 removes every definition of one document.

| edit | derive again (all) | delta joins, reach again | + DRed | + partitions again | + levels | kills (reach + others) |
|---|---|---|---|---|---|---|
| 1 | 4,500 M | 958 M | 13,346 M | 470 M | 1,153 M | 5 |
| 2 | 4,525 M | 967 M | 19,451 M | 800 M | 50 M | 15 |
| 3 | 4,494 M | 964 M | 17,107 M | 651 M | 48 M | 11 |
| 4 (reach unchanged) | 3,548 M | 2.5 M | 2.5 M | 2.5 M | 2.5 M | 2 |
| 5 (reach unchanged) | 3,553 M | 1.6 M | 1.6 M | 1.6 M | 1.6 M | 2 |
| 6 (reach unchanged) | 3,547 M | 4.6 M | 4.6 M | 4.6 M | 4.6 M | 11 |
| 7 | 4,520 M | 963 M | 19,451 M | 797 M | 53 M | 5 |
| 8 | 4,515 M | 962 M | 7,222 M | 253 M | 35 M | 4 |
| 9 (reach unchanged) | 3,558 M | 2.0 M | 2.0 M | 2.0 M | 2.0 M | 2 |
| 10 | 4,515 M | 963 M | 2.0 M | 132 M | 25 M | 6 |
| 11 (big) | 4,554 M | 999 M | 14,836 M | 1,026 M | 544 M | 361 |
| **mean** | 4,166 M | 617 M | 8,311 M | 376 M | 174 M | |

What the work was:
- **Delta joins:** 3–16 rechecks an edit outside `reach`; 121 on the big edit.
- **DRed** overdeletes 11,000–26,000 of the 48,000 `reach` rows to kill 4–15.
- **Partitions:** 44–185 of the 300 keys derived again (the imports cross packages, so a key reaches far).
- **Levels:** 26–194 candidates and 0–2 level writes on most edits. Edit 1 is the bad case: one edge was the
  low-level support of many rows, so 1,592 candidates and 680 rows written again at higher levels.

How to read it:
- The figures are the spike's evaluator: rows as `Vec<i64>`, hash sets of them. The absolute numbers are high;
  the ratios between ways are what the spike measures.
- The levels are a side map, computed outside the timed part. In a block they would be a column of the recursive
  relations, and a run would write only the rows whose level it sets (the level writes above).
- The workload is synthetic, though shaped like the engine's: documents with package-local imports, 5% across
  packages.

### What it shows

1. **Non-recursive strata need nothing but the deltas.** Where `reach` is unchanged, an edit costs 1.6–4.6 M
   against 3,550 M to derive again: about a thousandth. No count, no support, no level, no form. Counting buys
   nothing here that a recheck probe does not.
2. **DRed is the wrong default for recursion on blocks.** On a closure where many paths lead to the same rows,
   overdeleting costs 3–4 times as much as deriving the stratum again. 0090 had already moved recursive strata off
   it; the spike agrees.
3. **For recursion the choice is between a level column and no state:**
   - *Levels* is the cheapest on 7 of the 8 edits that change `reach`: 25–53 M against 132–800 M for
     partitions. Its bad case (edit 1, 1,153 M) costs about as much as deriving `reach` again, which the
     threshold would have chosen. It keeps 1 byte a row in recursive relations only, and it is written once as a
     row is pushed, except when a row is re-leveled.
   - *Partitions derived again* keeps nothing. It is 1.5–7 times cheaper than deriving again, and works only
     where the rules are partitioned by a column, which preparation can check. `reach` and every per-document
     closure are partitioned that way.

   This is an architectural choice, for the user. The recommendation is levels, with derive-again by 0090 §4's
   threshold; partitions as the stateless form where no levels are kept.
4. **Recursive strata are where the cost is**, as on `calls.rr` (40–45% of strata CPU): an edit that changes
   `reach` costs 25–1,000 M, one that does not costs 2–5 M.

### Details that still decide whether this works

- **A row killed and pushed again in one run gets a new id.** Freeze keeps the base's dead row dead, and the
  pushed row lives as a new delta row. The spike avoids it by never killing a candidate that is also a gain, and
  never pushing a row the old block holds.
- **Levels go stale only if a run skips them.** Derive-again writes them as it derives (by its waves). The
  roll-up check (the same rows before and after) should cover the level column too.
- **A rule edit** (0281's per-policy path) changes its head's derivations. That stratum is derived again.
- **On the GPU:**
  - a delta join is a join kernel seeded with the delta's rows;
  - a recheck is the same kernel with the head bound;
  - a level filter is a compare inside it.

  Nothing is a new kind of kernel.
- **The spike's edge join by document key** (`paces.rs`) is a special case of delta joins: the removed and added
  rows of one document, joined.

## Sizes, today against this decision (dl10)

| | Today | This decision |
|---|---|---|
| the rules | 1.35–1.64 MB a stored pass output; 10.6 MB of images | about 140 KB (3,416 rules × 40 B), once |
| what a pass leaves | a whole image | rule-id sets and small tables |
| a reached stratum's maintenance | a form of 8–17 rules: 7.3 KB stored; about 1 ms of placement, spec and pricing (0.246 s over 248) | its rules' delta plans: about 25 B a rule, placed with the program |
| counting | a support relation per rule, kept rows the size of its input | none: a recheck probe instead |
| recursion | supports or levels relations | a level column in recursive relations (1 B a row), or none (open) |
| old rows and deltas | relations `x__old`, `x__minus`, `x__plus` | the relation's base and its delta |

The first and third rows are estimates from the measured counts; the rest follow from them.

## Path

Each step is measured in instructions retired on dl10 and on the Python pack's 50 files, against today's default,
with each step's findings and rows compared with a cold run's.

1. **Delta joins and a recheck** (3, 6) for non-recursive strata, over today's relations, with counting off for
   them: the spike's way first, on dl10 and calls.rr.
2. **The rule record and the rule table** (1, 2) in the program block, beside today's image, with a check that
   both give the same programs; then the passes write metadata instead of images.
3. **Delta plans** (4) for non-recursive rules of joins and negations, and the evaluator's mode (3) that runs them
   over versioned relations (5). Forms then remain only for aggregates, recursion and text tests.
4. **The support relations and counting removed.**
5. **Aggregates by group, text tests, and recursion** (levels or partitions, as the user decides) in the same
   mode; then the forms' code goes (0298's storage with it).

## Open

- Why an edit of `calls.rr` costs 55–110% of a cold run. The kept rows' moves and the placement are about half of
  it, as on the other workloads, but the joins are large too: the recursive strata.
- How recursion is kept: a level column, or partitions derived again (above, "What it shows", point 3).
- The spike's recursive cases are one closure. A recursive stratum with negation or aggregation inside its
  component (stratified per component today) is not measured.
- The kept rows' moves to and from frames (26–30%) are 0266's `Facts` block (its step 4), not this decision's; this
  decision depends on it for (5).
