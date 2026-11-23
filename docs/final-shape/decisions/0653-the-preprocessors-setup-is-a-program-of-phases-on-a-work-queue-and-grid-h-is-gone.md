# 0653. The preprocessor's setup is a program of phases on a work queue, and `grid.h` is gone

Status: accepted (2026-11-23). Supersedes the persistent grid of 0468 §4, 0540 §3's join and 0568's residency for
the preprocessor, and 0588 §6's teams and 0613 §4's helpers. Takes 0540 step 6's scheme (and 0603's program) to the
preprocessor.

## Intro

`ukl/grid.h` was a persistent grid: every group of a dispatch resident at once, and a device-wide barrier between
phases, where groups spin until the last arrives. Decision 0540 bans new spinning barriers: a thread that needs
another's result pushes a continuation and takes other work. Only the C preprocessor still ran on `grid.h`. Its
setup (`pp_unit`: the front, the definitions, P1's iterations with the `#if` lines' and the text's expansion loops,
the `__COUNTER__` ranking) was about 100 phases between barriers, with scans, segmented scans and sorts that were the
grid's own. A batch of units ran as teams of groups, each team a grid of its own, with helpers that polled the units'
queues.

The user, 2026-11-23: remove the residency test and the barrier mechanism; first merge the Go/C/C++ branch, then
port the preprocessor, then remove it.

## Decision

1. **A unit's setup is a program of phases.** The code between two barriers is a phase (`pp_step`), with three
   parts:
   - its *body*, run by each of its tiles: the same `EACH` loops as before, over `g.group` = the tile of
     `g.groups` = its tiles, so a phase runs right over any number of tiles;
   - its *work*, the most items its loops run, which sets its tiles (at most the unit's `TMAX`, a word of its
     lay), or an exact count of tiles (`PW_TILES`);
   - its *after*: the control between barriers (the loops' counters and tests, the breaks) and the one-thread
     blocks (`ONE`), run by the thread that finishes the phase's last tile (`pp_advance`), which sets the next
     phase (`NEXT`) or ends the program.

   A phase with no work runs its after at once: `pp_advance` runs afters until a phase has tiles, at most
   `PP_ADVANCE` of them in one call. A local that crossed a barrier (P1's iteration, the loop's pass, a
   depth or a round, the loop's kept fronts, the `#if` flag, a return phase) is a word of the unit's program store
   (`PROG`).
2. **The grid's scans, segmented scans and sorts are phases too.** `grid.h`'s code is split at its barriers into
   stages (`ukl/stages.h`), each over the tiles that stage gets: a scan's sums, their scan by one tile, and the
   shares from their offsets; a small one tile 0's alone. A sort is its varying bits, then each digit's histogram,
   the histogram's scan and the scatter. Its few-rows path writes each row at its rank in the other buffers, and a
   last stage moves the rows back where its caller reads them. A call is a sub-program: its parameters and its
   return phase are words of `PROG`.
3. **A batch's units share one queue** (`pp_run`). An item is a unit's tile: the unit's index and the tile. A phase's
   finisher pushes the next phase's tiles, then takes on. A group takes any unit's tile. While one unit is between
   phases, the others' tiles keep the groups busy; this replaces the teams and their helpers. Within a unit's loop
   phase, the loop's own queue (`LPQ`, 0613 §4) is as before.
4. **Every launch has a budget** (0540 point 4, 0603 point 5). A launch counts the tiles its groups push
   (`PQ_PUSHED`); a group takes an item only while that count is below the launch's end (`PQ_END`), so a launch
   runs at most its budget (`BUDGET`, 4,096 tiles) and the tiles of the phases it opened. One command holds
   `PP_LAUNCHES` launches of `pp_run`, with `pp_rebase` between them: the ring's pause, which empties the slots
   taken, moves the base to the head, and sets the next launch's end a budget past its pushes. A launch's fresh groups take what the
   last one left. A worker that finds nothing to take ends at once (0540 step 7). So within a launch, a unit that
   is alone runs its phase changes on fewer groups, and each launch restores them. The host reads the units' program
   words after the command. Where a program is not done and items are left, it continues in another command from
   its words, as 0603's relaunch does, at most `COMMANDS` of them. R9 of 0448 (the host reads nothing between passes) holds within a command.
5. **The check is the program's last phase.** `pp_check` and `pp_check_teams` are gone. The output past its room
   (`X_ROOM`) is refused at the program's end, and a setup that did not reach its end is refused `X_QUEUE`.
   `X_BARRIER` is gone: there is no barrier to fail.
6. **Removed:** `ukl/grid.h` and its tests (the residency test `a_grid_past_residency_is_the_groups_that_run`
   included), the barrier, the join, the beats, the residency (`at_resident`, `halved`, `groups_for`, `refuse_past`),
   `pp_setup` and `pp_teams` with `lp_help` and `PG_HELP`, the probes of the grid's sort and barriers, the device
   crate's residency module (`grid.rs`), the CPU shim's resident count and `UKL_GRID_RESIDENT`/`UKL_GRID_SPINS`.
   A run's trace keeps each phase's line and clock (`PROG`'s words past `PR_TRACE`), and `STOP` still ends a run
   before a phase. The UKL library's grid tests are the stages' tests (`the_stages_scan_equals_the_host`, the
   segmented scans', the sort's), each stage a launch the host sequences.

## Sizes

- A phase's tiles: its work over the group's threads, at least 1, at most the unit's `TMAX` (the host's: its
  largest count of items over `TILE_ITEMS` = 4,096, `UKL_STAGE_SMALL`, at most `TILES_MOST` = 256 = `PP_TLIB`; 1
  on the CPU, whose groups take turns on one thread).
- The workers: 64 groups (`RERAIL_PP_GROUPS`), 1 on the CPU.
- The queue (`PQ_*` words, then the lay offsets, then the ring) holds `BUDGET + workers × max TMAX + Σ TMAX + 64`
  slots: a launch's pushes are at most its budget, plus what each worker pushes past it in the phase it finishes,
  and the items queued at once are at most each unit's current phase's tiles. A push past the ring sets the
  overflow word, and the unit is refused (`X_QUEUE`).
- `SUMS` holds a stage's tiles' sums: `4 × PP_TLIB + 4` words, a library stage at most `PP_TLIB` tiles.

## Tests

- The preprocessor's eval tests (`rerail-c-directives`, profile `second`) and the arena's tests, on the CPU
  backend (the user's order of 2026-11-20: no preprocessor run on Metal): 46 of 47 pass;
  `a_line_past_its_room_runs_again_larger` takes 7.8 s, as before this change (7.9 s; 0613's serial `#if` piece,
  recorded at the merge 8e3ba68cbb).
- The UKL library's stage tests (`rerail-gpu`): the scan, the segmented scans and the sort equal the host's at 1,
  8 and 32 tiles.
- The setup's Metal source splits into 13 units: the 11 stages, `pp_step` and `pp_advance`.
- No test or source names `grid.h`, `ukl_grid_` or `UKL_GRID_` (the arena's test of its source).
