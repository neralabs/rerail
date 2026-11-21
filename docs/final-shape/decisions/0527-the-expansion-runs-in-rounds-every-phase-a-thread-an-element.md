# 0527: The expansion runs in rounds, every phase a thread an element

Status: accepted (2026-11-20). Replaces 0448 §3's work queue. The user: "We are still laughably
slow because YOU ENTIRELY FORGOT THAT GPUS NEED TO BE PARALLEL. at each phase, all work needs to be
parallel"; "you may not run tests over 1 second; none of these cases should take more than a few
milliseconds"; then, to the plan below, "go ahead". Numbered past the highest in any worktree
(0522); 0527 in the hub, past its 0526 (0523 on Go-C-and-C-support).

## Intro

The design agreed on 2026-11-19 is five parallel passes:
1. the file, with a prefix sum for the line offsets;
2. the lines, for the definitions;
3. the evaluation, recursion included;
4. a prefix sum for the output's offsets;
5. a scatter.

0482 says how step 3 recurses: "each expansion round substitutes one level for all invocations,
then count → scan → scatter".

What was built kept the setup parallel and the expansion serial:
- **A queue with one SIMD group a list.** 0448 §3 chose it only because a round would have cost
  about 40 launches, against the 200-launch bound. 0468 removed that reason (one persistent kernel,
  barriers between its phases), and the queue stayed.
- **Its parallelism was the lists ready at once, not the tokens.** A unit with a few top calls ran
  a few SIMD groups while the rest polled an empty queue (up to 4,096 polls each).
- **A chain went one list at a time,** and a long list 32 tokens at a time on one SIMD group.
- **0486 and 0520 §4 added pauses between the queue's runs** to give stores back as a chain grew.

This decision is the expansion as rounds, every phase of a round one thread an element.

## 1. Rounds

A top's expansion is a sequence of rounds on the persistent grid:
- A round takes the items the round before made. Each item is a list's scan, a call's arguments,
  a substitution or a place, as in 0448 §3.
- It runs them all, then ends at a grid barrier.
- An item made in a round runs in the next. Dependencies are still counts (a call's last argument
  list final makes its substitution), so a round's items never wait on each other.
- The rounds end when a round makes nothing. Their number is the expansion's depth (a chain's
  length times the items a level), not its tokens.

**The items' store** is two halves of `Q_CAP` slots: the round's, and the next's (`QCTL` 1, taken by
an atomic add). `QCTL` 0 counts the items the top's rounds ran, against the work bound (`WORDS` 14,
`X_WORK`). A next round past its half is room (`X_ROOM`, `QCTL` 3), and `ctl` 13 is the largest
round, the room the next run is sized from.

**Giving back (0486 §10) is between rounds,** where a store's front passed its mark. No item runs
then, so the pause needs no stop between items. The ring's rebase is gone with the ring.

## 2. Every phase a thread an element

A round's items of one kind run as grid phases over all of them at once. Each element is one
thread: a token of every list the round scans, an argument token of every call, a body item and
then a written piece of every substitution, a piece of every place. A list's position comes from
a scan of the items' counts and a search. Within a list, the work that was a 32-lane walk becomes
the grid's:

