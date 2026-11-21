# 0540. A device wait is on a thread that is running, and every persistent launch has a budget

Status: decided (2026-11-20), by the user: "It sounds like we need a better solution for device-side enqueuing than
busy waits. What can we do? can we make the kernels discover their order based on an atomic counter and do the work
in those? Can we make a thread which wants to spin instead cooperatively take on part of the work it's waiting for?",
then "please do 2 and 3 first": 2, the 16z cache's waits; 3, the persistent grid's and a budget for each launch.
Amends `view::cache` (decision 0353 step 4b): "a miss waits for no thread" becomes a bounded wait for a writer.
Follows 0524 point 5 (a wait is a turn of its function's one loop).

## Intro

A GPU makes no promise that a thread which waits for another will see it go on. Two cases are not a matter of
luck, and the rest are:

- **A lane of the waiter's own SIMD group.** On Apple's GPUs a group's lanes do not each go their own way: lanes
  that go round a loop stand the rest of the group still (0524 point 5). A lane that waits for its own group waits
  for ever.
- **A group that is not resident.** A group not yet running runs only when a running one ends. A group that waits
  for it, and every other one that does too, keeps it from running.
- **A group that is running** goes on: no GPU stops a resident group for good while others spin (none promises it;
  Apple11's "compute shader forward progress guarantees" are the first that do).

So a wait is safe when the thread waited for is running and waits for nothing itself, and is bounded either way.
What a bound per wait does not bound is a launch's total GPU time: a command buffer that runs for a minute stops
macOS's compositor, and its watchdog reboots the machine (the M5 Max, twice on 2026-11-20, the C preprocessor's
persistent grid; the M3, on 16z reads that spun out). Killing the process does not stop it.

## Decision

1. **A device wait waits for a thread that is running and waits for nothing**, within a turn of the waiter's one
   loop, a bounded number of turns. Not on its own SIMD group, and not on a group that may not have started.
2. **The 16z cache's lookup waits for a block's writer** (`clif_tc_get`): the writer claimed the block in its own
   take, so it is resident, and from its claim to its entry's completion it decodes and stores, waiting for nothing.
   A lane of the writer's own group is never seen waiting: a group's take ends within the turn of its claim. After
   SPIN turns it decodes an entry of its own. Neither of the ways not to wait does better, measured on a DGX Spark's GB10:
   - **An entry of its own (a duplicate)**, through the device test's cache sized for the whole tape (7 blocks, room
     for 8): 477 failed lookups, the cache full and turned once. Waiting: none, 7 misses, no turn. A GPU's threads
     that miss one block at once are many (every group that starts on it), and their copies fill the head.
   - **Into the pending entry, beside its writer**, through a cache of three generations of one block fewer than
     the tape's, 4,096 threads reading 16 nodes each at random:

     | A lookup that finds its block's entry being written | Time | Failed lookups | Wrong reads |
     |---|---|---|---|
     | decodes into it, beside its writer | 20.7 s | 102,537 | 52,977 |
     | the same, in the statistics mode | 21.8 s | 121,577 | 59,149 |
     | waits, in turns of its loop (statistics mode) | 7.2 s | 0 | 0 |

     Every waiter decodes the whole block: 1,268,011 decodes for 353,121 hits. And two decoders of one entry undo
     each other's bytes: LZ4's match copies run 8 bytes past a match, into bytes a later sequence writes, so a
     decoder behind writes over what one ahead has written and read (node 65's code, read wrong, until every copy
     was made exact).

   Work a waiter takes on must be split, not repeated: a block's decode shared by its SIMD group's lanes (step 4)
   is that.
3. **A persistent grid's groups are the groups that are running**, found on the device (step 2): each group takes a
   ticket from an atomic counter as it starts, the first closes the count after a short window, and only the groups
   that took a ticket before it closed take part, numbered in ticket order. A barrier waits only for them. The host
   no longer guesses the resident count and halves it on a failure.
4. **Every persistent launch has a budget** (step 3), the user, 2026-11-21: "we need to relaunch when work is left.
   And the workers should be able to take more than their share if they finish faster, but up to a bound. The goal
   is to ensure that we can't ever wait forever." And the launches are "a limited, bounded number of launches and a
   limited occupancy of a single run", not a fixed count. A worker of a work queue takes items up to its budget, a
   few shares of a launch's items, then ends, as an idle one does; the host launches again while any are left
   (`control[2]`). A launch's GPU time is so its workers' budgets', not the work's, and the launches are bounded: a
   launch that leaves items had a worker that spent its budget, so it took that many at least, and a linear queue
   holds at most its capacity.

## Steps

