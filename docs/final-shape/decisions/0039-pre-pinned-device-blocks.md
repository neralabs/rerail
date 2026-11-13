# 0039: Device scratch buffers come from pre-pinned blocks that are reused

Status: accepted and built (2026-11-05, the user; list step 30)

## Intro

A device solve has two kinds of buffer:

- **Resident buffers** hold what lives across runs: the tapes and sources. These are already
  pinned once and kept, in slabs (decision 0032).
- **Scratch buffers** hold one call's inputs, working arrays and flags. Until now these were made
  for every call and dropped after it.

Making a scratch buffer costs four things:

1. a fresh mapping;
2. the first write of every page;
3. a no-copy Metal buffer over the mapping;
4. the device making the new pages resident before the kernel can start.

On the data-layer rule async-sync-boundary, the device run's host waited 287 ms for 13.4 ms of
GPU work (decision 0038, "Left"). The user's diagnosis was that this is almost always memory
pinning, and that the fix is pre-pinned blocks that are reused. This decision measures that
diagnosis, then builds the blocks.

## What the profile now shows

Every device record in the profile (`RERAIL_PROFILE`, section "Device (Metal)") now splits the
host's time into phases. The GPU's own timestamps on each command buffer (`kernelStartTime`,
`GPUStartTime`, `GPUEndTime`) are on the same clock as the host's, so each phase is measured,
not inferred:

| Phase | From | To | What it is |
|---|---|---|---|
| buffers made | lease | buffers ready | mapping, first page writes, no-copy buffer |
| command queue made | | | a new `MTLCommandQueue` |
| encode | first encode | commit | setting buffers, dispatches |
| submit | commit | `kernelStartTime` | the command reaching the GPU's scheduler |
| schedule | `kernelStartTime` | `GPUStartTime` | residency of the buffers, the GPU waking |
| GPU | `GPUStartTime` | `GPUEndTime` | the kernels |
| wake | `GPUEndTime` | the wait returns | the waiting thread running again |

The record also gives the buffer bytes made and reused. The phases add up to the wait, so
nothing is left out. For example, 41.3 ms of phases against 42.2 ms of wait.

## What was measured

**One 172K-node piece** (`nearest_costs_by_solver`, load 63–99). Each figure is per call, in ms.
"Fresh" makes buffers per call. "Blocks" reuses buffers that were made and written once.

| | Fresh | Blocks |
|---|---|---|
| buffers made | 0.19–0.36 | 0.013–0.04 |
| command queue made | 0.08–0.20 | 0 |
| schedule, calls back to back | 0.19–0.31 | 0.07–0.12 |
| schedule, GPU idle 300 ms first | 0.61–0.68 | 0.48–0.58 |
| GPU | 1.05–2.47 | 1.05–2.47 |

- Pinning per call is real. Blocks save 0.4–0.6 ms a call: about a third of the schedule phase,
  and all of the buffer and queue making.
- The rest of the schedule phase is the GPU waking from idle, not pinning. Between a run's chunks
  the GPU sits idle for hundreds of milliseconds. After 300 ms of idle, scheduling takes about
  0.5 ms even with blocks, and the kernels run at the slower idle clock (2.47 ms against
  1.71 ms back to back).

**The whole rule** (async-sync-boundary, 1 in 8 files, 14 pieces, on the device). Each figure is
summed over the run, in ms:

| Load | Buffers | Made | Queue | Submit | Schedule | GPU | Wake | Wait |
|---|---|---|---|---|---|---|---|---|
| 47–78 | Fresh | 4.4–5.3 | 2.7–2.8 | 1.1–1.8 | 12.7–14.1 | 13.8–14.7 | 2.6–2.7 | 39.7–42.2 |
| 47–78 | Kept | 1.3–1.4 | 0 | 1.0–1.2 | 11.3–11.4 | 13.3–14.1 | 1.6–2.1 | 30.6–31.7 |
| 94–102 | Fresh | 7.0–13.3 | 3.2–3.9 | 6.7–14.4 | 17.1–19.3 | 13.3 | 35.9–45.1 | 91.0–104.7 |
| 94–102 | Blocks | 0.9 | 0.6–2.7 | 20.9–22.8 | 15.3–15.9 | 13.3 | 19.6–40.3 | 72.9–104.2 |

"Kept" was the first experiment: one solve's buffers and queue held between calls. "Blocks" is
the pool built here.

