# 0208: The run's critical path is measured as its strata ran

Status: built (2026-11-14). The strata's timeline (`issue::Board::timeline`, the loop's own stamps without a
board), the path through it (`joins::measured_path`), and the lines noted by their seconds on it
(`joins::critical_timed`).

## Intro

Decision 0203 §4 finds a run's critical path by work. Each stratum is weighted by its join lines' rows of work, and
the heaviest chain of producers is the path. That is a prediction of where the run's time went. It assumes:

- time is proportional to rows of work;
- strata run one after another;
- nothing else takes time.

None of these hold.

- Strata run as jobs beside the loop (decision 0113), so a heavy stratum off the loop's thread may cost no time on
  the path.
- A stratum may wait: for answers to its requests, for its device wave, or for the loop's thread.
- Time goes to more than joins: SQL strata, loads, indexes and writes.

The user asked for "the _true_ critical path" to be captured after a run, not only the predicted one (2026-11-14).

## Decision

### 1. Every stratum's start and end are kept

The strata's board (decision 0113) already stamped each stratum's start, end and where it ran: on the loop's
thread, as a job, or at its component's retire. It did so only under `RERAIL_STRATA_TRACE`. Now it always does, at a
few words a stratum.

- **The loop's waits.** The loop ends the stratum it ran before it waits for answers or a device wave
  (`Board::close`). The wait is the gap before the next stratum, not part of the stratum before.
- **Without a board** (a maintained run, kept strata, a program run again, `RERAIL_STRANDS=off`), every stratum
  runs on the loop's thread. The loop stamps each one itself.

**Each line's seconds.** The engine already reports each probe's milliseconds in every run (the first figure of
its per-probe field). The driver adds them up per line, and a line's record (`LineRun`) carries them.

### 2. The path is what each stratum waited for

`joins::measured_path` starts from the stratum that ended last, and steps back to what it could not start before.
That is the latest to end, before its start, of:

- its producers;
- on the loop's thread, the stratum the loop ran before it.

Each step's gap is that stratum's wait. The path reports:

- its strata;
- the seconds they ran (busy) and waited;
- the span from the first's start to the last's end;
- how many of its strata are on the predicted path.

### 3. The measured path notes the lines

Where a run has its timeline, the measured path replaces the predicted one in noting lines (0203 §4). A line's
share is its seconds over the seconds of the path's lines. Where no seconds are known, its share is its work over
theirs. The predicted path is still computed. `RERAIL_JOINS_LOG` and the profile show both, with their overlap.

## Measured (2026-11-14, debug build, strict)

**Unit test** (`plan::joins`): s1 does the most work but runs as a job beside s2, which takes longer. The measured
path is s0, s2, s3, with s2's wait on the loop's thread after s0. The predicted path is s0, s1, s3. The lines are
noted by their seconds on the measured path, and s1's line is not noted.

**Planner tests:** 115 pass. **Engine tests:** 49 pass.

**Smoke run** on the 215-file Python fixture, with the self-join rule of decisions 0206 and 0207:

- Its heaviest run's measured path is `kind > inf > call_ret > J1 > rr_diagnostic_at`: 1.10 s busy, 0.026 s
  waiting. All 5 of the predicted strata are on it.
- In the program's library pass, the measured path has 10 strata against 4 predicted. The other 6 do no join work,
  so the predicted path does not weight them, but the run waited on them.
