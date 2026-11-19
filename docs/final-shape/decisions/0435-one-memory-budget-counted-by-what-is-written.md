# 0435. One memory budget, counted by what is written; chunks grow from small

Date: 2026-11-18. Status: decided (user, 2026-11-18, after the t49 spike); built in steps below.

## Intro

A run evaluates its documents in chunks, and each chunk's size has been a guess made before it runs, so that its
memory fits the run's budgets. t49 measured what forces the guess on posthog/hogql:

- On Apple's unified memory the run keeps two budgets (decision 0126: the run's line, 1 GB, and Metal's, 512 MB on
  top) with two measures. The run's line reads the footprint. Metal's reads lengths: every lease's whole size, and the
  driver's count, which also counts each no-copy wrap of memory the process already holds. Neither is what the engine
  wrote, and each sees the other's memory.
- A run's row blocks may take a 64 MB slab of the GPU pool while the Metal budget has room (decision 0342,
  `Device::May`). About 70 MB of rows held 260-290 MB of the Metal budget.
- So big chunks were refused, though they are the cheap ones: 71-72 B of instructions at 262,144 nodes against
  86-87 B at the guessed size.
- The footprint misses pages the GPU writes into a no-copy wrap until the host touches them, so it cannot be the
  measure either.

This decision makes the budget one, counted by what the engine wrote, and lets chunks start small and double.

## Decision

1. **A block the device may read is host memory.** A `Device::May` block (a run's relations, sources, kept rows) is
   anonymous memory, resident only where written. The device reads it through the block's no-copy wrap of its used
   extent (`Block::gpu_buffer`, remade at twice the length as the block grows). It is never a pool slab, so it holds
   no device room it has not written. Pool slabs stay for `Device::Must` (the residency's tapes, a device run's own
   relations), which the GPU writes and which are sized to their need.
2. **The Metal checks count the device's own buffers, never the driver.** What a Metal check weighs is the GPU pool's
   leases and idle buffers and `rerail-metal`'s ledger. A no-copy wrap of host memory adds nothing: that memory is
   counted where it is (§3). The driver's count (`currentAllocatedSize`) is reported, never compared.
3. **One count, one limit.** The run's memory is the heap's live bytes (jemalloc's `allocated`), plus host blocks'
   used bytes (`rerail_accel::blocks::host_held`), plus the GPU pool's leases and idle buffers, plus `rerail-metal`'s
   ledger (`rerail_accel::budget::written`). Its limit is the run's budget and Metal's together (0126's two numbers,
   summed). The Metal budget stays a bound on the device's own buffers within it.
4. **Chunks grow from small.** The first chunk is 65,536 nodes (at most the caller's). Each next is up to eight times
   the last, held to what the latest chunk added a node to §3's count at its peak (sampled every 10 ms), against three
   quarters of what the limit leaves. While chunks grow, one evaluates at a time; once the size holds, as many as the
   room holds at it. Nothing is sized from a prior or from the run's worst measure. A wave that does not fit the
   device splits by documents (0334). (`plan/corpus/sizing.rs`.)
5. **The footprint stays a report.** The profile prints the count and the footprint side by side, so a part of
   memory the count misses (a C library's own allocator) shows as their gap.

## Requirements, each locked by a test

- **R1** A `Device::May` block is host memory even when the Metal budget has room, and its device wrap is no longer
  than twice its used extent (at least 1 MB, at most its reservation).
- **R2** A no-copy wrap of a host block leaves the Metal budget's checks unchanged: a lease that fitted before it
  fits after.
- **R3** The count rises by what a host block has written, not by its reservation: an untouched reservation of
  64 MB adds less than 1 MB.
- **R4** The chunk sizing is a function of measures alone: from a first chunk it grows while the latest chunk's
  growth a node fits what the limit leaves, holds when it does not, and never passes the caller's most.
- **R5** hogql gives its 1,629 findings at the default sizing and with chunks growing, on the CPU and Metal bindings.

## Steps

1. R1 and R2 (the block policy, the Metal checks). Measure hogql.
2. R3 (the count) and §5's report.
3. R4 (the sizing), replacing 47785e475's worst-of-run rule and the spike's switch. Measure hogql: chunks,
   instructions, peak.

## Measured (2026-11-18, step 3)

posthog/hogql cold, profiling builds, format 15, an M-series Mac at load 20-90, two runs each, interleaved; 1,629
findings in every run, no Metal stop:

| | Instructions | Peak footprint | Chunks |
|---|---|---|---|
| Metal, step 1 (47785e475's sizing) | 79-82 B | 843-864 MB | 63-67 |
| Metal, step 3 | 72-73 B | 1,358-1,472 MB | 16-19 |
| CPU, step 1 | 87 B | 310-324 MB | 27 |
| CPU, step 3 | 87 B | 313-324 MB | 16-17 |

- Metal reaches the 262,144-node chunks t49 measured at 71-72 B. The count's most was 1,148 MB of the 1,536 MB limit
  (462 MB of it the GPU pool's leases); the footprint's rise past it is the first command buffer and GPU-written pages.
- Choices the measures made:
  - a first chunk of 4,096 nodes cost the CPU 1-5 B (92.5-93.7 B against 88.8-92.0 B at 65,536), with 20-30 distinct
    negation-split strata (`pre_k`) against 11; the cause is not yet known (Q261), so the first is 65,536;
  - doubling from 4,096 ran 36 chunks; growth up to eight times ran 20-23 and 2-4 B fewer on Metal (79.0/76.2 B
    against 75.7/74.0 B), measured together with the peak sampler, so not apart from it. An earlier "8 B" in the code
    and fe2007e38's message was written before it was measured, and was wrong;
  - the count read at a chunk's end missed its peak (its relations already given back), so a sampler keeps its most.

## Supersedes

- 0342's `Device::May` slab while the budget has room spare (`lease_block_spare`).
- Q260's checks by the driver's count (`make_device_room`, `idle_room`'s driver term): by §2 the driver is not
  weighed.
- 47785e475's sizing by the run's most bytes a node, and its credible-growth threshold.
