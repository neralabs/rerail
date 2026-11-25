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
   engine's statistics within it. `Fx::open` does not: it builds the whole component (its loaded inputs, its
   indexes, which the component's life places), and its sections were a few kilobytes in the census.
4. **Not `LifeRoom(Life::Run)`.** Inside a chunk's evaluation it places a structure in that chunk's open relation
   block (`open_chunk`'s `current_chunk`): a spec the memo keeps past the chunk would hold the chunk's block, and its
   relations' bytes with it. Its one mutex a life is also every thread's.
5. **Nothing is copied into a room.** A family is made where it will live; no region moves from a stack to an owner.

Tests: an owner room's region lies in its blocks, and the blocks go with the owner and its last region; a chunk freed
is taken again; two threads make regions in one owner's room (`an_owners_room_holds_its_regions_in_its_own_blocks`);
what the memo keeps is made in its room, not a thread's stack (`a_kept_value_is_made_in_the_memos_room`). The
planner run's statistics are checked by the census: no `LiveRel` family left on a stack. Then the census again (expect about 1.8 times
live, the replay's figure) and the edit's footprint against the pre-merge engine.

A third owner came from the census once the first two were in: a chunk's syntax values (`facts::vtab::syntax::Values`)
keep their key shards (`Keys::add`) for the chunk, and they are made in the `Values`' own room (`OwnerRoom`).

Measured (2026-11-24, the edit of 15 files, two runs each, against the pre-merge engine):

| Engine | Instructions | Peak resident | The census replay: the stacks' peak |
|---|---|---|---|
| Before the owner rooms (`d64e9f890a`) | 1.07 | 1.22 | |
| The memo's and the planner run's rooms | 1.08 | 1.20 | 1 |
| And the `Values`' rooms | 1.09 | 1.23 to 1.25 | 0.83 |

The rooms hold what they were made for, and the stacks' replayed peak falls by a sixth, but the edit's peak resident
does not move. Three trials found the same, each reverted:

- the stack keeping 1 MB of pages above its top, not 16 MB: the same peak resident;
- the run's plans' priced sizes (`Priced::sizes`, `live_sizes_at`) and the run's observations (`Obs::rule_mut`) in
  owners' rooms: in the replay they were most of the dead bytes left at the stacks' peak (it falls to about five
  times live), but the peak resident is the same, and the edit takes about 4% more instructions;
- the component's open (`Fx::open`) in the run's room as well (the replay: about 2.4 times live): the same peak
  resident, and the edit's CPU about 1.5 times, every input it loads a zeroed region under the room's mutex.

That conclusion was wrong: those trials measured the peak resident set (`time -l`, clean file pages in it), each
lever alone, and the census then counted only the stacks. Measured again at the footprint's peak (the process stopped
there, `vmmap`), and with the census counting every room, the next section has the cause.

## Amended (2026-11-24, user): one long-lived room a thread, beside its stack

### Where the edit's memory went

The edit's physical footprint at its peak (the process polled every 2 ms and stopped there): jemalloc's heap holds the
same in both engines, and the whole excess is in the run's rooms. With every structure that names no room on the heap
instead (an experiment build), the edit's peak is the pre-merge engine's. Each part on the heap alone, the rest as it
was:

| Part on the heap | Peak footprint, against all on the heap |
|---|---|
| None (the three owner rooms) | 1.22 |
| The threads' stacks (regions under `scratch::LARGE`) | 1.09 |
| The owners' rooms | 1.14 |
| The threads' large rooms | 1.20 |

The stacks' cost is pinning: at the peak they held about ten times what was live in them, the dead regions under a
few long-lived small ones. The census names them: the component's relations, input views and indexes
(`setfix::open_spec`), the run's priced plans' sizes (`live_sizes_at`) and observations (`Obs::rule_mut`), the issue
board's and the components' sections. With those in owners' rooms, the stacks cost half as much, but the owners'
rooms more: each owner opened blocks of its own, a freed chunk under `LARGE_RESIDENT` kept its pages, and a table that
grew by doubling left each old one's pages dirty.

### Decided

1. **Two places a thread: its stack, and its long-lived room** (`maps::long`, `long::LongRoom`). The stack stays the
   default room of a structure that names none. What is known to outlive the work it is made in is made within the
   thread's long room. Nothing else changes room.
2. **The long room is one a thread, shared by every long-lived family made on it**, not one an owner: the families
   share its pages, and a chunk one frees the next takes.
