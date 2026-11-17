# 0328: A rule is stored once, small; maintenance is metadata and a mode of the evaluator, not more rules

Status: draft (2026-11-16; on blocks, 2026-11-17), not settled. The user agrees with the direction, and asked
for the details on decision 0266's blocks to be worked out and explored before deciding (below, "On blocks"). It would retire the maintained forms (decisions 0117 step 3, 0241,
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
   from that atom's delta: the atoms before it over the new rows, those after it over the old. Removed rows give
   the rule's losses, added rows its gains. The rederive step (DRed) is the same rule probed with its head bound.
   None of `__del`, `__ins`, `__redo`, `__minus`, `__plus` or `__ready` is a declared relation or a rule.
4. **A rule's delta plans are metadata, made with its program.** For each atom that can change, a join order from
   that atom, with each step's access path: a few bytes a step, about 25 bytes a rule. For dl10's 1,348 cross-file
   rules that is about 35 KB, against 4.4 MB of forms after one edit. A run picks the plans of the atoms that
   changed. It writes, stores, reads, selects and places no rules, so the forms' 8–10% goes, and most of the
   placement and spec (20–21%), because the plans were placed with the program.
5. **A relation's old rows and its delta are one relation, versioned** (0266: a base and its delta, read as one).
   The old rows are the base, the rows now are the base read with its delta, and the delta's kills and adds are
   what was removed and added. The kept rows are read in place, not moved to and from frames (26–30% today).
6. **How a stratum is maintained is a byte of metadata, chosen per stratum by cost:**
   - delete-and-rederive;
   - counting, with a count per row held as a column of the relation;
   - levels, with a round per row held as a column;
   - its changed groups, for an aggregate;
   - derive it again.

   The column exists only where the stratum is maintained that way. No support relations are kept, so `kind` keeps
   no copy of `node`.

   A count is needed only where a head row can have more than one derivation: the rule projects a variable away, or
   several rules make the head. That is a property of the rule, known at preparation.

   Counts and levels are state, kept from run to run, so a stratum cannot change to counting for one edit: its
   counts would have to exist already. Two kinds of choice follow (amended 2026-11-17, "On blocks" below):
   - **What a stratum keeps** (a count column, a level column, or nothing) is chosen with the program, and changes
     only when the stratum is derived anew anyway: a rule of it edited, a derive-again, or a roll-up.
   - **How an edit is applied** is chosen per edit, among what the kept state allows. DRed and derive-again need
     nothing kept, so they are always open. Counting and levels are open where their column is kept. 0090 §4's
     threshold chooses derive-again.

   The import edit above is where it matters: counting saves 30% there, and DRed saves 5–8% on appends and
   comments.

## On blocks (0266): what counting and DRed keep, and where (2026-11-17)

The user asked for this before the design is settled. Decision 0266 puts a phase's cross-file relations in one
`Facts` block, and a maintained run writes a delta of it. Maintenance has to live in those structures, with no
`__del` or `__sup` relations beside them. This section works through what is kept, what is scratch, and what each
strategy does to a delta. It is checked against the spike's code (`native/crates/blocks`), not only 0266's text.

### What the spike's relation already has

`blocks::relation::Relation` holds:
- the rows, in chunks that double;
- `killed`, a log of the row ids a delta kills, of its base or its own;
- `dead`, bits over the base's ids and the delta's;
- the canonical `order`, the hash `indexes` and the statistics, all made at freeze.

A delta's ids continue after its base's (`base_rows`), so no base row is renumbered. `Facts::carry` opens a run's
delta by pushing the last delta's rows again in id order and repeating its dead bits as kills. So every id keeps
its meaning from run to run until a roll-up. A delta is cumulative: it holds every change since its full block,
not only this run's.

### Kept, per relation, in the `Facts` block

| Field | Where | Size | What it is for |
|---|---|---|---|
| `mark: {rows, kills}` | in a delta, set when it is carried | 8 B a relation | Where this run's changes start. A delta's rows past `mark.rows` are this run's adds, and its kills past `mark.kills` are this run's removals. These replace `x__plus` and `x__minus`: ranges, not relations. |
| `levels` | a column, optional (0266 already lists it) | 0–4 B a row, by the column's encoding | *Levels, checked* (0090) for a recursive stratum. Written once, as a row is pushed. A row that survives an edit keeps its level, so a delta never changes a base row's level. |
| `counts` | a column, optional | 0–4 B a row: a column of all 1s is the 16-byte descriptor alone | Counting, where the stratum keeps counts. |
| `recount: {id, count}` | in a delta: a log, and an index made at freeze | 8 B a changed count | A base row's count changed by a delta. The base is frozen, so its column cannot be written. A delta's own rows' counts are written in place, since carry pushed them again. |

