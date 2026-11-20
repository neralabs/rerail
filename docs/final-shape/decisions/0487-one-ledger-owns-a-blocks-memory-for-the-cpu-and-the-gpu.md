# 0487. One ledger owns a block's memory, for the CPU and the GPU

Date: 2026-11-20. Status: decided (the user, 2026-11-20, on Q337: "We need to understand live and queued readers and
writers and reference counts over blocks, what is borrowed, etc on the CPU and the GPU, of each and every block, and
have the memory management be accurate and taken care of by the system in a single place, not by each part of our
code doing cowboy memory management. We need tight invariants where any attempt to do something that won't work
fails."). Step 1 built; step 2 built for accel's `Pass`.

## Intro

The engine's rows, indexes and tapes live in blocks: large mappings, cut into chunks. The CPU and the GPU both read and
write those chunks in place (0353, 0478). A chunk can be in use in four ways at once:

- a CPU owner writes it, or CPU code reads it;
- a GPU command buffer that has been encoded or committed, but has not completed, reads it or writes it.

Until now nothing knew all four. Each part of the code kept its own piece:

| Where | What it kept by hand |
|---|---|
| `Block::free_chunk`, `free_chunk_fit`, `free_chunk_resident` (rows, relation, BlockVec) | gave a chunk back to the free list at once, whatever the GPU still had queued on it |
| `Block::freeze` → `mem::free_pages` | gave pages back to the OS; reached live neighbours' pages (Q337's first cause, fixed in 4c349a3e68) |
| `Block` drop, `Pooled` drop | unmapped the memory, or put the slab back in the pool, at once; `Pooled::after(cmd)` only where a caller remembered |
| `Batch::keep`, `read`, `held`, the old atoms directory, `Held::kept` | five lists, each a caller's promise to keep something alive until its work completes |
| `Block::relocate`, `grow_block` | "no command may be reading the block", in a comment |
| `Scratch` reset, the residency slabs' `retire` (`strong_count == 1`) | each its own guess at whether the GPU is done |

A free that forgets one of these lists does not fail. Its chunk goes back on the free list, the next relation writes
it, and a kernel that was still queued reads the new rows. Nothing reports it; a run loses a chunk's findings (Q337).

## Decision

1. **Every block has one ledger** (`rerail_blocks::ledger::Ledger`). It is the only code that hands a chunk out, takes
   a chunk back, gives pages to the OS, or lets the block's memory go. It is bookkeeping on the CPU (the user,
   2026-11-19: "Bookkeeping / scoreboards / schedules can be on the CPU for now"). The chunks stay in the block.
2. **The ledger knows every live chunk.** In a block that keeps a free list, a bit per 64 bytes says which bytes are
   live, and another says where each live chunk starts. Every allocation sets its bits, including ones made from the
   front and the block's own structures. A free clears them.
3. **A GPU use is registered before it is encoded.** A command buffer has one `Fence`. Binding a block or a chunk for
   a kernel registers a use on the fence: a read or a write, of the whole block or of one chunk. The fence's
   completion handler completes every use it holds. Dropping a fence completes it too: a command buffer never
   committed ran nothing.
4. **A free waits for the GPU.** A chunk freed while a GPU use that began before the free is still queued is retiring,
   not free. Its bits stay live, so nothing can be allocated over it, and no page of it goes to the OS. It goes on the
   free list when the last of those uses completes. Uses that begin after the free do not hold it.
5. **A block's memory waits for the GPU.** A block dropped while a use of it is queued hands its mapping (or its slab)
   to its ledger, which lets it go when the last use completes.
6. **What cannot work, fails, at once, with the chunk named:**
   - allocating over a live chunk;
   - freeing a chunk that is not live, or not where a chunk starts, or not its size (a double free);
   - a CPU write of a chunk with a GPU read or write of it queued, or a CPU read with a GPU write queued (the CPU first
     waits for the fence, `Fence::wait`, or does not touch it);
   - giving a page to the OS that holds a live byte;
   - moving a block (`relocate`) while a GPU use of it is queued.
7. **One place.** `mem::give_back`, `free_pages`, `reuse_pages` and `release` are the crate's own. No other crate can
   call them, so no other code can give memory back.

## Requirements, each locked by a test (`ledger::tests`, `free::tests`)

- **R1** Allocating over a live chunk fails (`allocating_over_a_live_chunk_fails`).
- **R2** A second free of a chunk fails, and so does a free of the wrong size or of no chunk's start
  (`a_second_free_of_a_chunk_fails`, `a_free_of_the_wrong_size_fails`).
- **R3** A chunk freed while a GPU read of its block is queued is handed out again only after the read completes; a
  use that begins after the free does not hold it (`a_chunk_freed_under_a_queued_gpu_read_waits_for_it`).
- **R4** A CPU write of a chunk with a GPU use of it queued fails, and so does a CPU read with a GPU write queued; both
  pass once the fence completes (`a_cpu_write_under_a_queued_gpu_use_fails`).
- **R5** A block dropped while a GPU use of it is queued keeps its memory until the use completes
  (`a_block_dropped_under_a_queued_gpu_use_keeps_its_memory`).
- **R6** No page that holds a live byte goes to the OS: not at a free, not at a freeze, not as a retiring chunk
  finishes (`a_page_with_a_live_byte_is_never_given_back`; the freeze's, `a_freeze_gives_back_no_page_a_live_chunk_shares`).
- **R7** Moving a block with a GPU use queued fails (`moving_a_block_under_a_queued_gpu_use_fails`).
- **R8** A buffer bound for a kernel is found to be its block's by its address, whoever made the buffer, and its use
  holds the block as any use does; a block's memory gone, its address is no block's
  (`a_bound_buffers_block_is_found_by_its_address`).

## Steps

1. **The ledger in `rerail-blocks`** (this record's first commit): R1-R7, the block's free list and page advice
   through it, `Fence` with uses of a block or a chunk.
2. **Every binding through a fence.** Each encoder has one `Fence`, completed by its command buffer's completion
   handler. Every buffer it binds or makes resident registers the block its contents lie in (`Fence::uses_address`,
   R8), so no caller has to remember: accel's `Pass` (the device joins) does it now; the other encoders are to be made
   through one constructor that does the same. `Batch::keep`, `read`, `held`, the atoms directory kept and `Held::kept` go: the
   ledger keeps what the work reads.
3. **The pool and the other regions.** `Pooled` drop, `Pooled::after`, `Scratch` resets, the residency slabs' `retire`
   and the deferrable slabs go through a ledger of their own regions, the same rules.
4. **CPU access through the ledger.** A chunk's owner (BlockVec, a relation's rows) asks the ledger before it writes a
   chunk the GPU may hold (R4's check at each write's start, not each value).

Until step 2, no GPU use is registered, so R3-R5 and R7 hold only where a fence is used.
