# 0040: Nearest ancestors are found in one dispatch per chunk

Status: accepted and built (2026-11-05, the user: "we need to launch once only"; list step 30)

## Intro

Decision 0038 found nearest ancestors on the device by pointer jumping in rounds. Each round was
a dispatch, 6 to a command buffer, and a tree deeper than 64 levels needed another command buffer
and another wait. Decision 0039 measured where a device call's time goes. With the buffers
already reused, what is left is paid once per wait: the command reaching the GPU, the GPU
starting, and the host thread waking. So each chunk now makes one launch: one command buffer, one
dispatch and one wait, whatever the depth.

## Decision

- A cell (set, node) holds one 32-bit state: a target node, with bit 31 set when the target is
  final.
  - **Final:** the target is the answer, the nearest strict ancestor in the set, or none.
  - **Not final:** the target is a strict ancestor not in the set, so the cell's answer is the
    target's answer.
- Each thread repeatedly takes its target's current state, and stores it as its own, until its
  own state is final (`metal_nearest.metal`).
  - Every state any thread writes keeps this meaning. So reading another cell's state is
    correct at any moment, before or after that cell moves, and plain relaxed 32-bit atomics are
    enough.
  - A thread whose target has already moved up skips ahead with it. That is the pointer jumping,
    now without rounds.
- Each step moves strictly up the tree, so a thread finishes within its depth's steps, and
  never waits on another thread. This is the same rule as the work queue's bounded loops (decision
  0027): no thread waits on another, because Metal promises no progress between threadgroups.
- The host checks that every cell is final. One that is not is a failure, never a CPU retry.

## Measured

One 172K-node piece (`nearest_costs_by_solver`, load 50–65), times per call:

| | Rounds (0038) | One dispatch |
|---|---|---|
| GPU, depth 39 | 1.05–2.47 ms | 0.036–0.045 ms |
| GPU, depth 1191 | 1.46–3.45 ms | 0.042–0.045 ms |
| Host, blocks reused | 1.3–4.0 ms | 0.35–0.75 ms |
| Most steps a cell took | 6–12 rounds | 10–24 steps |

async-sync-boundary on 1 in 8 files (14 chunks, on the device, load 49–54), summed over the run.
Findings are unchanged.

| | Before (0039, load 47–78) | One dispatch |
|---|---|---|
| Dispatches | 84 | 14 |
| GPU | 13.3–14.7 ms | 0.50–0.54 ms |
| Host wait | 30.6–42.2 ms | 18.6–21.5 ms |
| Schedule (GPU waking) | 11.3–14.1 ms | 11.7–12.5 ms |

## Left

- **The GPU waking is now most of the wait:** about 0.85 ms a chunk, after the GPU has been idle
  while the CPU joins. The chunks are streamed, and the next chunk's trees do not exist yet when
  this chunk's are solved, so the run cannot make a single launch for every chunk. Keeping the
  GPU warm between chunks is the remaining lever.
