# 0580. Where the preprocessor's GPU time goes: a chain of phases, and CUDA's pinned memory

Status: accepted (2026-11-21, the user: "this is still an incredibly slow preprocessor despite running in parallel on
a GPU. tell me why. You can start with metal and cuda as they have the best tooling"). A measurement, and the one
fix it found. The kernels' design (0468 §4, 0527) is unchanged.

## Intro

A batch of 50 of the corpus's C units (1 MB of source, 188k nodes, 236k tokens out with its C++ half) took 10.7 ms
of GPU time on the M3 Max and 85.6 ms on the GB10 under CUDA. That is 1 MB in 10–86 ms, on devices that move
hundreds of GB/s.

## The measurements

- **A trace** (`RERAIL_PP_TRACE`): a run at `STOP` = `PP_TRACE_ON` has the grid's first thread write, at every
  phase's tick, the phase's source line and the device's clock (`ukl_clock_ns`: CUDA's global timer, 0 elsewhere)
  into the grid store's words past its 8 (`pp_eval::Front::trace`). `on_grid` reports the time by line, most first.
  With `RERAIL_PP_TRACE_ALL` it also prints the phases in order. On CUDA the traced span is the GPU time to within 3%.
- **The barriers a run passed** (`Front::barriers`, `GRID[1] / 2`; `RERAIL_PP_BARRIERS` prints them a run).
- **Stops**: `batch_stages` takes `RERAIL_PP_STOPS` (a batch stopped at given phases, best of 9) and
  `RERAIL_PP_STAGES_ALL` (every phase). Metal gives a kernel no clock, and its GPU counters are refused through
  xctrace, so stops are its only per-phase measure. One phase apart they are too noisy to read: single steps swing by
  milliseconds both ways. Its section boundaries are readable.
- **`what_a_barrier_costs`** on each backend, and a standalone CUDA program over each kind of memory (below).

## Finding 1: the run is a chain of about 650 dependent phases, each with a floor of several µs

The batch's run is 643 phases and 756 grid barriers, the same on both backends. Its sections:

| Phases | What |
|---|---|
| 1–47 | the front, the definitions, P1's first iteration |
| 48–262 | P1's next two iterations, each with an `#if` line's drain |
| 263–283 | the text's setup (its sorts, scans and places) |
| 284–638 | the text's expansion rounds (0527): `px_r_calls`, `px_r_substs`, `px_r_scans` (14 a round), `px_r_walks`, `px_r_give`, `px_drain` |

Each phase is a few items a thread over 8,192 threads (32 groups of 256), then a barrier. Its cost is set by its
dependent round trips to memory, not by its items:

| Barrier at 32 groups | M3 Max | GB10, CUDA |
|---|---|---|
| grid barrier | 1.67 µs | 1.36 µs (host pages; below) |
| device fence | 0.37 µs | 0.48 µs |
| coherent dependent load | 0.085 µs | 0.137 µs |

Per phase, though, the batch averages about 16 µs on the M3 Max (10.7 ms / 643) and 9 µs on CUDA with host pages
(5.8 ms). On CUDA (the trace, 2026-11-21):

| Line | Time | Phases | Each |
|---|---|---|---|
| `px_drain` | 1.07 ms | 56 | 19 µs |
| `px_r_walks` | 0.81 ms | 96 | 8.5 µs |
| `px_r_scans` | 0.81 ms | 126 | 6.4 µs |
| `px_r_calls` | 0.67 ms | 39 | 17 µs |
| the rest, 300 phases | 2.4 ms | | 5–10 µs |

The expansion's rounds are 63% of the run. Their count is the batch's deepest rescan, the same for every unit in
the batch. On the M3 Max the stops give: to phase 284, 5.0–5.6 ms; the rounds, 5.1–5.6 ms of 10.7.

## Finding 2: CUDA's blocks were pinned host memory, whose atomics and fences are slow

Every block on CUDA was `cuMemHostAlloc`'s pinned, mapped host memory. On the GB10 the barrier's atomics and fences
on it take a slow path:

| GB10, a standalone program, 32 groups | Grid barrier | Store and fence | Volatile load |
|---|---|---|---|
| pinned (`cuMemHostAlloc`) | 24–39 µs | 0.60–0.79 µs | 0.43 µs |
| `cudaMalloc` | 0.67 µs | 0.28 µs | 0.18 µs |
| managed | 0.68 µs | 0.28 µs | 0.18 µs |
| `malloc`'d pages, read through ATS | 0.68 µs | 0.28 µs | 0.18 µs |
| `malloc`'d pages, `cuMemHostRegister`ed | 0.67 µs | 0.28 µs | 0.19 µs |

So the cost is the pinned allocation's, not the host memory's.

**Decision.** On a device that reads pageable memory through the host's page tables (`CudaDevice::pageable`, a
GB10), a pool block is the host's pages (`os::map_zeroed`), read at its host address, as `wrap_host_memory` already
reads a wrap there. Elsewhere it stays `cuMemHostAlloc`'s. A block is still the host's memory, read in place: no
copy (0353). `RERAIL_CUDA_PINNED` keeps pinned blocks, a measurement aid.

| GB10, CUDA | Pinned | Host pages |
|---|---|---|
| `what_a_barrier_costs`, grid barrier, 32 groups | 42.6 µs | 1.36 µs |
| the same, device fence | 17.3 µs | 0.48 µs |
| the corpus batch of 50 C units | 85.6 ms | 5.9 ms |
| the launch test's large runs | 25.3, 16.6 ms | 3.2, 2.5 ms |
| the slowest eval test | 0.84 s | 0.69 s |

## Also found

- **A size with a comparison broke a kernel's rows** (`rows::split_top`). It counted `<` and `>` as a template's
  brackets everywhere. CUDA's shim, and OpenCL's since 0573, stringize the rows expanded, and `UKL_GRID_SORT_WORDS`
  expands to `(A > B ? A : B)`, so `pb_sort_alone` read 44 row fields for 45 numbers. Inside parentheses they are
  comparisons now. Test: `rows::tests::a_comparison_in_parentheses_is_no_bracket`.
- **`the_grid_is_the_devices_resident_count` predated 0568's backend answer.** CUDA gives no core count, and its
  occupancy sets the grid: the test takes the answer where the backend gave one.
- **The first run of a CUDA process** has about 6 ms of GPU time outside its phases: the module's first launch.
- **A cold kernel store under the 1 s profile** stops a test before its kernel is compiled and kept. Metal's
  `the_paste_relexer_is_the_lexers` (10.8 s cold, 0.32 s after) did, as the CPU backend's tests did in 0573.

## What would change the floor (not decided here)

- Fewer phases. The rounds' count is the batch's deepest rescan, and P1 repeats its whole tree pass each iteration.
- Fewer dependent round trips a phase, and phases fused where their items do not cross groups.
- Units that ran alone at 1 group skip the barriers' cross-group cost. A batch takes its slowest unit's round count.

Results (2026-11-21): CUDA, the eval tests 34 of 34 under the 1 s profile, `pp_arena` and `rows` 13 of 13, the
device and accel crates 76 of 76; OpenCL, `pp_arena` 11 of 11; Metal (M3 Max), the eval tests 34 of 34 and
`pp_arena` 11 of 11; UKL's CPU backend, the eval tests 34 of 34.
