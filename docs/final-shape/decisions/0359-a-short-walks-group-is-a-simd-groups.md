# 0359. A short walk's group of 64 nodes is a SIMD group's; the device's parent stays a climb

Status: accepted (2026-11-17)

## Intro

The device's label walks (`label_micro_8`) took 172–217 ms at s1000, against 76–107 ms on the hub engine, for the
same walks: 97,241,194 steps and 68 launches in both. Work queue Q57 put the gap on the parent: format 16's
`parent` is a climb back over sibling slots, and the hub engine's walks read a parent column. The fix it named was a
walk that computes the parents of its 64-node window in one forward pass.

The user asked for a spike to check that and measure the savings. The spike found the parent is not the cost. The
cost is how few threads a dispatch had.

## What the spike measured

internal_dead_code at s1000, every matcher wave pinned to the GPU, `RERAIL_GPU_KERNEL_TIMES=1`
`RERAIL_GPU_WALK_STEPS=1`. Each engine was warmed by an s10 run, and the engines were run in turn at load 45–96, so
a kernel's time varies by about ±20% from run to run. Runs are compared only when their walks match: the walks per
tier, now printed in the profile, were identical.

| Variant | `label_micro_8` | What it shows |
|---|---|---|
| This branch (a thread per 64-node group) | 172–217 ms | |
| Every parent read from an oracle column, made by a kernel before the walks | 220 ms | No gain: the parent is not the cost |
| The window's parents from a forward pass over its slots | 170 ms | No gain |
| The root mask made twice | +8–16 ms | Making the mask is about 10 ms |
| A group's nodes over 8 / 16 / 32 threads | 110–112 / 94–110 / 94–97 ms | The cost is the dispatch's shape |
| The hub engine (a thread per listed root) | 76–107 ms | |

The climb counters show why the parent does not matter:
- there are 4.0 M parent climbs in 97 M steps;
- a climb takes 1.8 hops on average;
- 92% of the parents lie in the start's own 64-node window;
- 0.09 M steps escape to the side list.

## Why the shape costs

Each pass's items are its groups of 64 nodes. With a thread per group, a dispatch of `label_micro_8` had about
8,800 threads (600 k over 68 launches), against the hub engine's 23,000 (a thread per root). A SIMD group then
walked the roots of its 32 groups (about 560) in rounds of 32, one after another. There were too few threads in
flight to hide the walks' memory latency, and a SIMD group's long run of rounds made a long tail at the
dispatch's end.

## Decision

1. In the short walks' tiers (`label_micro_8`, `label_micro_32`), an item (a group of 64 nodes) is a SIMD group's.
   - Lane `l` finds the roots among the group's nodes `2l` and `2l + 1` (`root_mask_in`, `Shape::lanes`).
   - The SIMD group then walks the group's roots, 32 a round (`simd_roots`).
   - The dispatch has 32 threads an item, up to the tier's stack pool.
   - The host still lists nothing (0293 §10).
2. The loop tier (`label_micro`, `label_micro_deep`) keeps a thread an item (`TIER_LANES`).
   - Its pool is a fixed 4,096 threads, and its groups hold about 6 roots each.
   - A SIMD group an item would leave most of its lanes idle.
   - At s1000, 1, 4 and 32 lanes an item measured the same within the noise.
3. The device's parent stays format 16's climb (0340). No window table and no parent column. The tape is still read
   in place.
4. The clause pattern kernel keeps a thread an item. Its fact items hold one root each, and a SIMD group per fact
   would leave 31 lanes idle. Its 6.5 ms against the hub engine's 3 ms is Q57's next item.

## Measured

Same runs, same walks: t0 13,582,114 walks and 78,262,139 steps; t2 342,213 walks and 18,979,055 steps.

| Engine | `label_micro_8` | `label_micro` | Label passes (GPU) |
|---|---|---|---|
| Before | 172–217 ms | 160–181 | 351–408 ms |
| A short walk's group is a SIMD group's | 88–134 ms | 171–203 | 278–358 ms |
| The hub engine | 75–107 ms | 149–163 | 245–303 ms |

The findings are the same (0).

## Consequences

- `label_micro_8` is about 1.8× faster, at the hub engine's speed within the noise, with the tape read in place.
- The profile's walk section prints each tier's walks and steps over all passes. Two runs walked the same starts
  exactly when these match. A deep rerun (`run.deep`) walks a level again, so the counts differ between runs of
  one engine.
- The spike's kernels are not kept. Its results are recorded here and in t41.
