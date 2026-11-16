# 0295. Deferred writes are an application WAL of mapped slabs, archived into SQLite as rows in bulk

Status: decided (2026-11-16), by the user's two requirements and answers below. Not built. Converges 0266 (blocks)
with 0173/0241/0244/0276 (the program image and its versions), 0239 (the output image), 0240 §4 and 0280 (the
memo's pack), 0259 §5 (the spool) and 0260 §8 (the journal block). Keeps 0008 (facts read through virtual tables)
and 0024 §5 (work can be redone, so no `fsync`).

## Intro

0266 made a block the form of every big thing the engine holds: one slab, nested structures, self-relative
references, read the same on the CPU and the GPU. Three formats built on sibling branches already hold the same
things in forms of their own:

- the **program image** (0173, Typing-support): regions, and versions in stacks (views, overlays, derivations),
  folded when a delta reaches half its parent;
- the **output image** (0239): an image and up to eight overlays, which name the rows they take away by a 128-bit
  hash of their content, folded at depth eight or when an eighth of the rows changed;
- the **memo's pack** (0240 §4, 0280): each entry's image compressed with zstd, appended to a pack file, found by a
  SQLite row.

And there were two plans for writing a wave's work behind the run: 0259 §5's spool (one mapped image file per sealed
wave, applied by the writer and removed), and 0260 §8's journal (a file-mapped delta block per wave, applied to
SQLite in one transaction).

The user set two requirements (2026-11-16):

1. **Facts stay queryable from SQLite**, through some combination of real and virtual tables. It can be built later,
   but no format may rule it out.
2. **Everything possible of deferred work is written to memory-mapped, disk-backed pages**, already in an efficient
   format, so that the OS does the writeback and the engine can hand dirty memory to the OS after each wave; the
   writeback into SQLite then works over pages already mapped. This is done by the existing slab paths, which know
   about GPUs, as a new tier, **deferrable**, that works the same way as the others.

And answered: SQLite is written as rows, in bulk, as an archive of the pack: the slabs are an application WAL.
zstd is not used, at least for now.

## Decision

### 1. The deferrable tier

The pool (`accel/src/gpu/shared.rs`, 0266 §15–16) holds every GPU buffer under one budget, by size class, with one
eviction order (`make_room`). A **deferrable lease** is a lease of that pool whose slab is a file-backed shared
mapping (`MAP_SHARED`) in the store's WAL directory, not anonymous memory. Everything else is as for any lease:

- blocks are made in it by `lease_block` (a `deferrable` variant), grow by `grow_block`, and are frozen as any block;
- while it is open it counts in the one budget, as any live lease.

It differs in one thing, measured (§7): **the GPU never writes a deferrable slab.** A kernel's output is written to
an anonymous pooled slab, as today, and the host copies the frozen range into the deferrable slab at the wave's end
(one `memcpy`, about 0.1 s a GiB on one thread, split across threads for large ranges). The GPU may read a
deferrable slab through a no-copy wrap; reading is not affected.

At a wave's end each deferrable slab the wave wrote is frozen (truncated to its length, its header written last)
and **released to the OS**: it leaves the budget, and the mapping goes to the store's writer thread. Dirty pages of a
file mapping are not charged to the process (`phys_footprint`, measured §7), so the release is the hand-off; the
OS writes them back and drops them under pressure. No `msync` and no `munmap` on the wave's thread: on macOS both
write the dirty pages back before they return (§7). The writer unmaps a segment after archiving it. Nothing is
`fsync`ed: a segment that did not reach the disk whole does not check when read, and its wave is made again
(0024 §5).

The pool keeps archived segments' files as idle buffers of their class, as it keeps idle anonymous slabs, so a new
wave's lease reuses a file whose pages may still be cached instead of making one.

### 2. The format in the slabs: packed blocks, self-describing

A deferrable slab holds 0266's **packed** form (§14): 2.4 times smaller than the in-place form, read in place by the
CPU, the GPU and a virtual table. No zstd (the user, 2026-11-16).

A block that leaves the process is **self-describing**:

- each relation's spec records each column's type (int, node, text, and the branded kinds of 0270), not only its
  width;
- text columns hold indices into a string table the block carries (0239's rule, 0240's `memo::hold`); a reader maps
  them to its own atom ids once per block, one lookup per distinct string. In memory, columns hold the process's
  atom ids as now (**ids in memory, content at rest**);
- node ids are 0239's: the document key in the high bits, the document table by key and path.

### 3. The WAL: segments, read ahead of SQLite

A sealed wave's deferrable slabs are a **segment** of the WAL, numbered in sealing order. This is 0259 §5's spool,
with its files being the pool's slabs, and 0260 §8's journal block generalized to every kind of deferred write:
frames, memo entries, outputs, tapes and sources, later facts.

Between its sealing and its archive a segment is **read in place**:

- the run that wrote it reads its frames' snapshot and a queued write's `pending` from it (0260 §8.5);
- another run or process finds memo entries by each segment's own entry table (the `EntryLoc`s of
  `blocks::journal`, by document key and content digest), consulted after SQLite's rows, as a SQLite reader
  consults its WAL;
- a segment is trusted only once its header checks (state and length); one left by a process that ended is
  archived or removed, never read unchecked.

### 4. The archive: bulk inserts of rows, then the segment is free

The store's writer **archives** sealed segments into SQLite as **rows of real tables**, in bulk: as many whole
segments as are ready, in one transaction, each relation inserted from the segment (one prepared statement
stepped over the block's rows, or `INSERT … SELECT` from the segment's virtual table, §5). The same transaction moves
the store's `applied_wave` to the last segment taken (0259 §5), so an archive interrupted is done again whole, and a
segment at or below `applied_wave` is removed unread.

