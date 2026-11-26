# 0282. The critical path is issued and claimed first

Status: built (audited 2026-11-23). Decided (2026-11-15). Follows 0274, which trials the joins on a run's measured
critical path (0208). Amends the issue board (0113 §2–§3) and the host stream's claims (`accel/src/stream.rs`).

## Intro

The issue board runs a program's strata as jobs on the host stream. A stratum is ready once its producers
retire. The loop's thread packs ready strata into components and submits them. The stream's workers, and the
loop's thread while it waits, then claim queued jobs.

Decision 0274 found the python pack's longest measured path waiting, not working: 3.57 s of its 3.66 s, at a
machine load of 70. No trial of a join can shorten a wait. So this record looks at the waits.

At load 6, over the pack's 3,286 strata on 50 files, the measured path had a span of 0.33 s, and 0.20 s of it
was waiting. In every gap, the waiting stratum's component was ready but started late, while hundreds of jobs
off the path ran. The issue and the claim each favoured the work off the path:

- **The claim took the largest job first.** A small stratum on the path waited behind every larger one.
- **The issue took program order.** At most 256 members are in flight, counted from their component's
  submit until the loop absorbs it. The cap filled with the ready strata that came first in program order,
  and a ready stratum on the path waited for a slot.

## Decision

1. **A stratum's rank is its bottom level**, as list scheduling ranks a task (`Board::levels`): its estimate,
   plus the highest of its readers' levels. The levels are made once, at the board's first issue. A
   component's rank is the highest of its members'.
2. **The stream claims by rank, then by size** (`Stream::submit_ranked`). A plain `submit` ranks 0.
3. **The board issues ready strata by rank.** The stratum the loop is at goes first, then the rest by level.
   The 256-member cap stays. It now holds the strata that the run's end waits on longest.
4. `RERAIL_STRATA_RANK=off` ranks every stratum 0, a measurement aid: claims are then by size, and the
   issue is in program order, as before.

The trace (`RERAIL_STRATA_TRACE`) gains two things: each stratum's submit time, as a last column, and
`loop` lines giving the loop thread's seconds by activity (issue: pack, statistics, device waves, submit;
collect; help; pause; retire). These lines split a gap into its parts: ready to submitted is the loop's
delay, and submitted to started is the queue's.

## Measured

The python pack (317 rules) over 50 files of corpus0. Each run is cold and strict, with node attributes
eager. Times are the 3,286-strata part's measured path, in seconds.

The first table is the change made step by step, two runs each, at load 26–38:

| | waited | ready → submitted | submitted → started | span |
|---|---|---|---|---|
| before (claim by size, issue in program order) | 0.49 | 0.26–0.27 | 0.21–0.23 | 0.65–0.69 |
| claim by rank | 0.47–0.49 | 0.45–0.47 | about 0.01 | 0.64–0.76 |
| and issue by rank | 0.09–0.12 | 0.07–0.10 | 0.02 | 0.59–0.74 |

With the claim ranked, the queue's wait went to the issue. With the issue ranked too, most of the
path's wait went. Its busy time rose to 0.50–0.62 s, though: the path that is left is work.

The second table alternates the two forms three times, at load 46–55:

| | waited | ready → submitted | submitted → started | span | CPU s | G instructions |
|---|---|---|---|---|---|---|
| ranked | 0.57–1.03 | 0.45–0.82 | 0.11–0.20 | 1.22–1.87 | 17.5–18.1 | 166–168 |
| `RERAIL_STRATA_RANK=off` | 0.88–1.58 | 0.14–0.56 | 0.55–1.02 | 0.98–1.72 | 17.0–18.5 | 166–168 |

At this load, ranking moves the wait out of the queue and into the loop, and **the span does not shorten**.
The loop lines say why. The stream starts a worker only while the cores, divided by the load, leave a thread
beyond the caller's and others' jobs. At load 60, 18 cores give 5 threads. So the loop's own thread runs much
of the work: 0.46–0.91 s
running components at their retire (claimed while still queued), and 0.13–0.58 s helping with other jobs.
Issuing took 0.07–0.32 s, and collecting 0.02–0.06 s. While the loop runs a job, nothing that becomes ready is
issued, so a ready stratum on the path waits for the loop to finish its job.

Every run gave the same 63 rows. The peak was 838–930 MB in both forms. The rank costs no measured CPU or
instructions, and it is kept.

On a quieter machine (load 4.5 at the start, 16–19 during the runs), alternating three times:

| | waited | ready → submitted | span | wall s | CPU s | G instructions |
|---|---|---|---|---|---|---|
| ranked | 0.04–0.16 | 0.04–0.09 | 0.27–0.29 | 4.5–5.0 | 15.4–15.8 | 171–173 |
| `RERAIL_STRATA_RANK=off` | 0.17–0.23 | 0.15–0.18 | 0.34–0.38 | 4.5–4.7 | 15.6–15.7 | 171–172 |

Here the strata span is about a fifth shorter, and the run's wall is unchanged: the strata are 0.3 s of it.

The strata part is a small part of the run's wall. The 6.9–13.9 s walls (CPU 17–18 s) are about the main
thread's serial phases: combine 0.97 s, prepare 0.76 s, local 2.5–3.1 s (the strata span is 1–2 s of it),
and cross 0.53 s of CPU. With 200 K involuntary context switches, the main thread waits for a core.

## Not done

- **Only the loop's thread issues.** Under load, it runs components itself, and the board stops issuing
  while it does. The structural fix is for a job's end to retire its members and issue their ready
  readers, on the thread that ran it, so that no thread's job holds up the issue. This is
  O-loop-thread-issues.
- **The main thread's serial phases** (combine, prepare, local outside the strata) are the run's wall
  critical path. A measured path over the phases, like 0208's over the strata, would show them.
