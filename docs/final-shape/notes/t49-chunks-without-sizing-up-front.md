# t49. Chunks without sizing up front: a spike

## Intro

A run evaluates its files in chunks. A chunk is a set of documents that every file-local stratum runs over once.
Today the engine picks each chunk's size, in nodes, before the chunk runs: a guess from earlier chunks' growth, so
the chunk's memory fits the run's budget. A guess too small multiplies the work paid once a chunk. A guess too large
fails, and on Metal a failed wave stops the run (decision 0382 §6).

The question for this spike: can the engine stop sizing up front, either by growing a chunk as it goes or by
shrinking it once its real size is known?

Short answer:
- The guess is forced by how memory is counted, not by memory. On Apple's unified memory the engine keeps two budgets
  (the run's line, 1 GB, and Metal's, 512 MB "on top") with two measures: the footprint, and the full length of every
  Metal buffer. Neither is what the engine writes, and each sees the other's memory.
- Big chunks are the cheap ones: on hogql, 262,144-node chunks retire 71-72 B against the default sizing's 86-87 B,
  the same as before oscar's merge (69-70 B). That is the whole of Q312's rise.
- Big chunks need more memory than today's small ones (2.2-2.5 GB peak against 0.9 GB), part of it real (about
  390 MB of rows), part of it idle buffers and reservations a single budget of bytes written would not keep.

A first version of this note said big chunks needed no more memory, and that Metal cut chunks short. Both came from
runs where my script passed two variables as one (zsh does not split `$XENV`): those runs had the default sizing and
budget. §3 and §4 are measured again, and §7's first two questions are answered.

All runs: posthog/hogql cold (437 files, 2.6 M nodes), 1,629 findings each, profiling build of 95d4d439a plus the
spike's switch, an M-series Mac at load 15-40, two runs each, format 15.

## 1. What limits a chunk on Metal

The refusals that stop big chunks are small buffers (16 KB to 1.5 MB) asked of a device the driver says is full.
At 131,072-node chunks, a refusal saw:

| The driver's count | MB |
|---|---|
| Leased by relational work (live leases) | 380 |
| Of which the tape residency's slab | 64 |
| In no ledger (no-copy wraps of memory already held, §2) | about 150 |
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
4,000-108,000 nodes); the fixed room around it leaves the wave nothing.

## 2. What the driver counts, and what memory is

Probes (`scripts/dev/metal/footprint.swift`, `nocopy.swift`, `gpuwrite.swift`) compared the driver's count
(`currentAllocatedSize`) with the process's physical footprint:

