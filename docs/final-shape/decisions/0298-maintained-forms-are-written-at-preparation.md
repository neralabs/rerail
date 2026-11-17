# 0298: maintained forms are written once per program, and a run selects from them

Status: built (2026-11-16), the default; steps 1–4. Amends the maintained cross part (decision 0117 step 3) and its
forms (decision 0241), and decision 0104 (one read inside the waves, below). Each general form is a keyed step of
decision 0281 §6.

## Intro

A one-file edit reaches some of the cross-file part's strata: those reading a relation the edit changed. For each,
the run writes the stratum's maintained form (`maintain::form`): a small program of delta rules (`h__del`, `h__ins`,
`h__redo`, the deltas `r__minus` and `r__plus`), specialized to the reads that changed. On the data-layer pack over
dl10 (39 policies, a comment appended to one file), 256 strata are reached, and their forms cost 0.45–0.6 s of CPU,
about 1.75 ms each in a debug build, against 0.017 s for all their joins.

A stack sample of the forms shows no part proportional to the program: the time is the writing of 256 small program
images (`write_ir`, `together_ir`, `finish_overlay`, `Maint::written`), whose leaves are mostly a debug build's
checks. So no step is to be made cheaper; the forms are not to be written in a run.

The user weighed three ways (2026-11-16): forms written at preparation, forms stored under a key per stratum (0281
§6's keyed steps, not built), or the work handed to Typing-support. They chose the first: it removes the run's
writing rather than caching it, and a form that no longer changes per run is what later lets its placement be kept
too.

## Decision

1. **A general form per stratum, once per program and kept rows.** Each cross-file stratum's form is written as if
   every read whose old rows are kept had changed: a delta rule for each such read, reading the old rows (`x__old`)
   of the others. "Kept" is what the kept part holds, read or not yet (`Retained::holds_old`: a derived relation
   held or in the frames, an input whose source the last run kept), so a stratum's general form is keyed by the
   program, the stratum, and whether each relation it mentions has kept old rows, with levels or not. Per stratum:
   its form, and per rule the read whose delta drives it. It is written the first time an edit reaches the stratum
   (amended 2026-11-16, below), not for every stratum at once.
2. **A run selects, it does not write.** A reached stratum runs the rules of its general form driven by the reads
   that changed, and the rules that every form has (its own deltas, survivors, `redo`, the gates of its pieces): a
   view of the general version (`Placed::new` over a set of rule ids, as the split's parts are), not a written
   program. For a read that did not change, the rules the selection keeps read its old rows, which are its rows.
3. **Today's writer stays as the fallback.** A stratum is written as today when its selection would differ from
   what the narrow form computes: a read changed whose old rows are not kept, or a read changed with no delta (an
   earlier stratum was derived again). A recursive stratum has two general forms, by levels (decision 0090) and
   not, because whether its kept rows have levels is known only once they are read; the run takes the one its kept
   rows match.
4. **Each general form is a keyed step of the store (0281 §6)**, so a new process reads it instead of writing it:
   one step per stratum, keyed by the part (its version's identity and its atoms' names, `forms_identity`), the
   stratum, and the kept old rows of the relations it mentions. A form is one source of the store, under its key
   (amended 2026-11-16, "compact", below): its record, then its own layers over the part's atoms (the form's, then
   its maintenance's), not a folded image, which would hold every atom of the part; it is opened over the atoms the
   run makes once (`Builder::under`). The record is what a selection reads (each rule's driving read and inputs,
   the form's inputs and outputs, its maintenance), as varints: names as atoms of the form's image, relations as
   its ids, rules as its places. Nothing else is stored: the step's pass and version are in its key, and opening a
   layer checks it. A form holds the kind sets its stratum reads, not the program's every set. A stratum whose form
   the store lacks writes it, and the form is queued for the store's writer.
5. **A reached stratum's form is read inside the waves** (the user's call, 2026-11-16), decision 0104's one
   exception: its key names the kept old rows, which the run knows only as it reaches the stratum, and the strata
   reached are known only then. `profile::read_in_waves` runs the read as work behind the run, which the strict
   mode lets through; it reads alone, and the pins of what was read (one write for the run) and the forms written
   go to the store's writer.

## Validation (before building)

A probe in the edit's run wrote every cross stratum's general form over the run's kept part (`RERAIL_FORM_ALL_PROBE`,
not kept in the tree), dl10, the 39 policies, debug engine, CPU binding:

| Question | Measured |
|---|---|
| Cost of writing every general form | 655 strata, 10,379 rules: 1.0 s of CPU, once per program |
| Strata maintainable as general forms | all 655, when the reads are those whose old rows are kept; 380 not, when every read is assumed kept |
| Today's reached strata covered | all 256: each changed read is among its general form's reads |
| Size on the reached strata | 5,235 rules in general form against 3,908 written today; 562 of their 929 kept reads changed |

Running the whole general form would run 1,327 rules whose delta is empty, each priced, placed and specified: the
selection (2) runs today's rules instead. The reads whose old rows are not kept are mostly cross inputs (`doc` in 158
strata), which the run reads only where it first needs them (final-shape-rr-align's Q20), not inputs the frames
lack.

## Results (steps 1–3, `RERAIL_GENERAL_FORMS=1`)

dl10, the 39 policies, debug engine, CPU binding, each step a new process, both paths on their own stores. Each
step's per-policy outputs hold the same rows in all 421 relations (decoded with `rerail-engine csv` and sorted;
`outputs.rro`'s row order differs, as in 0290):

| Step | Today (G instructions) | Selected | Strata selected / written |
|---|---|---|---|
| cold | 165.9 | 170.7 | – |
| warm | 2.83 | 2.81 | – |
| one file edited | 78.8 | 92.4 | 241 / 15; with recursive strata (below), 256 / 0 |
| warm again | 2.24 | 2.24 | – |
| the edit reverted | 55.8 | 71.7 | 237 / 15; with recursive strata, 252 / 0 |

The 15 written were recursive strata, whose form by levels is chosen from the kept rows. With a general form by
levels beside each (decision 3), all are selected, and the five steps again hold the same rows. Profiled edit
(before that), CPU:

| Span | Today | Selected |
|---|---|---|
| the reached strata's forms (256) | 0.477 s | 0.208 s |
| the general forms, written in the run | – | 1.327 s |
| the cross-file part, in all | 1.678 s | 2.711 s |

So each process now pays 1.33 s to write the general forms, which storing them (step 4) removes; less that, the
part costs 1.384 s against 1.678 s: about 0.29 s saved per edit, as estimated before building. Until step 4, the
switch stays off.

### A selection's own cost (2026-11-16)

Outside the forms, a selection first cost about 0.1 s more per edit than a written form. Three causes, each fixed:

- **The part was made by two passes** (retain, then the part's fields), and a pipeline records each step's delta,
  comparing every declaration and fact of its input and output: the size of the program, per selection. Now one
  pass (`SelectPart`: one overlay that removes rules and states the inputs and outputs), and the pass manager's
  `Delta::of` compares no field an overlay of the input does not state (its own layer has no declaration, fact or
  `.about` row). 0.068 s → 0.015 s over 252 selections.
- **The selection read the general form's fields and every selected rule's body** to find its inputs. A general
  form now keeps its inputs, outputs and each rule's inputs (`General::rule_inputs`), written with it.
- **The choice to derive again (0090 §4) read the general form's maintenance**, whose groups list every read's
  delta: each delta of a read that did not move was looked up in SQLite (none is bound) and priced at a default
  size. A selection's maintenance lists the moved reads' deltas alone, as the form written for them does.
  `again_wanted`: 104 samples → as today's.

Profiled edit, CPU, after these and with the recursive strata selected:

| Span | Today | Selected |
|---|---|---|
| the reached strata's forms (252) | 0.517 s | 0.069 s |
| selecting (the part's pass) | – | 0.015 s |
| placement (levels and strata) | – | about +0.03 s, varying between runs |
| the general forms, written in the run | – | 2.07 s (with the recursive strata's second forms) |
| the cross-file part, less the general forms | 2.194 s | 1.839 s |

So, once the general forms are stored, about 0.36 s saved per edit. The five steps hold the same rows.

A first version wrote the general forms as if every input and derived relation kept its old rows. It read
`doc__old` where the run's form reads `doc` (whose old rows are not kept; Q20): the run failed, the policy ran
again, and the policies after it lost their cut-off. Binding a relation's rows as its old rows for such reads was
tried and dropped: a form reading a relation's rows is a different program from one reading its old rows, so the
general form now takes as kept exactly what the kept part holds.

## Results (step 4, stored, the default)

dl10, the 39 policies, debug engine, CPU binding, each step a new process, both paths on their own stores warmed on
the same build (a build's id is in every key), then the same comment appended to the same file on each:

| Edit | Today (G instructions) | Selected, forms stored |
|---|---|---|
| 1 | 79.58 | 77.73 |
| 2 | 79.31 | 77.70 |
| 3 | 79.55 | 77.40 |

Profiled (a fourth edit, CPU): the reached strata's forms 0.371 s → 0.054 s, plus 0.082 s to find and open the 253
stored forms; the cross-file part's spans 1.53 s → 1.26 s.

What the store costs: a cold run writes every reached stratum's form (659; 246.5 G against 243.7 G), and the first
edit after a cold run misses, because the kept part then holds old rows it did not (the relations the cold run
derived but kept no old rows of: 84.8 G against 79.3 G). From the second edit, 253 of 257 forms are read.

A first version stored a folded image per form. Each held the part's whole atom table: opening 253 cost 0.29 s and
591 MB of allocation, as much as writing the forms. As layers over the part's atoms: 0.063 s, 38 MB. A form's pins
were a write each (two per form); they are one, behind the run.

Each step's rows are compared with a cold run on a fresh store over the same files (below).

### Compact (2026-11-16)

The user asked for the stored forms to be as small as they can be. As first stored, a form was two sources (its
layers, and a facts record of 0281 §6's step), each with an alias and a pin, and the record held the step's key as
hex three times, its layers' digest as hex, and the selection's row as JSON: 677 B per form beside its layers. And
each form held every kind set of the program (`kindset_kind`'s rows, 57% of a layer's bytes).

| Per form (cold run, 39 policies) | First stored | Compact |
|---|---|---|
| Layers | 5.5 KB | 2.4 KB (the stratum's kind sets alone) |
| Record | 677 B: facts, hex and JSON | 94 B: varints |
| Sources, aliases, pins | 2 each | 1 each |

0281 §6's facts format stays as Typing-support's for their steps; a general form keeps its own (`StoredForms`).
With the kind sets gone, a one-file edit on a warmed store costs 49.9–51.0 G (77.4–77.7 G before), a cold run 261 G
on the same build. The five steps again hold the oracle's rows.

What is left of a layer is the program image's (0173): its header is 976 B whatever the layer holds (16 + 24 × 40
regions), 18% of the stored bytes, and the string buckets 10%; the rest is the rules' node trees (about 256 nodes a
form). Decision 0266's `Program` block replaces both.

## Path

1. `rewrite_over` reads the run's kept part through two questions (old rows kept, levels kept) and returns the old
   rows it reads instead of binding them, so it can write without a run.
2. The general forms, each with its rules' driving reads.
3. The driver selects from them (2), with the fallback (3); outputs compared with today's on the edit, revert and
   warm steps.
4. Stored through 0281 §6's keyed steps (the user's call, 2026-11-16, to wait for them): one step per stratum,
   written when first reached (the user's call, 2026-11-16, over one image of every stratum's form per part, which
   would have cost the first edit after each change of the rules about 2 s), read inside the waves (decision 5).
   Done; `RERAIL_GENERAL_FORMS` removed.

## Open


- Placement and spec of the reached strata (about 0.46 s on the same edit) stay per run. With forms fixed per
  program, what placement decides of a stratum can be kept with them.
