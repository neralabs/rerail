# 0206: A join is cut at its budget and handed off at the row it reached

Status: built (2026-11-14). The engine's cut and resume (`native/setfix.rs`), the driver's checks, handoff and
handback (`Run::drive`). Whole plans are cut too (O168, closed the same day).

## Intro

Decision 0203 flags a join that runs much slower than its price, and plans it again with better statistics. But it
checked only between chunks, once the engine returned. A join is one call into the engine. A seed join over a
whole input, or one wave's probe over a large delta, ran to its end in that call. However slow it was, the new plan
came too late for the rows that mattered: the run had already paid for them.

The user asked for joins that hit their threshold, were replanned, and whose replacement can take over, to be
stopped. Of the shapes offered, the user chose this one (2026-11-14):

- The slow join stops at row *r* of its first atom.
- The replacement reads rows *r* to the end only.
- If the replacement also passes its limit, the order with less measured work per row goes on from the current row.
- One thread, and no work done twice.

## Decision

### 1. The engine cuts a join at its budget

A join's walk counts its work as the steps it enters, the measure the planner already uses for a line's work
(`profile::plan_work`). Each probe of a component may have a budget (`setfix::set_budgets`).

When a walk passes its budget, it stops:

- No step is entered after that, and no candidate loop reads another row.
- The rows it made so far are kept. A component's relations are sets, so a row made twice is the same row.

**The rows left.** The first step's candidates are listed in the order they are read, once a budget is set. The
cut takes those from the row being read when it stopped, that row included, to the end. The row the walk stopped
in is the only row read twice.

**The wave goes on.** The run stops its wave there and returns `5`. It stashes the wave's rows so far, the probe
it stopped at, and the rows left (`Cut`). The next `run`:

- skips the SQL probes and the device's joins, which ran already;
- skips the probes before the cut one;
- runs the cut probe with its body as it is now, reading only the rows left;
- runs the rest of the wave as usual.

A wave cut in its first wave does not seed its closure or submit its device work again.

**Where the rows left are read.** A replacement may read the old first atom at another step. The engine finds
that step (`restricted`):

1. the step the driver names (`cut_step`), where it reads the same rows;
2. else the one positive atom reading those rows. A delta probe's delta is always one.
3. Else none is known, for example a self-join where two atoms read the same input. That step then reads all its
   rows. The rows already done are joined again, with the same result. `cut_counts` counts these.

Rows are matched by where they live: an input's rows, a relation's delta, or a relation's rows.

### 2. The driver sets budgets at the line's next check

Decision 0203 checks a line once its work passes `RERAIL_JOIN_SLOW_ROWS`, then each time its work doubles. Each
join's budget is now the work left to its next check (`joins::next_due`). So the check happens inside the join,
at the work it would have happened at, wherever the join is. This covers seed joins too, which 0203 never checked:
they ran in one call.

A line that is not slow at its check goes on from its row with the same order. The rows it left are read at its
first step.

A line that is slow is flagged as in 0203: its exact pair statistics, a wider search, and a replan.

- **A probe** is planned again in the chunk's round, as before.
- **A seed** is planned again at once over the rows it left, as a delta of that many rows (`Model::plan_for_rows`).
  Where the new order leads with the same atom, the driver names step 0 to the engine.

### 3. The handoff

When the replan gives another order, the replacement reads the rows left. Its limit is `RERAIL_JOIN_SLOW` times
its own price for those rows.

- If it finishes within the limit, the handoff is done.
- If it passes the limit, it is cut too. The driver compares the two orders' work per row of their first atom (the
  old order's at its cut, the replacement's at its own). The lesser goes on from the current row with no limit. If
  that is the old order, its spec goes back (a *handback*).

A cut never has a duplicate in flight: the old order has stopped before its replacement starts.

### Settings

None are new. Cuts are part of the joins' feedback: `RERAIL_JOINS=off` turns them off with the rest.
`RERAIL_JOINS_LOG` writes each cut and handoff. Only a component the planner drives in process is cut; the
extension's copy of the engine has no budgets.

## Measured (2026-11-14, debug build, strict)

**Engine test** (`setfix::cut_tests`):
- A 600-row delta is cut after 300 steps.
- With the same order, its rows read in the two runs add up to the delta's rows plus one: only the cut row is
  read twice.
- A later wave is cut and handed to another order, which leads with the input, not the delta. The fixpoint is the
  same as an unlimited run's.

**Planner tests:** 113 pass. **Engine tests:** 49 pass.