The old rows (`x__old`) are not kept apart. A row was live at the run's start when its id is below `mark.rows` and
it was not dead then: either it is not dead now, or it was killed after `mark.kills`. Those kills are few, so
freeze makes a small hash set of them for this test. The "old" view is then the delta itself, filtered, with no
copy.

### Kept in the `Program` block

All of this is versioned with the program (0266 §1), made at preparation, and read in place:
- each rule's record (decision 1);
- for each atom that can change, the rule's delta plan (decision 4);
- for DRed and levels, a rederive plan: the rule probed with its head bound, so the indexes it needs are known
  with the program;
- each stratum's kept state (none, counts or levels), and its relations' flags;
- whether a head row can have more than one derivation (decision 6). A relation that cannot needs no count column
  under counting.

### Scratch, made and dropped within a stratum

These live in the pool's scratch region, as freeze's sort keys already do (0266 §12):
- **Signed change rows** `(tuple, ±1)` for a counted head. A GPU claims them as it claims any rows.
- **Candidate ids** for DRed or levels: rows that may have lost a derivation.
- **The hash set of this run's kills** (above).

None of these is a relation of the program, none is stored, and none is planned as a stratum.

### A maintained stratum, step by step

**When it runs.** A stratum runs when one of its reads has rows past its mark or kills past it. That is two
comparisons per read, and it replaces today's planning of a form for the stratum.

**DRed** (no kept state):
1. Join each changed atom's removed rows over the old view of the others, using the delta plans. The heads found are
   candidates, as ids of the head relation (a probe of its index).
2. Probe each candidate with its rederive plan over the new view. If it still has a derivation, it is dropped from
   the candidates.
3. Push the inserts: each changed atom's added rows joined over the new view.
4. Kill each candidate that remains and was not inserted again.
5. Freeze. It drops a pushed row that is already live, as it does today.

For a non-recursive stratum the candidates are checked against strata already final, so step 2 is one probe each.
The over-deletion O125 measured comes from recursion, which levels handles.

**Counting** (a count column):
1. Join the changed atoms' removed rows (−1) and added rows (+1) over the old and new views, with the standard
   ordering of old and new, into signed change rows.
2. Freeze sums them per tuple instead of dropping duplicates.
3. A tuple the relation holds gets its count changed: a `recount` entry for a base row, the cell itself for a
   delta row.
4. A count that reaches 0 kills the row. A new tuple is pushed with its count.

There is no rederive probe. A count that changes but stays above 0 is no change of the relation: it is past
neither mark, so no stratum after it runs.

**Levels, checked** (a level column), 0090's algorithm on these structures:
- the candidates are sorted by their level column;
- "the live rows below level L" is the old view filtered by `level < L` and not gone: a predicate, not 0090's
  `h__live`;
- a new row's level is written as it is pushed (0090: the highest kept level plus the wave).

**Changed groups**, for an aggregate: the groups the input's added and removed rows touch are found from their
group columns and computed again through an index on those columns. A changed result row is killed and pushed
again.

**Derive again**: as the spike's `diff_into` does today. Derive into scratch, then walk the old and new canonical
orders together, killing and pushing.

### Details that decide whether this works

- **Counting changes freeze, and the cold run.** Freeze today drops duplicates. A counted relation's freeze
  counts them instead, so each distinct binding of the body must be pushed exactly once. A plan that stops at an
  atom's first match cannot be used for a counted stratum. The exception is the last atom, whose probe range's
  length is the count. A rule whose projected-away atoms come last in its plan counts at no extra cost. Any other
  must list every binding, cold and in each edit. This is counting's real cost on blocks, and it is a property of
  the plan, known at preparation.
- **What counting replaces is mostly free.** `kind(n, k) :- node(_, n, k, _, _)` has a count of 1 on every row.
  Its column is a descriptor of 16 bytes, where `kind__sup0` today is a second copy of the node table. A count
  column also adds no relation, where today's supports make more strata recursive: on `calls.rr`, 204 against
  DRed's 92.
