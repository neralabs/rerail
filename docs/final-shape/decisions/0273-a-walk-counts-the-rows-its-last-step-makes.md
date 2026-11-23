# 0273. A walk counts the rows its last step makes

Status: built (audited 2026-11-23). Decided (2026-11-15). Answers the leftovers of decisions 0247 and 0248: walks read
0.71 to 0.90 of the real work, with a 20–30% spread. `Walker::work_at` in `plan.rs`.

## Intro

A walk estimates an order's work by carrying a sample of bindings through its steps (0232, 0247). Decisions 0247
and 0248 measured walks against a line's real work on the data-layer files. They read low, 0.71 to 0.90 of the
work, and each walk varied by 20% to 30% (sd) over seeds. Their open items look for the cause in the walker's
sampling: a check's pass share at a key, or a wider walk.

Most of the low reading is not sampling. The walk and the engine count different things:

- **The engine's work** for a line (`profile::plan_work`) is the rows into each step, plus the rows the last step
  makes.
- **A walk's work** was the bindings reaching each step after the lead. It never counted the rows the last step
  makes, and the walker's tests wrote their truths the same way.

For example, `a_walk_past_a_rare_check_starts_from_the_checks_rows` held that its exact walk was
30,000 + 10 + 300 rows. The engine counts the 190 rows its last step makes too.

## How much

On the lines of 100,000 rows of work or more (three steps or more), the rows the last step makes are this share of
the engine's work. Cold runs over 50 files:

| | lines | median | mean | 75th percentile | weighted by work |
|---|---|---|---|---|---|
| data-layer candidates | 152 | 0.10 | 0.15 | 0.22 | 0.12 |
| python pack | 133 | 0.02 | 0.12 | 0.25 | 0.13 |

So a walk that is exact by its own count read about 0.87 of the engine's work. A line whose output is a quarter of
its work read 0.75. That is most of 0.71 to 0.90.

These rows are the same for every order of a line, since every order makes the same rows. Leaving them out kept
the walks' ranking, but made every ratio of two walks further from 1 than the real work's:

- the choice's `WALK_MARGIN`;
- the inline trial's walked proposals;
- `walked_out`.

A proposal could pass the margin on rows that no order can avoid.

## Decision

1. **A walk counts the rows its last step makes** (`Walker::work_at`). Its work is then the engine's count. The
   five walk tests count them in their truths. The ranking test's poor order is now 3.5 times the good one, not
   over 4 times: the rows both make narrow the ratio.
2. **The spread is left as it is.** It matters only through `WALK_MARGIN` (0.625, about two deviations), which
   decides when an order can be taken on its walk alone. The planner bounds the worst case (0271): a line over 8
   times its price is cut, whatever its walks read. A walk's 20–30% decides between orders within a factor of 2,
   which is the average case.

   If it is wanted later, the cheap lever is to average several walks of each order, each under its own seed
   (the walks have this already, `RERAIL_WALK_REPEAT`). Four walks halve the sd. The budget is there: at the runs'
   end the searches spent 17–30 ms of CPU, against budgets of 0.9–1.1 s (0272).

## Measured

Cold, strict, 50 files each:

- Python pack: 63 rows. Data-layer candidates: 75 rows. Both the same as before.
- No walked proposal passed the margin, before or after, in either pack. No decision changed.
- The inline trials' walks of the running order rose by 12–30%:
  - `p68_rr_cover24`: 171,111 to 222,550;
  - `pre_119`: 57,856 to 72,320;
  - `rr_agg57`: 19,448 to 21,879.

## Not done

- **0247/0248's lines are gone.** After 0267, the 50-file data-layer run has no line of 5.66–6.89 M rows. So the
  bias left between their orders (0.73 against 0.90) cannot be measured again there. Measuring it needs those
  lines captured as a fixture.
- **Few searches walk.** Only 4 requests at a time hold their relations to walk (`HOLDING_MAX`). The rest are
  priced, not walked. Of the 64 searches served at each run's end above, at most 4 walked. That limits what walks
  inform at all, more than their spread does.
