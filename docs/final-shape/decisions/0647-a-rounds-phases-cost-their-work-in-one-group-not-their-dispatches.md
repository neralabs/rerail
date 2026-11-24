# 0647. A round's phases cost their work in one group, not their dispatches

Status: spike (2026-11-23). Measured; nothing here changes 0641's times past the M3's run-to-run noise. The direct
dispatch and the chains are in the tree, on, the chains with a switch for a measure. The small scans are replaced by
a sized scan's levels that start no groups (below, "A scan's levels, started only where needed").

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
- **Small scans** (since replaced, below). A sized scan's first level that fits one tile (`2` in
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

## A phase timed alone

Each phase of the leveled reach was timed alone: a stack a phase (`RERAIL_STACK_PHASES=1`, chains off), so each is a
command buffer of its own and its GPU time is the phase's. The launch budget stops the run at 200 phases, about two
rounds. A command buffer's own cost sets a floor: about 13 µs here, and 40 µs on a second, busier run.

- **The scans are most of a round's phases**: 105 of the 200 (63 tiles' passes, 42 sums' passes), each at the floor.
  The size scripts are 27 more.
- **An index's placing was one thread's loop** (`ix_place_one`). The place before an empty bucket set that bucket's
  offset, and with no rows thread 0 set all `cap + 1`. A round's few rows against the room's many buckets made one
  thread write nearly the whole table: 247 µs a phase (median), 1.8 ms at most. An index's offsets are now a thread
  a bucket (`ix_offs_one`, `PK_IX_OFFS`, beside the rows' `PK_IX_PLACE` in its phase): a search of the sorted keys
  for the bucket's first row. Timed alone, the phase is now within 40 µs of the floor. The measure's totals did not
  move: in a stack, the phase's loop was not what a round waited on.

A phase that starts a group pays its descriptors' dependent loads before its first row: its item, its phase, its
segment and that segment's count, then its job's or scan level's words. The small scans emptied four scan phases but
each still started a group (a phase starts one when its counts need none) and made those loads, which fits their null
result. Phases removed, not emptied, are what this leaves to try.

## A scan's levels, started only where needed

The user, 2026-11-23, on the next step (a sized scan in fewer phases): "go ahead". The small scans' path is removed.

- **A segment may start no group** (`SEG_EMPTY`, its `per`'s top bit). A phase whose segments all have it starts
  none where their counts need none: its `join_grid` writes a grid of none and the dispatch starts nothing. Every other
  phase of segments still starts one group at least. A chain (`join_chain`) runs such a phase as nothing and goes on.
- **A sized scan's second and third levels are counted only where they are needed.** Its size script (`size_scan`)
  writes the second level's count (the first level's tiles) and the third's (the second's tiles) only where the
  first level's tiles are past 1,024, else none. It does so with saturating subtraction, a minimum and a constant
  product, as the script has no comparisons. The three passes over those levels then start no group.
- **Its first level's sums' pass is counted by `summed`** (a new word of `ScanWords`; `ScanLevel` is 40 bytes): its
  tiles where they are two or more, else none. One tile's offset is none.