**Smoke runs** on the 215-file Python fixture, with a three-way self-join rule (`call_ret`, a seed). Each run used
`RERAIL_JOIN_SLOW_ROWS=20000`, so that checks come early.

| Run | Cuts | Handoffs | Stratum work | Findings |
|---|---|---|---|---|
| Not cut (before) | 0 | 0 | 1.80 M | 545 |
| Cut at each check, none slow at 8× | 9 | 0 | 1.91 M | 545 |
| `RERAIL_JOIN_SLOW=0.5` | 4 | 1 | 0.45 M | 545 |

In the third run, the seed was flagged at its second check with no row of its first atom finished. Its exact pair
statistics were measured, and it was planned again over its 416 rows left. The replacement did a quarter of the
work, with the same findings.

At the default 8×, the seed was not flagged, though another order did 4× less work. Choosing the threshold by
measured gain, not by a fixed factor, is the job of the trials of decision 0207 (pending).

The 6% more work where nothing was replaced is the cut rows read twice, and checks at each doubling past 20,000
rows. At the default 100,000 there are fewer.

## Fixed: a cut inside a fused chunk lost rows (2026-11-14)

A first step fused with the filters after it (`rowcode`) reads its rows a chunk at a time. The chunk's work is
counted before its rows go on, so a budget can run out before the first row of a chunk, or partway through one.
The joins below then return at once. But the chunk's loop went on naming each row it passed over as the row
reached (`at0`). So the cut took the chunk's last row as its place, and the rows between were counted as read.
They were never read again, and what they derive was lost.

```text
a delta of 600 rows, one chunk; budget 300 steps, spent by the chunk's filters
before: cut at row 599 -> done 599, left 1      (599 rows never joined)
after:  cut before any row -> done 0, left 600
```

On the O125 replay (step 14, one file), cuts in the cold run of `tbv`, `ownq`, `fl350_walk` and others made
`type_of` differ from the full run: 13 rows extra, 5 missing. With cuts off (`RERAIL_JOINS=off`, or
`RERAIL_JOIN_SLOW_ROWS` out of reach), it matched.

Now the chunk stops at the row the cut stopped at. A cut before any first-step row leaves every row
(`seen0`). Test: `a_fused_first_step_cut_leaves_the_rows_after_its_row` (`setfix::cut_tests`). It fails without the
fix (left 1, done 599).

## Whole plans (O168, closed 2026-11-14)

A probe whose delta is all of its relation (a component's first wave) runs its whole plan, which reads the
relation's rows rather than a delta. It now has the probe's budget too.

- **The engine.** A cut in a whole plan says so (`CutReport::whole`), and the wave goes on with the whole plan.
  Trials of a whole cut run its whole body.
- **The driver.** It keeps each probe's whole order (`wholes`), set where it plans a whole plan. A cut whole plan
  is checked, trialled and handed off as a seed is: it leads with its first atom and reads no delta. It is planned
  again as a whole plan (`Reprobe::Whole`).
- **Its work.** The engine's step counts are the delta body's, so a whole plan's work is taken from its cut.

**Test** (`setfix::cut_tests`): a whole plan is cut in its first wave, trialled, and handed to another whole order.
The fixpoint is the unlimited run's.

## Correction (2026-11-14, decision 0216)

A line's recorded work held its last order's work only, not the orders it handed off. The handoff run above
(`RERAIL_JOIN_SLOW=0.5`) is understated by the replaced order's work. With every order counted, the default run of
decision 0207 did 0.70 M rows (not 0.37 M), still 2.5× under the 1.73 M of no cuts. With decision 0216's price of a
kind check, the line starts in its best order and does 0.39 M.

## A cut inside a fused chunk (2026-11-14)

A first step fused with the tests after it (`rowcode`) runs a chunk of rows at a time. Two faults lost or stalled
rows at a cut there:

- After the cut, the chunk's loop went on, so the row it stopped at (`at0`) moved to the chunk's last row. The rows
  between were reported done but never run. The fixpoint was short of rows. With joins on, typescript-eslint's
  no-shadow flagged six or seven valid fixtures (018, 028–033), varying from run to run, because its
  `count : { … } = 0` missed rows. The loop now stops at the cut. The paired-batch and per-value loops stop there
  too.
- The chunk's filter work was charged before its rows ran. A chunk whose filters alone spent the budget was cut
  before its first row on every run, so no run made progress. The work is now charged after the rows run, so each
  run makes progress.

Test: `a_cut_inside_a_fused_chunk_leaves_the_rows_it_did_not_run` (cut_tests.rs). It fails without the stop.
