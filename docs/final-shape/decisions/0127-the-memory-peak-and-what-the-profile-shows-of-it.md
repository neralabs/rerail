# 0127: The memory peak: atoms in segments, frames encoded as made, and a profile that names the holders

Status: built (2026-11-12).

## Intro

**The problem.** public_api_triples peaked at 1.53 GB native and 2.23 GB on Metal, against a
1 GB budget (0126). The profile showed the footprint but not who held it, so the causes could
not be found from a run.

**The decision.** First make the profile name what holds the peak. Then remove the copies it
shows: atoms are committed in bounded segments, and frames are encoded as they are made.

## What the profile shows

- **Holders.** Each memory sample reads the Metal driver's bytes, the device pool's reservations,
  relational leases (live and idle) and SQLite's heap, beside jemalloc's. The report lists them
  at the peak ("by holder then"), and `memory.tsv` has a column for each.
- **The live heap by span.** With `RERAIL_HEAP_SAMPLE=1`, the global allocator samples one
  allocation per 256 KB, plus every allocation of 64 KB or more. The Tree-sitter and SQLite
  allocator shims are sampled too. Each sample is keyed by the allocating thread's innermost
  span and by allocator. At the peak, the report lists the live heap by span (`rerail_prof::heap`).
- **Device leases by span.** Accel tells a hook about each lease and its drop (accel cannot
  depend on the profiler), so leases appear by span beside the heap, tagged as device leases.
- **A run stopped by timeout still writes its profile** (to `prof/interrupted/`). The memory
  sampler releases its lock before it reads the open spans. Before, the SIGTERM dump, the
  sampler and a thread closing a span waited on each other, and no report was written.

## Atoms are committed in segments

A run's new atoms (141 MB on public_api_triples) were written as one segment. The arena's bytes,
a copy, the statement's copy and SQLite's record of the row were all live at once. Now:

- `SEGMENT_BYTES` (8 MB) bounds a segment;
- the stored bytes move into the held segment instead of being copied;
- a frame's encoded blocks move into the snapshot after the write, instead of being cloned
  before it.

## Frames are encoded as they are made

The kept part's changes were collected as rows, queued as rows, and encoded by the writer. On
internal_dead_code, the end of the run held 305 MB of row copies beside their 138 MB encoding.
Now retain encodes each frame as it makes it (`Put::encoded`), the queue and the snapshot share
the encoded frame, and the writer encodes nothing.

## Measured

Strict peak footprint; findings unchanged throughout.

| | start | after segments | after frames encoded |
|---|---|---|---|
| public_api_triples, native | 1532 MB | 1344 MB | 1214 MB |
| public_api_triples, Metal | 2230 MB | 1813 MB | 1573 MB |
| internal_dead_code, native | 2201 MB | | 1882 MB |
| internal_dead_code, Metal | 2581 MB | | 2254 MB |

Bounding relational leases (0126) moved them at the peak from 522–599 MB to 208–283 MB.

## Open

- internal_dead_code's session kept part is about 500 MB, by design.
- Metal's unnamed "rest" is about 250 MB in some runs.