- **A row killed and pushed again in one run gets a new id.** Freeze keeps the base's dead row dead, and the pushed
  row lives as a new delta row. That is correct, but it reports the tuple as both removed and added. DRed avoids
  it by checking inserts against the candidates before killing (step 4). For other strategies, freeze can revive
  the old id when the base row it finds is dead by a kill past the mark. The spike's freeze already does that
  binary search; it would need the kill set above.
- **A delta grows with recounts.** The roll-up rule (0266 §6) charges a delta for what it carries, and carrying
  the recount log is part of that. A count that flips often, in a component that rows keep entering and leaving,
  costs 8 B a run. The roll-up applies the log and writes a contiguous column.
- **Carried state goes stale** (0090's pitfall). A stratum that one run does not maintain must leave its counts and
  levels consistent, or drop them. Derive-again writes them as it derives: counts by its freeze, levels by its
  waves. So no path leaves them stale. The roll-up check (the same rows before and after) should cover counts and
  levels too.
- **A rule edit** (0281's per-policy path) changes the derivations of its head. That stratum is derived again,
  which recomputes its counts. Removing the old rule's derivations as a delta would mean running the whole old rule.
- **On the GPU:**
  - signed change rows are claimed and written like any rows;
  - summing them is the sort freeze already does, plus a segmented sum;
  - a base row is found by binary search of its order (`rel_probe`);
  - a candidate probe is a join kernel with its head bound;
  - a level filter is a compare inside it.

  Nothing needs a device-side allocation beyond 0266 §5's reserve step.
- **One more strategy the spike suggests.** The spike maintains its edge join by key: it kills the edges whose
  columns name the edited document, then joins that document's rows again. No count and no overdelete join are
  needed. This holds when every body atom's document column reaches the head. That is a property of the rule, and
  could be a sixth strategy byte where it holds.

### What has to be measured before deciding

The spike is the place to measure, since it has every structure above:
1. Counting against DRed against derive-again on the spike's edge join, and on a projecting rule with a large
   fan-in. This is the case where counting must list every binding.
2. *Levels, checked* against derive-again on its reachability stratum. This is 680 M of the 977 M an edit costs
   at 3,000 docs today.
3. Freeze with counts against freeze with dedup, cold.
4. Delta growth with recounts over the spike's 40-edit runs, and where the roll-up rule fires.
5. How often a tuple is killed and pushed again in one run, with and without reviving its old id.

Each is checked against the from-scratch oracle, as the spike's runs already are.

## Sizes, today against this decision (dl10)

| | Today | This decision |
|---|---|---|
| the rules | 1.35–1.64 MB a stored pass output; 10.6 MB of images | about 140 KB (3,416 rules × 40 B), once |
| what a pass leaves | a whole image | rule-id sets and small tables |
| a reached stratum's maintenance | a form of 8–17 rules: 7.3 KB stored; about 1 ms of placement, spec and pricing (0.246 s over 248) | its rules' delta plans: about 25 B a rule, placed with the program |
| counting | a support relation per rule, kept rows the size of its input | a count column where chosen (4 B a row, or 1 B and an overflow table) |
| old rows and deltas | relations `x__old`, `x__minus`, `x__plus` | the relation's base and its delta |

The first and third rows are estimates from the measured counts; the rest follow from them.

## Path

Each step is measured in instructions retired on dl10 and on the Python pack's 50 files, against today's default,
with each step's findings and rows compared with a cold run's.

1. **The strategy per stratum** (6) over today's mechanisms: DRed or counting chosen per stratum from the
   measured edits, to show the choice is worth having before the mechanisms change.
2. **The rule record and the rule table** (1, 2) in the program block, beside today's image, with a check that
   both give the same programs; then the passes write metadata instead of images.
3. **Delta plans** (4) for non-recursive rules of joins and negations, and the evaluator's mode (3) that runs them
   over versioned relations (5). Forms then remain only for aggregates, recursion and text tests.
4. **Counting as a column** (6), and the support relations removed.
5. **Aggregates by group, text tests, and recursion by levels** in the same mode; then the forms' code goes (0298's
   storage with it).

## Open

- Why an edit of `calls.rr` costs 55–110% of a cold run. The kept rows' moves and the placement are about half of
  it, as on the other workloads, but the joins are large too: the recursive strata.
- The kept rows' moves to and from frames (26–30%) are 0266's `Facts` block (its step 4), not this decision's; this
  decision depends on it for (5).
