# 0491. A block's regions are held by handles that say what they may do

Date: 2026-11-20. Status: decided (the user, 2026-11-20, on 0487: "I was expecting some arc-type thing for blocks
that tracks frozenness, readers, writers, allocation, GPU placement, versions (after mutation), implicit freeing when
there are no users left and detection of invariant issues like two writers, simultaneous readers and writers, etc. It
should also give out references with constness properly set. Like the mldb block and memory region handling");
the handles built (`rerail_blocks::region`), no owner moved to them yet.

## Intro

0487 put one ledger under each block: it knows which chunks are live and what the GPU has queued, so a free waits
for the GPU and a misuse fails. But a chunk is still named by a bare offset: its owner frees it by hand, CPU code
reads or writes it through a raw pointer, and nothing says whether a holder may write. The user asked for what MLDB
has instead. In MLDB (`block/memory_region.h` and `block/compute_kernel_grid.h`, branch `opencl-random-forests`):

- **A region's info** (`MemoryRegionHandleInfo`): its type, whether it is const, its length, its parent and offset
  in it (a sub-range), a name, and a version.
- **Typed handles** (`MemoryArrayHandleT<T>`): a `const T` handle can be made from a `T` one, never the reverse.
- **Pins** (`Manager::pinAccess(op, access)`): each read or write by an operation is counted, by its name, and
  released when the pin goes. A write bumps the version, and a conflict throws, naming the operation.
- **A bound kernel** keeps its arguments' pins until it completes.
- **Freezing**: `MutableMemoryRegion::freeze` gives a `FrozenMemoryRegion`, whose `range` keeps the whole region
  alive.

## Decision

1. **A region is a run of `T`s in a block, held only by handles** (`region::Region` behind an `Arc`). It goes back to
   its block (through the ledger, 0487) when the last handle goes. A GPU use holds it as a handle does, so it is not
   freed before its work completes.
2. **Three handles, the access in the type:**
   - `Mutable<T>`: allocated, one owner. `write(&mut self)` gives `&mut [T]` under a guard, `read()` gives `&[T]`,
     `share()` a const handle, `freeze(self)` a frozen one.
   - `Shared<T>`: const and cloneable, over an open or a frozen region. `read()` gives `&[T]` under a guard; it
     cannot write.
   - `Frozen<T>`: immutable and cloneable. It derefs to `&[T]` without a guard, since nothing can write it.
3. **Uses are counted and checked at once, naming the region and where the other use began** (the caller's source
   line for a CPU guard, the operation's name for a GPU pin):
   - two writers;
   - a CPU write while a read (CPU or GPU) is live or queued;
   - a CPU read while a write (CPU or GPU) is;
   - a GPU write while a CPU read or write is live;
   - a GPU write of a frozen region;
   - a freeze while a CPU use is live or a GPU write is queued;
   - a range past the end.
4. **GPU placement is the region's:** `bind_read` / `bind_write` (and `buffer_read` / `buffer_write`, with the
   block's buffer) pin it on the command buffer's `Fence` and return its offset in its block's buffer.
5. **A version counts its writes:** a CPU write guard ended, or a GPU write completed. A read guard gives the version
   it reads.
6. **A range is a region of its own** that keeps its whole region alive and shares its uses, version and state.

## Requirements, each locked by a test (`region::tests`)

- **R1** A write while a read is live fails (`a_write_while_a_read_is_live_fails`).
- **R2** A read, or a second writer, while a write is live fails (`a_read_or_a_second_write_while_a_write_is_live_fails`).
- **R3** CPU and GPU uses conflict as readers and writers, and pass once the fence completes
  (`cpu_and_gpu_uses_of_a_region_conflict_as_readers_and_writers`).
- **R4** A frozen region reads without guards, and refuses a GPU write; a freeze while it is read fails
  (`a_frozen_region_reads_without_guards_and_refuses_writes`).
- **R5** A region is freed with its last handle, after its GPU work (`a_region_is_freed_with_its_last_handle_after_its_gpu_work`).
- **R6** A range keeps its region alive and shares its uses; one past the end fails
  (`a_range_keeps_its_region_and_shares_its_uses`).
- **R7** A write moves the version, a GPU write when it completes (`a_write_moves_the_version`).

## Steps

1. **The handles** (built).
2. **The owners move to them**, one at a time, each with its frees by hand removed: `Cells` (already an `Arc<Block>`
   and a slice), then a relation's columns and indexes (`BlockVec`, `rows`), then the device's handles (`DevRel`,
   `Held::kept`), whose `Batch` lists then go. A kernel binds a region only through `buffer_read` / `buffer_write`.
3. **The block itself as the root region:** `Facts` and the pool's slabs hold their block by handle, so a block's
   frozenness, placement and lifetime are read in the same place as its regions'.
