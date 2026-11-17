# 0319. A chunk's counted constants are estimates, and the branded ones are counted

Status: decided (2026-11-16), working on the user's "35 second cold run ... too long". Corrects 0203's inputs (a
line's price) and keeps 0048's demand bar as it was tuned. Builds on 0270 (kinds, fields and names as branded terms).

## Intro

The planner prices a probe by a constant (`field($fc, #f65561, $e)`: the rows of one field name) from the program's
constants counted in the chunk's inputs (`corpus::Own::postings`). Two things made those counts wrong on a cold
PostHog run:

- **Branded constants were never counted.** A kind, field or name in a rule is a branded term (0270), and
  `Own::constants_of` collected integer terms only. So no field name of any rule was counted, and `const_freq` read
  each as absent: a measured column with no count for the value is an absence proof, priced at one row.
- **The counts are estimates, read as exact.** They are counted over a sample of 8,192 rows of a mix's first chunk
  and scaled to every later chunk (`own_once`). A value that chunk lacks is not absent from the others.

`pre_115`'s seed (a comprehension's names) started from `field($fc, #f65561, $e)`, priced at one row per chunk and
reading about 400, then fanned out through `within` to 14.5 M rows into three anti-joins, before its kind filter cut
them to 1.7 M. Every chunk ran the plan (2,518 loops). The slow-plan check (0203) flagged it 47 times and measured exact
pair statistics each time (4.5 s), but the line kept its plan.

## Decision

1. `Own::constants_of` counts a branded constant by its atom, as the planner reads it (`Body::constant`:
   `Constant::Int(a.value())`).
2. `Stats::estimated` names the relations whose counted constants are estimates. The chunk loop marks every relation
   with program constants when it applies the mix's counts, and the guest rounds mark theirs. `const_freq` answers
   `None` (unknown) for a value missing from them, not an absence proof: the join pricer then takes one value's share
   (rows over distinct values), as it does for a literal it has no count for.
3. 0048's demand bar (`demand::over::base_rows`) reads such a value as none, as before. Its question is whether a
   scan is large, and a value the sample lacks is rare. The model that answers it, at preparation, has no distinct
   counts for the inputs, so one value's share is the whole relation there. Read that way, `#f65567` (a definition's
   type parameters) made `tp_name` demanded, and `tp_ref`'s split made every identifier with each of its ancestors:
   54 M rows, 17.8 s.

## Measured (M3 Max, PostHog, 29,885 files, cold; the merged hub build, fc1ce77d4 + ours)

| | Before | Counted and estimated (1-2) | And 0048 kept (3) |
|---|---|---|---|
| Join probes' CPU (profiled) | 94.0 s | 103.8 s | 79.4 s |
| `pre_115` | 8.8 s, 14.5 M rows into the anti-joins | 6.5 s | 4.2 s |
| Exact pair statistics | 4.8 s (46 checks) | none | none |
| `walk` | 12.9 s | 11.4 s | 6.4 s |
| Instructions retired (profiled) | 2,845 G | 2,944 G | 2,727 G |
| Findings | 29,747, fa0efeaed6ff | same | same |

Unprofiled cold runs after (3), load 6-9: 29.8-31.3 s wall, 192-198 s user. The M3's load ran 6-44 through these runs,
so the wall times are not comparable across them; instructions and the profile's CPU are.

## Not done

- The demand bar's model has no distinct counts at preparation. Giving it the first chunk's would let it take an
  unknown constant as the pricer does; until then the two read a missing value differently, on purpose (3).
- A line flagged slow re-measures its exact pair statistics at every flag and can keep its plan (47 measurements,
  `replans=0`). With the counts right this line no longer flags; the loop itself is 0203's to bound.
