# 0674: A program's phases are a chain of indirect dispatches, one launch in the ledger

Status: decided (2026-11-24), by the user: "does metal have indirect dispatch? if so, I want an UKL primitive that
allows us to use it, with a re-dispatcher for GPUs which don't support it", and "a chain is one launch in the ledger".

## What was measured

The join program (decision 0603) runs a stratum's device work as one persistent launch over a work queue: a phase's
items are pushed by the group that finishes the phase before, and a group that finds the queue empty ends at once
(decision 0540 step 7). Counted per phase on the M1 Max (32 cores), the keyed join of the timed test (100 K x 100 K,
249,484 rows), with a counter of the groups that took any item of each phase:

- 94 phases in series: 59 in the program that builds the index and runs the first join, 35 in the next. Each scan is
  5 phases (7 scans: 35), each size script a phase of one item (9), the index's sort 3 radix passes of 7 phases.
- No phase ran on more than 32 groups of the 6,250 to 9,746 dispatched. In the first program every phase from the
  stage scatter on ran on **one** group: `STAGE_SCATTER` 391 items, `DEDUP_CLAIM` and `DEDUP_MARK` 782 each,
  `COPY` 975, each done by one 256-thread group, item after item. At each phase of one item every other group found
  the queue empty and ended; the program went on with the groups left.
- Its GPU time, 11 to 15 ms, against 15 to 16 ms for the same program on one CPU thread.

The queue is right about waits (no group waits on another) and wrong about width: the width of a phase is whatever
groups happen to be alive when the phase before ends.

## The decision

A program's phases are a **chain**: a sequence of dispatches of one kernel in one submission, in order, each sized
on the device. Phase `p`'s group count is three words in the program's block, written by phase `p - 1` (its
finisher: the group that completes its last item), read when phase `p` starts. No group waits for another: the
order is the dispatches', which the device keeps. Every group dispatched has an item to run, so each phase is as
wide as its items.

- **The ledger: a chain is one launch** (the user's). Its dispatches are charged once, as one launch of its kernel,
  however many phases it has. `KERNEL_LAUNCH_LIMIT` bounds chains, not phases.
- **Its phases are bounded by its encoding**: the host encodes the phases it knows. A recursive component's rounds
  are encoded as a bounded number of rounds' phases; a round after the fixpoint has no groups (its counts 0, written
  by the round that found nothing new). A chain whose rounds ran out with work left is launched again: a new chain,
  a new launch in the ledger.

### The UKL primitive

Device side (`<ukl/chain.h>`):
- `ukl_chain_phase(ctl)`: the phase this dispatch is (the control's word 0, which the finisher before moved on).
- `ukl_chain_finish(done, groups)`: true in the one group that finishes its phase last (a counter, then a device
  fence): the finisher, the only group that writes what the next phase reads.
- `ukl_chain_next(ctl, phase, groups)`: the finisher's: the next phase's counts (0: the chain ends) and the phase
  word.
- A phase's group knows its item from its dispatch: its group index (`GID0 / LSZ0`) from the phase's first.

Host side: `Pass::chain_at(pipe, buffers, bytes, counts, offsets)`: one dispatch a phase, each indirect at its
counts' offset, a memory barrier between two (`ComputeQueue::memory_barrier`: Metal's `memoryBarrierWithScope:`
buffers, since the phases reach blocks by address, which no binding tracks; CUDA's stream, OpenCL's queue and the
CPU's run in order and need none). The chain's resident buffers are declared once, at its first dispatch: its
dispatches share one encoder. `Pass::dispatches` counts a chain once, as the ledger does.

### Each backend

- **Metal**: `dispatchThreadgroups(indirectBuffer:indirectBufferOffset:threadsPerThreadgroup:)` a phase, on the
  pass's one compute encoder, a buffer barrier between two.
- **The CPU executor**: the dispatches run in order, each reading its counts when it runs (`launch_indirect` reads
  them then already); the join program's Rust (`rerail_ukl_rust::join_run`) runs a phase's groups as it does a
  launch's.
- **A GPU with no indirect dispatch (CUDA, OpenCL 1.2): the re-dispatcher.** The host launches each phase at a
  fixed number of groups (`CHAIN_GROUPS`, the same for every phase: not the device's residency, not the phase's
  bound), and the shim maps the device's count onto them: a launched group runs virtual groups `g`, `g + launched`,
  `g + 2 launched`, … below the count, its `GID0` and `GSZ0` the virtual group's, as Metal's dispatch gives them. A
  count of 0 runs nothing; a count above the launched groups runs all of it (the early exit before this ran at most
  `max_groups` and dropped the rest). A group's barriers are its own, so a virtual group is a group. CUDA's
  device-side launch (a tail launch from the finisher, dynamic parallelism) is the faster re-dispatch where the
  device has it; measured before it is made the CUDA path.

## The join program on chains (step 2's design)

What each part of `join_run`'s queue (0603) becomes:

- **The counts are a dispatch's, not a phase's.** A chain of `D` dispatches has `D + 1` count slots; the finisher of
  dispatch `d` writes slot `d + 1`, whatever phase it runs next. So a round's end that jumps back (`ctl[7]`) is the
  finisher writing the jumped-to phase into the phase word and its items into the next slot; no phase is encoded
  twice by the host.
- **A group is an item.** Dispatch `d` runs phase `ctl[0]`'s items, one group each: group `g` finds its item in
  the phase's segments (`seg_items`, as `phase_push` counts them now). The finisher's `phase_reset` count (`target`)
  is the next dispatch's groups. A phase whose segments have no items (`none`) is one group that runs the first,
  as now. Seeds are phase 0's items, the host's slot 0.
- **The queue goes**: no `slots`, no tickets (`ctl[5]`, `ctl[8]`), no `UKL_WQ_TURNS` loop. `done[p]` and `target[p]`
  stay: they find the finisher.
- **The launch's budget is its dispatches** (0603 point 5 bounded a launch's GPU time by its items): a chain is `D`
  dispatches, `D` the program's phases (a program without rounds runs in one chain) and at most `CHAIN_DISPATCHES`
  for one with rounds. A program not done when its chain is (slot `D` not 0, no flag) is launched again: the host
  moves slot `D`'s three words to slot 0 (control words, not a block) and encodes a new chain. Each chain is a launch
  in the ledger, as each budget's relaunch is now.
