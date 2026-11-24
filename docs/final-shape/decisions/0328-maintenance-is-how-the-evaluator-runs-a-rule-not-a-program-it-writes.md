# 0328: A rule is stored once, small; maintenance is metadata and a mode of the evaluator, not more rules

Status: partly built (audited 2026-11-23). Left: the program image as a pass's output format (0173) is not retired;
record's own status still says nothing built. Draft (2026-11-16; on blocks and spiked, 2026-11-17), not settled. The
user agrees with the direction, asked for the details on decision 0266's blocks to be explored before deciding, and
then asked whether blocks and deltas make counting and DRed's machinery unnecessary. A spike says they do for every
stratum but a recursive one (below, "Maintenance from the deltas"). How recursion is kept is open for the user. It
would retire the maintained forms (decisions 0117 step 3, 0241, 0298), the support relations (`corpus/supports.rs`,
0090's counting), and the program image as the format of a pass's output (0173, as 0266 step 3 also plans). Nothing is
built.

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
- Q109, theirs, which lets `calls.rr` read a node's text from the command line;
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
   - **a recursive stratum: open again (2026-11-17, "The light pass" below).** Yes/no/maybe with one probe and
     nothing kept is **not sound**: a cycle that loses its only support can hold itself up through rows that never
     become candidates, and on `calls.rr`'s 14-file edit it kept 33 unfounded rows under one probe order of three.
     A sound "yes" needs a witness that cannot lean on the row itself: a level column (0090's *levels, checked*;
     one probe a candidate, the cheapest in the spike), or a proof from below with nothing kept (about DRed's work
     on `calls.rr`), or partitions derived again. **Decided by 0364: levels**, kept with the rows, never
     re-levelled on adds; none for a component the static check proves cannot cycle.
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

   That was the choice on a dense closure. On real edits of `calls.rr`, yes/no/maybe from the deltas is exact and
   keeps nothing ("Recursion in real edits" below), so the level column is a fallback, not the default.
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

## Recursion in real edits: what it has to handle (2026-11-17)

The spike's `reach` is a dense closure, synthetic. The question the user asked next: what problem do DRed and
levels solve here, what depends on it, and would something weaker do? It is answered on `calls.rr` over the O125
replay (77 files, 8 real commits), from the engine's own derivations
(`scripts/profiling/o125/recursion/`, its README).

### The problem, and who sees it

Adding rows is exact by joining from them. Only removal is hard, and only in a recursive component: rows on a cycle
of the derivation graph can support each other after their real support is gone. A recheck then finds that circular
derivation and keeps them. A row kept so is *unfounded*: a cold run would not derive it.

What reads `calls.rr`'s recursive relations (`pv`, `bv`, `fv`, `reach_sync`, `open_sync`, …):
- **positively:** `call_target`, `calls`, `chain_step`, `finding`. A stale row gives a false positive, for
  example ASYNC_BLOCKING on a call that no longer reaches `time.sleep`.
- **under negation:** `async_open` (`!reach_sync(g)`), `mro_dropped`. A stale row hides a row that should be
  there.
- **as a lattice:** `workload.rr`'s `cv` is pinned at `top`.

Errors go one way only, toward rows that should be gone. They persist, and pile up until a cold run.

### Measured

`calls.rr` has 13 recursive components. The biggest is the points-to component: 41 relations, 123 rules. A state
has 283,000–304,000 recursive rows from 304,000–330,000 derivations.

**On a cycle of the derivation graph: 656–664 rows, 0.23%.** They are in four places:
- class openness (`attr_open`, `base_open`): about 370;
- module re-exports (`tbv`, `tqv`): 222;
- class membership (`acls`, `ainst`): 60–63;
- `open_sync` over recursive calls: 4–6.

| Edit | Removed | Removed on an old cycle | Recheck alone keeps wrongly | DRed overdeletes | Yes/no/maybe: candidates | Maybe | Settled back | Rounds | Errors |
|---|---|---|---|---|---|---|---|---|---|
| 00→01 | 690 | 0 | 0 | 962 | 842 | 112 | 0 | 1 | 0 |
| 01→02 | 582 | 0 | 0 | 3,817 | 601 | 28 | 1 | 1 | 0 |
| 02→03 | 1,757 | 2 | 0 | 6,149 | 2,037 | 626 | 3 | 1 | 0 |
| 03→04 | 135 | 0 | 0 | 167 | 148 | 39 | 0 | 1 | 0 |
| 04→05 (14 files) | 4,980 | 18 | **33** (`pv` 28, `bv` 3, `fret` 2) | 11,592 | 5,372 | 1,685 | 21 | 8 | 0 |
| 05→06 | 72 | 0 | 0 | 3,609 | 84 | 15 | 0 | 1 | 0 |
| 06→07 | 289 | 0 | 0 | 6,660 | 391 | 73 | 0 | 1 | 0 |
| 07→08 | 2,452 | 2 | 0 | 8,597 | 2,604 | 686 | 2 | 1 | 0 |

What each column is:
- **Recheck alone:** a candidate is killed when no derivation over the rows still present holds, and the rows it
  supported become candidates. No overdelete, nothing kept. This is the greatest fixpoint, so it can keep a cycle.
- **Yes/no/maybe** (from the deltas, nothing kept):
  - the candidates are the old rows that lost a derivation;
  - a candidate with a derivation (in the new state) over rows that are not candidates is **yes**, and its
    dependents are not touched;
  - one that is not is **maybe** (a derivation only through other candidates) or **no** (none), and its
    dependents become candidates;
  - then every candidate is settled together, as the least fixpoint from the rows outside them. A row that does
    not come back is removed, and its dependents become candidates, until none is added.

  It agreed exactly with the next state on all eight edits in that run. That was the probe order's luck: under
  another order the 14-file edit keeps 33 unfounded rows ("A correction" below).

### What it shows

1. **Recheck alone was exact on 7 of 8 edits.** The eighth (14 files) left 33 unfounded rows of `pv`, `bv` and
   `fret`: 0.7% of its removals, and points-to rows, which `call_target` and the findings read. Recheck alone is not
   correct, but it is a good estimate of where the work is.
2. **DRed is the wrong default.** It overdeletes 1.1–50 times the rows that are really removed (3,609 for 72), and
   rederives almost all of them.
3. **Yes/no/maybe's work follows the change** (but one probe is not sound, "A correction" below). Its candidates are 1.03–1.17 times
   the rows really removed. Seven edits settle in one round, the eighth in 8. Each candidate costs one probe with
   its head bound, plus the settling fixpoint over the open ones. It is the published Backward/Forward idea
   (Motik et al.): look for another derivation before propagating a deletion.
4. **Counts are not needed here.** Levels are, or another well-founded witness: see "A correction" below.

### Is an approximation better?

The user asked about storing yes, no or maybe per row for deletion. The data say no, for two reasons:
- **Maybe is almost always "gone".** Of 1,685 maybes in the large edit, 21 came back. Elsewhere, 0–3 of up to
  686. So keeping a maybe row would leave the wrong answer almost every time. Treating it as gone would be wrong for
  0–21 rows, missing rows that a later pass restores.
- **Settling costs little:** one fixpoint over the open candidates, in 1–8 rounds. It is cheaper than whatever
  would have to catch the approximation's errors later. Nothing stored would stay right either: a cycle can form
  in the new state between rows that were never on one (the 33 rows above were on no old cycle).

So the exact form is the cheap one. Stored state would add something only for a dense closure like the spike's
`reach`, where most candidates are maybes and levels order them. `calls.rr`'s recursion is not dense.

### What this changes

- ~~Decision 6's recursive case: yes/no/maybe from the deltas, nothing kept.~~ Reopened by "A correction" below.
- This is a measure of rows, candidates and probes, not of instructions. Instructions come when the evaluator's
  mode (decision 3) runs it.

## The light pass, and a correction: one probe is not sound (2026-11-17)

The user asked for a light pass over the other places (`workload.rr`'s lattice components, dl10, the packs), and
how a dense closure like the spike's `reach` would be handled if real data had one. The same tools measured it
(`scripts/profiling/o125/recursion/`, README): `prov.py` now reads core.rr's declarations, keeps built-ins and
aggregates as filters, and leaves lattice components out; `dump.sh` takes a tree per state (`STATES`) and runs
`logic` without a policy's scope (`LOGIC=1`); `rec.py` takes a stratum the engine did not run again from the last
step that dumped it.

### The other places

**`workload.rr`** (the O125 replay, 8 real edits): the lattice `cv` is **file-local**. The split puts it in each
document's part, which an edit derives again for the edited files, so it needs no cross-file maintenance. The
cross-file recursion is `returns` + `type_of` (7 rules): 32,000–34,000 rows, 30 on a cycle.

| Edit | Removed | Candidates | On an old cycle | Recheck alone wrong | DRed overdeletes | Errors, one probe | Errors, proof |
|---|---|---|---|---|---|---|---|
| 00→01 | 83 | 84 | 0 | 0 | 84 | 0 | 0 |
| 01→02 | 143 | 143 | 0 | 0 | 143 | 0 | 0 |
| 02→03 | 146 | 148 | 0 | 0 | 148 | 0 | 0 |
| 03→04 | 12 | 12 | 0 | 0 | 12 | 0 | 0 |
| 04→05 | 674 | 683 | 0 | 0 | 707 | 0 | 0 |
| 05→06 | 10 | 11 | 0 | 0 | 24 | 0 | 0 |
| 06→07 | 45 | 45 | 0 | 0 | 45 | 0 | 0 |
| 07→08 | 282 | 299 | 0 | 0 | 299 | 0 | 0 |

**dl10** (10 files of the data-layer repository, which has no history here: three edits of
`plan_composition.py`: cut to its first two thirds, two classes renamed, restored). The four data-layer policies
read the same library components: 27–29 cross-file recursive ones, 44,000–68,000 rows, 25–27 of them on a cycle,
all in `name_reach`. Per edit (`conditional-architecture`; the others alike):

| Edit | Removed | Candidates | On an old cycle | Recheck alone wrong | DRed overdeletes | Errors, one probe | Errors, proof |
|---|---|---|---|---|---|---|---|
| cut | 9,244 | 9,244 | 2 | 2 (`name_reach`) | 9,244 | 0 | 0 |
| rename | 3,468 | 3,472 | 1 | 1 | 3,472 | 0 | 0 |
| restore | 3,468 | 3,472 | 1 | 1 | 3,472 | 0 | 0 |

**The Python pack** (`B-now`, 50 files): its cross part has 64 recursive components. The largest are `truth_of`
(23 rules), `negate` + `number`, `p224_allows` (17), `literal` (17) and `imm_prefix` + `immutable_ann` (13). Most
recurse over the tree: children, parents, wrappers, type applications. ~~Every one does.~~ Corrected below ("Levels
on real data"): the library's module and class graphs (`reaches`, `exports`, `subclass`, `name_reach`,
`pkg_above`) are in it too, and those can cycle. Getting the split at all fixed two engine bugs (`on_version::base_locations`
on an undeclared axis; the version's guest hiding of negated kind sets, `preparation/hide.rs`).

### A correction: one probe is not sound

Yes/no/maybe as written above trusts a row that is not a candidate. A cycle can hold itself up that way. Take a
ring `r1 → … → r6 → r1`, entered by one edge `e → r1`, with `x → e` before it, and remove the edge into the ring.
The two candidates, `reach(e, r1)` and `reach(x, r1)`, each have a derivation through `reach(e, r6)` or
`reach(x, r6)`, which are not candidates, so both are "yes". Their dependents never become candidates, and all 12
rows `reach(e, ri)` and `reach(x, ri)`, which should go, stay (`ynm_reach.py tmp/ynm-ring`).

It happens on real data. On `calls.rr`'s 14-file edit, 25–26 candidates are on an old cycle. Which of them are
"yes" depends on the order of the probes:

| Probe order (`PYTHONHASHSEED`) | Candidates | Rounds | Wrong rows kept |
|---|---|---|---|
| 1 | 5,411 | 3 | 33 (`pv`, `bv`, `fret`) |
| 2 | 5,371 | 6 | 0 |
| 3 | 5,370 | 8 | 0 |

The other sets were exact because they have few rows on a cycle (`workload.rr` none among its candidates; dl10 1–2;
the pack none possible). That is the data's luck, not a property of the method.

**A sound "yes" needs a witness that cannot lean on the row itself.** Three kinds, measured:
- **a proof from below, nothing kept** (Backward/Forward's backward phase): a candidate stays when its cone, the
  rows its derivations read, transitively, derives it again from derivations with no row of its own component.
  Proved and refuted rows are kept for the rest of the edit. `rec.py`'s `prove`: exact on every edit, every set,
  every probe order.
- **levels** (0090's *levels, checked*): a "yes" only through rows of a lower level. One probe a candidate. The
  level is a column of the recursive relation; a derive gives it free (the round a row is first derived in).
- **partitions derived again** (the spike's `delta+part`): sound where preparation proves the rules partitioned by a
  column.

Rows touched an edit, on `calls.rr`:

| Edit | Removed | Candidates (one probe, unsound) | Proof: rows proved or refuted | DRed overdeletes |
|---|---|---|---|---|
| 00→01 | 690 | 842 | 1,593 | 962 |
| 01→02 | 582 | 600 | 1,662 | 3,817 |
| 02→03 | 1,757 | 2,037 | 6,139 | 6,149 |
| 03→04 | 135 | 148 | 167 | 167 |
| 04→05 | 4,980 | 5,364 | 12,141 | 11,592 |
| 05→06 | 72 | 84 | 1,124 | 3,609 |
| 06→07 | 289 | 391 | 5,363 | 6,660 |
| 07→08 | 2,452 | 2,604 | 8,924 | 8,597 |

The proof with nothing kept touches about what DRed does (0.3–1.7 times DRed's rows, about 1 on four edits): to trust a row
it has to prove the rows under it, and nothing says which of them are safe. Levels say it in one comparison, so
their work is the candidates.

### The synthetic reach, if real data had it

The spike's `reach` (300 documents, 3,681 edges, 48,000 rows), its edges dumped per edit (`maintain`,
`MAINTAIN_EDGES`) and counted the same way (`ynm_reach.py`):

| Edit | Edges removed | `reach` removed | One probe: candidates (errors) | Proof: rows proved or refuted | DRed overdeletes | Levels (spike): candidates, level writes |
|---|---|---|---|---|---|---|
| 1 | 1 | 0 | 80 (0) | 5,600 | 17,600 | 1,592, 680 |
| 2 | 2 | 0 | 160 (0) | 14,400 | 25,600 | 194, 2 |
| 3 | 1 | 0 | 120 (0) | 8,400 | 24,000 | 154, 2 |
| 7 | 1 | 0 | 160 (0) | 14,400 | 25,600 | 188, 2 |
| 8 | 1 | 0 | 40 (0) | 1,200 | 11,200 | 58, 1 |
| 9 (a module deleted) | 9 | 140 | 1,820 (0) | 11,200 | 25,200 | 2,036, 0 |

Edits 4–6 change no edge. (The spike counts levels' candidates over every relation, the others over `reach`.)

How it would go:
1. **A dense closure is the easy case for the probe.** Every lost path has many others, so every candidate is
   "yes" at once and nothing propagates. The one probe was right here, but only because no candidate's other
   path ran back through itself; nothing checked that.
2. **A sound probe on it costs what the witness costs.** With levels, a "yes" is a comparison, and the work is the
   candidates (58–2,036 an edit; edit 1, levels' bad case, rewrites 680 levels). With nothing kept, the proof
   re-derives each candidate's source's row of `reach` (1,200–14,400 rows), and DRed overdeletes half the relation.
   Partitions are the proof by key (spike: 44–185 keys of 300).
3. **The thin cycle is the hard case** (the ring, `calls.rr`'s points-to): few rows, but exactly where one probe
   fails. Levels and the proof both get it right. Neither needs to know the closure is dense.
4. So density does not need detecting. The witness makes every case sound, and 0090 §4's threshold still sends an
   edit that reaches too much of a stratum to deriving it again.

### What this changes

- Decision 6's recursive case is open again: one probe with nothing kept is out. The choice is between a level
  column (one probe a candidate; a column of the recursive relations, written by every derive) and a proof with
  nothing kept (about DRed's rows on `calls.rr`). Partitions remain where the rules are partitioned. The user
  decides (they kept levels against partitions aside, 2026-11-17).
- The engine fixes found on the way are committed with this section.

## Levels on real data (2026-11-17)

The open points of the section above, measured in one hour from the dumps already taken. The tools are in
`scripts/profiling/o125/recursion/`:
- `levels.py`: 0090's *levels, checked*;
- `rec.py` and `ynm_reach.py`: run it beside one probe, the proof and DRed;
- `wf.py`: the static check.

Levels here:
- A row's level is the round in which it was first derived.
- Candidates are taken in level order. One stays when a derivation reads only rows of lower levels that are
  still present.
- One that does not goes for now, and its dependents become candidates.
- The rows gone and the rows added are then levelled from the rows present. A gone row derived again comes back
  at a new level, which counts as a rewrite.
- *Fresh*: minimal levels from the old state. *Carried*: levels kept from the last edit and never lowered.

### Levels are exact everywhere, at about one probe's cost

**`calls.rr`** (8 real edits). One probe is shown under seed 1, the order that breaks it.

| Edit | Removed | One probe: candidates, errors | Levels: candidates, rewrites, errors | Proof: rows | DRed |
|---|---|---|---|---|---|
| 00→01 | 690 | 842, 0 | 897, 137, 0 | 1,593 | 962 |
| 01→02 | 582 | 601, 0 | 602, 3, 0 | 1,662 | 3,817 |
| 02→03 | 1,757 | 2,037, 0 | 2,155, 234, 0 | 6,139 | 6,149 |
| 03→04 | 135 | 148, 0 | 156, 9, 0 | 167 | 167 |
| 04→05 | 4,980 | 5,411, **33** | 5,651, 378, 0 | 12,141 | 11,592 |
| 05→06 | 72 | 84, 0 | 93, 8, 0 | 1,124 | 3,609 |
| 06→07 | 289 | 391, 0 | 397, 84, 0 | 5,363 | 6,660 |
| 07→08 | 2,452 | 2,604, 0 | 2,617, 63, 0 | 8,924 | 8,597 |

**The data layer's real history** (`l2060_query_builder`, 37 files, the last commits that touch it,
`conditional-architecture`). A state has 477,000–480,000 recursive rows, 728–730 of them on a cycle
(`name_reach`).

| Commit | Removed | One probe: candidates (errors, seeds 1 and 2) | Levels: candidates, rewrites, errors | Proof: rows | DRed | Candidates on a cycle, kept |
|---|---|---|---|---|---|---|
| ca984ce8 (9 files) | 7,860 | 8,866 (0, 0) | 8,866, 0, 0 | 8,883 | 8,895 | 0, 0 |
| 3471aab1 (6 files) | 2,778 | 3,500 (0, 0) | 3,500, 4, 0 | 3,513 | 3,585 | 1, 0 |
| 49832032 (3 files) | 7,027 | 8,843 (0, 0) | 8,843, 34, 0 | 9,029 | 8,988 | 19, 17 |

The last commit is where one probe can fail: 17 candidates on a cycle stay. It held under both seeds. The 85-file
version of this ran over the 2-minute cap, and its first state's chunked run dumped 210 `dotted_expr` rows that
no derivation holds (`REC_LAX`; an open item below).

**`workload.rr`** (8 real edits): levels' candidates 12–707, 0–33 rewrites, 0 errors. **dl10** (3 made-up
edits) and **the pack's `ruff-pyi001`** over the same trees (36 recursive components, 88,000 rows): levels'
candidates equal one probe's, 0 rewrites, 0 errors.

**The ring:** 12 candidates, 12 gone, 0 errors. **The dense reach:** levels' candidates are 54–1,820 and
rewrites 0–2, except edit 1, levels' bad case (1,487 candidates, 680 rewrites), as in the spike. Errors 0.

### Levels on added rows: keep them, never re-level

The spike's edits only remove edges, and on real edits carried levels barely move (mean drift 0–0.03). So a
sequence of edits on the dense graph that also adds shortcuts, 5 edges added and 5 removed in each
(`tmp/ynm-drift`):

| 15 edits | Candidates | Rewrites | Errors | Mean drift after 15 edits |
|---|---|---|---|---|
| Minimal levels, recomputed every edit | 33,575 | 1,950 | 0 | 0 |
| Levels carried, never lowered | 28,564 | 1,035 | 0 | +3.35 |

Carried levels drift, but they cost less: 15% fewer candidates and 47% fewer rewrites. They were worse on two
edits (5,425 against 4,187; 2,171 against 1,991). A level only has to be a witness, and a higher one seems to depend on fewer rows that
change. **So: never re-level on added rows.** A stratum derived again by 0090 §4's threshold gets fresh levels
for free.

### The static check: what could skip levels

`wf.py` proves a component cannot hold a row up by itself. It needs a node column per relation such that every
recursive step goes the same way down (or up) the tree. Same-node steps are allowed if they have no cycle among
them, and so is an index moved by `@add`/`@sub`. Tree helpers (`unparen`, `attr_of`, …) are summarized from their
own rules. It is conservative.

| Program | Recursive components | Proved well-founded |
|---|---|---|
| The Python pack (317 policies, components counted once) | 57 | 35 |
| A data-layer policy (`conditional-architecture`) | 27 | 18 |
| `calls.rr` | 13 | 5 |
| `workload.rr` | 5 | 3 |

What it does not prove:
- **graphs that can cycle:** module and class graphs (`reaches`, `exports`, `subclass`, `name_reach`, `loads`),
  calls (`reach_sync`), points-to;
- **string chains:** `pkg_above`, `py_minor`, a package path made shorter each step, so well-founded by length,
  which the check does not know;
- **tree recursion through steps it cannot read:** `literal`, `list_item`, `union_down`, `same_pair`.

The tree components it proves are mostly file-local anyway. The cross-file components that matter are the graphs,
and they need levels.

### Levels or partitions

A component is partitioned when every recursive atom reads the head's first column there. That holds for 3 of
`calls.rr`'s 13 components, 7 of the data layer's 27, 7 of `ruff-pyi001`'s 36 and none of `workload.rr`'s.
Within those components, the rows derived again for the keys an edit reaches, against levels' candidates there:

| | Partitions: rows derived again | Levels: candidates |
|---|---|---|
| `calls.rr`, 8 edits | 54–2,407 | 20–1,315 (1.0–4.7 times fewer) |
| Data-layer history, 3 commits | 482–1,332 | 473–1,181 (about the same) |

Partitions cover a minority of components, and cost the same as levels or more. Levels cover all of them.

### Rewrites, and the blocks

Rewrites per edit:
- `calls.rr`: 3–378 (0.001–0.13% of its rows);
- the data layer: 0–34;
- `workload.rr`: 0–33;
- the dense closure: up to 680.

That is few enough for a rewrite to be the relation's kill and add in the edit's delta (0266). There is no need
for an in-place update of the column.

### Cross-file recursive lattices

A scan of 2,046 programs found two recursive components with a lattice:
- every rule set under `docs/rule-syntax` (expanded);
- the 317 pack policies;
- the engine's own programs.

The two:
- **`workload.rr`'s `cv`:** file-local.
- **`hierarchy` (a `shortlex_min` route):** only in `internal_dead_code.hierarchy_unwidened.rr`, the variant the
  engine refuses as an unbounded lattice (open items, the least-route item).

No running program has a cross-file recursive lattice. Closing that item with a `min` lattice would make one, and
it would then need its own handling.

### Still open

- **Instructions.** Every figure here is rows, candidates and probes. Instructions come with the evaluator's mode
  (decision 3).
- **The chunked dump.** A chunked first run dumped 210 `dotted_expr` rows that no `drv` rule derives. It is a
  file-local relation, and the rows measured are the derivations' heads, but the cause is not known.
- **The decision** was taken as 0364: levels, carried and never re-levelled on added rows, for every component the
  static check does not prove; rewrites as kills and adds. 0364 holds the full data and method.

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
- Yes/no/maybe measured on `workload.rr`'s lattice components and on dl10 and the packs' recursion; then in
  instructions, in the evaluator's mode.
- The spike's recursive cases are one closure. A recursive stratum with negation or aggregation inside its
  component (stratified per component today) is not measured.
- The kept rows' moves to and from frames (26–30%) are 0266's `Facts` block (its step 4), not this decision's; this
  decision depends on it for (5).
