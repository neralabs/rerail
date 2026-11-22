# 0603. A stratum's device work is one launch: a work queue whose phases and rounds are continuations

Status: accepted (2026-11-22), steps open. Follows 0601 (every launch counts, and past the budget is a hard failure)
and takes 0540 step 6's scheme (a level's formulas as a work queue with continuations) to the device joins
(`metal_join::Batch`, `setfix::device`).

## Intro

After 0601's levels, a stratum's joins launch each kernel once a level for all its rules. A level is still several
launches (the copies, the indexes' bucket, sort and place, the maps and counts, the scan, the scatters), and the
host turns between them for nothing but to launch the next. A recursive stratum is worse: its rounds are host
turns. The host reads each round's counts, sizes the next round's tables from them, and decides whether to go on, so
its launches are its rounds times a round's (about 30), and a recursive stratum of 7 rounds passes the 200 an
evaluation has. The user, 2026-11-22: "Cut join launches first, then do the recursion on the device."

Measured at 0601's levels with the copies and indexes fused (spark2, the CPU backend, the setfix device tests): 33
of 36 pass; the three that fail are the recursive strata (`a_recursive_stratum_on_the_device_is_the_cpus_fixpoint`,
`a_leveled_recursion_on_the_device_has_the_cpus_levels`, `parity_combinators`), each past 200 with 12 to 16 dedup
levels, 9 to 14 copy levels, 6 to 12 index sorts and 59 to 120 library launches (the scans' and sorts').

## Decision

1. **A stratum's levels are a program, run by one launch.** The host encodes the levels as it does now (the jobs,
   `JobDesc`), as phases: a phase is a kernel kind over its jobs, each job's threads in groups. A worker kernel takes
   an item (phase, job, group) from a queue (`<ukl/workqueue.h>`) and runs the job's body (the `_one` bodies
   0601's levels made) for the group's threads. The group that finishes a phase's last item pushes the next phase's
   items and takes on; nothing waits for another group (0540 step 6, the user's: "they should enqueue a continuation
   and pick up more work rather than spin"). A level's phases follow one another as its launches did, and a
   level's last phase pushes the next level's first.
2. **The scan and the sort are phases too.** The level's scan is its blocks' sums, the sums' scan, and the blocks'
   offsets added. An index is its buckets counted, their scan (the index's offsets), its rows placed in their
   buckets, and each bucket's rows put in id order: the stable sort's index, the host builder's, with no library
   sort.
3. **A recursive stratum's rounds are a loop in the program.** The round's last phase's finisher reads the round's
   new rows (a word of the program's). With any, it resets the round's words, makes the new rows the next round's
   delta, and pushes the round's first phase again; with none, the program ends. The relations' rows and counts are
   device words the stages read (a stage's probed rows from a word, not the host's `nr`); the host reads the
   program's words once, at its end.
4. **Sizes are bounds the host chooses.** Each table has a capacity. A stage that passes its capacity sets the
   program's overflow word and pushes no further phase; the round's relations are as the round found them. The host
   grows that table and launches again from that round: a rerun (0540).
5. **Every launch of it has a budget** (0540 point 4): a worker takes items up to its budget and ends; the host
   launches again while items are left. The launches are bounded by the program's items over the budget, not by its
   rounds.

## Steps

1. A stratum's levels as one launch (non-recursive first): the program, its worker, the scan as phases; the setfix
   seeds test at one launch a stratum and a few for the queue's control.
2. The indexes as phases (point 2).
3. A recursive stratum's rounds as the loop (point 3), the stages' relation rows read from words; a set head's new
   rows (union, distinct, less its relation's) in the round's phases.
4. A lattice head's fold as phases.
5. Overflow and the rerun from its round (point 4).

## Tests

- The setfix device tests on the CPU backend and on Metal, each stratum within the budget: the three recursive ones
  included, with no test changed.
- A program's launches: one stratum, one launch and its budget's relaunches, whatever its rules and rounds.

## Done (2026-11-22): steps 1 and 2

- **The program.** `Batch::flush` appends each level's phases to the stratum's program (`metal_join::Program`): the
  copies; each index's keys, its radix passes and its place; the dedups' clear and the maps; the claims; the counts;
  the counts' scan; the scatters. A job's `base` is the program's, not the level's, so one counts buffer serves every
  level. `run_program` makes the program's buffers (the jobs, the items, the phases, the scan levels, the sort
  passes, the queue's control and slots, the counts) and launches `join_run` once, over as many groups as the first
  phase has items. A group takes an item, runs its job's `_one` body for its threads (`prog_item`), and the group
  that finishes a phase's last item pushes the next phase's items. `finish` reads the program's control word: a
  program that did not reach its last phase is a failure.
- **The scan and the sort as tile phases.** The bodies of `<ukl/scan.h>` and `<ukl/radix.h>` are tiles
  (`<ukl/body/scan_tile.h>`, `<ukl/body/radix_tile.h>`), included once with global pointers for the library's
  kernels and once with the program's for its phases. Point 2's index is the radix sort's passes (the library's
  stable sort, as phases), not a bucket sort of its own: one sort.
- **Coherence.** A phase reads what other groups of the same launch wrote, so on Metal the program's blocks are
  `coherent(device)` (MSL 3.2). `<ukl/clif.h>`'s `CLIF_BLOCK_SPACE` is `UKL_COHERENT` in a kernel that defines
  `CLIF_BLOCKS_COHERENT` (the program's variant: `JOIN_PROGRAM`), else `UKL_GLOBAL`; the reads written in CLIF
  (`rerail_clif::ukl`) take a relation's or a line's block in it, and `<ukl/memory.h>`'s atomics take a coherent
  base too. The views and the tape stay global: nothing in the launch writes them.
- **Measured** (the setfix device tests): spark2's CPU backend, 35 of 36; the mini's Metal, 23 of 24. A
  non-recursive stratum is one launch; `a_recursive_stratum_on_the_device_is_the_cpus_fixpoint` and
  `parity_combinators` now pass within the budget (one launch a round). The one left is
  `a_leveled_recursion_on_the_device_has_the_cpus_levels`, at 83 program launches (its rounds) and 117 of the
  lattice's (`lat_*`): steps 3 and 4.