| Step | Footprint | Driver's count |
|---|---|---|
| Wrap 1 GB of reserved, untouched memory | +0 MB | +1,024 MB |
| The host writes 16 MB of it | +16 MB | +1,024 MB |
| `newBuffer` of 256 MB, untouched | +0 MB | +256 MB |
| The first command buffer of the process | +114 MB | +0 MB |
| A GPU fill of 64 MB of a `newBuffer` | +64 MB | (counted at creation) |
| A GPU fill of 512 MB of an untouched wrap | +0 MB | (counted at the wrap) |
| The host then reads those 512 MB (they hold the GPU's bytes) | +512 MB | +0 MB |

- The driver counts lengths: a wrap of memory already held counts again, and reserved room counts as if written.
  The ~150 MB "in no ledger" of §1 is such wraps: `rerail_blocks`' device wraps (`block.rs`, `gpu.rs`), the executor's
  wraps of host columns (`accel/src/gpu/mod.rs`), metal_join's starts. The driver's own setup is under 1 MB.
- The footprint misses pages the GPU writes into a wrap until the host touches them. It cannot be the device's
  measure either.
- What the engine writes is the one count that sees all of it: `Block::used` for blocks, a lease's written end for
  the rest. The engine knows it whichever processor wrote.
- The first version's "GPU-written pages counted twice" was the first command buffer's one-time 114 MB.
- Wrapping costs 3 µs for a prefix and 23 µs for 256 MB.

## 3. What a chunk's size costs

Metal binding, the budget lifted to 4 GB so nothing is refused:

| Metal | Instructions | Peak footprint | Chunks |
|---|---|---|---|
| 512 MB budget, default sizing | 86-87 B | 894-914 MB | 75-84 |
| 512 MB, fixed 65,536-node chunks or more | - | - | every run stops |
| 4 GB, fixed 262,144-node chunks | 71-72 B | 2,227-2,493 MB | 16 |
| 4 GB, default sizing | 94-98 B | 2,109-2,141 MB | 90-93 |

CPU binding, fixed sizes:

| Chunk nodes | Chunks | Stratum runs | Instructions | Peak |
|---|---|---|---|---|
| 8,192 | 149 | 7,919 | 98-99 B | 275 MB |
| 32,768 | 61 | 3,206 | 84 B | 275 MB |
| 131,072 | 24 | 1,277 | 86-87 B | 327-367 MB |
| 262,144 (the most) | 16-17 | 867 | 82-86 B | 338-409 MB |

- On Metal, chunk size is the whole of the rise since oscar's merge: 71-72 B at 262,144 nodes.
- On Metal a 262,144-node chunk leases 1.0-1.4 GB for about 390 MB of rows written (a run's 19 blocks, 359 MB of
  822 MB), and keeps 300-477 MB idle, which a 4 GB budget lets the pool hold. The peak's largest rise (1.5 GB) is in
  the guests' `ann_ctx` stratum.
- Metal and the CPU cut the same 16 chunks at 262,144 nodes.

## 4. Chunks that grow

The spike's switch, `RERAIL_CHUNK_GROW=double` (`plan/corpus.rs`, marked SPIKE): the first chunk is 4,096 nodes,
and each next is twice the last, held to what the latest chunk added a node on the host (footprint) and on the device
(the driver's peak while it ran). One chunk at a time.

| | Instructions | Peak | Chunks |
|---|---|---|---|
| CPU binding | 96 B | 246-249 MB | 64-66 |
| Metal, 512 MB budget | 90-93 B | 900-941 MB | 170-186 |
| Metal, 4 GB budget | 106 B | 1,940-1,983 MB | 240-244 |

It fails for the reason of §2, measured from both sides:
- On Metal at 512 MB the device starts each chunk at 357-413 MB of the 384 MB allowed (§1's reservations), so growth
  stops at once.
- At 4 GB the device's blocks and idle buffers stay in the footprint (one memory), which starts each chunk at
  0.7-1.5 GB, past the run's 1 GB line, so the host's fit is 0.
- On the CPU it reached 262,144 nodes within three chunks; its extra chunks are short batches.

Growing works only once there is one budget with one count.

## 5. Growing and shrinking a block without copying

A block's references are offsets within it, so its base address does not matter. Growth needs no copy either:
- Reserve a large range of address space for the block (free on a 64-bit machine). Pages become memory as rows are
  written.
- Hand Metal a no-copy buffer over the range once. The budget counts the bytes written, not the wrap's length.
- Shrink by releasing the unused tail's pages (`madvise`). Nothing points past the written end.

Today a block is a fixed lease (64 MB or 16 MB), and a tail whose block is full moves to another by one copy
(`facts::rows`). The relations inside are already segmented: their columns are doubling chunks (`rerail_blocks`), and
a relation's segments chain without copying.

## 6. Recommendation

1. One memory budget, counted by what the engine wrote: blocks' written bytes, leases' written ends, the heap's live
   bytes. Not the footprint, not the driver's lengths. The GPU pool's idle buffers are kept within it.
2. Row blocks reserved large and grown in place: one reservation a block, wrapped once for the device, its tail's
   pages released when it closes. The `FACTS_BYTES` choice goes.
3. Chunks grow from small, each from the latest chunk's measure in that one count, with no up-front size. A wave that
   still does not fit splits by documents (0334's split, which exists for the matcher's roots).

## 7. The open questions, answered

- GPU-written pages counted twice: no. It was the first command buffer's one-time 114 MB. But the footprint misses
  GPU-written wrapped pages, so the budget counts what the engine wrote (§2).
- Metal cutting chunks short: no. Those runs had the default sizing (my script's bug). Metal cuts 16 chunks at
  262,144 nodes, as the CPU does.
- The 15 B after oscar's merge: chunk size, all of it (§3).
- New: what the guests' `ann_ctx` stratum holds at a big chunk (the peak's 1.5 GB rise), measured as part of
  step 1.
