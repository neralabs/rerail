# 0641. A program's phases are indirect on Metal too: its kernels call-free

Status: accepted (2026-11-23). Amends 0638 (a program's levels are stacked dispatches): its Metal exception, a
phase dispatched directly with its bound's groups, is removed.

## Intro

0638 dispatched each phase of a program indirectly, its grid written by the device (`join_grid`), except on Metal: on
the M3 the indirect dispatch hung the GPU, so Metal started each phase's bound (4,096 groups for a phase of segments),
and those past the grid returned. Its recursive strata took 11 to 47 times the GPU time of a launch a level.

The user, 2026-11-23, on 0638's next step: "find why Metal's indirect dispatch hangs, so Metal gets exact grids as the
other backends do, then stop stacks once a component ends".

## Why the M3 hung

On the M3, the indirect dispatch of a pipeline that makes calls hangs the GPU (`kIOGPUCommandBufferCallbackErrorHang`)
once its grid has two groups or more. It hangs even when every thread returns at the kernel's first statement. A
direct dispatch of the same pipeline does not hang, and neither does an indirect dispatch of a call-free one.

On Metal, `join_phase` makes calls: its rare and once-only bodies (`UKL_COLD`, `UKL_ONCE`, and `UKL_RARE` through
them) are `noinline`, so the kernel has a call stack. `join_grid`, a one-thread kernel that makes none, ran indirectly
at 106 groups with no hang.

The experiments, each on the M3 (`a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts`, a stack a phase):

| Dispatched indirectly | Grid | Result |
|---|---|---|
| `join_phase`, returning at its first statement | 1 group | ran |
| `join_phase`, returning at its first statement | 2, 10, 24, 64 or 106 groups | hung |
| `join_grid` (no calls) | 106 groups | ran |
| `join_phase` with `UKL_COLD` and `UKL_ONCE` inlined, returning first | 106 groups | ran |
| `join_phase` with them inlined, the program's own phases | each phase's grid | every device test passed |

The mini did not hang at 106 groups with the calls left in. Not the cause, each tried first:

- a buffer barrier before the dispatch;
- a compute encoder of its own;
- the pipeline's limits (1,024 threads a group, 11,408 bytes of group memory);
- the arena reserved and wrapped (a plain one hung too);
- grids the device wrote (grids the host wrote hung too).

## Decision

- **A program's kernels are call-free on Metal.** `program_pipes` compiles `join_grid` and `join_phase` with
  `UKL_COLD` and `UKL_ONCE` defined as inline (`rerail_device::device::is_metal`). The join library's other kernels
  keep their calls: only the program's are dispatched indirectly.
- **Every backend dispatches a phase indirectly, at its grid's groups** (`Pass::run_bytes_indirect_at`). Metal's direct
  dispatch of the bound is removed. `PHASE_GROUPS` is the bound of the backends with no indirect launch (CUDA, OpenCL),
  whose groups past the grid return.
- **A fence finds a bound address's block once** (`Fence::held_at`). Each bind (`region::bind_address`) searched the
  live blocks for the one holding the address. A stack binds the same buffers at each of its thousands of
  dispatches. The fence now keeps each address's ledger, or none, until it completes. It holds that ledger meanwhile,
  so no other block can take the address before. The region's pin is still taken at a bind, once a fence and access.
- **A pass makes its resident buffers resident once a span** (`ComputeQueue::residency_span`). It made its whole
  resident list resident at every dispatch.
  - Metal's span is its open compute encoder: `useResource` holds for the encoder's life.
  - Every other backend keeps what a launch uses with that launch alone (OpenCL's indirect-access list, the CPU's and
    CUDA's kept buffers), so each of their launches is a span.

## Measured

On the M3: decision 0603's measure (`measure_one_program_against_a_launch_a_level`, seven warm runs, medians).

| Stratum | Launch a level: GPU | 0638: GPU | Now: GPU | 0638: wall | Now: wall | Launches |
|---|---|---|---|---|---|---|
| seeds | 11.3 ms | 10.8 ms | 6.2 ms | n/a | 77 ms | 1 |
| seeds ×10 | 47.0 ms | 56.8 ms | 50.9 ms | n/a | 688 ms | 1 |
| closure | 11.3 ms | 526 ms | 50.0 ms | 4.3 s | 175 ms | 10 |
| leveled reach | 9.0 ms | 298 ms | 44.3 ms | n/a | 116 ms | 7 |
| leveled dist (min) | 5.8 ms | 127 ms | 23.2 ms | n/a | 59 ms | 5 |

The wall had been the host's encoding, not the GPU. One closure stack's encoding took 385 ms against 5 to 12 ms of GPU
run. Of it:

- the binds' block searches were half (212 ms left once each address was found once a fence);
- the resident list made resident at every dispatch was most of the rest (11 ms a stack once it is made resident
  once a span).

## What is left

The recursive strata still take 2 to 5 times a launch a level's GPU time:

- **A round's dependent dispatches.** A round of the leveled strata is about 100 phases, nearly all of one item, each
  two dependent dispatches (`join_grid`, then `join_phase`). That is about 0.55 ms of GPU time a round, about 2.75 µs
  a dispatch, against 0.15 ms a level launched a level at a time.
  - A stack of one round (`RERAIL_STACK_ROUNDS=1`) took about 2 ms of GPU time a round.
  - A stack of up to 64 rounds took about 0.55 ms a round.
- **The rounds past a component's end** cost about 0.1 ms each: every `join_grid` writes a grid of no groups, and its
  `join_phase` starts none. A stack's rounds double, so its rounds past the end are at most the rounds before it. The
  leveled distance's last stack ran 8 rounds of its 32 in 7.3 ms.
- **The closure's remaps.** Its 8 rounds took 10 launches, 8 of them its arena mapped further (0603's arena mapped as
  its claims reach it).