1. **Done (2026-11-20): the 16z cache's waits.**
   - `clif_tc_get` waits for a pending entry in turns, as before. Two fixes:
     - a lookup whose wait ran out now takes an entry of its own (`own`). Before, its take found the block's
       pending slot and took nothing, and the lookup spent its tries and failed;
     - a fill that fails (a promotion whose source turned under its copy) gives its slot back as no key's (its
       number with no tag). Before, the key's tag stayed in it: every lookup of the block in that epoch found the
       entry neither complete nor pending and spent its tries.
   - The probe looks past a pending slot, so it finds a complete entry past it.
   - The CLIF (the host's) has both fixes too.
   - A new device test, `the_device_reads_16z_through_a_cache_that_turns`, reads at random through a cache that
     thrashes. Each thread checks the cache's failure count after each read: no read is wrong while it is 0. It
     passes on the Mac mini's Metal, a DGX Spark's CUDA and a Linux host's CPU. On Metal most of its lookups fail
     (a SIMD group's turn outlasts several turns of the cache, and after the first failure a pending lookup does
     not wait); on CUDA a few do. A batch's cache is sized for its blocks (`Spec::sized`), so it thrashes only past
     its budget, where its batch is refused (0463).
2. **Done (2026-11-20): the persistent grid.**
   - `ukl_grid_join` (point 3) in `<ukl/grid.h>`: a fourth control word holds the tickets, and `UKL_GRID_CLOSED`
     once the first group closes the count, after `UKL_GRID_WINDOW` (2,048) polls with no new join, or all joined.
     A longer window only slows a dispatch past residency: at 65,536 polls, 843 ms on a GB10 against 30 ms. The CPU
     backend runs a dispatch's groups one at a time, so its grid is one group.
   - **A wait reads by polls** (`ukl_atomic_poll`, on every backend; a read-modify-write that changes nothing on
     Metal). The barrier's tests failed at 8 groups on the M1 Max (Q367), and on the Mac mini too. A trace of each
     group's view showed why: at the first barrier the last group stepped the generation, and every other group
     polled it a million times and saw it unchanged, while their compare-and-swaps after found it stepped. That is
     Q123, the M1 Max's stale polls, which the block kernels' look-backs already poll around (`BLOCK_POLL_RMW`).
     The grid's words are read only by polls now, and so are the 16z cache's words where a stale copy breaks its
     protocol (a writer's stamp check, a clear's writers, a promotion's source) and in a wait's turns, and the work
     queue's head, tail and active count (`<ukl/workqueue.h>`, `work_queue.metal`).
   - `a_grid_past_residency_fails_its_barrier` is `a_grid_past_residency_is_the_groups_that_run`: 4,096 groups of
     1,024 dispatched, 24 took part on the Mac mini (in 9.8 ms), 48 on a GB10, 1 on the CPU, and every barrier passed.
   - The barrier, scan, segmented scan and sort tests pass on the Mac mini's Metal, a DGX Spark's CUDA and a Linux CPU.
     They stay ignored (Q367) until the M3 and the M1 Max pass them (step 5).
3. **The budget.**
   - **Done (2026-11-21): the queue and the lattice.**
     - `<ukl/workqueue.h>`: `UklWorkQueue.budget`, a worker's items in a launch at most (a SIMD group's takes that
       got any, for `ukl_wq_take_sg`); `ukl_wq_spent` says when a worker has spent it.
     - `metal_work_queue::run` relaunches within `capacity / budget + 1` launches, not 8. `budget` is 4 shares of
       2^20 items over the workers: 820 visits a worker for the lattice test's 20 groups, which still takes one
       launch.
     - The lattice's workers stop at it.
     - `a_work_queue_budget_ends_a_launch_and_the_next_takes_the_rest`: at a budget of 1 and 4, every node of a
       100,000-node tree is visited once, in more than one launch and within the bound. It passes on a Linux CPU,
       a DGX Spark's CUDA and the Mac mini's Metal.
   - The label passes' spill queue needs no item budget: its frames are walks' pieces, and every walk is within
     its task's step budget (decision 0334), past which it stops and goes to W or runs again.
   - **Still to do: `pp_expand`'s workers.** Its host already encodes a bounded number of work launches
     (`WORK_LAUNCHES`) and refuses what is left (`px_check`). Its workers are Metal-only, and pp runs on Metal are
     off until they are allowed again.
4. A 16z block decoded by its SIMD group's lanes together, so a miss costs a group a block once.
5. The M3, then the M1 Max: the grid's tests, the device's tape tests and the engine's Metal tests, one at a time.
   - **The M3 (2026-11-21):**
     - These pass: the work queue's tests (the budget's among them), every grid test (the barrier, past residency:
       120 groups of 4,096 took part, the scans and the sort), and `the_device_reads_16z_through_a_cache_that_turns`.
     - `the_device_reads_read_every_node_as_the_crate_does` fails on 16z, through a cache sized for the whole tape.
       In the statistics mode: 35 misses for 7 blocks, 96,723 hits, 2,670 waits that ended, 135 duplicates (waits
       that ran out their 65,536 turns), 5 turns of the cache, then 1,594,231 failed lookups, all in 0.37 s.
     - A decode of a block takes well under a millisecond, so the writers waited for stood still while their
       waiters spun. Point 2's premise, that a claimant goes on, does not hold on the M3. The bound held: nothing
       hung.
