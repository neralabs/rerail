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
     passes on the Mac mini's Metal, spark1's CUDA and a Linux host's CPU. On Metal most of its lookups fail
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
   - The barrier, scan, segmented scan and sort tests pass on the Mac mini's Metal, spark1's CUDA and a Linux CPU.
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
       spark1's CUDA and the Mac mini's Metal.
   - The label passes' spill queue needs no item budget: its frames are walks' pieces, and every walk is within
     its task's step budget (decision 0334), past which it stops and goes to W or runs again.
   - **`pp_expand`'s workers:** another agent's work (2026-11-21); a budget written here for them was taken out
     again, its files as they were.
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
     - **The experiments (2026-11-21), each one run of that test on the M3:**
       - *Not a lane waiting for its own SIMD group.* With one lane in 32 reading (the rest returned at once) it
         failed worse: 828 waits ran out, none ended with its block found, 631 turns of the cache.
       - *Not a writer that waits for a turn on its core.* Waits that read memory one turn in 64, over 64 times the
         turns, and that stalled on loads of the tape's own bytes between: 15 s of waiting, all 7 blocks filled, and
         still 8,510 waits ran out.
       - *What a wait that runs out finds.* Every one (117 of 117) finds a slot of its key claimed, with no entry
         yet. At the first, no writer is counted in the head, and every claim made so far has been settled (given
         an entry or given back). The claimed slot read complete a moment before, the same way on every run. An
         add of a zero the compiler cannot know, a poll and a load all agree that it then reads claimed. No clear
         touched the head, and no decode or copy wrote past its entry.
       - *Takes that find the head full with 8 entries.* 1,667 claims in one run found no free slot among their 16
         probes while most of the head's slots were empty: the cache turned with 8 entries in it. A lookup's first
         turn reads the epoch and the take's stamp check by loads, which may be stale copies (Q123). With the stamp
         check a poll and every turn after a failed try polling, such claims were 57 to 2,673 a run, and the test
         still failed.
       - *A won claim is lost (2026-11-21, a log of every write to the cache's index, in the dead entry).* With one
         read a node and the decode skipped, the 7 blocks' first claims are made at once, each by lane 0 of its own
         SIMD group, into slots of one 128-byte line. Every run, the same 4 of them are lost: right after the claim
         loop the claimant's slot reads 0, or (two blocks whose probes meet) the other block's tag, and no logged
         write was made to it. Inside the loop a second compare-and-swap, of the tag for the tag, still finds the
         tag. At that moment no generation's index holds the 4 losers' tags anywhere, while it holds the 3 others'.
         Two claims of one slot both found it 0, by compare-and-swap and by `fetch_or`, which coherent atomics
         cannot do. So the losers' atomics act on a copy of the line that no load sees, and that never reaches the
         buffer. Every write of the cache's code to an index slot is logged, and none explains it.
       - *What does not change it:* one lane of a SIMD group's swap at a time, every store an exchange, the cold
         reads inlined, a claim by `fetch_or` in place of a swap, and the cache's atomics on `coherent(device)`
         words. The counts are the same to a few in every run.
       - *What differs:* the M3 Max runs macOS 15.7.7 (Metal 3). The Mac mini (an M1) runs macOS 27.2.1 and the
         M1 Max macOS 26.2 (Metal 4), and both run the same instrumented kernel cleanly: 7 claims, 7 numbers. Metal
         compiles the source on the device, so the M3's compiler and driver differ from theirs, as well as its GPU.
         A runtime compile cannot turn Metal's optimizer off, so the M3 itself on macOS 26 is what separates the two.
       - *Not the compiler (2026-11-21).* Each Mac's own runtime compile of the kernel's source (kept by
         `RERAIL_METAL_SOURCE_DIR`), in a binary archive: its AIR, from macOS 15.7.7 and from macOS 27.2.1, is the
         same instruction for instruction, but for the operand Metal 4 adds to each atomic. The claim is one relaxed
         device-scope swap. And the M3's AIR compiled for G15 by Xcode 26's back end (`metal-tt`), run on the M3
         from that archive (`RERAIL_METAL_BINARY_ARCHIVE`, the archive's code confirmed), loses the same 4 claims.
         So the front end does not make the difference, and neither does the back end's version: both back ends
         compile the code for G15 otherwise than for G13. A user on an M3, whatever their Metal version, may meet
         it. Apple's tools have no disassembler for G15 code ("no instruction printer for target agx3").
       - **Found (2026-11-21): two address foldings of G15's back end, by reduction.** The kernel was cut down on
         the M3, one piece at a time, while the loss still showed (a harness that compiles a `.metal` file and runs
         it once on the test's dumped inputs). Both come from how the back end folds an address computed from a
         loaded value; both reproduce in kernels of a few lines, from the runtime compiler of macOS 15.7.7 and from
         Xcode 26's `metal-tt`.
         - *The claim's swap.* A compare-and-swap at `gen + ((h + p) & (slots - 1)) * 4 + INDEX`, with `slots` read
           from the header, acts on another word than its address: two threads both win one slot, and a load of the
           slot after finds neither's tag. With the index masked by a constant as well, it acts on the slot. A
           constant mask up to 2^29 - 1 is enough; 0x3FFFFFFF is not. So the slot's index is masked by
           `view::cache::SLOTS_MAX - 1` too (131,072 slots, the most `Spec::new` makes for the most entries), in
           `tape_body.h` and in the CLIF, and `Spec::check` and `init_zeroed` refuse a cache of more slots.
         - *The decode's match.* LZ4's copy reads at the output's place less the match's offset, a zero-extended
           16-bit load: `dst + at - zext16(off)`. G15 folds it as `dst + at + (uint16)(-off)`, a place 2^16 bytes
           past the source, so every match read the entry's zeros. This is what failed the blocks once the claims
           were kept. The decode now computes its places as 32-bit offsets from the output's start and adds each
           to `dst` (`dst + (uint64_t)(uint32_t)(at - off)`), which G15 compiles right.
         - With both: `the_device_reads_read_every_node_as_the_crate_does` and
           `the_device_reads_16z_through_a_cache_that_turns` pass on the M3 Max, the Mac mini and the M1 Max, and the
           host's tests (the CLIF's reads, which gain the mask, and the generated relation reads) pass on Linux.
         - Other device code that subtracts a loaded narrow value from an address, or swaps at an index masked only
           by a loaded count, may meet the same foldings.
     - **The engine's Metal tests on the M3 (2026-11-21, `--features metal-tests`, one at a time).** Of the 118
       conformance tests (`metal_semantic_conformance`), 117 pass. Of the 15 others, 10 pass. What fails, and what
       the M1 Max (macOS 26.2) says of the same tests:
       - *M3 only: the step budgets' tests* (`a_misplanned_label_sweep_runs_again_and_finds_the_full_budgets_roots`,
         `on_metal_an_unfinished_pass_goes_to_w_where_w_costs_less_and_runs_again_where_it_does_not`): no task runs
         over its budget. Both pass on the M1 Max. The same evaluation (one program, one file, 34 tasks, a budget of
         256 steps each on both) takes 477 steps at most a task on the M1 Max and 18 on the M3, with the roots the
         CPU finds (`lowered_clauses_find_the_same_roots_on_the_resident_interpreter` passes on the M3). So the M3
         counts a task's steps short, or explores fewer of its pending paths. Not yet reduced.
       - *M3 only: `retained_grid_runtime::inputs_compact_stage_capturing_sibling_closures_preserve_maximal_prefixes`*
         fails as `BackendFailure` after 3 attempts, where the CPU's JIT agrees with the expected output. On the M1
         Max the same test held the GPU: the farm's watchdog killed it after a one-thread dispatch had not finished
         for 16 s. Not to be run again until the wait is found in the code.
       - *Both Macs: the launch limit.* `lowered_clauses_find_the_same_roots_on_metal` (2 ruf028 clauses) and both
         wave tests stop with `BackendLaunchLimit`. A chain's reach with more levels than `label_phase`'s programs
         hold fails its run (`ChainLevels`), which runs again with twice the levels, from 1. These clauses need 4:
         three runs of about 69 launches each, every run's launches in the evaluation's ledger, past its 200
         (`KERNEL_LAUNCH_LIMIT`, a user decision). Not a GPU's difference.
       - **Decided (user, 2026-11-21): a rerun charges a budget of its own**, which allows one rerun an evaluation on
         average. `launch_budget::Reruns` is the run's (decision 0442): each evaluation adds `KERNEL_LAUNCH_LIMIT` as
         its outermost scope opens, and a rerun (`launch::rerun`: every run of the label sweep after its first, for
         more chain levels, more budget, wider lanes, more rounds) takes its launches from it, not from the
         evaluation's own 200; one that needs more takes what others did not. A rerun's plan is checked against what
         it has left (`launch::rerun_available`), not against the launches before it. The ruf028 clauses pass on the
         M1 Max and the M3 (`lowered_clauses_find_the_same_roots_on_metal`, all 492).
       - *Found with it: the plan left out a recursive group's anchors.* `encode` evaluates them once before the
         group's sweeps, rounds or fused dispatch (Q133), but the plan's `fixed` counted only plain steps' levels, so
         a wave's plan said 198 launches and its run took more. It counts them now (each anchor once, in `encode`'s
         order; a round's levels only their formula passes): the wave's plan is 337 and its run 345 (8 files: 326
         and 342; a level's formulas past the phases' scratch split into more dispatches, which the plan does not
         see). So the wave's first run alone is past 200, and both wave tests stop at the plan with
         `BackendLaunchLimit`. Run once with the limit at 400 on the M1 Max, `the_wave_finds_ws_matches_on_metal`
         finds 219 clauses' matches missing that W finds, and `the_wave_is_one_evaluation_on_metal` runs past its
         240 s: more than the limit stands between the wave and passing.
       - *Fixed: the CPU interpreter's labels copied every label set at every write.* `LoweredMatcher::label` held
         its views (each an `Arc` of the file's sets) while it wrote a label's new set, so `Arc::make_mut` cloned the
         whole table, a label at a time, every round (decision 0353's copy). `ruff-pyi041`'s
         `redundant-beside-complex` sat in it past the test's 240 s on the M3; with the views dropped before the
         write, 1.6 s. `sweeps_needed` (the probe) did the same per marked node, and drops its view too. The whole
         gate then passes on the M3: 492 of 492 clauses equal on 60 fixtures, 41 s, at most 96 launches a clause.
       - *Open: the resident interpreter's gate* (`lowered_clauses_find_the_same_roots_on_the_resident_interpreter`,
         every walk interpreted). With reruns funded it passed once on the M1 Max (134 s, the host loaded). With the
         same GPU code, its next run there was killed by the farm's GPU watchdog (no one-thread dispatch finished for
         15 s) after the clause `ruff-e711-none-comparison/none-comparison-equal`. On the M3 it finishes in 18 s with
         14 of 123 clauses differing from the CPU (roots missing and extra: e741, pie790, b013, b021, b029, f707,
         f901, n801, ret502, sim905, tid252, try400, up035, up039) and 12 refused at the launch limit. Before reruns
         had a budget, a clause that needed one was refused, and a refused clause is counted, not compared. Not run
         on Metal again until the cause is found in code. Also open on the M3 only: the two step-budget tests
         (`a_misplanned_label_sweep_…`, `on_metal_an_unfinished_pass_goes_to_w_…`): no task runs over its budget.
       - *Fixed: the wave tests aborted on the M3* on a pool thread's read of the machine's jobs' walls outside any
         run (decision 0442): the label kernels' warm-up, and `threads::spawn_in`, read them before they entered
         their caller's run. They enter first now.
6. **Done (2026-11-21): a level's formulas are one launch, a work queue with continuations, not a launch a phase.**
   - *Why.* The wave's plan was 337 launches, past the 200 an evaluation has: 212 of them `label_phase`'s, a launch a
     phase of each level's formula with the most (up to 12), most formulas needing 1 to 3. Then leaves 49, programs
     and sinks 34, settles 23, recursive groups 13, patterns 5, parents 1.
   - **Decided (user, 2026-11-21): no cross-thread barrier that spins.** "Those that launch should launch and do
     whatever useful work is available. If they need to wait for another thread, they should enqueue a continuation
     and pick up more work rather than spin." What was tried first, and why it is not the design:
     - *A persistent grid* (`ukl_grid_join`, a grid barrier between phases). On the M3 Max (40 cores), 1,024 groups of
       64 threads all joined, and 520 to 950 reached the first barrier; at 640, every group finished its items and
       exactly 520 (13 a core) reached it, the other 120 running only once those gave up at the bound. A group that
       has started can be kept from going on there.
     - *The grid sized by occupancy* (the cores, from the IORegistry's `gpu-core-count`, times the groups
       `maxTotalThreadsPerThreadgroup` admits). Right on the M1 Max (512 groups, the clause passed); on the M3 Max
       the pipeline says 1,024 threads a core and 832 went on. No bound read from the pipeline holds there.
   - *The queue* (`label_phase`, `<ukl/workqueue.h>`): an item is a phase of a formula over a block of the region's
     words, a group of 64 threads an item. Each formula's phase 0 is there from the start (seeds, taken from a
     counter); the group whose item finishes a formula's phase (its count of done items reaches the phase's end)
     pushes the next phase's blocks and goes on taking items, so a pushed item has a worker that is running. A
     group that finds nothing ends. Nothing waits for another group, and the groups that run at once are not this
     kernel's concern. Ordering: a group's writes, a device barrier and fence, then its count; the pusher's fence,
     then its pushes; the taker's fence after its take. The slots' plain reads and writes are coherent.
   - *Its capacity*: the continuations (each formula's phases past its first, its blocks each), and a slot a dispatched
     group more: a take that comes before its push's write gives that slot up, and a group ends at a take that finds
     nothing, so it gives up one at most. Counting only the continuations, a clause overflowed (`BackendCapacity`).
     After the run the host reads each queue: overflow is `BackendCapacity`, an item left active `BackendFailure`.
   - *No worker budget* (point 4) here: a budget ends a worker with items left, and an evaluation is one command, so
     nothing would launch again for them. The launch's work is the plan's items, each a phase of one block, and no
     worker waits: its time is its items', which the launches a phase took in the same command buffer.
   - The plan counts a level's formulas as 2 launches (their leaves, their phases).
   - *Measured*, the lowered-clauses gate (492 clauses, 60 fixtures): 492 equal on the M3 Max and the M1 Max; 8,332
     launches against 12,685, at most 52 a clause against 96; Metal 15.5 s against 15.3 s on the M3 Max, the host
     as loaded (CPU 21.5 s both). The wave: planned 174, launched 126 (345 before), one command buffer, 5.7 s. Its
     matches still differ from the oracle in 406 (clause, file) pairs, the same clauses first (B404, B603, A005 in
     `f000.py`) that a run with a launch a phase and the limit at 400 missed: the wave's, not the phases'.
   - The label-sweep tests on the M3 Max fail as before and no more: the two step-budget tests, and the resident
     interpreter's (the same 14 clauses; 99 equal, 97 before, two fewer refused at the limit).
7. **No device thread waits for another by polling (user, 2026-11-21): "find any places where we're spinning in the
   same way elsewhere in the system and replace it with the scheme" of step 6.** The places, and what each became:
   - **Done: an idle work-queue worker ends at once.** `ukl_wq_idle` and `ukl_wq_idle_sg` polled the queue's active
     count up to a limit for others' pushes (the label sweep's spill queue: its first 512 threads 4,096 polls, the
     rest 1; the lattice: 4,096). They are gone: a take that finds nothing ends the worker. A worker that pushed is
     running and takes on, and a dispatch's later workers start as earlier ones end and take what is queued then.
     The gate on the M3 Max: Metal 17.6 s without the polls, 18.4 s with them, run after run on one host (a first
     run at 49.8 s was the host's load: the same code ran at 17.6 s next). The queue's tests pass on the Mac mini's
     Metal and spark1; the device fix waves (the lattice) and the spilled walks on the M3 Max.
   - **Done: a lane's lazy arena block has no pending state.** `ensure_lazy_block` (`batch_runtime.uglykl`) claimed a
     block by CAS (state 1) and spun on state 1 for another thread's reservation. A lane is one thread's own (a lane a
     thread: `lane_arenas_full` in `run_batch` and the label sweep), so no thread ever saw another's claim: the wait
     was a spin in form only. It reads the state, and at the lane's first need reserves and stores it, with no CAS
     and no loop. The host's model of it (`LazyArenaBlock` in `packed_arena.rs`, which `std::hint::spin_loop`ed on
     the same state) is the same: `ensure(&mut self)`, the block one lane's.
   - **Kept: the 16z cache's wait for a pending entry** (`clif_tc_get` in `<ukl/reads/tape_body.h>`). It is not a
     barrier: the thread waited for has claimed the entry and is decoding it, so it is running and owes no other
     thread anything; the wait is bounded (`CLIF_T_C_SPIN` turns, then the block decoded into an entry of its own),
     and ends at once once any lookup has failed. A read inside an evaluation has no continuation to push, and
     decoding instead of waiting was measured worse (21.8 s and 121,577 failed lookups against 7.2 s and none).
   - **Not a launch: the leaves.** The run reads every anchor's and formula pass's leaves in one `label_leaves`
     launch where their rows fit the formulas' scratch (`run_leaf_tests`, `leaves_fit`), and did before; the plan
     counted a leaf launch at each level with formulas. It now counts the run's one (the wave's plan: 49 leaves
     were 48 launches that never ran).
