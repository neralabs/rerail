# 0206: A join is cut at its budget and handed off at the row it reached

Status: built (2026-11-14). The engine's cut and resume (`native/setfix.rs`), the driver's checks, handoff and
handback (`Run::drive`). Open: whole plans are not cut (O168).

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

## Open

- **O168, whole plans are not cut.** A probe run as its whole plan (its delta is all of its relation, in the first
  wave) has no budget. The driver plans whole plans apart, and its handoff would need that plan's own replacement.
