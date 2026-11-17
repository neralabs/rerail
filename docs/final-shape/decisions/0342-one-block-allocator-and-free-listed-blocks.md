# 0342. One block allocator, and blocks with a free list

Date: 2026-11-17. Status: decided (user, 2026-11-17); built in steps below.

## Intro

A block (`rerail_blocks::Block`) is one mapping that structures are allocated in. Until now each place that
needed one chose its memory itself: setfix's relations leased a slab of the GPU pool when the run was bound to
the device and had room spare, else took anonymous memory; the Metal slabs leased within the Metal budget; the
content store's journal, the syntax vtab's cells and four builders inside the blocks crate took anonymous memory
directly. No one place saw them all, so no one place could weigh the run's memory as a whole.

And every block was bump-allocated: memory given out was never taken back until the whole block went. A chunk's
relations that are dropped, or that move when their block fills, kept their written pages until the chunk ended
(Q58: about 35 MB of median footprint on PostHog cold).

## Decided

### One allocator, one policy

Every block is made through `rerail_blocks::alloc::block(&Need)`. A caller states what it needs, never which
memory:

```rust
Need {
    kind,           // the block kind (FACTS, CHUNK, JOURNAL, ...)
    bytes,          // what it reserves
    writes,         // Writes::Once: built and frozen (bump); Writes::Churn: open structures made and dropped
    device,         // Device::No | Device::May (read in place if bound) | Device::Must (a device slab)
}
```

The policy that answers it is installed once (`alloc::install`), by `rerail_accel::blocks` at the pool's setup;
without one (the blocks crate's own tests) a block is anonymous. The policy is the one place that sees memory
pressure: the Metal budget and its live leases (the pool's), and the host blocks it made (a ledger of their bytes,
`rerail_accel::blocks::host_held`, which the profile reads beside the pool's). It chooses:

- `Device::Must`: a slab of the pool within the Metal budget (what may evict for it is the pool's eviction order).
- `Device::May`: a slab only when the budget holds it with nothing evicted (the old `lease_block_spare`), else
  anonymous memory; the device reads an anonymous block in place through a no-copy buffer.
- `Device::No`: anonymous memory.

The run's binding is the caller's to state (a CPU run's relations ask `Device::No`): it is the run's context,
which the allocator below the engine does not see.

### Two kinds of block

- **Bump** (`Writes::Once`): as before. Built front to back, then frozen.
- **Free-listed** (`Writes::Churn`): chunks given back are reused. The design is held to what a GPU kernel runs
  as well as the host, and to big pieces, not millions of small ones:
  - Classes by size, powers of two from 64 B. A relation's column chunks already are powers of two in bytes
    (rows a chunk times a 4- or 8-byte width), so a chunk is never split and free chunks are never merged.
  - Each class's list is a stack whose head lies in the block's own memory (so it is mapped with the block and
    a kernel can bind it): 32 bits of offset in 64-byte units and a 32-bit tag, one 64-bit compare-and-swap a
    push or pop. A free chunk's first word holds the next one's offset. Self-relative, as every block reference.
  - Taking a chunk: pop its class, else bump from the front. A reused chunk is zeroed, as a fresh one reads.
  - Freezing drops the lists: free chunks of a page or more are given back to the OS, and an anonymous block's
    reservation past its length is released.
- Small structures (a relation's descriptor and column directories) are still bump-allocated in either kind.

A relation is given back when its last holder drops it: setfix's `Part` releases its chunks to the block's lists,
and a reader of the block in place (`InBlock`) holds the part, so nothing it reads is reused under it.

## Steps

1. Free-listed blocks and relation release in `rerail_blocks` (tests there).
2. `rerail_blocks::alloc` (the need and the hook) and `rerail_accel::blocks` (the policy, its ledger).
3. Every production block through it: setfix's relations (`Writes::Churn`), Metal slabs (`Device::Must`), the
   journal, the vtab's cells, the blocks crate's builders.
4. Measured on PostHog cold with the memory line lifted (the fast path, decision of 2026-11-17).

## Not now

- The free list's kernel form (MSL): the device never makes chunks during a pass (they are made between passes,
  `rerail_blocks::rows`), so only the host takes and gives chunks today. The layout and the two operations are
  the kernel's to share when it does.
- Moving a frozen free-listed slab to a smaller one (a pooled slab cannot be truncated).

## Measured (2026-11-17, M3, PostHog cold, profiled, the memory line lifted: `RERAIL_RUN_MB`)

| | user CPU | instructions | peak | chunks |
|---|---|---|---|---|
| before 0325 (recorded, load 26–35) | 208.6 s | 2,696 G | 2,297 MB | 170 |
| 0325 (recorded, load 78) | 196.7 s | 2,382 G | 2,917 MB | 178 |
| 0342, two runs (load 8–11) | 192.1, 191.6 s | 2,498, 2,491 G | 2,472, 2,529 MB | 174 |

Findings 29,747, the same hash. Median footprint 1,849 and 1,837 MB. The host blocks (the new ledger): median
155–166 MB, at most 274–287 MB, of which a median 40–48 MB lies free on their lists.

- The peak is about 400 MB under 0325's, and still about 200 MB over the run before it.
- What the lists hold free and is not taken again (40–48 MB) is chunks of a class no later chunk asks for: a
  list does not split a chunk. A relation's descriptor and column directories (about 800 B a column) are still
  bump-allocated and never given back.
- About 110 G more instructions than 0325's run; not yet attributed (a reused chunk is zeroed, where a fresh page
  came zero from the OS).

## Descriptors given back; rows not zeroed; the instructions traced (2026-11-17)

- A relation's descriptor and both row sets' column directories are one chunk, given back whole by `release`.
- A relation's rows are written before they are read (claimed, then put), so a chunk taken again for them is not
  zeroed (`try_alloc_chunk_any`), nor a dirty slab's allocation for them. A descriptor still is. The bytes zeroed
  are counted (`block::zeroed_bytes`, in the profile's memory report): 33–34 MB a PostHog run, a few million
  instructions at most.
- The 110 G more instructions than 0325's recorded run were not the zeroing. Measured alternated on the M3, same
  build of each, the memory line lifted (load 26–40):

| | instructions | user CPU | page faults | peak | median footprint |
|---|---|---|---|---|---|
| 0325 (49dad1939) | 2,407, 2,489 G | 190.8, 198.0 s | 2.33, 2.40 M | 2,872, 2,683 MB | 1,983, 2,133 MB |
| now (4947193ba) | 2,428, 2,620 G | 194.5, 223.2 s | 2.90, 2.46 M | 2,489, 2,360 MB | 1,818, 1,859 MB |

  The first pair, at the same load, differs by 21 G (1%); the second by 131 G, with the load rising to 40 and the
  run's wall to 68 s. The same build's two runs differ by 82 G and 192 G: at this load a difference under about
  5% is not measurable from two runs, and none is shown. The memory is: the peak 300–500 MB under 0325's, the
  median footprint 150–300 MB under.