- **Parks end the chain** (slot `d + 1` written 0) with what to resume in `ctl[9]`, as now:
  - a size script past its arena's pages: the parked item, uncounted. The host maps the arena; the next chain's slot
    0 is one group, which runs `ctl[9]`'s item (the item put back) and goes on as the phase's finisher would;
  - texts that found the text arena full: the next chain's slot 0 is the phase's text items (`phase_texts` counts
    them; group `g` runs the `g`-th);
  - a gate's room mapped past: the next chain starts at the phase after the gate (its `phase_reset` count, the
    host's slot 0 from the finisher's write of it into `ctl`).
  The host's checks of `ctl[6]`'s flags in `programs_done` stay as they are; `compact` and `put_back` go with the
  queue.
- **The CPU executor**: `rerail_ukl_rust::join_run`'s Rust follows the same source (its digests): one launch a
  dispatch, the counts read as it runs.

## Writing against running

- A phase boundary costs a dispatch's start and a buffer barrier, a few microseconds, against the queue's ticket per
  item and its collapse to the groups left alive.
- A phase of one item (a size script, a scan's top) is a dispatch of one group. So the steps of the join program are
  folded where they are a finisher's work: a size script runs in the finisher of the phase before; a scan is one
  phase over its tiles' sums, its tiles' downsweep done by the phase that reads the offsets.

## The steps

1. **Done.** The primitive: `<ukl/chain.h>`, `Pass::chain_at`, the ledger's charge (`launch::attempt_chained`),
   the re-dispatcher in the CUDA and OpenCL shims (the CPU backend, the vec shim's too, launches exactly the counts
   it reads, so needs none). `a_chains_phases_size_each_other_and_are_one_launch`: 205 phases, sized 3, 1, 2,000
   (past `CHAIN_GROUPS`), 7, then 0; each phase's groups run once each; one launch in the ledger. Passes on Metal
   (M1 Max), CUDA (GB10, re-dispatched) and the CPU executor.
2. **Done, as designed above.** `join_run` is a chain: dispatch 0 its start (begin, or resume a budget's end, an item
   put back, texts or a gate released), then a phase a dispatch, a group an item (`TGID0`), the finisher writing the
   next dispatch's counts. The queue, its slots and its tickets are gone (`ProgP` is the phases and the chain's
   dispatches). A chain is the program's phases (four times them for a program of rounds), at least
   `PROGRAM_BUDGET` (16), at most `PROGRAM_DISPATCHES` (1024); the test switch `RERAIL_PROGRAM_BUDGET` sets it.
   The Rust (`rerail_ukl_rust::join_run`) runs each group as the kernel does; its digests are the new source's.
   The stratum tests (`setfix::device::tests`, 27) pass on Metal (M1 Max) and the CPU backend.

   Measured (M1 Max, the timed test, `nowrite`, best of the warm runs):

   | | before (queue) | chain |
   |---|---|---|
   | keyed join, Metal: program | 12.5 ms | 14.1 to 14.5 ms |
   | keyed join, Metal: encoding (host CPU) | | 2.7 ms (4.0 before residency was declared once) |
   | keyed join, CPU executor: program | 15.1 ms | 19.0 ms |
   | closure (400 nodes), CPU executor: program | 27.8 ms | 72 to 84 ms |
   | closure, CUDA (GB10, re-dispatched, rooms mapped) | | 156 ms |

   Not faster yet, and why (the CPU executor's trace of each phase): the keyed join's second program is 35 phases,
   18 of them one group; the closure is **2,388 phases** in series (51 rounds of about 47), 1,861 of them one group:
   per round about 20 scan phases (a scan's 5 levels, several scans) and 10 size scripts. Each phase is now a
   dispatch: a dispatch's start on the GPU, a launch on the CPU executor (about 5 us there). The width is right now;
   the phases' number is the cost. That is step 3.

   **Open: the closure hangs on the M1 Max's Metal.** It ran 929 phases over 12 chains (gate parks at phases 82 and
   129, rooms grown, released, each right), then a chain after a release at phase 82 held the GPU past the farm's
   watchdog (twice) or Metal's timeout (once). The same chains run right on the CPU executor with rooms mapped
   (`RERAIL_ROOMS_MAPPED=1`) and on CUDA, whose kernel is this source: so a Metal-only fault. The buffer barrier did
   not change it. Decision 0672's G13 exit-code check (another branch's, applied here for the test and backed out)
   passed `join_run` under 255 and the hang stayed. The queue's program hung on the mini's Metal the same way
   (decision 0671). Not run on the M3 (unreachable then).
3. Its phases folded: size scripts into finishers, a scan one phase.
4. Measured: the keyed join and the closure, per phase and whole, Metal against the CPU executor.