3. **Small regions in runs of one size class** (`long::CLASSES`, four classes a doubling, as jemalloc's, so a region
   wastes an eighth on average, not a quarter): a run is 64 KB of slots of its class, a freed slot on its run's list,
   a run whose slots are all free gives its pages back and is taken by any class next. Regions of `scratch::LARGE`
   and more go to the thread's large room, as on the stack.
4. **Any thread frees a region of it** (a structure made on a strand is freed by its parent): the room's one mutex,
   uncontended but for those.
5. **What is long-lived is the census's to say**: a site whose regions live while many regions are taken and freed
   above them on its stack. Those of the edit's census are made in the long room: the component's open, the memo's
   kept values (`derived::placed`), the native engine's statistics (`Fx::stats`), the planner's observations, the
   priced plans' sizes and the store's plans read, a chunk's syntax values' keys, and the run's decoded outputs. A
   family no one marked is found by the next census as a pinner.
6. **The owners' rooms go** (`FreeRoom::owner`, `OwnerRoom`, `Run::room`, `Memo::room`, `Values::room`).

Tests: a long room's region lies in its block and reads zero; a freed slot is taken again by its class; a run whose
slots are all freed gives its pages back and is taken by another class; a region freed on another thread goes back to
its room; what the memo keeps is made in the long room. Then the census and the footprint at the peak against the
pre-merge engine.

The issue board's sections were not marked: once the rest were, the census found a larger pinner, the run's decoded
outputs (`decoded_outputs`, made `long`), and the board's dead bytes are no longer the stacks' largest. The stack
keeps 1 MB of pages above its top (`scratch::KEEP`), not 16 MB: with nothing pinned it pops, and 16 MB a thread held
each thread's high-water mark.

### Measured (2026-11-24, M3, 15 files of corpus0, one-file edits, against the pre-merge engine)

| Engine | Instructions | CPU | Peak footprint | Peak resident (`time -l`) |
|---|---|---|---|---|
| The owners' rooms (`060e5b9a76`) | 1.09 | 1.05 | 1.25 | 1.24 |
| One long-lived room a thread | 1.09 | 1.06 | 1.11 | 1.16 |

At the footprint's peak the census has the stacks holding 9.6 MB for 1.8 MB live, from 31.5 MB for 4.7 MB; the
long rooms 20.8 MB for 14.8 MB live. With the stacks' regions on the heap instead (the experiment build), the peak
falls by about another 2%, and with the long rooms' on the heap it rises: the long room is denser than jemalloc for
its families.

What remains is not the rooms: with every structure on the heap, the edit's peak is 1.06 times the pre-merge
engine's, its jemalloc heap at the peak a sixth larger. The structures this engine keeps hold more than the
pre-merge engine's did, wherever they lie; that is the next question.

## Amended (2026-11-24, user): the rooms' own costs in the edit

A time profile of the edit against the pre-merge engine had three costs of the rooms themselves, none of them in the
pre-merge engine's profile:

1. **A long room's emptied run gave its pages back at once** (`mem::give_back`), and the next run taken faulted them in
   again (`reuse_pages`): a region made and freed in a loop did both each time, 56 `madvise` calls against 1. A room
   now keeps the pages of up to 16 emptied runs (`long::RESIDENT`, 1 MB) and takes those first. Past that, the
   pages go back as before.
2. **Every thread reserved 64 GiB twice**, its stack and its long room. A reservation that large costs the kernel's
   map about 60 times a 1 GiB one, and freeing it 6 times. With the threads making theirs at once, that was about a
   fortieth of the edit's CPU. Both are 4 GiB now (`scratch::ANON`, `long::RESERVE`): a stack's bytes under its top
   stay resident, so a thread that filled 4 GiB would hold that much memory anyway.
3. **A vector's every growth read the registry's lock** (`default_room`, a lock every thread shares) to find the run's
   room. The provider is registered once, at load, so a thread keeps it once found.

Test: `emptied_runs_keep_their_pages_up_to_resident`.

### Measured (2026-11-24, M3, 15 files of corpus0, one-file edits, against the pre-merge engine)

| Engine | Instructions | CPU | Wall |
|---|---|---|---|
| One long-lived room a thread (`84e7b917c1`) | 1.09 | 1.06 | 0.98 |
| These three, and phase 0's batch (0639) | 1.07 | 1.01 | 0.95 |

The edit's CPU is now the pre-merge engine's. The instructions still left are spread out: the merged engine's own
work (the background write-back's packing, the strands' joins), with no single cost of the rooms among them.
