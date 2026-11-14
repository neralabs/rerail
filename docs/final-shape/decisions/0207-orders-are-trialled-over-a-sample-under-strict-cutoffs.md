# 0207: A line's orders are trialled over a sample of its rows, under strict cutoffs

Status: built (2026-11-14). The engine's trial (`setfix::trial`), the driver's trials at a line's check
(`Run::drive`), the measured ratios kept in the memo (`joins::put_tried`). Open: O169.

## Intro

Decisions 0203 and 0206 choose a join's order from prices. An order's price is the planner's estimate of its work.
Measured work corrects that price, but only for orders that ran. Every other candidate is still a guess, and the
guess is what fails: in the smoke run below, the planner priced the running order as fine (within 8×), while
another order did a quarter of its work.

The user asked for the proposed orders to be actually tested, "at least in a pressure-test kind of way, with strict
cutoffs etc, in order to properly price them" (2026-11-14). A trial is that test. It runs a candidate order over a
small sample of the line's real rows, stops it early once it is clearly losing, and keeps what it measured as the
order's price.

## Decision

### 1. The engine runs an order over a sample

`setfix::trial(probe, spec, step, sample, budget)` runs one order of a probe's rule:

- **The order.** Either the probe's own body, or a spec planned for it.
- **The sample.** At most `sample` rows of the probe's first atom, at a stride. If the probe was cut (decision
  0206), they are drawn from the rows it left; otherwise from every row its first step reads. Every order is run
  over the same rows, at the step that reads them (`restricted`, as a cut's resume finds it).
- **The budget.** The walk is cut once it passes `budget` steps (0206 §1), and its work is then a lower bound.
- **No side effects.** The rows made are counted, then dropped: nothing is added to any relation. The probe's step
  counts are restored afterwards, so the planner's observations are as they were.

Run over the same sample, two orders of one rule make the same rows. Only their work differs.

### 2. When a line is trialled

At a line's check (0206 §2), for a rule of two positive atoms or more:

- **Once per process.** A line is trialled at its first check where the sample is worth having, and once more after
  it is flagged slow.
- **What it may spend.** Each trial may spend `RERAIL_JOIN_TRIAL_SHARE` (10%) of the line's work so far. That is
  planning paid for by running (decision 0122).
- **The sample's size.** As many rows as that share pays for, at the running order's measured work per row, up to
  `RERAIL_JOIN_TRIAL_ROWS` (256). Under 8 rows, the trials wait for a later check, when the line has done more.

### 3. The candidates and their cutoffs

The candidates, at most six and all distinct:

- the running order, trialled first;
- the order that was cut, where a replan replaced it;
- the best order for the rows left, and its runners-up (`Model::plan_for_rows`, decision 0203 §1);
- the line's proposals from a wider search (0203 §5).

The running order is cut at the share. Each later order is cut at `RERAIL_JOIN_TRIAL_CUTOFF` (2) times the least
work measured so far. A clearly losing order stops early, so trials cost little more than the winner's work.

### 4. What a trial decides

- **Its price.** Each trial's work over the order's price for the sample is kept as an order tried
  (`joins::put_tried`). Trials and runs share one geometric mean. A choice (0203 §2) then corrects that order's
  price by it.
- **The order that goes on.** The order with the least measured work that was not cut goes on if it beats the
  running order by `RERAIL_JOIN_TRIAL_MARGIN` (1.25×), or the running order was cut. It is handed the rows left as
  in 0206 §3, with the same handback if it does worse there.

`RERAIL_JOIN_TRIALS=off` turns trials off. A slow seed is then planned again over its rows left, as in 0206.
`RERAIL_JOINS_LOG` writes each line's trials: each order's work, `+` where cut, `*` the running one.

## Measured (2026-11-14, debug build, strict)

**Engine test** (`setfix::cut_tests`):
- Two orders over the same 64-row sample make the same rows, at different work.
- A budget of half the work cuts the trial.
- The probe's counts are untouched.

**Memo test** (`plan::joins`): a line is trialled once and again after a flag; trials' ratios are kept as
geometric means.

**Planner tests:** 114 pass. **Engine tests:** 49 pass.

**Smoke runs** on the 215-file Python fixture, with the three-way self-join seed of decision 0206 (`call_ret`):

| Run | Trials | The line's work | Trials' work | Findings |
|---|---|---|---|---|
| Before (no cuts) | — | 1.80 M | — | 545 |
| Default settings | 5 orders over 8 rows, at 200 k work | 0.37 M | 0.12 M | 545 |
| `RERAIL_JOIN_SLOW_ROWS=20000` | 5 orders over 16 rows, at 320 k work | 0.32 M | 0.18 M | 545 |

At default settings:

- The running order passed its cutoff of 40,001 over the 8 rows.
- Another order did them in 11,379.
- The three others passed their cutoff of twice that, 22,759.
- The winner took over at row 41 of 392.

In all, 3.6× less work, with the same findings. The slow check alone (decision 0206 at 8×) had left this order
running.

## Open

- **O169, a trial's rows are made twice.** A trial's rows are dropped, then made again by the order that goes on.
  Over a sample that is most of the rows left, the work is done twice. Keeping the winner's rows (they are a set's)
  would save that.
