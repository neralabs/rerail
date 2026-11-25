# 0679. A program cut from another runs once that one is whole

Status: done (2026-11-24).

## Intro

On Metal, the 300-file corpus lost findings in some runs: about 1 run in 3 gave fewer than the CPU's 2,183. The
CPU gave 2,183 every time. This record traces the loss to its cause and fixes it there.

A batch's device work runs as programs (decision 0603). Each stratum is one program, run in stacks (decision
0648). A program can stop partway and be launched again from `Batch::finish`. This happens in three cases:

- a park: its size script's claims passed its arena, and the arena is mapped further;
- a text-arena park;
- a gate's room.

It can also go on to its next stack.

`Batch::cut` ends a program early. It does this when a job reaches a block through a wrap made since the program
began, and the batch cannot launch again at a new wrap (a waved batch). The work after the cut goes into a new
program, which reads what the first one wrote: its placed tables, its counts, the indexes it builds.

## The cause

The program after a cut was launched at once. The design assumed it would run after the first on the one queue,
and it did. But the first program might park and only finish later, at `finish`. The second had then already run:

- It copied by a count still at 0, from a table not yet placed.
- Nothing launched it again.

Probes on the M3 showed this in a waved stratum's heads:

- `encode_place`'s final copy (`place_rows`) fell after a cut, in the program after the stratum's.
- The stratum's program (85 phases) parked at its arena.
- The copy ran before the program's later phases, so the head rooms kept their old bytes. A sentinel written into
  a room's row 0 was still there at `claim`.
- The host took those bytes as the heads' rows.

The evidence was consistent:

- With a first arena of 64 MB, so that no program parks, 14 of 14 runs gave 2,183.
- Every short run had a parked program that a cut followed.
- Two other suspects did not change the result: keeping freed pages, and binding the blocks' current wraps at a
  relaunch.

## The fix

A program that goes on from another is held until every program launched before it is whole.

- `Program::continues` marks it:
  - `cut` sets it on the program that follows;
  - so does a probe of an index an earlier program of the batch builds (`index_of`).
- `run_program` holds such a program (`Batch::held`) when any program is still out or already held. Its builders
  are marked with its control words, as its launch would mark them, and `Building::held` keeps them from being
  told to other batches. No submission committed before its launch holds their work.
- `finish` waits for the programs launched and launches each one's agains. Once none are left, it launches the
  first held program. That program's builders are told with its submission, and the loop goes on.
- `launch` already rebases an arena that moved and places texts again, so a held program launches as any
  relaunch does.

When a program fails, the held ones are dropped with the batch's other work, and its strata run on the CPU, as
before.

A first try marked the held builders `queued`. Another batch's probe then found no `InFlight` record for them and
panicked ("a committed build is told to its block"). `Building::held` replaced that mark.

## Results

- **M3, sub300 corpus, Metal:**
  - 16 runs with the fix: 2,183 each, no panics, 4.9 to 6.3 s each.
  - Interleaved with runs where the hold was switched off (a probe), the times were the same (about 5.8 s
    either way). On those runs the loss did not show: about 1 run in 3 loses findings.
- **Test:** `a_program_cut_from_one_that_parks_reads_its_rows`.
  - It sets a first arena of 16 bytes, places a dedup's rows in a program, cuts, and copies them by their count.
  - With the hold switched off, it fails: the count is right but the rows copied are not. With the hold, it passes.
  - It ran on the Linux CPU backend, as Metal tests are paused (2026-11-24).
- **Tests:** device_metal's transducer and setfix-device tests, 70 of 70.
