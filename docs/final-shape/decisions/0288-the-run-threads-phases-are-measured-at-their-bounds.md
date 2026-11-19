# 0288. The run's thread is measured phase by phase, and in the profile by span

Status: decided (2026-11-15). Follows 0282, which found the strata loop is a small part of a run's wall, and the run
thread's serial phases most of it.

## Intro

A corpus run moves through phases on one thread: combine, constants, prepare, wait, local, stats, cross and write.
Some phases hand work to other threads. Others run alone, and while one runs alone, the run's wall is that thread's.

The run already reported each phase's CPU on that thread, and the whole process's instructions. Those cannot say:

- how long a phase took (its wall),
- whether the thread was on a core, blocked, or waiting for a core,
- how much the other threads did meanwhile,
- what in a phase took its wall.

Nor did they cover the command's time outside the run: reading the rule files and the run's setup before it, and
the run's end after it.

## Decision

1. **Every phase's clocks, at its bounds** (`profile::PhaseClock`, the `Report`'s `phase_clock`). Each is a few clock
   reads per phase, so it is measured on every run, with the profile or without:

   - **wall**: the phase's duration;
   - **own**: the run thread's CPU, less the pool's jobs it ran while it waited;
   - **helping**: the CPU of those pool jobs (`rerail_accel::exec::helped`);
   - **off a core**: wall less both, the time the thread was blocked or was ready with no core;
   - **the other threads**: the process's CPU less the run thread's;
   - **parallel**: the process's CPU over the wall;
   - **ready with no core**: the process's runnable time less its CPU. macOS's `ri_runnable_time` counts a thread
     from when it is ready until it blocks, running or not, so this part is the machine's load;
   - **involuntary switches**: the process's.

   The command line's `logic` prints one line per phase. The command also gets two phases outside the run:
   `before`, from the command's start (`profile::command_begins` in `tools::cli`) to the run's first phase, and
   `after the run`. It prints how long the process ran before the command began, too. Both on stderr: a command's
   stdout is its output (`run-rr`'s and `sql`'s JSON), which a line after it made unreadable (2026-11-16, 0296's
   merge of the hub).
2. **In the profile, the run thread's time by span** (`profile::track_run_thread`, `run_thread_phase`). Each time
   the thread opens or closes a span, the time since its last open or close is charged to the span that was
   innermost: wall and the thread's CPU. So a span's own time is split exactly at its children's bounds, and a
   phase's time is split where the phase ends.

   A pool job the thread runs while it waits opens spans of its own, and its CPU goes to them. Tracking starts at the
   command's start, so the time before the run is the phase `before`. `profile.txt` gains the section "The run's
   thread, phase by phase": the clock table, then for each phase its most expensive spans (innermost), with wall,
   CPU and wall off a core.
3. The CLI's steps before the run get spans: reading the rule files, the dialects in the corpus, upkeep stopping
   for the run, and watching the corpus and the library.

## Measured

The python pack (317 rules) on 50 files of corpus0, cold and strict, without the profile, at load 15–30. The numbers
are the run thread's, in seconds:

| phase | wall | own | helping | off a core | others' CPU | parallel | ready, no core |
|---|---|---|---|---|---|---|---|
| before | 0.63 | 0.18 | 0.01 | 0.44 | 0.30 | 0.77 | 0.13 |
| combine | 0.46–0.54 | 0.36–0.42 | 0 | 0.07–0.11 | 0.55–0.94 | 2.0–2.7 | 0.16–0.26 |
| prepare | 0.48–0.68 | 0.47–0.67 | 0 | 0.01–0.02 | 0.02–0.05 | 1.0–1.1 | 0.01–0.02 |
| wait | 1.31–1.85 | 0.02–0.04 | 0.27–1.04 | 0.27–1.54 | 4.4–6.0 | 3.4–4.6 | 0.27–4.75 |
| local | 0.27–0.29 | 0.27–0.29 | 0 | 0 | 0 | 1.0 | 0 |
| cross | 0.29–0.30 | 0.28–0.29 | 0 | 0–0.01 | 0.19–0.20 | 1.6 | 0.04–0.10 |
| after the run | 0.20–0.27 | 0.18–0.24 | — | 0.02–0.03 | 0.02–0.04 | — | 0 |

Prepare and local run alone: parallel 1.0, all on the run's thread. Combine and cross are partly parallel. Wait is
the chunks evaluating at once (0254), and the run's thread mostly helps or waits there.

Three things the instrument showed at once:

- **A freshly built binary's first launch** began its command 3.0 s after the process started. A second launch
  began it 0.01 s after. macOS checks a new executable when it first runs, so a measurement skips the first run
  after a build.
- **`before` is mostly blocked.** In the profile, reading the rule files runs a nested run, whose watch of the
  corpus and library (`observe`, `watch`) took 0.466 s of wall and 0.014 s of CPU. Reading the files took 0.37 s
  more.
- Under the profile, the run's work grows (575 G instructions against 131–135 G). The profile's span table is for
  finding where a phase's time goes; the plain run's table is for how long it takes.

Every run gave the same 4,465 rows. The plain run's instructions were 128–139 G. The clocks are four reads at each
of about ten phase bounds, which was not measured apart.

## Not done

- Off-core time is not split into blocked and waiting for a core per thread. macOS gives the runnable time per
  process only. A phase with high off-core time and low no-core time is blocked.
- The phases of a nested run (phase 0's, a phase's, the constants') are not tracked apart. Their spans are in their
  outer run's phase.
