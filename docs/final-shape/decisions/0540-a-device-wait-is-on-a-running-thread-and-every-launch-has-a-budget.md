# 0540. A device wait is on a thread that is running, and every persistent launch has a budget

Status: decided (2026-11-20), by the user: "It sounds like we need a better solution for device-side enqueuing than
busy waits. What can we do? can we make the kernels discover their order based on an atomic counter and do the work
in those? Can we make a thread which wants to spin instead cooperatively take on part of the work it's waiting for?",
then "please do 2 and 3 first": 2, the 16z cache's waits; 3, the persistent grid's and a budget for each launch.
Amends `view::cache` (decision 0353 step 4b): "a miss waits for no thread" becomes a bounded wait for a writer.
Follows 0518 point 5 (a wait is a turn of its function's one loop).

## Intro

A GPU makes no promise that a thread which waits for another will see it go on. Two cases are not a matter of
luck, and the rest are:

- **A lane of the waiter's own SIMD group.** On Apple's GPUs a group's lanes do not each go their own way: lanes
  that go round a loop stand the rest of the group still (0518 point 5). A lane that waits for its own group waits
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
   SPIN turns it decodes an entry of its own. Neither of the ways not to wait does better, measured on spark1's GB10:
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
4. **Every persistent launch has a budget** (step 3): a worker of a work queue takes at most a launch's share of
   items, then ends, as an idle one does; the host launches again while any are left (`control[2]`), as it does
   now. A launch's GPU time is so its budget's items, not the work's.

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
     passes on the Mac mini's Metal, spark1's CUDA and a Linux host's CPU. On Metal most of its lookups fail
     (a SIMD group's turn outlasts several turns of the cache, and after the first failure a pending lookup does
     not wait); on CUDA a few do. A batch's cache is sized for its blocks (`Spec::sized`), so it thrashes only past
     its budget, where its batch is refused (0463).
2. The persistent grid: `ukl_grid_join` (point 3) in `<ukl/grid.h>`, its tests in `gpu`'s library tests.
3. The budget: the work queue's workers (`label_passes`, `lattice`, `pp_expand`) end after their launch's share.
4. A 16z block decoded by its SIMD group's lanes together, so a miss costs a group a block once.
