# 0342. One block allocator, and blocks with a free list

Status: built (audited 2026-11-23).

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

## After the hub's 0325 step 3 (merge of 2026-11-17)

0325 step 3 moved the rows' storage to `facts::rows`, in the `rerail-facts` crate, with a `Life` for each relation's
blocks. This record's parts are carried there:
- a run's and a batch's blocks come from the policy (`Writes::Churn`, a free list);
- a `Part` is shared and gives its relation's chunks back with its last holder, and an `InBlock` reader (the
  device's `DevRel::in_block`) holds it;
- the policy's `Device::May` leaves half the Metal budget to the device's own buffers (0325 step 3's `keep`).

The crate cannot see the engine's binding (`placement`), so a chunk carries what its blocks ask the policy
(`in_chunk(device, …)`, the engine passing `placement::block_device()`). Outside a chunk a block asks `Device::May`.

**Still outside the policy:** a device run's own relation (`Life::Device`, 0325 step 5) is leased as a device
join's buffers are (`lease_block_for_join`: bounded inside the join's budget, a refusal running it on the CPU).
The policy has no need for that yet.

## Amended (2026-11-20, user; 0494 step 6): the lists in the ledger, buddies merged

Measured against 0458 (0494 step 5), a cold run of the data layer's 50 files held about 120 MB more footprint, and
it was the free lists: at the end of the run they held 316 MB, about 200 MB of it resident. A free chunk's first
word was its link, so its first page stayed; and free chunks never merged, so a chunk under a page never freed one.
Three ways were tried (0494 step 6): the lists outside the chunks, the small structures out of the free-listed
blocks, and buddies merged. The user chose the third, which is built on the first:

- **The lists are the ledger's** (decision 0487's, `ledger::State::lists`): a class's free chunks are a set of
  offsets there, and a free chunk holds nothing. With a bit a page for the pages given to the OS, a free gives back
  every page it leaves with no live byte (the ledger's live bits say which, R6), and a chunk handed out marks the
  given pages it reaches in use again (`MADV_FREE_REUSE` on macOS). A freeze gives back what its free chunks leave.
- **Buddies.** A class's chunk lies aligned to its size, up to 1 MB (`free::ALIGN_MAX`: a block made for a chunk
  has room for it and 1 MB more; a larger chunk lies at a multiple of 1 MB, has no buddy, and its pieces go back by
  their own sizes, as every piece did before). From the front, the bytes skipped to align it go onto the lists as
  pieces, each aligned to its own size. A chunk freed merges with its buddy (`at ^ size`) while the buddy
  is free, and a class with none free splits the smallest larger chunk, each upper half onto its list. The lowest
  free chunk is taken first. A fitted chunk's pieces, and its rest when it is made, are each aligned to their size,
  so they merge too.
