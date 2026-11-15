# 0245: Background walks split by heavy keys and stop at their budget; an order leading elsewhere is trialled over its own lead

Status: built (2026-11-15). Addresses the three open items of decision 0243. `WalkSnap::link`, `starts` and `walk`
in `plan.rs`; `choose` and `search_one` in `plan/joins.rs`; `trial_own` in `native/setfix.rs`.

## Intro

Decision 0243 moved the join-order search to the background. The search walks a line's candidate orders over the
relations the line held, and its walked work ranks them. It left three gaps:

- **A search ran to its end once started**, so one search could pass the run-end budget.
- **The background walks were noisy on skewed keys.** On the data-layer files, two orders with the same work walked
  4.90 M and 6.14 M. The inline walks split their rows by a key's heavy values (0221, 0233); the background did not.
- **An order leading with another atom than the cut could not be trialled.** A trial runs over the rows a cut left,
  and such an order reads them at no one step (`restricted`). Its walked work reached the plan only through a ratio.

Fixing them exposed a fourth problem, in how walks reach the plan.

## Decision

### 1. A search stops at its budget

The search is given whether its time is up (`Search` takes `over`). It walks the order that ran first, then the rest
by price, and walks no further order once the time is up. The run-end drain and the idle job both pass their budget.

A search can still pass its budget by the one order it is walking: 150 ms against 145 ms on the fixture.

### 2. A background walk splits its lead rows by a heavy key

A request keeps each body column's heavy values: the `HEAVY` (64) most common, as `Run::heavy_parts` takes them. A
walk then splits its lead's rows past the checks, where it can:

- **The key.** It is either a variable of the lead at a column with heavy values, or a variable of the one atom that
  holds both such a key and a variable of the lead (`inf(f, m)` from `kind(m, return)`). This is the same rule as
  the inline split.
- **The rest.** The uniform draw's survivors whose key is not heavy. They stand for their share of all the rows past
  the checks.
- **The heavy part.** It is drawn by key:
  - each heavy value's link rows are sampled in proportion to the value's rows;
  - the lead rows they reach that pass the checks are kept;
  - each value's survivors are scaled by its rows over those drawn.
  
  Draws repeat until 128 rows pass or 8,192 are drawn, the same cap as the uniform draw. The uniform draw's heavy
  survivors are walked with them, as they sample the same rows.
- **Not split:** a delta's sample, whose rows are no relation to draw by key, and a draw that read every lead row.

A lead row that two heavy values reach is counted twice.

### 3. Walked orders are costed in the walks' units

0243 kept each walked order's walk over its price as an order tried, and the choice multiplied that ratio by the
price it computed. The two prices come from different models:

- the walk's ratio was over a price made in the forked model of the request;
- the choice multiplies by a price made now, under the next run's statistics.

On the fixture this took an order that walked at 1.35 times the order that ran, and its line did 1.64 times the
work.

So the search keeps no orders tried. The choice (`choose`) costs an order that was walked but never measured as:

    its walked work × (the measured cost of the order that ran / its walked work)

The order that ran is the candidate with a measured ratio whose walked work is the note's. Orders measured by a run
or a trial keep their measured ratio.

### 4. An order leading with another atom is trialled over its own lead

When a trial cannot place an order in the cut's rows, it runs the order over a stride sample of its own lead's rows
(`trial_own`). The rows it makes are dropped; none are kept for the wave (O169). The order is not placed mid-line:
it would read again the rows the cut finished.

- **Sample size.** The sample fits half the trial's cap at the order's walked work per lead row.
- **Its work over the line** is scaled by two phases (0233):

      walked work × the trial's work per sampled row / its walk per sampled row

  The background's walk over the lead is the first phase. A walk over the sample's rows that match the lead's
  constants, at 4 times the width, is the second.
- **It is kept** as an order tried: its work over the line, over its price for the line. The next plan's choice reads
  it.

Inline, such an order is not walked from the cut lead's rows. That walk measured 248 M for a line of 5.7 M, and the
cutoff dropped the order.

## Measured (2026-11-15, debug build, strict, every run under `tmp/capped.sh`, load 14–57)

The runs below drew the heavy part with seed `0x0244_5eed`; it was renamed to `0x0245_5eed` with the decision's
number, and the tests passed again with it.

**Tests** (planner 124, engine 50):

- `a_background_walk_splits_its_lead_rows_by_a_heavy_key`. The data: 1,000 functions of 30 nodes with 5 calls each,
  and 2 functions of 2,000 nodes with 50 calls each. The large functions hold 2% of the calls and 56% of the work.
  - Split, the walk measures 1.04 of the work. Without the split, it measures 1.76.
  - The heavy calls are estimated within 15%.
- `walked_orders_are_costed_against_the_walked_order_that_ran`. An order priced at 10 times the one that ran, walked
  at a quarter of it, is chosen.
- The ranking test of 0243 still passes. Its check of the walks' tried ratios is removed with them.

**The fixture** (`call_ret`, 545 findings in every run):

- The search was served in 145–150 ms CPU against budgets of 142–147 ms.
- The next run kept the order that ran, at 388,673 rows of work. Under 0243's ratios it had taken 0.1.2.3.4.5, which
  did 638,640.

**50 data-layer files** (3,462 findings in every run). Each run after the first is from scratch (memo, K4, kept
parts off).

The orders as walked and as run:

| Order | Walked, 0243 | Walked, split | Its line's work |
|---|---|---|---|
| 3.2.0.1.4.5 (ran first) | 6.14 M | 6.14 M | 5.66 M |
| 2.3.0.1.4.5 | 2.72 M (4.90 M at first) | 3.67 M | 5.69 M |
| 1.0.2.3.4.5 | 3.29 M | 2.66 M | 6.89 M |

The runs:

| Run | Order run | Line's work | Own-lead trial |
|---|---|---|---|
| 1 | 3.2.0.1.4.5 | 5.66 M | search served in 180 ms CPU (353 ms budget) |
| 2 | 1.0.2.3.4.5 (best walked) | 6.89 M | 2.3.0.1.4.5: 4.74 M estimated for 5.69 M |
| 3 | 2.3.0.1.4.5 | 5.69 M | 1.0.2.3.4.5: 3.25 M estimated for 6.89 M |

Run 2 took the best walked order. Its line measured that order as worse, and run 3 went on with the next.

Along the way, the own-lead trial at its rows alone estimated the fixture's 2.3.0.1.4.5 at 49,612 for 388,673. Before
its walk was limited to lead rows matching the constants, it estimated 1.0.2.3.4.5 at 175 k.

## Open

All three addressed by decision 0247 (2026-11-15).

- **The walker underestimates orders that fan out after the lead.** 1.0.2.3.4.5 walks at 0.39 of its work, inline
  and in the background alike. After `inf(f, n)` each call has several enclosing functions, and thinning to 64
  bindings at that step loses the large ones. The lead's split cannot see this. The own-lead trial inherits it
  through its first phase (0.47). A split at each thinning step, by the step's key's heavy values, would carry it.
- **A walked order is tried by running it.** A walk below the order that ran is only a proposal. Run 2 paid 1.2 M
  rows for the measure.
- **A trial's estimate and a run's measure merge as equals** in an order's tried ratio.
