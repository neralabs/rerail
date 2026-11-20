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
7. **Each memory space holds a version** (the user, 2026-11-20: "if I run a kernel on a GPU that mutates a block,
   the GPU handle has access to a different (more recent) epoch than the CPU with a handle to the same block. And we
   will need to transfer it back to the CPU"). A region has its latest version, and the version its host bytes hold.
   On a device with memory of its own (a discrete GPU), it also has the version the device's copy holds, kept by a
   `Mirror` the backend attaches (`Region::mirror_on`):
   - a CPU write ended: the host holds the latest; the device's copy is behind;
   - a kernel's bind of a region whose device copy is behind queues its upload first, on the same command buffer;
   - a GPU write completed: the device holds the latest; the host's bytes are behind;
   - a CPU read or write, or a freeze, while the host is behind or a GPU write is queued fails, saying to sync;
   - `sync_host` waits for the queued GPU writes, then downloads when the host is behind (MLDB's
     `transferToHostSync`). Waiting on a write whose command buffer was never committed fails, as it would never
     end (`Fence::mark_committed`).

   With unified memory (Apple's) there is one copy: no mirror, nothing moves, and a sync is the wait alone.

## Against 0440 §7.2: memory domains, replicas and the directory

CUDA-support decided how blocks cross memory domains (0440 §7.2, the user's Q1). It reached this branch in the merge
of 2026-11-20 (fa09c9c761). Item 7 above is that model for one region and two spaces. Where they meet:

- **The same.** A version names what a copy holds. A frozen version may have a copy in every domain, and none is ever
  invalidated. A mutable region is written in one domain at a time. Within one domain (Apple, the GB10) nothing
  moves, and a sync is a wait. A copy is ordered before the kernel that reads it, on its fence.
- **Domains, not "host" and "device".** 0440 counts every domain: a host's RAM, each discrete GPU's memory, another
  host's RAM, the disk. A host with two cards has three. A region's `host` and `device` versions become one entry a
  domain: its version and its `Mirror`, for each domain that holds a copy.
- **A mutable region has one owner domain.** 0440 lets one domain hold the mutable version. Item 7's rule says the
  same: after a GPU write, a CPU access fails until `sync_host`. That is ownership moving to the device, and back. The
  entry that holds the latest version is the owner. A write elsewhere first takes ownership, by moving the bytes, so
  two domains never write one region.
- **Freezing publishes.** 0440 freezes a block when its writer's work ends. Here a `Mutable` stays mutable across
  writes, and each write moves its version. `freeze` is still what makes copies free to keep everywhere. A kernel's
  output that no one writes again is frozen as its work completes.
- **The directory is the regions' state.** 0440 asks for one table a process: each block's version, its owner and
  its copies. That table is the live regions' `Access` entries, with the ledger's address index (0487 R8) to find a
  buffer's region. It is the machine's (0442), not a second store beside the regions.
- **Kernels say their reads and writes.** UKL's rows already do (`ROBUFFER` reads, `RWBUFFER` and `WOBUFFER`
  write). A launch pins each argument by its row, read or write, so no caller picks `bind_read` or `bind_write` by
  hand. The device layer does not give the host those rows yet; that is a step below.
- **Eviction is the one cache.** A frozen copy in a domain that another domain or the disk also holds may go. It is
  an entry of the one cache the CPU and the GPU share, bound by LRU (the user's rule). A mutable region's owner copy
  is written back before its pages go.

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
- **R8** A device copy is uploaded only when behind, and after a GPU write the host is behind until `sync_host`
  brings it back; a read or a freeze before then fails (`a_device_copy_is_uploaded_when_behind_and_synced_back_after_a_write`).
- **R9** With unified memory a sync is a wait, and a sync on an uncommitted write fails
  (`with_unified_memory_a_sync_is_a_wait`).

## Steps

1. **The handles** (built).
2. **The owners move to them**, one at a time, each with its frees by hand removed: `Cells` (already an `Arc<Block>`
   and a slice), then a relation's columns and indexes (`BlockVec`, `rows`), then the device's handles (`DevRel`,
   `Held::kept`), whose `Batch` lists then go. A kernel binds a region only through `buffer_read` / `buffer_write`,
   and `Batch` marks each command buffer's fence committed as it commits it.
3. **The block itself as the root region:** `Facts` and the pool's slabs hold their block by handle, so a block's
   frozenness, placement and lifetime are read in the same place as its regions'.
4. **Domains (0440 §7.2):** a region's host and device versions become an entry a domain, with the owner domain of a
   mutable region, and each device in the device layer names its domain. A test with two domains besides the host
   (two mirrors) locks it: a write in one makes the other behind, and only the owner may write.
5. **Kernels' rows bind their arguments:** the device layer gives each kernel's parameter rows (read, write) to the
   host, and `Pass::run` pins each argument by its row.