After the transaction, the segment's file goes back to the pool (§1). This is a WAL's checkpoint and reset; it
replaces 0240 §4's compaction of the pack, since a pack never outlives its archive.

When to archive (the writer's pace) is the writer's choice: at the run's end, when idle, or when the unarchived
bytes pass a bound (0259 §5's disk and pace limits). Producers never wait for the archive while the disk has room.

### 5. SQL over both: real tables and virtual tables

A relation is queryable as **its archived rows (a real table) plus its unarchived rows (a virtual table over the
WAL's segments)**, one view uniting them. One virtual-table module reads any relation's packed block in place,
pushing filters in and returning integers (0008's rules), mapping text through the block's string table. In-memory
blocks of a running wave can be served by the same module. This is built later; §2's self-description is what it
needs now.

### 6. What this settles of 0266's convergence

1. **The program image is ported by Typing-support**, onto this layer. They are building on it now (0281 step 2,
   analyses held by their version), and their trace validation on dl1 is what judges whether 0266's delta expresses
   0244's views at their measured cost. The target: 0173's `Version` interface unchanged above; blocks below; a
   stored program image is a packed block in a deferrable slab, read in place (0260 already removes its copy into a
   `Vec<u128>`). This branch builds the tier and the block layer. Handed off in the work queue (Q26).
2. **0260 §8's journal is kept and generalized** (§1, §3): not a journal format of its own, but the deferrable tier
   plus §3–4's rules. 0240 §4's pack becomes the segments; its rows stay as the archive's memo rows; zstd goes.
3. **One fold rule: deferred, with a deadline.** Base and delta now persist, and a virtual table must merge them, so
   when to fold also sets SQL read cost. It is decided at 0266's path step 4 (facts as blocks) or the program
   image's port, whichever comes first. Until then Outputs keep 0239's stack and fold, and the memo its entries and
   0280's identity records, each inside blocks.
4. **Kills in deltas.** By content (0239's 128-bit row hash): needed when facts persist across processes, so decided
   with step 4. By range: only programs' views need it; decided with the program image's port.

### 7. Measured: file pages, the GPU, and writeback

`native/crates/blocks/examples/deferrable.rs` (0260's open Q-R5): a block in a `MAP_SHARED` file, 64 MiB to 1 GiB,
filled by a Metal kernel through a no-copy wrap or by the host; frozen; `msync(MS_ASYNC)` or not; unmapped; read
back from the file; then polled with `mincore` until no page is dirty, and the file's allocated blocks counted (it is
sparse, so blocks on disk are bytes written back). M5 Max, macOS 26.4, under load (load average 7 to 37).

| Filled by | Read back equal | Dirty (`mincore`) | Written back after 3–20 s | `msync` | `munmap` of dirty pages |
|---|---|---|---|---|---|
| host | yes | 100% | all (1 GiB: 1,024 MiB on disk) | 0.1–13 s at 1 GiB: waits for the writes | 0.1–17 s at 1 GiB: waits for the writes |
| GPU, wrap | yes (the cached pages) | 100% | **none: 0 MiB on disk**, all pages still dirty | 0.1–7 ms: finds nothing to write | 0.1 ms |
| GPU, wrap released, host touch | yes | 100% | at 1 GiB all; at 64 and 256 MiB none | as host / as GPU | |

Neither releasing the wrap, an autorelease pool around the dispatch, nor the host touching each page while the wrap
lives made the GPU's writes reach the disk. Touching after the release did at 1 GiB only: when Metal lets go of the
pages is not the engine's to know. A page the OS takes as clean is dropped under pressure, and a later read gets the
file's old bytes, so GPU writes through a wrap of file pages could be lost. Hence §1's copy.

`phys_footprint` did not count dirty file-backed pages the host wrote (0 MiB where an anonymous block counted
1,024), mapped or not; resident size did while they were mapped. Neither counter saw pages a kernel wrote through a
wrap, anonymous or file-backed, so the pool's own budget is what counts those.

## Supersedes and keeps

- **0259 §2's one transaction a wave** becomes one transaction an archive, of whole waves; §1, §3 and §4 stand.
  **0259 §5** is built as §3–4 here: its files are the pool's slabs; its image is a packed block.
- **0260 §8** is generalized (§1); its steps 1–6 hold, with the region a pool lease and §8.4's application the
  archive. Its open Q-R5 (file-backed pages under a no-copy GPU buffer) is the first thing measured.
- **0240 §4**: the pack and its compaction are replaced by segments and the archive; zstd at rest goes. Its rule
  that a row naming bytes that do not decode is a miss carries over to segments.
- **0102**'s writer is the archiver.

## Path

1. **Measured** (§7): the GPU does not write file pages; the release hands the mapping to the writer.
2. **The tier**: deferrable leases in the pool, the host copy of GPU output at freeze, release at a wave's end to
   the writer thread, the files reused as idle buffers, `blocks::journal` as its first user.
3. **Outputs on blocks** (0266 path step 1), 0239's tests unchanged.
4. **The memo's entries in segments**: 0240's rows found by the archive, the pack retired.
5. **Frames** (the largest writes), then tapes and sources; 0259's guard (no `write_back` queues a closure).
6. **Facts as blocks** (0266 step 4) with the fold rule and kills by content decided; **the virtual-table module**.
7. The program image, by Typing-support (Q26).

## Open

- The archive's cost against today's per-wave transactions, measured once the memo is in segments: rows a second,
  and the store's size, now that memo rows are archived as rows.
- Several processes writing segments at once: one numbering per store (a counter in SQLite, taken at sealing) or
  per process (segments named by process and sequence, archived in either order).
- Bump allocation from several writers in one slab (0260's open item stands).
- A GPU with its own memory (CUDA later): a deferrable lease there is a host mapping plus a device copy, and the
  budget splits.
