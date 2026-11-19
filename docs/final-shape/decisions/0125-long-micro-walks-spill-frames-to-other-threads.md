# 0125: Long micro walks spill their frames to other GPU threads

Status: built (2026-11-12).

## Intro

**The problem.** A matcher's label pass runs on Metal as micro walks, one thread per (pass,
start) (decisions 0015, 0019, 0021). Most walks take under 20 steps. A walk over a module's
descendants takes about 10^5 steps, all on one thread, and the whole dispatch waits for it. On
internal_dead_code the longest walk was 158,102 steps.

**The decision.** Past a step budget, a walk gives part of its work to the dispatch's work queue
(decision 0027), so a long walk is many threads' work, not one thread's critical path.

## How it works

- After `SPILL_BUDGET` steps (512), and again every 512 steps, a walk gives away its lowest move
  frame that has candidates left. A descendant range of at least `2 * MICRO_CHUNK` (128) nodes
  is halved rather than given whole.
- A queue item (`MicroQ`) carries the frame, its walk's start, its pass and its document. Items
  are written and read with atomics, so they are coherent between threadgroups.
- The tier kernels drain the queue after their roots. Only the first `MICRO_STANDBY` (512)
  threads wait for spilled frames. When every thread polled, internal_dead_code timed out.
- A full queue leaves the frame with its walk. A fused group's walks do not spill (capacity 0).
- `RERAIL_MICRO_SPILL=off` turns spilling off.

## Checked

`spilled_walks_find_the_cpus_roots` runs 491 clauses in waves of 12 over 24 fixtures, with a
budget of one step, so nearly every walk spills. Metal's roots equal the CPU's. Dropping drained
frames fails it: 13 clauses differ.

Measured with the matcher wave pinned to the GPU:

| | before | after |
|---|---|---|
| public_api_triples: label passes, GPU | 952 ms | 300 ms |
| public_api_triples: wall | 18.6 s | 14.4 s |
| internal_dead_code: micro kernels, GPU | 928 ms | 179 ms (no deep reruns) |
| longest walk | 158,102 steps | 3,716 steps |

## The profile, for this work

- `RERAIL_PROFILE` turns on the stage counters, listed under "Stages": the device's host stages,
  its GPU time, and `RERAIL_GPU_KERNEL_TIMES`' split by kernel.
- `RERAIL_GPU_WALK_STEPS`: each micro pass's walks, steps and longest walk.
- Every label-pass `BackendCapacity` refusal is counted by its line, with what the driver, the
  label pool and accel's leases held then.
- A matcher wave whose Metal run failed and fell back to the CPU is a warning with its error. A
  kernel library that failed to compile had exited 0.
