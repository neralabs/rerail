# 0398. A kernel calls its walk once

Status: accepted (2026-11-18)

## Intro

After 0397, format 16's `label_micro` still took about 6 s of the label pipelines' cold compile, against 1 s for
format 15 (work queue Q181). The compiler inlines everything, so a kernel's size is its call tree, written out:

- The tape's reads (0310, translated by `rerail_clif::msl`) build on each other. Format 16's `tape_next` is
  `parent`, then `end_of`, then `next_in`, each over `size`. Each may escape to a side list (`tape_f16_side`, a
  binary search). Inlined, one `tape_next` is about 1,000 lines, most of them copies of that search. The function
  constant `tape_format` prunes format 16's paths from a format 15 pipeline, so only format 16 pays.
- `label_micro` called its walk (`micro_from`, with every read under it) from two places: `micro_walk`, for an
  item's roots, and `micro_drain`, for a spilled frame. `root_records` called `root_walk` twice, to count a root's
  records and then to write them. Each call was a copy of the walk.

## Decision

1. **A kernel calls its walk once.**
   - `label_micro` runs one loop (`micro_run`): its items' SIMD rounds, then the queue's spilled frames, each turn
     setting up a walk and one call of `micro_from`. What decides the rounds (`item - lane`, `r.total`) is the same
     on every lane of a SIMD group, so the shuffles still see the whole group; the queue's turns are each lane's own,
     as before.
   - `root_records_run` walks in a loop of two, counting and then writing, one call of `root_walk`.
2. **The reads stay inline.** Compiling the side lists' search once, out of line (`__attribute__((noinline))`),
   halved format 16's compile, but every walk paid for the call (below). The JIT's cold reads (`Read::hot`) stay
   calls on the CPU only.
3. **The probe can test both** (`label_kernels_compile_cost`): `RERAIL_PROBE_NOINLINE` and `RERAIL_PROBE_INLINE`
   change a generated read's mark in the probe's source only, and each pipeline's thread limit is printed.

## Evidence

M3 Max, debug build. The host's Metal compiler service is shared with other runs, so wall times vary up to 2×
between identical runs; each comparison below ran back to back in one job.

| format 16 compile | before | after |
|---|---|---|
| `label_micro` (two runs each, load about 30) | 6.10, 5.89 s | 2.42, 2.55 s |
| `root_records` | 1.83–2.02 s | 0.80, 0.84 s |
| every pipeline | 9.72–10.10 s | 5.50, 5.54 s (load 64–77) |

Every pipeline keeps a limit of 1,024 threads. Device time (`on_metal_anchored_walks_find_the_unanchored_roots`, ten
of corpus0's files, the warmed anchored run): 487 ms (`label_micro_8` 23.7 ms) against 483 ms (22.2 ms) just before.
The walks without anchors: `root_records_8` and `_32` 1.9 and 1.3 ms against 2.0 and 1.1.

The out-of-line search, against all inline, back to back (load about 35): `label_micro_8` 44.7 against 22.2 ms,
`label_group` 248.5 against 186.5 ms, all device time 619.3 against 482.8 ms, for a compile of 2.99 against 6.31 s
for `label_micro`.

Tests: `the_label_kernels_compile_for_every_format`, `on_metal_anchored_walks_find_the_unanchored_roots` (both
modes, every tier), `spilled_walks_find_the_cpus_roots` (41 waves, 1,665 roots, every walk spilling, through the
merged loop's queue turns).

## Not yet

- `label_group` (1.4–1.6 s) calls `micro_from` once; what is left is the reads' own expansion under format 16.
  Sharing the reads' work (`tape_next` computes `size` four times) would need the reads written so in CLIF.
