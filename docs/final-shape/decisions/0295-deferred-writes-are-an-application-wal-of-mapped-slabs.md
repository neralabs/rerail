# 0295. Deferred writes are an application WAL of mapped slabs, archived into SQLite as rows in bulk

Status: built (audited 2026-11-23). Decided (2026-11-16), by the user's two requirements and answers below. Path steps
2 (the tier), 3 (Outputs on blocks), 4 (the memo in segments) and 5's frames, tapes and sources built. Converges 0266
(blocks) with 0173/0241/0244/0276 (the program image and its versions), 0239 (the output image), 0240 §4 and 0280 (the
memo's pack), 0259 §5 (the spool) and 0260 §8 (the journal block). Keeps 0008 (facts read through virtual tables) and
0024 §5 (work can be redone, so no `fsync`).

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

The GPU writes it through a no-copy wrap, as any slab. What the OS does with pages a kernel wrote (§7) sets how a
slab is given back:

- A kernel's writes are marked dirty when the driver wires the pages for the device, not by the writes. While the
  driver holds them wired, no writeback takes them, and it holds them until the process **unmaps the range**: not
  when the buffer is released or its deallocator runs.
- Only an `msync` of the range writes them back once unwired; a `sync`, the OS's periodic flush and the process's
  exit do not.

So at a wave's end, once the wave's command buffers have completed:

1. **The wave's thread** drops the slab's wrap, writes the block's length, and hands the slab to the store's writer
   thread. It leaves the budget. It does not `msync` or `munmap` (on macOS both wait for the host's dirty pages to be
   written, §7). Readers in the process keep reading it in place (0260 §8.5).
2. **The writer** `msync`s the slab (`MS_ASYNC`: the host's dirty pages), unmaps it once its readers let go (the
   driver lets go of the GPU's pages; 0.1 ms, nothing is written then), maps the file again read-only, and
   `msync`s it with `MS_SYNC` until `mincore` shows no page dirty (the GPU's pages; the writer waits, no one else
   does). The driver lets go some 190–260 ms after the unmap, whatever the size, and an `msync` before that skips
   the wired pages and still succeeds, hence the repeat (bounded: 10 s, then the segment is given up). Then it
   writes the header's state, frozen, last: a segment whose state says frozen has every page written back before it.
3. Readers and the archive read the read-only mapping. Dirty and written-back pages of a file mapping are not
   charged to the process (`phys_footprint`, §7); the OS drops them under pressure.

Nothing is `fsync`ed. A segment whose header does not say frozen (the process ended first) is not trusted: it is
removed, and its wave is made again (0024 §5). The state written last is what makes the GPU's pages safe, since
nothing else would write them back.

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


**A segment's table past the estimate** (2026-11-17). A writer is told how many entries a write makes, and each
segment's entry table has room for those still to come. The frames placed as encoded (Q89) are told the
retained part's relations plus four an input, but an input has a frame per changed granule: about 3,500 frames
against the estimate. Once the estimate was spent, every new segment's table held one entry, so each frame was
a segment of its own, leased, mapped, named and written back: 13,677 segment files for a write of 166 MB on 50
Python files of B-now, against 34 before, and 0.09 → 3.3 s of the store writer's CPU. A segment past the estimate
now has room for as many entries as were placed so far (`SegmentWriter::slots`): 45 files, the same bytes.

Frames placed as encoded are also encoded outside the writer's lock (`cross_rows::encode_windows`): each column
into a buffer of the job's in one pass over the rows, then copied into the entry under the lock. Counted in a
first pass and written in a second under the lock, the rows were read twice and the jobs framing a write's
inputs took turns: 0.25 s of CPU became 1.8 s, and their waits more.
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

**The memo's entries are archived as pointers, not rows** (the user, 2026-11-16). An entry stays in its segment, in
the mapped form (0239's image, uncompressed, framed in the segment's block): reading it back is a memory map. The
archive inserts one row an entry, which finds it (space, part, path, content, document key) and points at it
(segment, offset, length); the segment is not freed, and SQL reads the entries' facts through the virtual table
(§5). Since a segment outlives its archive, the pack's compaction stays: once the bytes no row names are most of
the segments, the named ones are copied to a new segment and the old are removed (never given back to the pool:
another process may hold them mapped).

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
lives made the GPU's writes reach the disk. Touching after the release did at 1 GiB only. The deallocator of the
wrap (`DEALLOC=1`) runs at once (0.0 ms), and still no page is written back. After the process ended, a `sync`
left the files at 0 MiB on disk with their bytes in the cache; a new process's read-only map and `msync` then wrote
them back (64 and 256 MiB), with no store to any page.

In the process: **unmapping the range, then mapping the file again and `msync`ing it** (`REMSYNC=1`) wrote back every
page the GPU wrote, 64 MiB to 1 GiB, with or without the wrap released or a deallocator; the unmap took 0.1 ms. §1's
protocol is this.

From XNU's source: a wire for a device to write (`vm_object_iopl_request` without `UPL_COPYOUT_FROM`) marks each
page dirty; a pageout (`vm_object_upl_request` with `UPL_COPYOUT_FROM`) skips wired pages. So the pages are dirty
from the start and cannot be written while wired, and the GPU driver's wire lasts as long as the range is mapped.
A host touch adds nothing (a load does not set the modified bit, and the page is dirty already).

A bounce buffer (`PWRITE=1`): the block is anonymous, filled by the kernel or the host, frozen, and written to the
file by `pwrite`. Every size reached the disk whole (64 MiB to 1 GiB, all pages clean 1–24 ms after the check), the
GPU's writes as the host's. `pwrite` took 40 ms at 64 MiB, 99–133 ms at 256 MiB, 0.9–2.2 s at 1 GiB (load average
18): the page cache pushes large writes out as it takes them, so it too runs on the writer thread.

Why, from elsewhere:

- XNU (`osfmk/vm/vm_map.c`, `vm_object.c`, `memory_object.c`): `msync(MS_ASYNC)` returns each dirty page to its
  pager from the calling thread and only skips waiting for the I/O, so it costs the issuing of every page's write.
  A page is written back when `vmp_dirty` or the CPU's modified bit (`pmap_is_modified`) says so; a device's writes
  set neither after the wire.
- Linux has the same limit, written down: a device writing page-cache pages it pinned bypasses `page_mkwrite` (the
  file system's chance to allocate blocks, for a sparse or copy-on-write file such as APFS's) and races writeback,
  which may clean a page the device then writes again (LPC 2018, "get_user_pages() + DMA"; the kernel's
  `pin_user_pages` documentation leaves writeback of pinned pages a TODO). The supported ways are a bounce buffer
  written with `write`, or a file-system-aware DMA path (NVIDIA's GPUDirect Storage), which Metal has only for
  reads (`MTLIOCommandQueue` loads files into resources).
- SQLite and LMDB both read through a map and write with `write`: SQLite's map is read-only (stray pointers cannot
  corrupt the file, and changes stay private until commit); LMDB's writable map (`MDB_WRITEMAP`) is an option, "slower
  for DBs larger than RAM", and loses that protection.

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

1. **Measured** (§7): the GPU's writes to file pages are written back once the range is unmapped and `msync`ed.
2. **The tier** (built): `rerail_blocks::deferrable` (`Deferrable::create`, `seal`, `Sealed::reader`,
   `Sealed::write_back`), `Block::seal` and `Block::over_with`; in the pool (`accel/src/gpu/shared.rs`)
   `lease_deferrable`, `lease_deferrable_within` (counted live until sealed or dropped) and `return_deferrable` (an
   archived file kept by directory and class, reused renamed, 16 at most); `blocks::journal::init`/`add` on any
   block. Tests: a host-written slab is written back and only then frozen; a kernel's writes reach the disk; a reused
   file reads zero where allocated; a journal in a pool lease is handed off, read, frozen and its file reused. The
   writer thread itself is the store's, wired in with the memo (step 4).
3. **Outputs on blocks** (built): the outputs file (`outputs.rro`) is frozen blocks of kind `OUTPUTS` one after
   another, each one image at its root (`rerail_blocks::block::frame` to write, `frozen_at` to read, the same check
   as `Block::open_file`), read in place as before; a block cut short or not frozen is refused, and an image without
   its block. The image format, the stack and its fold are 0239's (§6.3); images in memory (a run's, a memo
   entry's) are their bytes alone. 0239's tests pass unchanged, with one more
   (`the_file_holds_each_image_as_a_frozen_block`); a session's and reuse's tests are the check (59 of 60; the
   other, `value_types_are_ws_value_types`, fails the same without this change: work queue Q27). `rerail-blocks`
   is a dependency of the logic crate on every target, so it builds without a mapping too (`blocks::mem`: a heap
   reservation on WASI; `deferrable` is Unix-only); the engine checks for `wasm32-wasip1` again, which it had not
   since `rerail-accel` took `rerail-blocks`.
4. **The memo's entries in segments** (built; §4, archived as pointers): each batch the store's writer writes is one
   segment, `<store>.memo/<n>.seg`, a journal block (`rerail_blocks::journal::put`, an entry table of its own) of
   the entries' images uncompressed at 64 bytes, written through a host-only deferrable lease of the pool
   (`lease_deferrable_host`: no device wrap, whose wire would hold the writeback ~200 ms), written back and frozen
   before the write transaction, which numbers it, renames it into place and inserts the rows (`framed` 1). A
   reader maps the segment (`journal::open`) and reads the image in place (`SegmentBytes`, `Image::in_block`); no
   zstd. Compaction copies the named entries to one new segment and removes the rest, with the packs of before
   (`framed` 0, not read). On WASI the block is made in memory and written as one file. Measured: a segment write
   is a few ms (store tests: nine writes and a compaction in 89 ms). Then the run writes them itself: it encodes
   each entry into its segment's mapped pages (`SegmentWriter`, the image's regions written there by
   `Finished::write`, no copy of it whole first), seals the segment, drops the decoded entries, and queues the
   sealed segments (`SealedSegments`) with the rows; the writer's job writes them back and inserts the rows, and is
   run again whole after a busy store (`put_memo_segments`: written back once, renamed once the rows commit, its
   files removed if never named). A queued entry's readers in the run read its image there (`Found::Frame`), not a
   decoded copy; of an entry kept over the stored one, the stored entry with the new identities over it. A segment
   no device wrapped is written back in place (`Sealed::write_back`: one `MS_SYNC`, its mapping and readers stay),
   so a reader holding it never holds up the writer. The queue counts a job's rows, not its images: those are the
   OS's pages. Test: `a_queued_entry_is_read_from_the_segment_the_run_wrote`. Not yet: the virtual table (step 6).
5. **Frames** (the largest writes; built), then tapes and sources; 0259's guard (no `write_back` queues a closure).
   Step 4's segments are now the store's, not the memo's (`content_store::segments`: `SegmentWriter`,
   `SealedSegments`, `SegmentBytes`, `SegmentMaps`). A frame's encoded bytes are placed by the run, when it queues
   them, into a segment of `<store>.frames/` (`place_frames`; one copy of the encoded bytes, the heap copy then
   dropped), and the queued frame reads them there. The writer writes the segments back, numbers them
   (`frame_segments`), writes each frame as a pointer row (`frames.seg`, `seg_at`, `seg_len`; `data` empty, `seg`
   0 for a frame whose bytes are inline), and names the files once the rows commit. Every read of a frame (the
   snapshot, cross-space reads, a part's frames, an admitted part) maps its segment (`Bytes::Mapped`); a frame
   whose segment is gone is not found, and is derived again. A forked process writes its frames inline, as before:
   the writer renumbers their symbols. Upkeep compacts the segments as it does the memo's
   (`compact_frame_segments`). The queue counts what the job holds in memory (the frames not placed, and 64 bytes a
   row), not the placed bytes, which are the OS's pages. Pointer rows
   change what an older binary would read, so the store's floor is 5 (`content-v5.sqlite`): an older binary keeps
   its own file. Test: `a_queued_frame_is_placed_in_a_segment_and_read_there`; `frames_are_queued_encoded` checks
   that the queue counts under a hundredth of the bytes. Not yet: frames encoded straight into the segment (the
   encoder writes to the heap today).
   **Tapes and sources** (built; the user's answer: raw, read in place): a run's parse workers add each parsed
   file's source and tape (held as the run holds them, no copy) to the run's gather (`write_back::Gather`, one per
   run and kind, kept beside its wave still being written). Once it holds 16 MB of the budget (`gather_cap`), or the
   wave is sealed, it is placed in one segment of `<store>.objects/` and queued as one job of that wave; a read of a
   queued tape (`ContentStore::queued_tape`) finds it gathered, then placed, by its key, and `settle` seals the wave
   that holds it. The writer writes the segment back, numbers it (`object_segments`), and writes each source and tape
   as a row of codec 0 pointing at its range (`seg`, `seg_at`, `seg_len`; no frame), moving a stored row there
   (upsert), so a row whose segment went missing is whole again once its file is parsed again. Every read of either
   (the corpus's tapes before the first wave, residency, the bulk reads, search indexing, the prefilter's tapes) maps
   the segment. **No inline rows (2026-11-19, the user: "why do we have inline sql rows? ... We should not have them
   anymore").** Inline zstd rows (`seg` 0) had stayed for writes outside a run's waves (a standalone build, a
   publication, a snapshot, `put_source`/`put_tape`), for rows written before, and for a gather that found no room on
   disk. Now every write places its objects in a segment: outside the waves `Placing` writes one segment of the
   write's sources and tapes before its transaction, which numbers it (`max(seg)+1`) and inserts rows pointing at it,
   and names the file once the rows commit (an overlay publication's pass gives its rows `seg` 0 as "this pass's
   segment", which its SQL resolves to the number the pass's transaction registered); a gather with no room is dropped
   and its files parsed again later. No source or tape is compressed (`compress_source`, `compress_tape`, the
   shipped dictionaries' use and the decompressed read cache are gone: a read is the bytes in their segment). The
   floor is 7 (`content-v7.sqlite`): an older store, which may hold inline rows, is refused, not read. Results,
   relationships, the facts memo and tree pages keep their zstd rows (not sources or tapes). Tests:
   `objects_whose_rows_disagree_with_their_segments_fail_closed`, `source_and_tape_objects_deduplicate_and_round_trip`
   (no row holds bytes). Upkeep compacts
   the segments; a row whose segment is gone is marked `seg` -1 (never a segment's number) rather than deleted, as
   other rows name it. The store's disk budget (`capacity`) counts the three segment directories, which it did not.
   Test: `a_runs_tapes_and_sources_are_gathered_into_a_segment_and_read_there`.
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
