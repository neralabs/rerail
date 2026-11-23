# 0638. A program's levels are stacked dispatches, one launch a stack

Status: accepted (2026-11-23). Supersedes 0637 (a program's idle groups poll) and 0603's single launch (one
`join_run` over a work queue). 0603's program keeps everything else: its phases, items, segments and size scripts,
its placed tables, its arena mapped as its claims reach it, and its gates' rooms.

## Intro

0603 made a stratum's device work one launch: a work queue whose phases were continuations. On the M3 it took 1.3 to
3.7 times the GPU time of the launches a level before it, even after 0637's fixes (0637, "Measured"). What was left
was the handover between phases: about 40 µs a phase on the device, against about 7.5 µs a launch.

The user, 2026-11-23: "okay, go back to a launch a level please / can we stack the launches up several levels at a
time so that we don't pay the level cost (the same kernel launched multiple times with a dependency between each)?
This should count as one launch only and reduce the overhead". Of the ways to do it, the user chose "Device-sized,
stack rounds".

## Decision

- **A phase is a dispatch.** Each phase of a program (a level's step: its jobs' items, a scan level, a sort pass) is
  one dispatch of `join_phase`, a group an item. A phase's segments still take as many groups as their rows need. The
  device, not the host, says how many: a one-thread dispatch of `join_grid` before each phase reads the counts the
  phases before it wrote and writes the phase's grid. The phase is an indirect dispatch of that grid
  (`Pass::run_bytes_indirect_at`), bounded by its items where it has no segments, else by `PHASE_GROUPS` (2^12): a
  CUDA or OpenCL launch starts the bound's blocks, and those past the grid return. A segment past its share strides
  its rows, as before.
- **Not Metal's indirect dispatch.** On the M3, the indirect dispatch hung the GPU
  (`kIOGPUCommandBufferCallbackErrorHang`). The same kernel did not hang when dispatched directly, with the same
  grid and the same groups past the count returning. Neither a buffer barrier before the indirect dispatch nor a
  compute encoder of its own helped. So on Metal alone (`rerail_device::device::is_metal`) a phase is dispatched
  directly with its bound's groups, and those past `join_grid`'s count return. The CPU backend's indirect launch runs
  the grid's groups alone. Its bound's groups, each a coroutine, timed six tests out.
- **A stack's submission is its own, and carries the program's residency.** A stack commits what is open first, so a
  stack is one submission and one launch. What the levels' encoding made resident in that open submission (a view's
  tapes, the atoms' blocks, the rolling cache, each reached by address: `resident_in`) is kept with the program
  (`Pass::resident`, `Kit::resident`), and every stack makes it resident again. Without it, Metal read those buffers
  as zeros: every text came back empty. The CPU backend has no residency, so its runs passed regardless.
- **A stack is one launch.** The host encodes a program's phases into one submission, along its walk:
  - each phase is followed by the next;
  - a round's end is followed by its next round's first phase (`Program::jumps`, `walk_next`);
  - the program's tail (0603's lattice heads' maps grown) is reached only by a park's release.

  The ledger charges a stacked submission once, at its first dispatch, and records the rest (`Pass::stack`,
  `launch::attempt_stacked`). Its report counts its launches (`Pass::launches`), not its dispatches.
- **A stack's rounds.** A program's first stack runs 2 rounds (`STACK_ROUNDS_FIRST`). Each stack after it runs twice
  the rounds of the one before, up to 32 (`STACK_ROUNDS_MOST`). A stack holds 1,024 phases at most (`STACK_PHASES`).
  So a component of few rounds ends in its first launch, and one of many takes about the logarithm of its rounds. A
  round's end that finds no new rows (flag 16), or a round past the component's most (flag 4), ends the program. Each
  `join_grid` after it writes a grid of no groups, so the rest of the stack costs its empty dispatches only.
- **Parks stop the stack where they are.** These still park:
  - a gate's room mapped past (flag 128);
  - texts that found the text arena full (64);
  - a size script past its arena's pages (32, its item listed in `slots`).

  The next `join_grid` sees the park and writes no groups from then on. It notes the parked phase in `ctl[9]`; the
  phase whose grid ran last is in `ctl[11]`. The host maps the room or arena further (`programs_done`). It then
  launches a stack that first runs the parked items again (a direct dispatch over the list, `PhaseP::list`), then goes
  on along the walk from the phase after the parked one. A gate whose lattice map grew is released through the
  tail, whose jump the host now sets to the phase after the gate. The device no longer writes it (`RoundP::next`).
- **Removed:**
  - the one-launch program `join_run` and its work queue;
  - its dispenser (`phase_hand_out`, `phase_texts`);
  - its idle polls (0637, `ukl_wq_ready`, `ukl_wq_take_ready`);
  - its workers' budget (`PROGRAM_BUDGET`, `PROGRAM_WORKERS`, `RERAIL_PROGRAM_BUDGET`) and the report's `taken` and
    `launch_items`.

  A stack's GPU time is bounded by its rounds, not by a worker's budget.
- **The check of every stratum** (`one_launch`, test builds) is now:
  - its launches are one program's stacks and remaps, none a rerun;
  - its stacks after the first are at most its rounds.

  The test `a_stratum_is_one_programs_stacks` runs the closure twice:
  - with the stacks' default rounds: its relaunches are fewer than half its rounds;
  - with a stack of one round (`RERAIL_STACK_ROUNDS=1`) and a first arena of 16 bytes: a stack a round, its arena's
    pages mapped as its claims reach them.

## Measured

On the M3: decision 0603's measure (`measure_one_program_against_a_launch_a_level`, seven warm runs, medians), its
GPU time against the launch a level and 0637's program.

| Stratum | Launch a level | 0637's program | Stacks | Stacks' launches |
|---|---|---|---|---|
| seeds | 11.3 ms | 15.2 ms | 10.8 ms | 1 |
| seeds ×10 | 47.0 ms | 103 ms | 56.8 ms | 1 |
| closure | 11.3 ms | 28.1 ms | 526 ms | 10 |
| leveled reach | 9.0 ms | 33.5 ms | 298 ms | 7 |
| leveled dist (min) | 5.8 ms | 17.4 ms | 127 ms | 5 |

The seeds are one launch and at a launch a level's GPU time. The recursive strata are many times slower:

- **The bound's groups.** A phase of segments starts 4,096 groups, most of which return at once. With a bound of
  256, the closure took 155 ms; at that bound two text tests failed, so 4,096 stays.
- **A dispatch more a phase.** Each phase has its `join_grid`.
- **A stack's rounds past the component's end.** These still run their empty dispatches.
- **The host's wait.** The closure's wall is 4.3 s against 0.53 s of GPU time: its stacks' submissions, a few
  thousand dispatches each, wait long before the GPU starts them.

The way back to a launch a level's time is each phase's exact groups, Metal's indirect dispatch included. Its hang on
the M3 is not explained yet.

Under the stacks, two stride faults showed that a large bound had hidden: a radix pass ran only its own tile, and a
scan's or a radix pass's tiles reused the group's memory with no barrier between them. Both now stride and wait, as a
segment past its share must.
