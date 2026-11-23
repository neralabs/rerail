# 0647. A round's phases cost their work in one group, not their dispatches

Status: spike (2026-11-23). Measured; nothing here changes 0641's times past the M3's run-to-run noise. Each of the
three mechanisms below is in the tree, on, with a switch for a measure.

## Intro

After 0641, the recursive strata took 2 to 5 times a launch a level's GPU time. 0641 put the cause in a round's
dependent dispatches: about 100 phases, nearly all of one item, each a `join_grid` and a `join_phase`.

The user, 2026-11-23: "I'd go after the dependent dispatches next. Phases with no segments have a fixed group count,
so they can skip join_grid and do its flag checks themselves." Then, on the result, "don't remove it yet", and
"please spike on both of these":

- small scans in one phase: "When a level's counts fit one scan tile (the usual case here), the first scan phase
  does the whole scan, and the other four start no groups."
- a level's chain in one group: "When every step of a level fits one group, a single dispatch runs the whole chain
  with barriers between steps. If a step turns out to need more groups, it hands back to the normal phases."

## What was built

- **A phase of no segments is dispatched directly** (`phase_begins`). Its groups are its items, known when it is
  encoded, so it has no `join_grid`. The first of its groups to begin makes `join_grid`'s checks of the program's flags
  and its parks, and swaps its dispatch's sequence and the flags into `ctl[12]`. Every other group reads them there.
- **Small scans** (`SMALL_SCANS`, `RERAIL_SMALL_SCANS=0` off). A sized scan's first level that fits one tile (`2` in
  its offsets word) is scanned whole by its sums' pass. That pass empties the two levels after it, so the second
  level's tiles pass and the first level's segments (its count, the second level's) start no groups. The first level's
  tiles pass returns at once: its phase still starts a group, a segment's first item when its count needs none.
  - Found on the way: the scan's last phase, started for no rows, scanned the scanned values again (a total of 24,976
    for 224 values), and a size script parked on it.
- **Chains** (`join_chain`, `RERAIL_CHAIN_PHASES`, default 16, 0 none). Before a run of a round's phases (up to 16,
  to a round's end), one dispatch of one group runs them in order:
  - each phase's flag checks and its `ctl[11]`, as `join_grid` makes them;
  - its groups, while they are 4 at most (`CHAIN_GROUPS`), one after another, a barrier over device memory after each;
  - at its first phase of more groups, or the first the flags stop, it stops and writes that phase in `ctl[13]`.
  
  The phases' own dispatches follow it, encoded as before. Each passes over a phase below `ctl[13]`: its grid is none,
  or its direct groups return.

Every device test passes on the M3 (`setfix::device`, 39) and on the CPU backend on a Linux host, with all three on.

## Measured

On the M3: `measure_one_program_against_a_launch_a_level`, seven warm runs a measure, the median GPU time. The
closure's own swing between runs of the same build was 39 to 63 ms.

| Stratum | 0641 | Small scans off | Small scans on | Chains of 16 | Chains of 64 |
|---|---|---|---|---|---|
| seeds | 6.2 ms | 8.5 to 9.2 ms | 9.1 to 9.2 ms | 7.4 ms | 8.4 ms |
| seeds ×10 | 50.9 ms | 47.7 to 49.2 ms | 47.6 ms | 48.0 ms | 47.9 ms |
| closure | 50.0 ms | 48.4 to 63.4 ms | 49.2 to 60.6 ms | 56.6 ms | 59.8 ms |
| leveled reach | 44.3 ms | 45.5 to 46.0 ms | 44.9 to 46.5 ms | 46.4 ms | 45.9 ms |
| leveled dist (min) | 23.2 ms | 23.1 to 23.4 ms | 22.9 to 23.7 ms | 20.5 ms | 19.8 ms |

The direct dispatch of a phase of no segments measured the same as `join_grid` before it. The chains' columns have small
scans on; with both off in the same session, the leveled distance took 23.8 ms.

## Why nothing moved

The chains ran nearly every phase. In the measure's programs, a chain ran 76 to 1,876 phases and handed back 1 to 116.
The handed-back phases were the joins of 42 or 52 groups. So nearly every phase's own dispatches were empty: a grid of
none, or groups that return. A round still cost about 0.5 ms (the leveled distance: 38 rounds, 19.8 ms).

- The empty dispatches cost about 0.1 ms a round, as 0641's rounds past a component's end did.
- The rest, about 0.4 ms a round, is the phases' work, one after another in one group: about 4 µs a phase.

So a round's cost is its phases' work, each a few microseconds in one group. The dispatches between them are not the
cost. Running a phase's work in another dispatch, or in none, leaves that time where it is. The small scans cut the
work of four scan phases that each did one tile's, which was little.

## What is left

- **What a phase's microseconds are.** A one-group phase of 224 values takes about 4 µs, the whole of a launch a
  level's time for some of its levels. Its loads are dependent ones (an item, its phase, its segment, its count, its
  descriptor, a scan level's words) before any row is read, and a size script runs on one thread. Neither is measured
  yet. The M3 has no shader clock, and its counters sample at a stage's boundaries only, not a dispatch's.
- **Whether the three stay.** Each is correct and none is a measured gain. 0641 is the design they would be removed to.