- **Its first level's tiles add the sums before them themselves** where the second level's count is none
  (`ScanWords::offsets`' 2, `ukl_scan_tile_from`): at most 1,023 sums, four words a thread, then the tile scanned
  from that base.

So the usual scan (its values one tile) starts one group in one phase, where it started five. A scan of up to 1,024
tiles starts groups in two phases.

Tested: `a_sized_scan_past_its_own_tiles_scans_its_sums_as_levels` runs the closure with the threshold at one tile
(`RERAIL_SCAN_OWN_TILES=1`, a test build's switch), so every scan of two tiles or more takes the levels. With the
levels' offsets broken for a check, that test failed and the recursive stratum's own test passed: no other test
reaches the levels. `setfix::device` (40) passes on the M3 and on the CPU backend on a Linux host.

On the M3, the measure (medians of seven runs):

| Stratum | Chains off | Chains of 16 |
|---|---|---|
| seeds | 8.8 ms | 10.9 ms |
| seeds ×10 | 46.8 ms | 48.4 ms |
| closure | 50.2 ms | 59.9 ms |
| leveled reach | 53.0 ms | 42.5 ms |
| leveled dist (min) | 28.3 ms | 20.0 ms |

None of it moved past the noise. A scan phase's group (its descriptors' loads and a tile of one group) is not what a
round waits on either.

From the per-phase table above, the size scripts are the largest kind left above the floor: 27 of 200 phases, a
median of 22.6 µs against a floor of 13, up to 167 µs. One thread runs each script's ops in turn, its claims atomic
adds on the arena's top. That is a lead from phases timed alone, which the index's placing showed can differ from a
stack's.

## A real run's recursive strata

The user, 2026-11-23, on the next step: "batch the axis lookups into stacked launches, the way 0648 did for joins,
so a real run stays within the budget. Then rerun this test to see real recursive strata on the device, rounds
included."

The test is the python pack (540 of its 546 rules; the other six do not lower or load yet) over 10 files of the
django corpus, with every stratum the device can run sent to it (`RERAIL_DEVICE=always`), on the M3, in a profiling build.

- **The kind-set axis solve was over the budget.** A tape-value layer's solve (`axis_in_chunk`, 0289) went out as a
  submission a chunk for its count and scan, then one a chunk for its writes. With about 99 layers, the run's first
  evaluation hard-failed the 200-launch budget. Its chunks now go out stacked (`Pass::stack`, 0648): every chunk's
  count and scan in one submission, then every chunk's writes in another. A stacked pass's library kernels are
  charged as stacked (`launch::charge_made_stacked`), not as launches each.
- **The profile has each stratum's GPU time and rounds.** `placement.tsv` has `gpu_s` (its command buffers' time)
  and `gpu_rounds` (its components' rounds on the device, `fixpoint`). The GPU times of strata sent in one wave
  overlap, so they can sum past the run's.

Measured, cold (a fresh store), on the M3:

- The run took 24.6 s wall and 2.3 GB at its peak, and 1.4 s warm. The CPU run took 30.9 s cold.
- Both runs' results are the same: 1,109 relations, 2,108 rows, compared as CSV.
- Of 112 recursive strata, 91 ran on the device. 8 lack a form for a computed term, 1 a string join, and 12 were
  sent back at run time.
- The 91 took 0.91 s of GPU time, against 2.43 s of CPU time for the same strata in the CPU run.
- They ran 392 rounds: a median of 3 a stratum, at most 44. The deep ones took 1 to 3 ms a round, against the
  measure's 0.5 ms.

| Stratum | Rounds | GPU | CPU run |
|---|---|---|---|
| runs_at_import | 3 | 102.3 ms | 2.0 ms |
| p453_rr_near_def_kind | 25 | 73.6 ms | 65.7 ms |
| rr_up102 | 25 | 54.9 ms | 100.5 ms |
| rr_up103 | 22 | 48.8 ms | 112.2 ms |
| py_minor | 44 | 44.2 ms | 1.4 ms |
| pth_unpack | 2 | 24.6 ms | 15.4 ms |
| ast_parent | 4 | 24.0 ms | 97.4 ms |
| pep604_member | 6 | 21.4 ms | 194.6 ms |

The costliest recursive stratum on the device, `runs_at_import`, took 102 ms of GPU time for 3 rounds, where the CPU
took 2 ms. (An earlier version of this record said it made 6.7 million rows: that was the planner's estimate of its
rows, `gpu_rows`, not rows made.) Its tables are sized from that estimate. 0657 hands it back after its first round. The strata of many rounds and few rows
(`py_minor`: 44 rounds, 3 rows) are where a round's fixed cost shows.