- Given up: the lists' heads in the block's memory and the in-chunk links, the form a kernel could have bound ("Not
  now" above). Only the host takes and gives chunks; a kernel's form, when one is wanted, is the ledger's to give.

Tests: `free_buddies_merge_and_a_larger_chunk_splits`, `a_page_goes_back_once_no_live_chunk_shares_it`,
`a_large_chunk_fits_a_block_made_for_it`, and
`a_page_with_a_live_byte_is_never_given_back` (R6, now through the frees themselves); Q337's freeze test with a
chunk under a page, which is the case that shares a page now.

## Amended (2026-11-24, user): a run's small structures on a stack a thread, long-lived ones in their owners' rooms

Decision 0628 put every structure of a run in blocks: what no life places takes the run's anonymous room
(`maps::AnonRoom`), free-listed blocks sharded by thread. Against the run before that merge (its arrays on the heap),
an edit spent much of its time in the room: each region is a free list's class taken and given back through the
ledger's sets, with a buddy merge, pages given back and marked in use again, and two heap objects of its own. The
free list was not made for this. This record's own words (2026-11-17): it is held "to big pieces, not millions of
small ones", and 0494 step 6 chose its lists for footprint, never for speed. An edit hands the room millions of
small regions.

### The census

`RERAIL_ANON_CENSUS=path` (a measuring build) recorded each region the room gave over one edit of the data layer's
store: its element type, bytes, the stack that asked for it, its thread, and when it went, in regions handed out
since and in time.

- 99.2% of regions go within the run, 97% on the thread that made them.
- By regions handed out while a region lived: half live through under 16, 91% under about 65 thousand.
- `Layouts::infer` cloned its whole map of facts for each union or recursive component: a quarter of the regions.
  It now reads the component's supposition beside the facts (`Known`, f30917becd); the regions fell by a quarter.
- After that, the largest sites are a few percent each (`Section::of`, `Keys::add`, `granules::number`,
  `StoredPlan::of_row`); 64% of regions go within 16 more.

### What the census says the room should be

The event log was replayed through allocators, footprint at its peak against the live bytes at theirs:

| Placement | Footprint peak / live peak |
|---|---|
| Today's free-listed blocks (buddy classes) | 1.37 |
| One stack a thread, every region on it | 5.9 |
| A stack a thread; regions living past 1,024 more elsewhere (oracle) | 1.00 |
| The same, by the asking site (owned if over 1% of its regions live past 1,024) | 1.00 |

A stack alone is pinned by the few regions that outlive what was made after them. Sent to their owners, those leave
the stack at its live size. By site, the owned regions are about 40% of the regions and 37% of the bytes, from
about 290 sites in about 15 modules (the tape writer, the syntax tables' keys, the stored plans, the plan tables,
sections, the program image, the corpus memo, setfix, the planner); 257 sites only ever make short-lived regions.

### Decided

1. **A stack a thread** (`scratch::Stack`): one anonymous block, reserved once a thread. A region is a 16-byte
   header (the previous header's offset, a state word) and its bytes, bumped from the top. Its going sets the state
   free, one atomic store, from any thread; the owning thread pops freed headers off the top at its next take
   (`settle`) and moves the top back. Past `KEEP` bytes above the top, pages go back to the OS, and a take that
   reaches them marks them in use again. A room's region is zeroed, as the anonymous room's chunks were. A
   use's takes are not, and neither is a relation block's chunk (`LifeRoom`, a home's room): a structure writes a
   byte before it reads it, and nothing may skip a write on the strength of a room's zeros (a vector's growth that
   did made phase 0's bootstrap run read stale bytes).
2. **The thread's scratch is that stack** (`scratch::with`): a use's takes are its regions, tagged with the use, and
   its end frees them, so a freeze's temporaries and the room's regions are one memory, and a use may nest. Its
   ledger is the block's: a use's end with GPU work that binds its bytes queued fails, as before.
3. **Where a region lies is still a block and an offset** (`place_of`), so a kernel and the JIT read a stack's
   structure in place.
4. **Large regions go to free-listed blocks** (`maps::LargeRoom`, one a thread): a region of `scratch::LARGE` bytes
   (4 KB) or more. The census showed the stack's pinning was mostly large arrays freed out of its order (a tape
   writer's parts, a growing vector's later regions), not long-lived small ones: replayed, every region on the stack
   held 9.0 times the live bytes (headers counted), and with the large ones off it 3.2 times. They are about 2% of
   the regions, the big pieces this record's free lists were made for. A large room's chunk under 256 KB keeps its
   pages when freed (`Block::keeping_pages_below`), as a growing vector's next step takes them again.
5. **The stack is the run's default room** (`default_room`), and `AnonRoom` is gone: its free-listed blocks are the
   large room, a thread's own, with no shards.
6. **An owner's family is made in its room** (`maps::within(room, f)`: every structure made with no room on the
   thread while `f` runs takes `room`). What still pins the stack is the planner's run: its join specs' and step
   plans' sections (`JoinSpec::encode`, `setfix::analysed`) and its strata's `LiveRel` maps, small and many, living
   for the run. Placed in the run's own room they bring the replay to 1.8 times live. That room is the next step: it
   must not keep a run's blocks for what a cache holds past the run.

Tests: a use's takes go back at its end; a region outliving the use is kept, the stack held under it until it goes;
a free from another thread is settled by the owner; pages past `KEEP` go back and are taken again; nothing is popped
under queued GPU work; the block's used bytes reach every region (the device wraps them); a large region is not on
the stack; a large room's small chunk keeps its pages; a structure
made within an owner takes the owner's room.

### Measured (2026-11-24, M3, 15 files of corpus0, one-file edits)

Each engine from nothing (a cold run, two warm, two edits), twice, alternating; against the engine before the merge:

| One edit | Instructions | CPU | Resident |
|---|---|---|---|
| `AnonRoom` (before this amendment) | 1.41 | 1.33 | 1.18 |
| Every region on the stack | 1.26 | 1.30 | 1.48 |
| The stack and the large room (built) | 1.26 | 1.36 | 1.22 |

Time profiles put the allocators' code, vectors' growth with it, at about 5% of an edit's samples, under the
pre-merge engine's share in jemalloc. The rest of the edit's gap to the pre-merge engine is not allocation.

### The run's own rooms (designed, 2026-11-24)

Item 6's families have two owners, not one. The census's long-lived regions by the site that asked:

| Family | Made by | Held by | Lives |
|---|---|---|---|
| Join specs' sections (`JoinSpec::encode`) | `Run::join_spec`, on whichever thread plans the stratum | the corpus run's memo (`derived::value("join spec")`), and the component specs that name them | the corpus run (`Keep`) |
| Step plans' and component specs' sections (`setfix::analysed`, `ComponentSpec::encode`) | `Fx::open`, `level::prefetch` | the native engine's component | the planner run |
| `LiveRel`'s frequency maps, keys and counts (`LiveRel::of_native`, `component::live`) | `Run::drive`, on a strand's thread too | the planner run's model (`Stats::dfreq`, `keys`, `counts`); a strand's fork is taken into its parent's (`take_from`) | the planner run |

1. **One room kind for an owner: free-listed blocks the owner holds** (`maps::FreeRoom`). It is the large room's
   mechanism (decision item 4) made an owner's: a mutex over the open block, the next leased when it is full, a block
   gone with its last region. The large room becomes a `FreeRoom` a thread; an owner's is a `FreeRoom` the owner
   holds in an `Arc`, shared by the threads that make its structures. Its blocks reserve address space (pages are
   touched as used) and keep a freed chunk's pages below `LARGE_RESIDENT`. A small region takes the free list's
   smallest class (64 bytes); the families above are a few thousand regions a run, not millions.
2. **The memo's room** (`derived::Memo::room`): what the memo keeps is made in it. `derived::value` gives `make`
   no room; the value's maker encodes what is kept within it (`derived::placed(|| spec.encode())`), so a spec's
   planning temporaries stay on the stack and only its section lies in the memo's blocks. The memo goes with the
   corpus run; a value cloned out of it keeps only its own region's block.
3. **The planner run's room** (`Run::room`): a strand's run is given its parent's (`Run::strand`), so a map made on a
   strand's thread lies where the parent that takes it holds it. `Fx::stats` and `stats_changed` read the native
   engine's statistics within it; `Fx::open` and the prefetch's `join_rule_bound` encode within it.
4. **Not `LifeRoom(Life::Run)`.** Inside a chunk's evaluation it places a structure in that chunk's open relation
   block (`open_chunk`'s `current_chunk`): a spec the memo keeps past the chunk would hold the chunk's block, and its
   relations' bytes with it. Its one mutex a life is also every thread's.
5. **Nothing is copied into a room.** A family is made where it will live; no region moves from a stack to an owner.

Tests: an owner room's region lies in its blocks, and the blocks go with the owner and its last region; a chunk freed
is taken again; two threads make regions in one owner's room; a kept join spec lies in the memo's room, not a
thread's stack; a strand's live statistics lie in its parent run's room. Then the census again (expect about 1.8 times
live, the replay's figure) and the edit's footprint against the pre-merge engine.
