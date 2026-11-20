# 0518. The Metal budget is the overall budget

Status: accepted (2026-11-20). Changes 0126's default for the Metal budget; the flags, the variables and the way the
parts scale are unchanged.

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

## Tests

- `rerail-accel`'s `budget::tests::budgets_default_set_and_share`: the default, the scaling of a Metal part at it,
  and a set overall budget the Metal one follows.
