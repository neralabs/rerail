# 0236: Walks size a trial's sample and choose the orders tried; a walk's probe reads at most its limit

Status: built (2026-11-14). Follows decision 0233 (two-phase walks): in the trials in `plan.rs`, and
`InRel::probe_sample` in `setfix.rs`.

## Intro

Decision 0233 scaled each trial by two walks. Its open item: on the 50 data-layer files every trial was cut, so
nothing was walked. Measured there:

- **The sample did not fit the budget.** A trial's sample was sized at the line's mean work per row, and a quarter
  of it was drawn from rows of heavy keys. Those rows cost far more than the mean, so every order, the running one
  included, passed its half of the budget. No order finished, and none could win.
- **With the sample fixed, the poor orders cost as much as the line.** Each candidate ran until it was cut, at twice
  the least work measured. Four cut orders made the trials 4.19 M rows of work, about as much as the line's own,
  which breaks the rule that planning costs no more than running (decision 0122).
- **A poor order's walk read every candidate of a probe.** An order probing `kind(n, "call")` by its constant alone
  read every call, to keep 1,024 of them.

## Decision

### 1. Each part's sample fits its budget

The running order's first-phase walks (decision 0233) give its work per row in each part. They are put in the
units of the line's work so far: the line's work per row done, over the walks' mean. Each part then takes the rows
that fit half its budget, at least 2 of the heavy part's and 1 of the rest, and at most `trial_rows`.

### 2. Walks choose the orders tried

Every candidate order is walked over the first phase's rows, part by part. That is a few thousand probes each,
fixed by the sample. Its walked work over the line is the part's rows times its walk, summed over the parts.

- An order whose walked work is more than `trial_cutoff` (2) times the least walked is not tried.
- The running order is always tried: the others' trials lead from its cut.
- The rest are tried in the order of their walked work, so the least budget is set early.
- Where the walks leave only the running order, no trial runs: it would decide nothing.

The log gives each order's walked work, and marks the orders not tried.

### 3. A probe reads at most its limit (`InRel::probe_sample`)

Some probes have more candidates than the walk's reservoir (its width, or 16 times that before checks). Such a
probe draws that many of them by position, with replacement. It estimates its matches as the candidates times the
share drawn that match. The candidates come from the relation's own index:

- a whole relation;
- a run index's runs (`runs::Found::nth`);
- a source's sorted rows;
- a hash index's ids.

A source's other lookups are read whole, as before.

A probe with fewer candidates than the limit reads them all, and is exact, as before. Rows are drawn by position
in proportion to their probe's estimated matches. The reservoir then mixes bindings from sampled and exact probes
equally, which is a small bias in the next step's sample. The test below measures it on the triangle.

### 4. CPU, not wall

The walk's times in the log are the thread's CPU. Per step, the log also gives the candidate rows the probes read.
The machine's load ran from 70 to 141 through this work, and wall times moved by 2× from one run to the next.

## Measured (2026-11-14, debug build, strict, every run under `tmp/capped.sh`)

**Tests.**

- `plan::joins::tests::a_sampled_probe_reads_its_limit_and_estimates_its_matches`:
  - a key of 50 rows is read whole and is exact;
  - a key of 10,000 rows reads 64 a probe, and the mean of 200 estimates is within 1%;
  - on two columns, a quarter of the key, the mean is within 3%.
- `an_orders_work_by_a_walk_is_its_run_through_a_triangle` adds a walk 2 bindings wide (32 before checks). The
  triangle's 90 edges pass the probe's limit, and the mean over 2,000 walks is within 5% of the exact 215.
- Planner tests 121 pass, engine 50.

**The fixture's self-join seed `call_ret`** (545 findings):

| | Before (0233) | Now |
|---|---|---|
| Sample: heavy / the rest | 2 / 7 rows | 2 / 36 rows |
| Orders tried | 5 (4 cut) | 1 (4 walked out, at 5.6× to 474× the least walked) |
| The trials' work | 33.7 k | 33.1 k |
| The winner's estimate, as a share of the line's 874 a row | 1.00 | 1.00 |
| Walking, all orders | — | 26.3 k probes, 80 ms CPU |

**The data-layer repository, 50 files** (3,462 findings in every run):

| | Before (0233) | Sample fitted (§1) | And walks choose (§2, §3) |
|---|---|---|---|
| The running order's trial | cut | finished | finished |
| Orders tried | 5, all cut | 5 (4 cut) | 1 (4 walked out, at 4.5× to 121×) |
| The trials' work | 1.21 M | 4.19 M | 0.54 M |
| Walking | — | 18.6 k probes | 35.0 k probes, 137 ms CPU |

The walks rank the five orders as the planner's prices do, and as the trials did where they finished. Once the
lone trial was skipped (§2, last item), neither run tries anything: the fixture ran in 7 s and the 50 files in 6 s,
at load 5. Earlier runs, at load 70 to 141, are not comparable.

**328 files** (every 17th Python file, this hour's one long run, capped at 600 s and 1,000 MB): killed at 600 s,
before the line finished. The planning was not what took the time:

- the second wave's `call_ret` line: 1,353 rows left, its six orders walked in 24.3 k probes and 108 ms CPU;
- the running order walked at 1.6 G rows of work over the line, the next at 2.1 times that;
- before the skip, the lone trial was cut after 8.6 s;
- the line's own work is the rule's: in each function, its calls times its returns times its identifiers.

## Open

- **The 328 files are not run to the end.** The seed rule's line there is about 1.6 G rows of work by its best order.
- **The orders walked out are not tried, so the planner's correction (`put_tried`) learns nothing of them.** Their
  walked work is logged only.
- **Comparisons and assignments are taken at 1** (0232).
