# 0274. Trials follow the observed critical path

Status: decided (2026-11-15), by the user: "trialing should focus on what has been observed to be on the critical
path of a run ... if we continually improve the critical path, and discover the new critical paths once we've
optimized each one, we will eventually end up very, very fast". Amends decision 0207 §2 (when a line is
trialled). Builds on 0203 §4 and 0208 (the critical path, measured) and 0271 (a slow line is bounded on every
path).

## Intro

A trial (0207) runs a line's candidate orders over a sample of its rows, and keeps what each measured. Until now
every line was trialled at its first check in the process, once its work passed `slow_rows` (100,000 rows).

On the python pack over 50 files, 119 lines were trialled or walked. Eleven of them were slow, and one trial won:
`rr_agg287`, 11,096 rows of work against 15,461. The trials cost 3–5 G of the run's 171–174 G instructions
(O-post-merge-cpu). Most of them measured lines whose speed does not decide when the run ends.

## Decision

1. **A line is trialled only where it has been observed on a run's critical path** (`joins::take_trial`,
   `on_critical_path`).
   - At each run's end, the critical path is measured (0208): the strata as they ran, by seconds where they are
     known.
   - Each line on the path keeps its share of it in its note, a moving average over runs (`Note::critical`).
   - A line is trialled where that share is at least `critical_share` (5%): at its first check in the process,
     and once more after it is flagged slow, as before.
   - The share is kept in the store with the note, so the next process trials the lines this one found.
2. **The path moves as it is made faster.** A line that leaves the path halves its share each run, and stops
   being trialled within five runs. The lines of the new path are trialled in their turn.
3. **A slow line off the path is still bounded.** It is planned again at its check with exact statistics, and its
   replacement runs under the handoff's limit (0271, `joins::trial_due` in `measured_below`). The worst case is
   bounded on every line. Trials, the average-case work, go where the run's time goes.

A first run in a process, with no notes in the store, trials nothing.

## Measured

Cold, strict, node attributes eager, 50 files each, under load (load average 55–72):

- **Python pack, first run:**
  - 166.7 G instructions, against 171–174 G with every line trialled; 63 rows.
  - No trials ran. 12 lines were slow, and 2 of them were planned again at their check.
- **Python pack, a second cold run in the same process,** with the first run's paths in memory:
  - No trials ran. Its slow lines (`rr_agg57`, `p140_joinee`, …) had a share of 0 of any measured path.
  - The run's longest measured path (6 of 3,286 strata predicted, 27 measured) spent 3.57 s of its 3.66 s
    waiting, and 0.09 s busy. What decides when this run ends is not a join order.
- **Data-layer candidates, first run:**
  - 245.7 G instructions, against 242–243 G before. Its trials were cheap, and the difference is within the
    load's noise. 75 rows.
  - No trials ran. 3 slow lines were planned again at their check.

`joins::tests::trials_are_for_lines_observed_on_the_critical_path` follows a path that moves: from line 600 to
line 602, over runs.

## Not done

- **A line's share is of its part's path.** A run measures a path for each part it evaluates: each chunk's
  file-local program, a library's, and the cross-file part. A line holding 5% of a small part's path is noted
  even where that part is off the run's own path. Such parts' lines rarely reach a check, so few are trialled.
  The exact form is a share of the whole run's measured path.
- **The python pack's path is waits.** Its longest path waits 3.57 s of 3.66 s. Whether that is the machine's
  load or the scheduler's, it is the next critical path to look at, and no trial reaches it.
