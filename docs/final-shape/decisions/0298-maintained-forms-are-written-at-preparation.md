# 0298: maintained forms are written once per program, and a run selects from them

Status: decided (2026-11-16); steps 1–3 built behind `RERAIL_GENERAL_FORMS`, step 4 waits on 0281 step 5. Amends the maintained cross part (decision 0117 step 3) and its forms
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

1. **A general form per stratum, once per program and kept part.** Each cross-file stratum's form is written as if
   every read whose old rows are kept had changed: a delta rule for each such read, reading the old rows (`x__old`)
   of the others. "Kept" is what the kept part holds, read or not yet (`Retained::holds_old`: a derived relation
   held or in the frames, an input whose source the last run kept), so the general forms are keyed by the program
   and the signature of that set (`olds_signature`). Per stratum: its form, and per rule the read whose delta drives
   it.
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

## Results (steps 1–3, `RERAIL_GENERAL_FORMS=1`)

dl10, the 39 policies, debug engine, CPU binding, each step a new process, both paths on their own stores. Each
step's per-policy outputs hold the same rows in all 421 relations (decoded with `rerail-engine csv` and sorted;
`outputs.rro`'s row order differs, as in 0290):

| Step | Today (G instructions) | Selected | Strata selected / written |
|---|---|---|---|
| cold | 165.9 | 170.7 | – |
| warm | 2.83 | 2.81 | – |
| one file edited | 78.8 | 92.4 | 241 / 15 |
| warm again | 2.24 | 2.24 | – |
| the edit reverted | 55.8 | 71.7 | 237 / 15 |

The 15 written are recursive strata (their form by levels is chosen from the kept rows). Profiled edit, CPU:

| Span | Today | Selected |
|---|---|---|
| the reached strata's forms (256) | 0.477 s | 0.208 s |
| the general forms, written in the run | – | 1.327 s |
| the cross-file part, in all | 1.678 s | 2.711 s |

So each process now pays 1.33 s to write the general forms, which storing them (step 4) removes; less that, the
part costs 1.384 s against 1.678 s: about 0.29 s saved per edit, as estimated before building. Until step 4, the
switch stays off.

A first version wrote the general forms as if every input and derived relation kept its old rows. It read
`doc__old` where the run's form reads `doc` (whose old rows are not kept; Q20): the run failed, the policy ran
again, and the policies after it lost their cut-off. Binding a relation's rows as its old rows for such reads was
tried and dropped: a form reading a relation's rows is a different program from one reading its old rows, so the
general form now takes as kept exactly what the kept part holds.

## Path

1. `rewrite_over` reads the run's kept part through two questions (old rows kept, levels kept) and returns the old
   rows it reads instead of binding them, so it can write without a run.
2. The general forms written at preparation and held on the prepared program, with each rule's driving read.
3. The driver selects from them (2), with the fallback (3); outputs compared with today's on the edit, revert and
   warm steps.
4. Stored with the prepared program: through 0281 step 5's keyed preparation steps, when Typing-support lands it
   (the user's call, 2026-11-16), keyed by the program and `olds_signature`. Then the switch becomes the default.

## Open

- Recursive strata are written per run: a general form by levels needs the levels the kept rows have.

- Placement and spec of the reached strata (about 0.46 s on the same edit) stay per run. With forms fixed per
  program, what placement decides of a stratum can be kept with them.
