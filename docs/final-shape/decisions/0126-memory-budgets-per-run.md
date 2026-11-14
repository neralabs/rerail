# 0126: Memory budgets are set per run: 1 GB overall, Metal half of it on top

Status: built (2026-11-12).

## Intro

**The problem.** The engine's memory limits were constants spread over the code: a 256 MB device
buffer limit, a 192 MB write queue, a 96 MB working set, a 256 MB session kept part, and fixed
device-join sizes. None could be changed for a run, and the device's relational work leased
buffers outside every limit: a level's wave held 400–600 MB of leases on public_api_triples.

**The decision.** Two budgets, set per run:

| Budget | Default | Flag | Variable |
|---|---|---|---|
| overall | 1024 MB | `--memory-mb N` | `RERAIL_MEMORY_MB` |
| Metal | half the overall (512 MB), **on top of it** | `--metal-mb N` | `RERAIL_METAL_MB` |

So a run with Metal may use 50% more memory than the overall budget. Every command takes both
flags. The profile's report names both in its header.

## Where they live

`rerail_accel::budget` holds both: `memory()`, `metal()`, `set_memory_mb`, `set_metal_mb`.

- `share(at_default)`: a part of the overall budget, scaled from its size at 1 GB. These parts
  use it:
  - the write queue (`write_back::cap_bytes_default`, 192 MB at 1 GB);
  - the working set (96 MB);
  - the session's kept part (256 MB, unless `RERAIL_CROSS_RETAIN_MB` is set).
- `metal_share(at_default)`: a part of the Metal budget, scaled from its size at 512 MB. The
  device joins' table, batch, arena, resident and headroom sizes use it.
- The device buffer limit (`metal_buffer_limit`) is the Metal budget.

## Relational leases are counted and bounded

- The budget left (`relational_bytes_left`) subtracts the buffer pool's reservations, and also
  what relational work holds in leases, both live and idle.
- A stratum that finds the headroom gone runs in a submission of its own, released when it ends
  ("strata run apart: the wave's budget was spent").
- While device joins encode (`within_budget`), a lease past the Metal budget is refused, and the
  stratum runs on the CPU. The executor's lane has no CPU fallback, so it is not bounded.
- For a lease refused partway through an encode: a `Pass` dropped mid-encode ends its encoder. A
  `Batch` dropped with uncommitted work drops that work instead of committing it. Committing at
  a thread's end went through a thread-local that was already destroyed, and aborted.

## Checked

`the_budget_left_counts_relational_leases` and `a_bounded_lease_keeps_within_the_metal_budget`
(both mutation-checked).

| strict on Metal | relational leases at 512 MB | at `--metal-mb 256` |
|---|---|---|
| public_api_triples | 381 MB | 186 MB |
| internal_dead_code | 395 MB | 197 MB |

Findings are the same as native's at both budgets. At 256 MB, many matcher waves fall back to
the CPU; at 512 MB, few do.

## Open

The overall budget bounds the parts above, not the whole process: the peaks in 0127 are over
1 GB.
