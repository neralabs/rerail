# 0397. A walk's stack tiers are one kernel, and a format's label pipelines compile together

Status: accepted (2026-11-18)

## Intro

The label passes run on Metal through 16 kernels in one library, each made a pipeline per tape format (a function
constant). With nothing in Metal's shader cache, a run compiled them for about 100 s before its first dispatch
(work queue Q181). A user pays that after every engine update.

A probe (`label_kernels_compile_cost`, ignored) times each step: the library's front end, then per kernel its
function and its pipeline. On the M3 Max, under load, it found:

- The front end is about 1 s for all 16 kernels (139 KB of MSL). The back end is the rest: format 16 took 67.2 s,
  format 0 52.8 s, format 15 7.3 s.
- The walk interpreter was compiled 11 times. `label_micro` came in four stack tiers (8, 32, 64 frames, and the
  deep run's 128), `label_group` in five (8 to 256) and `root_records` in two (8, 32), each a template instance on
  `FRAMES`. Each copy took 3–10 s for format 16.
- Yet every tier's stack is in device memory (`stacks`, a thread's frames at `tid * FRAMES`). `FRAMES` was only a
  depth bound and a stride, not a register array.
- The pipelines compiled one after another.

## Decision

1. **A walk's frames are the dispatch's, not the kernel's.** `Shape::frames` carries them, and `micro_walk`,
   `micro_drain`, `micro_from`, `label_group_run`, `root_walk` and `root_records_run` take them as a value. One
   kernel each serves every tier: `label_micro`, `label_group` and `root_records`. `KERNELS` goes from 16 to 8.
2. **The tiers keep their names.** A dispatch is labelled with its tier (`label_micro_8`, `label_group_64`,
   `root_records_32`, `label_micro_deep`, …), so `RERAIL_GPU_KERNEL_TIMES`, the profile's `kernel.*` and
   `threads.*` counts, and the notes that cite them read as before. The pools, strides and stack buffers are
   unchanged.
3. **A format's pipelines compile together** (`pipelines(format)`): the first kernel alone, which builds the
   library, then the other seven on threads of their own. Metal's compiler service runs them at once. A failed
   compile still fails the run (0382): a thread's panic goes on in the caller.
4. **Not a `.metallib` built with the engine, not here.** 0395's machine-wide store of compiled kernels (on
   `benchmarks-on-integration`) is the cache across processes. This cuts what any cold compile costs.

## Evidence

M3 Max, debug build, the probe on format 16, load average 170 on the first, about 40–55 on the before:

| | before | after |
|---|---|---|
| kernels | 16 | 8 |
| pipelines, one after another | 67.2 s | 10.7 s |
| pipelines in parallel (wall) | 22.9 s | 6.9 s |
| the heaviest pipeline | 10.5 s (one of four `label_micro_*`) | 6.5 s (`label_micro`) |

After: `label_micro` 6.47 s, `root_records` 2.02 s, `label_group` 1.48 s, `label_formula` 0.54 s, the rest under
0.2 s each. The parallel wall is now set by `label_micro` alone.

The walks are as fast with their frames a value. `on_metal_anchored_walks_find_the_unanchored_roots`, ten of
corpus0's files, `RERAIL_GPU_KERNEL_TIMES`, the warmed anchored run, device µs, against 0396's evidence (same machine,
load average 41–75):

| kernel | 0396 | 0397 |
|---|---|---|
| `label_group` (8, 32 and 64) | 200,702 | 198,033 |
| `label_formula` + `label_leaves` | 309,354 | 289,847 |
| all device time | 543,861 | 523,712 |

`the_label_kernels_compile_for_every_format` (three formats, 8 kernels and the pattern kernel each) took 46.5 s,
the pipelines cached from the probe's run on the same host for format 16 only.

## Locked by

- `the_label_kernels_compile_for_every_format`: 8 label kernels and the pattern kernel, every format.
- The Metal walks' comparisons (`on_metal_anchored_walks_find_the_unanchored_roots`,
  `lowered_clauses_find_the_same_roots_on_metal`, `spilled_walks_find_the_cpus_roots`) run every tier.

## Not yet

- `label_micro` is 6.5 s of the 6.9 s. Format 16's reads (from CLIF, `metal_tape_reads.metal`) cost 9× format 15's
  in the back end; that is the next lever.
