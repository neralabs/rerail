# t49. Chunks without sizing up front: a spike

## Intro

A run evaluates its files in chunks. A chunk is a set of documents that every file-local stratum runs over once.
Today the engine picks each chunk's size, in nodes, before the chunk runs: a guess from earlier chunks' growth, so
the chunk's memory fits the run's budget. A guess too small multiplies the work paid once a chunk. A guess too large
fails, and on Metal a failed wave stops the run (decision 0382 §6).

The question for this spike: can the engine stop sizing up front, either by growing a chunk as it goes or by
shrinking it once its real size is known?

Short answer: the sizing up front is forced by how the Metal budget counts memory. It charges the full length of every
buffer and block reserved, while real memory is the pages written. Counted by what is written, the device stops
limiting chunk size on hogql, and growing chunks cost what large fixed ones do.

All runs: posthog/hogql cold (437 files, 2.6 M nodes), 1,629 findings each, profiling build of 95d4d439a plus the
spike's switch, M-series Mac at load 15-40, two runs each.

## 1. What limits a chunk on Metal

The refusals that stop big chunks are small buffers (16 KB to 1.5 MB) asked of a device the driver says is full.
At 131,072-node chunks, a refusal saw:

| The driver's count | MB |
|---|---|
| Leased by relational work (live leases) | 380 |
| Of which the tape residency's slab | 64 |
| In no ledger (the matcher's workspaces, kernels) | about 150 |
| Idle in the pool | 0 |
| **The driver's count, against a 512 MB budget** | **595** |

The leases are mostly the relations' row blocks. Each block (`facts::rows::Facts`) is leased whole from the GPU
pool before rows go in: 64 MB for a run's relations, 16 MB for a batch's sources and for kept rows. At each chunk's
start in a default Metal run:

| Rows' blocks | Written | Reserved |
|---|---|---|
| A run's (9-19 blocks) | 13-56 MB | 354-663 MB |
| Sources' and kept rows' | 2-8 MB each | 48-112 MB each |
| Leased on the device in all | | 261-288 MB, plus 30-111 MB idle |

About 70 MB of rows hold 260-290 MB of the 512 MB budget. A chunk's own growth is small (2-47 MB on the device at
4,000-108,000 nodes). It is the fixed room around it that leaves the wave nothing.

## 2. What the driver counts, and what memory is

A probe (`scripts/dev/metal/footprint.swift` and `nocopy.swift`) wrapped reserved memory in no-copy Metal buffers and compared the driver's count
(`currentAllocatedSize`) with the process's physical footprint:

| Step | Footprint | Driver's count |
|---|---|---|
| Wrap 1 GB of reserved, untouched memory | +0 MB | +1,024 MB |
| The host writes 16 MB of it | +16 MB | +1,024 MB |
| The GPU fills 64 MB of it | +130 MB | +1,024 MB |
| The GPU reads 1 MB at 900 MB (untouched) | +131 MB | +1,025 MB |
| `newBuffer` of 256 MB, untouched | +0 MB | +256 MB |

- The driver counts lengths. Real memory is the pages written, for a wrapped reservation and for an ordinary buffer.
- The GPU reads untouched pages of a wrapped reservation.
- Wrapping costs 3 µs for a prefix and 23 µs for 256 MB.
- Open: the GPU's 64 MB fill added 114 MB to the footprint, as if pages the GPU writes counted twice there. Measure
  it before relying on the footprint for GPU-written blocks.

## 3. Chunks with the device budget counting what is real

The same hogql run with the Metal budget at 4 GB, so the device's count no longer cuts anything and only the host's
footprint (real memory) bounds the run:

| Metal binding | Instructions | Peak footprint | Refusals |
|---|---|---|---|
| 512 MB budget, default sizing | 89-92 B | 907-944 MB | 0 (after Q260) |
| 512 MB, fixed 65,536-node chunks or more | - | - | every run stops |
| 4 GB budget, fixed 262,144-node chunks | 86-87 B | 906-927 MB | 0 |
| 4 GB budget, chunks that grow (below) | 85-86 B | 897-910 MB | 0 |
| 4 GB budget, default sizing | 94-98 B | 2,109-2,141 MB | 0 |

