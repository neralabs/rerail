# 0549. The Metal budget is the overall budget

Status: partly built (audited 2026-11-23). Left: Remove each thread's own resident copies and sorts of the same
relations, so the Metal budget can come back down. Accepted (2026-11-20). Changes 0126's default for the Metal budget;
the flags, the variables and the way the parts scale are unchanged.

## Intro

A run keeps two memory budgets (0126): the overall one, for what the process keeps resident on the host, and the
Metal one, for the device's buffers, on top of it. The Metal budget was half the overall one by default: 512 MB at the
default 1 GB.

At 512 MB the device's strata do not fit together. Q203 (c) measured it on the M3 (300 files, strict, `--metal`): at a
refused lease about ten strata hold 8-113 MB each, and 446-674 strata runs a run go to the CPU after they were placed
on the device (7.5-10.2 s of CPU). Counting more of each stratum at admission only moved larger strata to the CPU
(work queue, Q203 (c)).

## Decision

1. **The Metal budget is the overall budget by default**: 1 GB at the default 1 GB, so a run with `--metal` may use
   2 GB in all. `--metal-mb` and `RERAIL_METAL_MB` still set it.
2. **The parts sized from it keep their proportion.** `metal_share(at_default)` still gives a part's size at a 512 MB
   budget, scaled: at the default 1 GB the device joins' table, batch, arena, resident and headroom sizes are twice
   what they were.
3. **The room is not the fix.** The work that remains is to make the device's strata need less of it, so that the
   budget can come back down: the measures are the strata run on the CPU after the device refused them and the peak
   of the device's leases, at 512 MB and at 1 GB.

## Measured

ab104 (the M3, 2026-11-20, 300 files, strict, `--metal`, a 1 GB overall budget, profiled, load 8-15), one binary at
both Metal budgets, alternated:

| Metal budget | 512 MB | 1 GB (this decision) |
|---|---|---|
| Instructions | 585, 587, 603 G | 501, 495, 497 G |
| Strata runs sent to the CPU after the device refused them | 696, 668, 582 | 116, 109, 110 |
| Their CPU time | 10.9, 9.6, 9.8 s | 4.1, 3.6, 2.9 s |
| Refused device leases | 109, 86, 152 | 14, 8, 13 |
| Footprint near the peak (host and device) | 2.70-2.78 GB | 3.08-3.11 GB |
| Violations | 463 each | 463 each |

The runs still refused at 1 GB are large strata (26-35 ms a run on the CPU).

## What threads keep (the work left)

A thread kept on the device, past every admission, its device joins' sort scratch (two sets, each slot grown to the
largest sort it ran and never given back) and its own copies of the relations its strata read (`resident`: their
sorts, views made, uploads, texts). Counted (the profile's "device kept by threads between strata", ab105, 1 GB):
scratch 276 MB at most and all of it at the end, resident relations 171 MB at most.

The scratch now goes back when a thread's outermost device stratum ends with no stratum waiting for its wave
(`join::scratch_return`), to Metal, which frees it once the command buffers that hold it completed. Given to the
pool instead, a sort still running wrote into a block the pool's next lease made of it (Q265's check stopped the
run). ab108 (one run, 1 GB, load 9): scratch 96 MB at most, 12 MB at the end, 2,375 MB given back over the run;
490 G instructions, 80 runs back to the CPU, 463 violations. The resident copies grew into the room (281 MB at most):
each thread's own copy and sort of the same relation is what is left to remove.

## Tests

- `rerail-accel`'s `budget::tests::budgets_default_set_and_share`: the default, the scaling of a Metal part at it,
  and a set overall budget the Metal one follows.
