# 0573. The preprocessor runs on CUDA and OpenCL, its grid sized by its work

Status: accepted (2026-11-21, the user: "first merge in the hub and the cuda support branch, before you start chasing
cuda and opencl"). The preprocessor's device passes (0468) and its tests (0448) now run on NVIDIA's CUDA and OpenCL as
on Metal and UKL's CPU backend. The kernels' source is unchanged. This record covers what running it there found.

## Intro

pp's kernels are one UKL source (0468 §5), so CUDA and OpenCL were meant to need no change. The eval tests had been
built for macOS only. Built for Linux and run on a GB10, they found five defects in the backends and the
arena, and one cost that every backend paid.

## Decision

1. **The eval tests build wherever the lane has a device**: macOS, Linux and Windows (`c_directives/src/eval.rs`).
2. **OpenCL's kernel rows are expanded before they are stringized** (`ukl_opencl_shim.h`'s `DEFINE_KERNEL2`, through
   an inner macro, as the CUDA and CPU shims do). A kernel may give its rows as a macro (pp's grid kernels share
   `PP_GRID_ROWS`). The info kernel's `#__VA_ARGS__` stringized the name, not the rows, so the reflection read "1 row
   fields and 24 layout numbers".
3. **An arena's stores are aligned to what the device binds** (`pp_arena::store_align`: `max(256,
   Device::bind_align())`). On OpenCL a store is a sub-buffer, whose origin must be a multiple of
   `CL_DEVICE_MEM_BASE_ADDR_ALIGN` (512 bytes on NVIDIA's). 256 was refused. Elsewhere `bind_align` is 1.
4. **A `synced` OpenCL device maps a block once, by its root buffer** (`ClComputeQueue::keep_buffer`). NVIDIA's
   driver keeps its own image of a buffer over host pages, made current by a map (0440's `synced`). An arena's stores
   are sub-buffers of one block, and each was mapped on its own. OpenCL leaves overlapping maps undefined: results
   were wrong, then a SIGSEGV. The root buffer is mapped whole now, once a submission.
5. **NVIDIA's driver is given the hardware queues the process uses, not its maximum** (`os::nvidia_hardware_queues`,
   `CUDA_DEVICE_MAX_CONNECTIONS`, read by the CUDA driver and by NVIDIA's OpenCL, which is built on it). Each queue
   costs the context's creation about 18 ms, in every process:

   | GB10 | 32 | default (8) | 2 |
   |---|---|---|---|
   | CUDA, `cuDevicePrimaryCtxRetain` | 0.63 s | 0.19 s | 0.09 s |
   | OpenCL, `clCreateContext` | 0.65 s | 0.17 s | 0.09 s |

   - **CUDA: 2, one a stream.** The waiting root and the queue root make one stream each. A third stream is refused
     (`STREAMS`): it would share a hardware queue with the waiting root's, whose device-side wait for the host would
     hold it back. That is the deadlock `CudaQueueRoot`'s notes record.
   - **OpenCL: 2.** OpenCL also makes short-lived queues: the probes at open, and one for each `reflect`. They can
     share a root's hardware queue at any count. That is safe: a root holds each wait for the host in its backlog
     (`ClQueueRoot::drive`), never on the device. So a queue sharing a hardware queue waits only behind work already
     enqueued, which completes.
   - A value the environment sets is kept.
6. **A grid's groups are sized by its work** (`pp_arena::groups_for`): one group for each `GRID_SMALL` (4,096, grid.h's
   `UKL_GRID_SMALL`) of the unit's nodes, or of its expansion's tokens, from 1 up to `groups()` (0568's residency).
   - Every grid entry point takes it: `evaluate`, `definitions_from`, `lookups_sized` and each run of `expand_sized`.
     The expansion's run takes it again as a refused run grows its token room.
   - Past its work a group only waits at the barriers, and a barrier costs more with every group. On the GB10 under
     CUDA, the 18 runs of one eval test took 58 ms of GPU time at 1 group, 173 ms at 8 and 640 ms at 32, ten times
     OpenCL's. OpenCL's device bound gave it 8.
7. **The room test is three tests** (`a_unit_past_its_token_room_runs_again_larger`, `…_scratch_room_…`,
   `a_line_past_its_room_runs_again_larger`, over `past_its_room_runs_again_larger`). Each is one shape past one room.
   The single test ran all three shapes twice (C and C++) and took more than 1 s on every backend. 0448, 0468, 0486 and
   0520 name the three.

## Tests

- `pp_arena::tests::a_units_groups_are_one_a_small_grids_nodes`: `groups_for` gives one group for each `GRID_SMALL`
  items, from 1 to `groups()`, and `GRID_SMALL` is grid.h's `UKL_GRID_SMALL`.
- The eval tests (0448), run on each backend: items 2 to 4 were each found by a red one on OpenCL.

## Measured

2026-11-21, debug builds, one test thread (`--profile second -j 1`):

| | Eval tests (1 s each) | Other |
|---|---|---|
| GB10, CUDA | 34 of 34 | `rerail-device` and `rerail-accel` 76 of 76 |
| GB10, OpenCL | 34 of 34; the slowest, `a_line_past_its_room_runs_again_larger`, 0.93 s | the same crates 76 of 76 |
| M3 Max, Metal | 34 of 34 | `pp_arena` 11 of 11 |
| M3 Max, UKL's CPU backend | 34 of 34 | `pp_arena`, `split` and `grid` 16 of 16 |

- **OpenCL's slowest test, per process.** At 1.09 s before item 5:
  - the driver's load and its platforms, 0.14 s;
  - the context, 0.17 s (0.09 s at 2 queues);
  - the source's preprocessing, 0.14 s;
  - the program from the store, 0.02 s. The store holds NVIDIA's PTX, which the driver's own cache keeps compiled;
  - the C run's three rungs, 0.24 s, then the C++ run's, 0.23 s.
- **Many test threads on one GPU.** At nextest's default threads (about 20 processes on the GB10's one GPU), 14 of 34
  stopped at 1 s, under CUDA and OpenCL both. Each process makes a context and runs its grids at once on the GPU, and
  each test then takes about twice as long. One thread: 34 of 34.
- **The CPU backend's first process** compiles its kernels (3.5 s on the M3 Max) and keeps them in the store. Under the
  1 s profile a cold store stops every test before the compile is kept. One run without the profile fills it.
