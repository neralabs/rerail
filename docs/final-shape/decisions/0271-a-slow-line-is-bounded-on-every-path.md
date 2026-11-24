# 0271. A slow line is bounded on every path

Status: built (audited 2026-11-23). Decided (2026-11-15). Closes O-adaptive-worst-case. Amends decisions 0203 §3 (a
slow seed is planned again), 0206 §3 (a handoff's replacement cut too) and 0207 §2 (when a line is trialled).

## Intro

The adaptive join planner exists to bound the worst case. A join that runs far over its price is cut at its
check, and replaced. Decisions 0203, 0206 and 0207 give three ways to replace it:

- planned again with exact statistics (0203);
- trialled over a sample of its rows (0207);
- handed off to a new order that runs under a limit, and compared with the old order at that limit (0206).

Two paths left a slow line with none of these.

**A slow seed no trial can sample.** With trials on, a seed or whole plan flagged slow was not planned again at
its check, because "the orders are measured below instead". But a trial's sample is what a tenth of the line's
work pays for at its work per lead row, and under 8 rows no trial runs. So a line whose few lead rows each cost
more than its check's budget got neither a trial nor a new plan. Over 35 files of the data-layer candidates
(before 0267), `pre_570` was cut at 201,431 rows of work with 0 of its 423 lead rows done. It was flagged slow,
and then ran for 22 s on one worker. The planner's own order runs it in 6 ms.

**A replacement cut too.** At a handoff's limit, whichever of the two orders had less work per row done went on
with no limit at all (0206 §3).

## Decision

1. **A slow line that no trial measures is planned again at its check** (`joins::measured_below`). That is a slow
   seed or whole plan whose sample is under `TRIAL_LEAST`. It is planned with the exact pair statistics just
   measured, as with trials off. Its replacement runs under the handoff's limit (0206): `slow_factor` (8) times
   its price over the rows left. A line already in a handoff is judged by that handoff.
2. **The order that goes on at a second cut keeps a limit** (`joins::limit_again`). The limit is `slow_factor`
   times its own measured work per row over the rows left, past the work it has done. The old order, placed back,
   has done none. The two orders are compared again at that limit, so a line can change orders as often as its
   rows' cost shifts, and every change is made at a limit. An order that has done no row yet can double its work
   before the next comparison. The limit is always finite.

Neither change touches a line that runs within its price. That is the average case, and it costs nothing more.

## Measured

Cold, strict, node attributes eager, one machine under load (load average 35–55).

- **Data-layer candidates, 50 files.** Same 75 rows. Both paths are hit:
  - `q4_la_vis` was flagged with 55 lead rows done, under a trial's 8. It was planned again at its check, and
    its replacement was cut at its limit (56,730). The replacement did 1,662 rows of work a row, against the
    old order's 1,856, so it went on, under a limit of 15.6 M.
  - CPU 22.8 s.
- **Python pack, 50 files.** Same 63 rows. `rr_agg287`'s replacement was cut at 746 rows of work a row, against
  the old order's 50.8. The old order went back, under a limit of 708,362.
  - 172.0 G instructions, the same as before this change (170.9–173.8).
- `joins::tests::a_slow_line_is_bounded_on_every_path` pins both rules.

## Not done

- A slow line whose new plan is the order it runs keeps running with no limit. Its price is now from exact
  statistics, and 0203 leaves it there.
