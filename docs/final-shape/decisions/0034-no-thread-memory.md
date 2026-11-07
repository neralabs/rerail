# 0034: The device keeps no per-thread memory; walks run in pools with device stacks

Status: accepted (2026-11-04, list step 28, O39)

## Intro

A Metal kernel's thread memory is not memory the engine owns. The thread memory covers its
private arrays, its spilled registers and its non-inlined call frames.
- **How the driver sizes it:** the driver gives the process "scratch" for it. The size is every
  thread the GPU can hold in flight times the kernel's whole private size, whatever the dispatch.
- **It is kept:** the process holds the scratch for its life, at the largest size any kernel has
  needed.
- **It is invisible to the engine:** it is not in the Metal buffer budget, and `footprint` shows
  it as "Owned physical footprint (unmapped) (graphics)".

Measured alone (`metal_tape_tests`, the `a_*_costs_footprint` diagnostics):
- **The first submission:** a process's first submission costs a fixed ~113 MB. The dispatch size
  and the number of queues don't change it.
- **A private array of 64, 512 or 2,048 words** then adds 32, 240 or 970 MB.
- **The kernels the cold audit ran:**
  - the 8-frame root walk (`root_records_8`: 8 frames of 31 words in thread memory) and the
    8-frame walks and groups;
  - the pattern kernel, one per wave holding all 59 of S7's patterns. Alone it added 88 MB,
    because a kernel that large spills. Any 8 of its patterns cost nothing; 30 cost 16–48 MB.

On S7's sample cold on Metal, this was ~150 MB of the footprint peak.

## Prior work consulted

- **The spike indexes (design and history):** no entry on GPU thread memory. Their "spill"
  lessons are SQL sorts and hashes, which say the same thing in another layer: the working set
  must be what the plan sizes, not what the executor decides.
- **Lessons ledger #26:** a budget must be structural. Scratch is a budget nobody set: it scales
  with the GPU's threads in flight, not with the work.
- **Decision 0032:** the Metal buffer budget holds the residency and the workspaces. Scratch was
  outside it.
- **Device enqueue as a work queue** (the memory note): bounded pools, not unbounded
  per-thread state.

## Decision

1. **No kernel on the wave's path keeps an array in thread memory.**
   - **Every walk tier strides over its items from a pool.** Its stack is in device memory, in one
     buffer of the runner (`Runner::stacks`, `STACK_WORDS`) that every tier and root walk shares.
     The buffer is tracked, so the kernels that share it run in order.
   - **Pool sizes:** 8-frame walks run 32,768 lanes and 8-frame root walks 17,975; the loop and
     deep tiers keep their pools.
   - **Fused groups:** their stacks are per (file, lane), in a buffer per depth.
2. **Pattern equality has no stack.** A repeated metavariable's structural equality (`exact`)
   compares the two subtrees' preorder runs in the tape: kind and child count pair by pair, text at
   a named leaf, a shared node skipped whole. A subtree is contiguous in the tape, so this is the
   host's recursive `exact`. The 64-pair refusal goes.
3. **Pattern kernels hold `patterns::GROUP` (8) patterns each.** A plan's patterns are compiled
   as one pipeline per group, numbered in the plan's order; facts go with the first group. Each
   group is dispatched over its own items.

## Consequences

- **S7's sample, cold on Metal** (load 35; CPU is user + sys):
  - 1,259 keys.
  - Audit CPU 6.65 s.
  - Footprint peak 603 MB (752 before), and 335 MB after the audit (457).
  - Driver-private memory after the first label run: 200 MB, of which 113 MB is the process's
    first submission.
- **Tests:**
  - S2: 158 of 158; interpreted, 37 of 39.
  - The regression set: 195.
  - Real files: 1,259 keys on all four backends.
  - `phase_four`/`jit`: 109, with the same 11 failures.
- **More pipelines:** a wave compiles 8 pattern pipelines instead of 1. They are smaller and are
  cached by source, so a group shared by two waves compiles once.
- **The interpreter (`run_batch`) has not been measured this way.** It does not run on S7's
  sample. Its private state should be measured before it is on the wave's path again.
