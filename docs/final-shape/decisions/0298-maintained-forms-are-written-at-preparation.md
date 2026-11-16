# 0298: maintained forms are written once per program, and a run selects from them

Status: decided (2026-11-16), building. Amends the maintained cross part (decision 0117 step 3) and its forms
(decision 0241). Fits decision 0281: it is one more preparation pass, which 0281 step 5 moves onto the pass manager
with the others.

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

1. **A general form per stratum, once per program.** Each cross-file stratum's form is written as if every read whose
   old rows are kept had changed: a delta rule for each such read, reading the old rows (`x__old`) of the others.
   One version holds every stratum's general form, with, per stratum, its rules' range and, per rule, the read whose
   delta drives it.
2. **A run selects, it does not write.** A reached stratum runs the rules of its general form driven by the reads
   that changed, and the rules that every form has (its own deltas, survivors, `redo`, the gates of its pieces): a
   view of the general version (`Placed::new` over a set of rule ids, as the split's parts are), not a written
   program. For a read that did not change, the rules the selection keeps read its old rows, which are its rows.
3. **Today's writer stays as the fallback.** A stratum is written as today when its selection would differ from
   what the narrow form computes: a read changed whose old rows are not kept, a read changed with no delta (an
   earlier stratum was derived again), or a recursive stratum whose kept rows lack the levels its general form
   assumed.
4. **The general forms are stored with the prepared program**, so a new process reads them with it.

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

## Path

1. `rewrite_over` reads the run's kept part through two questions (old rows kept, levels kept) and returns the old
   rows it reads instead of binding them, so it can write without a run.
2. The general forms written at preparation and held on the prepared program, with each rule's driving read.
3. The driver selects from them (2), with the fallback (3); outputs compared with today's on the edit, revert and
   warm steps.
4. Stored with the prepared program.

## Open

- Placement and spec of the reached strata (about 0.46 s on the same edit) stay per run. With forms fixed per
  program, what placement decides of a stratum can be kept with them.