- 262,144-node chunks need no more real memory than the default's small ones: the 512 MB they failed was reserved
  length.
- The default sizing at 4 GB runs more chunks at once (its headroom grows with the budget), and its peak doubles.

## 4. Chunks that grow

The spike's switch, `RERAIL_CHUNK_GROW=double` (`plan/corpus.rs`, marked SPIKE): the first chunk is 4,096 nodes,
and each next is twice the last, held to what the latest chunk added a node on the host and on the device (the
driver's peak while it ran), against the run's headroom and three quarters of the Metal budget. One chunk at a
time. No prior, and no worst-of-run measure (decision 47785e475 keeps the most bytes a node seen in the run).

| | Instructions | Peak | Chunks |
|---|---|---|---|
| CPU binding, default | 86-87 B | 342-349 MB | 27 |
| CPU binding, growing | 96 B | 246-249 MB | 64-66 |
| Metal 512 MB, growing | 90-93 B | 900-941 MB | 170-186 |
| Metal 4 GB, growing | 85-86 B | 897-910 MB | 72-73 |

- At 512 MB the device starts every chunk at 357-413 MB of the 384 MB allowed, so growth often stops at once: the
  fixed reservations of §1 decide, not the chunk.
- With the device counted by what is real (4 GB here), growing chunks match large fixed ones.
- On the CPU, growth reached 262,144 nodes within three chunks. Its extra chunks are short batches (the end of a
  load's inputs), which no sizing rule changes.

What a chunk's size costs, on the CPU binding at fixed sizes:

| Chunk nodes | Chunks | Stratum runs | Instructions | Peak |
|---|---|---|---|---|
| 8,192 | 149 | 7,919 | 98-99 B | 275 MB |
| 32,768 | 61 | 3,206 | 84 B | 275 MB |
| 131,072 | 24 | 1,277 | 86-87 B | 327-367 MB |
| 262,144 (the most) | 16-17 | 867 | 82-86 B | 338-409 MB |

Above about 32,000 nodes the per-chunk work stops showing. Below it, it is about 0.16 B of instructions a chunk.

## 5. Growing and shrinking a block without copying

A block's references are offsets within it, so its base address does not matter. Growth needs no copy either:
- Reserve a large range of address space for the block (free on a 64-bit machine). Pages become memory as rows are
  written.
- Hand Metal a no-copy buffer over the range. The driver's count is then not the budget's measure: the budget counts
  the bytes written (`Block::used`, which the engine already knows), or the footprint.
- Shrink by releasing the unused tail's pages (`madvise`). Nothing points past the written end.

Today a block is a fixed lease (64 MB or 16 MB), and a tail whose block is full moves to another by one copy
(`facts::rows`). The relations inside are already segmented: their columns are doubling chunks (`rerail_blocks`), and
a relation's segments chain without copying.

## 6. Recommendation

1. The Metal budget counts bytes written, not lengths reserved. Every ledger then agrees with real memory: the GPU
   pool's leases, `rerail-metal`'s pool, the residency's slabs, and Q260's driver check.
2. Row blocks are reserved large and grown in place: one reservation a block, wrapped once for the device, its tail's
   pages released when it closes. The `FACTS_BYTES` choice goes.
3. Chunks grow from small, each from the latest chunk's measure (this spike's rule), with no up-front size. A wave
   that still does not fit splits by documents (0334's split, which exists for the matcher's roots).

Before any of it: the open questions below.

## 7. Open

- The GPU-written pages counted twice in the footprint (§2).
- Metal runs cut 71-75 chunks at a 262,144-node target (median 20,148 nodes), against 16 on the CPU. Something on
  the Metal path cuts chunks short; find it.
- The tip retires 85-87 B at large chunks, against 69-70 B before oscar's merge. Chunking explains the default
  sizing's extra 5-10 B, not that gap (Q259).