| Item | Phases, each a thread an element |
|---|---|
| scan | classify (a name's definition by search, its paint by its chain); depth, a segmented scan by list; each `(`'s `)` by a search of the round's close keys (list, depth, place), sorted as the top level's are; the outermost calls, a segmented max scan of their ends; the calls' lists, a scan of their flags |
| call | the splitting commas, a segmented scan of depth by call; the arguments' ranges, a scan of the commas; the parameters used expanded, a thread a body item |
| substitute | each body item's pieces counted; a scan; each piece written by its own thread; each `##` chain folded by its head; the compaction, a scan of the kept pieces |
| final, place | a list's size from its live children, the rollback's claims as a segmented max scan; then the places top-down, a scan of the pieces' sizes |

The pieces (0520) are what every phase writes: references to the inputs and to what a round made,
never copies.

## 3. On the CPU

The CPU backend runs a launch's groups as coroutines on one host thread. Every barrier and every
SIMD-group operation switches each thread of the group.
- **One group** on the CPU (`pp_arena::groups`): past one, a group adds no core, only its
  threads' switches at every barrier.
- **256 threads a group,** as on a GPU: the radix sort's tiles are written for
  `UKL_RADIX_THREADS`. A 32-thread group cut a small unit's time from 10 to 3 ms, but its sorts of
  more than 32 keys took the tile path and faulted.
- **A SIMD-group operation is one wait.** The CPU had no native reductions or scans: each was five
  shuffles of two waits, so a sum suspended all 256 coroutines ten times, and the small sort's 32
  sums a SIMD group were most of a round. Now a reduction, a scan, a segmented scan, a shuffle and
  a ballot each write the lane's value to one of two banks and wait once (the next operation
  writes the other bank, so no second wait guards the first's reads).
- **The scheduler counts its waits.** A waiting thread hands on to the next runnable one (one
  switch a wait, not two), and the counts of threads at the barrier, at each SIMD group's
  operation and done replace three scans of the 256 threads at every release (22% of a run's
  samples before, under 1% after).
- **One program a source:** each kernel of the preprocessor's source was its own program,
  preprocessed and loaded again (`accel::gpu::kernels::elsewhere`, keyed by the kernel's name). Now
  the program is keyed by the source's library key.

## Requirements

| | Requirement | Test |
|---|---|---|
| R1 | Every evaluation test finishes within 1 s, under cargo on the CPU backend | `cargo nextest run -p rerail-c-directives --lib --profile second` (stops each test at 1 s) |
| R2 | The expansion's tests give the serial pass's tokens | the eval tests (0448 R5–R8, 0485, 0486, 0520) |
| R3 | A round's items run on every SIMD group of the grid; nothing polls | `px_worker` takes its share by index, with no queue |

## Steps

1. **Rounds in place of the queue,** the items' bodies as they were. Done: all 31 eval tests pass
   under cargo (CPU backend, a farm Mac). The times went from what the queue took to what the
   rounds take, then with one CPU group:

   | Test | Queue | Rounds, one CPU group |
   |---|---|---|
   | `a_unit_past_its_room_runs_again_larger` | 55.0 s | 32.7 s |
   | `an_evaluation_is_a_fixed_handful_of_launches` | 20.0 s | 9.5 s |
   | `a_chains_use_is_flat_in_its_length` | 10.8 s | 3.1 s |
   | `reclaimed_tokens_keep_a_chain_at_rung_0` | 8.9 s | 8.2 s |
   | `calls_past_an_end_are_the_serial_passes` | 7.6 s | 0.63 s |
   | `a_flat_argument_is_read_in_place` | 6.9 s | 0.35 s |
   | `a_batch_is_its_units` | 6.6 s | 0.26 s |

   On the CPU backend, under Relax, a small unit's device time is 10 ms, and a 4,500-token unit's
   went from 58 ms at 8 groups to 33 ms at one.
2. **The scan as grid phases** (§2's first row), then the substitution, the call, and the final and
   place. Each kind's SIMD-group body goes when its phases pass the same tests.

   Done so far:
   - **The scan,** as §2's first row.
   - **The library's group primitives without per-step waits.** A threadgroup's segmented scan is
     each SIMD group's, then the carries from the SIMD groups before (3 waits a tile, not 18). The
     small sort ranks a row a thread against all the rows (no wait, not 32 SIMD sums).
   - **Fewer rounds a level.** A call whose arguments need no list substitutes in its own round.
     A list made in a round (an argument's) is scanned in that round's scan.
   - **`ctl` 25 counts the rounds** (`Expanded::rounds`, printed under `RERAIL_PP_TIME`).

   Measured on the CPU backend, here (an M-series laptop, one CPU group), device time:

   | Unit | Rounds | Before step 2 | Now |
   |---|---|---|---|
   | A chain of 150 plain levels | 302 | 232 ms | 122 ms |
   | 150 levels, an argument scanned at each | 902 | 648 ms | 344 ms |
   | 150 levels, an exit at each | 452 | 400 ms | 206 ms |

   About 0.4 ms a round. What was left per round was the item bodies, a SIMD group a list: a
   list's final climbed its chain on one SIMD group walking each level's pieces, and the places
   went top-down a level a round (a chain took two rounds a level).
   - **The final and the place are the walks** (§2's last row), `px_wk_*`. `px_done` and a place
     add the list to a set (`px_set_later`, two halves as the items'), and after its scan a round
     walks the set it made final, then the set it places, an iteration a set until one makes
     none. Each iteration is phases over all its lists' tokens:
     - a thread a list for its children in order (the rollback's exit-bearing claims decided one
       after another, as the SIMD walk's ballot did);
     - segmented max scans for the claims and the children's ends;
     - a thread a token for its life, its source piece and its paint;
     - segmented sums for its places;
     - the runs' lengths;
     - the writes, and the children's bases.

     `px_walk`, `px_final` and `px_place` are gone.
   - **A set a group holds is group 0's.** An iteration whose lists and tokens fit
     `UKL_GRID_SMALL` runs on group 0 alone with its own barriers, one such after another, so a
     chain's climb crosses no grid barrier a level. The grid joins for a larger set or the end.

   | Unit | Rounds | Device time |
   |---|---|---|
   | A chain of 150 plain levels | 152 | 188 ms |
   | 150 levels, an argument scanned at each | 452 | 540 ms |
   | 150 levels, an exit at each | 302 | 319 ms |
   | A 32,768-token `#if` line, a run | 12 | 720 ms |

   The rounds halved, but the CPU's time did not: the climb and the places still cross a phase's
   barriers a level (a group's barrier switches the CPU's 256 coroutines as a grid's does). The
   `#if` line's time is `px_subst`'s body, a SIMD group a list (half its samples).

   Next: the substitution, then the call, as grid phases.
3. **The tests past 1 s brought under it:** what each one runs is sized to what it checks.
