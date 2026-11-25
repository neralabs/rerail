# 0687. An operation's own memory is the stack and its frame, not blocks

Status: decided (the user, 2026-11-25), being built (note t60's work order).

## Intro

Decision 0628 moved every run data structure into blocks. Hub commit 8f4fdb7293 then put every `Vector` and map made
with no room into the run's open relation block (0628 §8).

A survey of one cold PostHog run (note t60) counted those arrays:
- **59.1 M arrays**, about 590,000 a second.
- **None was handed to a relation.**
- **Three quarters** were at most 256 bytes and lived under 131 µs on one thread.

Each array paid for a home it never used: the block ledger's lock, the R4 guard and the block's free lists.

The user's direction, 2026-11-25:

> The goal of putting everything in arrays is to make the form compatible with GPUs, and to gradually move all of the
> processing steps from different host/GPU implementation to a single ULK operation. This stops us from doing so much
> host work to make the GPU kernel simple that we lose all benefit of the CPU. But once we decide the boundary (the
> GPU should operate in-place on all of its pre-mapped inputs, with NO host work before or after), what happens
> inside the equivalent host operation is unimportant; scratch arrays would ideally be stack-based (even short
> variable-length ones) and any data with operation lifetime would come out of a bump arena on the stack or dedicated
> scratch blocks.

The user then approved note t60's plan and the choices in its §7 ("go ahead").

## Decision

1. **The boundary is the operation.**
   - An operation is the unit that becomes one UKL op: for example a join step, a group's wave, a document's parse,
     a memo encoding, a planner search, a digest or a sort.
   - It reads its inputs in their blocks and writes its outputs once, in place, in their blocks.
   - On the device it is one launch, and the host does no work before or after it.
2. **What an operation makes for itself is operation memory.** It has three classes, each with the same form on the
   CPU and the GPU:

   | Class | CPU | GPU |
   |---|---|---|
   | `Inline` | the stack: `[T; N]`, `InlineVec<T, N>` | `UKL_PRIVATE` |
   | `Frame` | the thread's bump frame, nested last-in-first-out | `UKL_LOCAL`, the rest in `OpScratch` |
   | `OpScratch` | the same frame over a dedicated scratch block | one buffer bound to the launch, laid out by the operation's declaration |

3. **Operation memory pays none of the costs of blocks.** It takes no relation block, no ledger lock, no R4 guard, no
   heap-profile charge per array, no zero fill unless asked, and no growth copy at the top of the frame.
   - An `OpScratch` that a kernel binds keeps its one ledger (0487 step 3).
   - 0353 holds. Operation memory holds intermediate values, never a block's data for later readers to read instead
     of the block.
4. **Data stays in blocks** (0628).
   - Anything living across operations goes in chunk or run blocks, as relations, tables or overlays. It grows by
     segments, never by copying.
   - An array handed to another thread is an output: its destination is sized first, and each worker writes its
     part.
5. **The operations we write use no `std::Vec`.** What they make goes through the frame or `Inline`. Third-party
   code keeps its own allocations.
6. **A frame array that grows while it is not at the top moves within the frame.** Its reads stay one slice, and
   the space it leaves is reclaimed when the frame ends.
7. **The default room goes** once the call sites have their classes (t60 step 7). Until then, arrays with no room
   stay in the run room.
8. **The survey's probe stays, off by default** (`RERAIL_PROBE_SCRATCH`). It measures each step: the default room's
   regions and, through the engine's allocator, the heap's.

## Consequences

This narrows 0628: operation memory is not a run data structure, because no reader outside its operation reads it.

The work order and the call sites, with each one's class and fix, are in note t60 (§5, §6).

## With 0342's stack (merge of Typing-support, 4d6ed1c0f8, 2026-11-25)

Typing-support amended 0342 on 2026-11-24 (the user's): a run's structures made with no room go on a stack a thread
(`scratch::Stack`, `StackRoom`, the default room). Large regions go to free-listed blocks, and what is known to live
long goes to the thread's long room (`maps::long`). `scratch::with` is a nested use of that stack.

The merge keeps both records:

- **Point 7's interim is 0342's stack, not the run room.** A `Vector` made with no room takes the calling thread's
  stack. The run room no longer stands in until step 7, and `install_run_room` is gone.
- **Operation memory is 0687's**: `Inline` (`InlineVec`) and `Frame` (`op`, `FrameVec`). An operation's own arrays
  are made there, never as a `Vector` with no room.
- **0342's census decides what lives long**: a structure that outlives its operation, and is not data in a relation's
  block, is made within `maps::long`.

The frame is still an arena of its own (heap chunks a thread), beside 0342's stack: two stacks a thread. Note t60 §9
is the work that makes them one.
