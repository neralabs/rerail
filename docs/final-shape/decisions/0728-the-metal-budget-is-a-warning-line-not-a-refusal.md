# 0728. The Metal budget is a warning line, not a refusal

Status: decided (2026-11-27), by the user: "from now on I want breaking the device budget to be a warning not a
refusal. I don't want to deal with memory optimization just yet." Built. Amends 0126 (leases kept within the Metal
budget), Q203's admission (a stratum that does not fit runs on the CPU) and 0549 (the Metal budget).

## Intro

At index-aware step 1 (b54a2a5a65), 6,725 runs of 327 strata were placed on the device and then refused at run time:
`device admission: N MB short`. They ran on the scalar walker, so step 1's profile measured a different placement
from step 0's: the join program's items fell by 183 G instructions and the scalar joins rose by 118 G. With a
Metal budget of 4,000 MB instead of the default 1 GB, the same build refused 794 runs. So the budget, not the
work, decided where those strata ran.

Until memory is the work at hand, a stratum placed on the device runs there whatever the budget says, and the run
says it passed the line.

## What it is

- `admit` (accel `gpu::shared`) still waits for room another thread is about to give back, as before. Where it
  used to refuse (past the whole budget; the room held by threads that go on; past its patience), it reserves the
  bytes past the budget instead.
- `lease_within` and `lease_deferrable_within` lease past the budget where `make_room` cannot make room.
- A device join's binding table past one table's share of the budget (`table_max`) is counted, not refused.
- Each pass is counted (`budget_passed`: the passes and the most bytes past), the first four said on stderr, and the
  profiler's hook told, which names what holds the device's leases then. The profile's `== Warnings` gives the count.

What still splits work is unchanged: a submission past `batch_max` or the wave's headroom goes into the next one,
on the device.

## Not changed

The host's own budget (`--memory-mb`, the run's memory line, decision 0435) is untouched: this is the device's
budget alone.
