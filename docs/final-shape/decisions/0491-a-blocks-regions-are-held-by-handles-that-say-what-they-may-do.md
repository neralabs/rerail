# 0491. A block's regions are held by handles that say what they may do

Status: built (audited 2026-11-23).

Date: 2026-11-20. Status: decided (the user, 2026-11-20, on 0487: "I was expecting some arc-type thing for blocks
that tracks frozenness, readers, writers, allocation, GPU placement, versions (after mutation), implicit freeing when
there are no users left and detection of invariant issues like two writers, simultaneous readers and writers, etc. It
should also give out references with constness properly set. Like the mldb block and memory region handling");
steps 1 to 5 built (`rerail_blocks::region`, 2026-11-20); what is left is in step 3.

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
- **R10** A truncated region gives its tail back; a truncate while shared fails (`a_truncated_region_gives_back_its_tail`).
- **R11** A region's GPU pin is a chunk use of its block's ledger, which raw-offset writers are checked against
  (`a_regions_gpu_pin_is_its_ledgers_chunk_use`, `a_block_vec_under_a_queued_gpu_use_of_its_chunk_fails`).
- **R12** A pooled lease bound by address goes back to the pool only after its fence
  (`a_tracked_lease_bound_by_address_goes_after_its_fence`, `a_pooled_held_buffer_is_found_by_its_address`).
- **R13** A block's own region is pinned by a kernel's bind found by address, by the kernel's declared access; a
  frozen block's refuses a GPU write (`a_blocks_bind_by_address_pins_its_own_region`,
  `a_frozen_blocks_region_refuses_a_gpu_write`, `a_kernels_write_moves_its_blocks_version_and_a_read_does_not`).
- **R14** Two device domains: one owner; the other catches up through the host; two domains never write one region
  (`two_domains_hold_one_owner_and_the_other_catches_up_through_the_host`, `the_device_names_its_memory_domain`).

## Steps

1. **The handles** (built).
2. **The owners move to them**, one at a time, each with its frees by hand removed: `Cells` (done: a
   `Frozen<T>`, built through `Mutable::truncate`, which gives a front allocation's unwritten tail back), then a
   relation's arrays (done: `BlockVec` owns its chunk as a `Mutable<T>`, placed near its relation by itself and
   taken with `Mutable::adopt`, handed to a directory with `into_chunk`; its pushes write through its pointer, each
   checked by the ledger, 0487 R4, with no lock: a region's GPU pin is a chunk use of its ledger too), then the rows
   themselves (`rows`: nothing to move. A `Part` is already the handle, an `Arc<Held>` whose last drop releases the
   relation's chunks, and the ledger keeps a freed chunk until the GPU's fence completes. The chunks are owned by the
   relation's directory in its block, by self-relative offsets that a block mapped from disk keeps, where no host
   handle can lie), then the device's handles (`DevRel`,
   `Held::kept`), whose `Batch` lists then go. A kernel binds a region only through `buffer_read` / `buffer_write`.
   (`Pass::commit` marks its fence committed: done.)

   **What held the lists, and what holds them now (2026-11-20).** A launch binds `&Buffer`, so the pass could not
   keep the buffer's owner. A block's buffer did no harm: the pass's fence holds the block in its ledger
   (`uses_address`), and a chunk freed meanwhile retires until the fence completes. A pooled buffer
   (`shared::Pooled`) was no block's: the ledger did not know it, and its lease went back to the pool the moment it
   was dropped. `Batch`'s `keep`, `read` and `held` lists held those buffers until `finish`. (A keep could not be
   attached to the open pass: `Batch::block` kept a parameter block before `Batch::pass(n)` could commit that pass.)
   The user chose (2026-11-20) the pool's buffers held as blocks' memory, by the one ledger, over a second ledger in
   the pool. Done:
   - **A pooled lease is held by a ledger of its own** (`rerail_blocks::ledger::Tracked`): registered at its bytes'
     address as a block's is, so a bind finds it, and dropped while a fence uses it, it goes back to the pool when
     the fence completes (`let_go`, R5). The join's buffers (`join::Held`), the thread's scratch slots and a staged
     column's store (`col::Store`) are leases held so. A lease is not yet a region of a slab block: the pool's
     classes, budgets and trimming stay as they are (step 3).
   - **Every buffer a kernel binds is held by its pass**: a launch's (`Pass::holds`), a resident read or write's,
     and a UKL library call's, which now names what it binds (`Pass::library`, `ScanSpace::buffers`,
     `RadixSpace::buffers`). A test build fails on a mapped buffer bound that no ledger knows.
   - **`Batch`'s lists are gone**, with `Batch::keeps` and `Batch::hold`. `Batch::block` returns its `Held`, which
     the caller drops once its launch is encoded. A relation built into (`Building`) is still held until `finish`,
     which marks its index built.
   - A scratch slot that grows still hands its old lease to Metal, not the pool: callers hold its buffer by clone.
   - Found on the way: `Scan::size`'s sizes buffer was bound only by library launches, so no fence held it. The
     lists hid that. Without them it went back to the pool before its kernels ran, and the parity tests' rows
     differed (`a_pooled_held_buffer_is_found_by_its_address`, the setfix parity tests).

3. **The block itself as the root region** (done): `region::whole(block)` is the block's own region, one a block
   while a handle holds it (its ledger keeps a weak reference, so a bind found by address reaches it). A frozen
   block's is frozen. Its pins hold the block, not a chunk: a relation's CPU writers write other chunks meanwhile.
   `Facts` holds its block by `Arc`. The device's handles of a block (`join::Held::of_block`: `DevRel`'s, an
   index's) hold it by this region, so the block's frozenness, versions and pins are read where its regions' are.
   A pool lease stays a ledger of its own (`Tracked`), not a region of a slab block: a block puts its header at
   offset 0, and the join binds a lease's buffer at offset 0 at about 100 sites. A lease's region in a block would
   bind at its offset; that is the one piece left, and it buys no check the lease's ledger lacks.
4. **Domains (0440 §7.2)** (done): `Access` keeps a version a domain and each queued GPU write its domain;
   `mirror_in(domain, …)`, `bind_read_on` / `bind_write_on` and `owner()`. A bind in a domain whose copy is behind
   takes the latest through the host, from the owner's copy; a read or write in one domain while another domain's
   write is queued fails (one owner domain). The device layer names each device's domain (`Device::domain`: 0 for
   Apple's GPU, the CPU, OpenCL and a CUDA device reading the host's pages; 1 for a discrete card).
5. **Kernels' rows bind their arguments** (done): the device layer gives each kernel's arguments with their access
   (`ROBUFFER` read, `RWBUFFER` / `WOBUFFER` write), and a launch pins each bound buffer's block by it
   (`join::Pipe::writes`, `region::bind_address`). A UKL library call's binds are held as written.