- **Verified:** buffer making and pinning cost 7–16 ms of the wait over the run, and blocks remove
  it. Over 14 pieces, 3 MB of blocks were made and 30 MB reused.
- **Not pinning:** under load most of the wait is submit and wake. Those are 1–2.7 ms at load
  60 and 7–45 ms at load 95. They grow with CPU load, not with buffers, because they are
  the time to get a thread onto a busy CPU. The 287 ms of 0038 was measured at load 155.
- **Also not pinning:** the remaining schedule time (about 0.8 ms a piece) is the GPU waking
  after the idle gap between chunks.

## What the prior work says

- **The Tempo evidence in the PIAT spec, §20 preamble:** first-use Metal mapping cost 213 ms
  against 6.1 ms of GPU, and reused buffers cut the wait 30–60×. **Agrees:** buffers that are
  reused, not made per call. Here the cost per call is smaller (0.4–0.6 ms), because the buffers
  are 2 MB, not hundreds.
- **PIAT §20.4.2:** "Pinning means retention. Buffers over resident objects are never created per
  batch or per evaluation." **Extended:** the same now holds for scratch buffers.
- **PIAT §20.4.3:** the warm-up command that touches pinned slabs. **Adopted in its simplest
  form:** a block's pages are written once when it is made, so they are backed before any
  kernel reads them.
- **PIAT §20.12 Q-R2:** `MTLResidencySet` against retained buffers, measured on first-dispatch
  latency after idle. This measurement gives Q-R2's first number: after 300 ms of idle, retained
  buffers still take about 0.5 ms to schedule. A residency set is not adopted here, because the
  idle wake is the GPU's, not the buffers'.
- **PIAT §20.12 Q-R7:** pin cost per MB. Fresh buffers of 1.7 MB cost 0.2–0.36 ms to make, which
  is 120–210 µs per MB including the copy.
- **The AST-index spike's history index, T15:** position 1 pays the warm-up, so measure off/on/off/on.
  **Agrees:** the corpus runs alternate fresh and blocks, at noted load.

## Decision

1. **Scratch buffers come from a pool of pre-pinned blocks** (`transducer/packed/metal_blocks.rs`).
   - A block is an anonymous mapping with one retained no-copy shared `MTLBuffer`, the same kind
     of buffer as a residency slab. Every page is written once when the block is made.
   - Sizes are powers of two from 16 KiB, so a lease wastes at most half a block.
   - When a lease drops, its block goes back to the pool for the next call of that size. The
     holder waits for every command that reads the block first, which the solves already do.
   - The pool keeps at most 32 MiB of idle blocks out of the 256 MB Metal buffer budget. A block
     returned past that is released.
   - Blocks are dropped with the residency slabs, two minutes after the last execution.
2. **The solves that use blocks share one kept command queue.**
3. **The users are the device paths a rule run takes:**
   - nearest ancestors (`metal_nearest`);
   - the lattice fixpoint and its work queue (`metal_lattice`, `metal_work_queue`).

   Other Metal paths are copies placed earlier and are not rewritten here.
4. **Every device record in the profile shows its phases and its bytes made and reused.**
   - `RERAIL_NEAREST_BUFFERS=fresh` makes nearest's buffers per call, to measure blocks against.
   - The benchmark compares both, and `RERAIL_NEAREST_IDLE_MS` leaves the GPU idle before each
     call.

## Not chosen

- **Buffers kept per solve.** One solve's buffers held in a static, as the first experiment did.
  That gives each kernel its own memory and none is shared. The pool serves every solve from the
  same blocks.
- **Residency slabs for scratch.** Slabs are bump allocated and retire whole. Scratch is leased
  and returned per call, so it needs size classes, not a bump.
- **`MTLResidencySet` now.** What remains after idle is the GPU's wake, which a residency set does
  not change. It stays open as Q-R2.

## Left

- **Submit and wake dominate under CPU load.** Under load the waits are mostly the host thread
  being rescheduled, twice per command buffer. The fix is fewer waits: encode every piece of a
  chunk into one command buffer and wait once per chunk. That pays submit, schedule and wake once
  per chunk instead of once per piece. Built as decision 0040: nearest now makes one dispatch, one command
  buffer and one wait per chunk.
- **The GPU wakes from idle once per chunk.** One command buffer per chunk also pays this once.
  Keeping the GPU busy across chunks would remove it.
