# t60. Operation memory: where each scratch array belongs, and one way to ask for it (a plan)

A plan, not a decision. Each step that changes a rule is written as a decision record when the user takes it.

## Intro

Since hub commit 8f4fdb7293, every `Vector` and block map made with no room of its own lies in the run's open relation
block (decision 0628 §8). There it pays the block ledger's lock, the R4 guard and the block's free lists. A probe
counted every such array over one cold PostHog run (m1max, about 100 s, findings unchanged at 29,930):

- **59.1 M arrays**, about 590,000 a second. None was handed to a relation (`into_region`, `into_owner`).
- **10.9 M growth moves**: an array copied into a bigger region as it grew.
- **82% are 64 bytes or less, and 46% live under 1 µs.**
- **The footprint is concentrated:** 99% of bytes × time is one run-long accumulator (`ChunkCounts::add`, §5.5).

The user's direction (2026-11-25):

> The goal of putting everything in arrays is to make the form compatible with GPUs, and to gradually move all of the
> processing steps from different host/GPU implementation to a single ULK operation. [...] once we decide the boundary
> (the GPU should operate in-place on all of its pre-mapped inputs, with NO host work before or after), what happens
> inside the equivalent host operation is unimportant; scratch arrays would ideally be stack-based (even short
> variable-length ones) and any data with operation lifetime would come out of a bump arena on the stack or dedicated
> scratch blocks.

The plan in short:

1. **Draw the boundary at the operation.** An operation is what becomes one UKL op. It reads its inputs in their
   blocks and writes its outputs in their blocks. Everything else it uses is operation memory.
2. **Give each array one of five homes.** Operation memory has three: `Inline` (the stack), `Frame` (the thread's
   bump arena) and `OpScratch` (a scratch block). Data has two: chunk blocks and run blocks.
3. **Free operation memory from the block rules.** No ledger, no guard, no relation block, no zero fill, and no
   growth copies at the top of the frame.
4. **Make each call site choose.** The default room goes, so `Vector::new()` names no home and each site must pick
   one.

The survey's tools: the probe (`crates/vector/src/probe.rs`, uncommitted, on with `RERAIL_PROBE_SCRATCH=<file>`),
`working-data/m3/syms.py`, `scratch2.py`, and `classify.py` (scratchpad; §5's classes).

## 1. The boundary

An **operation** is the unit that will become one UKL op: a join step over its inputs, a stratum's wave over a group
of documents, a parse of one document, a memo entry's encoding, a planner's search, a freeze or a sort. Operations
nest. A stratum's wave holds join steps, and each step holds probes.

- **Inputs:** blocks already mapped (tapes, relations, indexes). The operation reads them in place (0353).
- **Outputs:** written once, in place, into their blocks: a relation's rows, a tape, an index, an image section.
- **Operation memory:** everything else the operation makes. It dies when the operation ends, and only that
  operation reads it.

On the device the same operation is a kernel launch. Its inputs and outputs are the same blocks, already mapped.
Its operation memory is the kernel's own: registers, threadgroup memory, and one scratch buffer bound to the
launch. The host does no work before or after the launch: no staging, no compaction, and no copy back.

Operation memory is not a copy under 0353 as long as it holds intermediate values (keys, candidates, counts,
partial results). If it held a block's rows for later readers to read instead of the block, it would be a copy.
The rule still forbids that.

## 2. The classes

Each class is named by lifetime and is the same on the CPU and the GPU.

| Class | Lifetime | CPU | GPU (UKL) | Sized by | Grows? |
|---|---|---|---|---|---|
| **Inline** | within a function | the stack or registers: `[T; N]`, `InlineVec<T, N>` | `UKL_PRIVATE` arrays | a bound known when compiled, or the program's (a rule's slot count) | no; past `N` it spills to the frame |
| **Frame** | one operation, on one thread | the thread's bump frame, a region kept per thread whose first pages stay in L1 and L2 | `UKL_LOCAL` (threadgroup memory) up to the tile, the rest in `OpScratch` | the operation's tile, or a bound from its inputs | at the top of the frame, in place |
| **OpScratch** | one operation, large or bound to a kernel | the same frame over a dedicated scratch block (`blocks::scratch::Scratch::over` a pool slab) | one device buffer bound to the launch, laid out by the operation | declared by the operation from its inputs' sizes; for the device, the high-water of earlier runs (0486) | CPU: yes; device: no (declared up front) |
| **Chunk** | a chunk or batch, across operations | a relation, table or overlay in the chunk's block (`LifeRoom(Life::Chunk)`) | the same block, mapped | rows as written | append in segments; never grown by copying |
| **Run** | the run | a relation or table in a run block | the same block, mapped | rows as written | append in segments; snapshots as overlays (0628 §1) |

A **hand-off** (an array made on one thread and ended on another) is not a class. It is an output. The receiver's
place is sized first, and each worker writes its own part there. Either that place is a split of the parent's frame
(`split_at_mut`, handed to scoped workers) or it is a chunk relation's rows.

## 3. One way to ask for each

All of this lives in `rerail_vector`, below the tape format, so the writer's columns can use it too. `blocks::scratch`'s
thread region moves down here and becomes the frame's backing, so each thread has one region.

- **`Inline<T, N>` and `InlineVec<T, N>`.** A const-generic array on the stack and a short variable-length one.
  Past `N`, `InlineVec` spills to the current frame, never to a block or the heap. A fixed set becomes a fixed type:
  `VarSet` becomes `[u64; W]`, where `W` is the program's widest rule.
- **`scratch::op(|f: &Frame<'_>| ...)`.** Opens an operation's frame on this thread at a mark, and releases to the
  mark when the closure ends, unwinding included. Frames nest LIFO. A stolen rayon job runs to completion on the
  thief's stack, so it opens and releases its frame inside whatever that thread was doing, and LIFO order holds.
  - `f.vec::<T>()` and `f.vec_with::<T>(n)` give a `FrameVec<'f, T>`. At the top of the frame it grows in place.
    Below the top it moves within the frame, and the space it leaves is reclaimed when the frame ends.
  - `f.take::<T>(n)` gives uninitialised memory; `f.zeroed::<T>(n)` gives zeros.
  - `f.sub(|g| ...)` opens a nested frame.
  - The lifetime `'f` stops anything leaving the operation at compile time.
  - `Frame` is not `Send`. It reaches a worker only as a `&mut [T]` split.
- **`scratch::op_on(slab, |f| ...)`.** The same frame over a dedicated scratch block, for `OpScratch`. A kernel that
  binds the block holds its ledger, as `Scratch` does today (0487 step 3). This is the only operation memory a
  guard sees.
- **Data: `Vector::new_in(room)` and `Map::new_in(room)`, always with a room.** The default room
  (`rerail_vector::default_room`, `rows::install_run_room`) is removed. `HeapRoom` remains only for a crate's own
  tests. A roomless `Vector::new()` then has no meaning outside tests, so each of the 888 sites must choose.
- **No function returns an owned scratch array.** Today, per-call lookups return a `Vector` for the caller to read
  and drop: the vtab `lookup`, `range_lookup`, `lookup_columns`, setfix's `find_keyed` and `find`, `children`,
  `containers_on`. Each changes in one of two ways:
  - it writes into the caller's `&mut FrameVec<T>`;
  - or it gives a read in place: `Seq`, which `lookup_seq` already gives for runs of rows and lists a source holds,
    or an iterator over the tape.

**On the device.** An operation meant for the GPU declares its memory: `PRIVATE` bounds, `LOCAL` words per
threadgroup, and an `OpScratch` layout (named buffers and their sizes as functions of the inputs' sizes). The
kernel takes that layout from the declaration. The CPU version of the same operation, the UKL `vec` backend or the
host loop it replaces, takes the same layout from its frame. A device operation cannot grow. Its declaration is the
bound, and a run that would pass it is refused and sized again (0486), never finished on the host
(no CPU racing of device work).

## 4. The restrictions lifted for operation memory

What operation memory (`Inline`, `Frame`, `OpScratch`) no longer pays, and what replaces each cost:

| Today (every roomless array) | Operation memory |
|---|---|
| A region in the run's open relation block (0628 §8) | the thread's frame: no `Arc<dyn Room>`, no `default_room()` per array |
| A ledger chunk taken and freed under the ledger's mutex, with `madvise` under the same lock | a bump of an offset, with no lock; pages given back only when the thread is idle, past `KEEP` |
| The R4 guard's count read on every access (0684) | none, since no kernel binds a frame; an `OpScratch` a kernel binds keeps its one ledger |
| A heap-profile charge per region | the frame's high-water, counted once per operation |
| Zeroed memory (`HeapRoom`'s `alloc_zeroed`; fresh block pages) | uninitialised (`Pod`), with `zeroed` on request |
| A growth copy to a new region and a free of the old | growth in place at the top; `reserve` from a bound below it |

What stays:

- Chunk and run data lie in blocks as relations and tables, under the ledger and R4 (0628, 0487).
- Hand-offs are outputs.
- 0353 holds: no block's data is copied into operation memory for readers to read there.

This narrows 0628's order ("every run data structure moves into blocks"). Operation memory is not a run data
structure, since no reader outside its operation reads it. That narrowing is the user's direction above, and is
written as a decision record in step 0 when the user takes it.

## 5. Where each user goes

Each maker is put in the class most of its arrays fall in. These are the makers' dominant classes; the run's own
class totals by array are in the survey (A 75.5%, B1 10.1%, B2 1.6%, C1 6.7%, C2 3.1%, D 3.1%).

| Class | Makers | Arrays | GB·s |
|---|---|---|---|
| A: ≤256 B, under 131 µs | 239 | 48.5 M | 200 |
| B1: up to 4 KB, under 17 ms | 128 | 3.8 M | 569 |
| B2: larger, under 17 ms | 16 | 62 K | 6 |
| C1: 17 ms to 1 s | 34 | 3.9 M | 644 |
| C2: over 1 s | 27 | 1.2 M | 47,264 |
| D: ends on another thread (over 30%) | 54 | 1.7 M | 1,292 |

### 5.1 Inline, or a read in place (A: 48.5 M arrays)

| Arrays | Maker ← caller | Shape | Fix |
|---|---|---|---|
| 8.27 M | `setfix::View::find_keyed` ← `cands::Rows::each` | u32, 39 B on average, up to 256 KB | Candidates written into the caller's `FrameVec`, or read as the source's `Seq` (`find_seq` exists; `each` should take it) |
| 7.51 M | `within::Within::lookup` ← `values::Layered::lookup` | usize, 57 B | The vtab `lookup` writes into the caller's frame vec (`lookup_into`); `Layered` passes the vec down, with no array per layer |
| 4.57 M | `granules::number` ← `DefIds::new` | u32, 205 B, up to 826 KB, 0.32 growths | A frame vec reserved from the token count |
| 3.8 M + 0.93 M + 0.73 M | rayon `collect` into `Result<HashMap<…>>` | i64, 32 B, fixed | `Inline<[i64; 4]>` per item; the collect's own output is a hand-off (§5.6) |
| 2.66 M | `corpus::run_observed_within` | i64, 130 B, up to 1 MB | A frame vec in the observation's operation |
| 2.59 M | `Within::containers_on` ← `Within::rows` | u32, 16 B, at most 64 B | `InlineVec<u32, 16>` |
| 1.94 M | `FactsIn::children` ← `Dialect::field_child` | u32, 22 B | An iterator over the tape's children, with no list (`syntax.rs` `children` already is one) |
| 1.89 M | `setfix::Probed::clone` ← `cands::candidates` | u32, 16 B | `InlineVec<u32, 8>` |
| 1.44 M | `vtab::Relation::staged` ← `Batch::make_rows` | i64, always 32 B | Its rows are an iterator of `Vector<i64>`, one array per row: they become slices of one frame vec, a row its width |
| 1.26 M | `Within::contained_on` | u32, 119 B, 1.2 growths | A frame vec |
| 1.10 M | `plan_search::Model::plan_search` | usize, 35 B | `InlineVec` |
| 0.99 M + 0.99 M | `output::Builder::rows_as_is` ← `memo::image::encode_parts` | the same sizes twice | The rows are written once, by the encoder, into the image section |
| 0.94 M, 0.79 M, 0.25 M, 0.19 M | `VarSet`: `term_vars`, `minus`, `extend`, `insert` | u64, always 32 B | `VarSet([u64; W])` |
| 0.69 M | `tape_rows::RowsView::lookup` ← `DocRow::find` | usize, 32 B | `Seq` (a run of rows) |
| 0.11 M, 0.2 M | `rowcode::fused`, `compile_micro::Micro::of` | 8 B and 36 B, fixed | Inline |

### 5.2 Frame (B1: 3.8 M arrays)

| Arrays | Maker | Shape | Fix |
|---|---|---|---|
| 522 K | `planner::Walker::work_at` ← `InRel::probe_sample` | i64, 36 B | A frame vec |
| 444 K | `f16::ops::Writer::begin_with` | u8, 19.6 KB on average, up to 8 MB, 0.49 growths | The writer's working columns go in the parse's frame. The finished image goes in its block, the tape's output, and nothing is copied: the image is written there by `finish` |
| 336 K | `compile_sets::Postings::of` | u32, 2.5 KB, 14% on another thread | A frame vec in the matcher's compile operation; the 14% hand-off is to `Reader::row` (§5.6) |
| 240 K | `own::Own::scaled` (hash maps) | 3.5 KB | A frame map (open addressing over a frame slice) |
| 181 K | `own::in_order` ← `Own::put` | [i64; 3], 3.7 growths | Reserved from `put`'s count |
| 177 K | `run_join_in_each::enter` | i64, 6.4 KB, 1.3 growths | A frame vec per join step |
| 166 K | `tape_format::Writer::new` ← `python_lean::parse_from` | u8, 178 KB | As `begin_with` |
| 160 K | `LiveRel::of_native` (hash maps) | 2.2 KB, 173 GB·s | A frame map in `Run::drive`'s operation; if it is read across strata, it is chunk data (§5.4) |
| 138 K | `Step::write_out` ← `section::write_list` | u8, 4.3 growths | The section writer writes into the image section's region directly (0628 §4); no list first |
| 94 K, 80 K, 73 K | `JoinRule::encoded`, `Writer::commit` (3.8 growths), `joins::read` | small | Frame vecs, reserved |
| 60 K, 58 K | `Builder::text` (3.8 growths), `Batch::make_rows` (1.55 growths) | | Frame vecs, reserved |

### 5.3 OpScratch (B2: 62 K arrays, but large)

| Arrays | Maker | Shape | Fix |
|---|---|---|---|
| 39 K | `cands::Rows::each` ← `Cands::any` | i64, 3.6 KB, 5.7 growths | Frame, reserved from the candidates' count |
| 11 K | `tape_format::Writer::to` | u8, 22 KB | As the writer above |
| 6 K, 2 K, 495 | `Cells::key`, `Cells::sum_one`, `InRel::each_live_batch` | i64, 45 to 62 KB | One `OpScratch` per digest operation |
| 884 | `Within::solve` ← `Within::starts` | u32, 289 KB, up to 1.4 MB | `OpScratch`; the device's form of the same solve takes the same layout |
| 835, 17 | `level::prefetch` (7.4 growths), `trial_ids` (9.8 growths) | u32, up to 256 KB | `OpScratch`, reserved |
| 278 | `rerail_accel::rows::radix` ← `rows::sort` | [u32; 256] histograms | `LOCAL` on the device and the frame on the CPU: the sort is already shaped for the GPU |
| 210 | `strings::Table::put` | [u64; 8], 4 growths | Chunk data (the program image's strings), with a reserve |

### 5.4 Chunk data (C1: 3.9 M arrays, 644 GB·s)

| Arrays | Maker | Shape | Fix |
|---|---|---|---|
| 2.91 M | `values::Keys::add` ← `Values::add` | block hash maps, 162 B, living 0.1 to 1 s, 431 GB·s | Each copy is `Arc::make_mut` on one of 256 shards when a snapshot shares it. They become one chunk table with an overlay per snapshot, as 0628 §1 made the planner's statistics: a snapshot costs what it writes |
| 549 K | `corpus::cone_digests` ← `Exports::visit` | i64, 32 B, fixed | One array per row becomes rows of one relation (a column of `[i64; 4]` in its chunk block) |
| 77 K, 51 K, 42 K, 26 K, 14 K | `ColMap`, `RelMap`, `NamedMap::shown_of`, `Model::fork_with`, `insert_at_rel` | hash maps per fork | A planner fork still copies maps. 0628 §1's overlays are finished for these tables |
| 57 K | `Batch::ensure_group` | usize | Chunk table |
| 39 K, 30 K | `issue::Board::issue`, `Board::new` | usize | The board's lists in one chunk table |
| 29 K | `section::write_list` ← `Section::of` | u8, 3.5 growths | Written into its image section directly (as §5.2) |
| 3.7 K | `Relation::range_lookup` ← `Layered::range_lookup` | usize, 28 KB, 27% on another thread, 135 GB·s | A range lookup's rows become a `Seq` over its index range, not a list held for a second |

### 5.5 Run data (C2: 1.2 M arrays, 47,264 GB·s)

| Arrays | Maker | Shape | Fix |
|---|---|---|---|
| 4 | `ChunkCounts::add` ← `run_observed_within` | i64, up to 338 MB, 8 growths each, the whole run: 99.5% of all footprint | `ChunkHeld::rows` concatenates each chunk's held rows (`extend_from_slice`). Instead, keep each chunk's rows where that chunk wrote them, as a list of the chunks' regions (a segmented relation), and have `SemijoinCounts` read the segments. Nothing is concatenated or grown |
| 609 K | `cone_rounds::Answers::record` ← `Asks::came` | i64, 32 B, fixed | Rows of one run relation, not an array per answer |
| 579 K | `planner::add_answers` ← `Run::drive` | i64, 32 B, fixed | The same |
| 225 | `Exports::add_owned` (maps) | 52 KB, 160 GB·s | A run table |

### 5.6 Hand-offs (D: 1.7 M arrays)

| Arrays | Maker | Shape | Fix |
|---|---|---|---|
| 549 K | `Exports::rows_of` ← `Exports::visit` | i64, 32 B, fixed, always ended on another thread | The row is written into the receiving relation; nothing is passed across |
| 448 K | `own::Own::put` | i64, 1.2 KB, 86% on another thread | Written into the owner's chunk table |
| 120 K | `memo::cover` | i64, 43% on another thread | The same |
| 95 K, 72 K, 64 K, 25 K, 24 K | rayon `bridge_producer_consumer` collects | i64, 0.35 to 1.2 KB, 2 to 3.5 growths, 89 to 100% on another thread | The output is sized first (or counted in a first pass), and each worker writes its disjoint split |
| 90 K | `corpus::held_rows` | i64, 32 B, fixed | Run relation rows (as §5.5) |
| 60 K | `compile_sets::Reader::row` | u64, 41% on another thread | The matcher's compiled sets, chunk data |
| 32 K | `Model::plan_rule_reused_in` | f64, 77% on another thread | Planner overlay (as §5.4) |
| 18.6 K | `bloom::Sample::offer` ← `Sketch::add` | u64, fixed at 2 KB, 5.9 growths | A fixed-size array, reserved whole, in the sketch's chunk table |
| 17 K, 15 K, 8 K | `InRel::index`, `Rel::new`, `view` | usize, 32 B | Inline, in the relation's record |
| 6 K | `threads::spawn_in` ← `exec::as_job` | [u64; 2], 22 KB | The job's output, sized first |

### 5.7 What the survey does not cover

`std::vec::Vec` appears at about 22,000 sites in `crates/`, tests included, and goes through the heap, not a room.
The probe saw none of them. Step 0 extends the probe to the global allocator, keyed by the same sites, so that
those arrays get the same classes:

- a `Vec` inside an operation may stay where it is, or move to the frame where that is measured to pay;
- a `Vec` holding chunk or run data moves to blocks (0628's order stands for data).

## 6. Work order

Each step:
- keeps findings unchanged (29,930 on cold PostHog);
- is measured by instructions retired on the same input (the head with 0684 is about 3.28 T; pre-merge was 2.30 T)
  and by the probe's counts;
- runs the targeted tests of the code it touched.

**Step 0: the record and the baseline.**
- Write the boundary (§1) and the classes (§2, §4) as a decision record, once the user takes them.
- Extend the probe to `Vec` (§5.7).
- Keep the probe as a profiling tool, off by default, or remove it (the user's call).

**Step 1: the mechanism.**
- In `rerail_vector`: `Inline`, `InlineVec`, `Frame`, `FrameVec`, `scratch::op` and `op_on`.
- `blocks::scratch`'s per-thread region moves down and becomes the frame's backing.
- Tests:
  - growth in place at the top, and a move below it;
  - nested frames, and release on unwind;
  - a compile-fail test that a `FrameVec` cannot outlive its frame;
  - a split handed to scoped workers.
- A micro-benchmark: a 32-byte array made and dropped, frame against `Vector::new` in the run room.

**Step 2: no owned array returned.**
- The vtab `lookup`, `range_lookup` and `lookup_columns` write into a caller's vec or give a `Seq`.
- So do setfix's `find_keyed`, `find_columns` and `find`, `FactsIn::children`, `Within::containers_on` and
  `contained_on`, and `Relation::staged`.
- This removes most of class A, about 30 M arrays.

**Step 3: fixed sizes become types.** `VarSet`, `Probed`, `rowcode::fused`, `Micro::of`, the 32-byte rows of the
rayon collects.

**Step 4: operations open frames.**
- The entry points get `scratch::op`: a join step (`run_join_in_each`), a group's wave (`Batch::make_rows`), a
  document's parse (`parse_from`, the f16 writer), a memo encoding (`encode_parts`), a planner search
  (`plan_search`), a digest (`Cells::digest_batches`), a sort (`rows::sort`).
- Classes B1 and B2 move into those frames, each reserved from a bound.
- The writers' finished images go in their blocks.

**Step 5: chunk and run data become relations.**
- `ChunkCounts`' held rows become segments.
- `Answers::record`, `add_answers`, `cone_digests`, `rows_of` and `held_rows` become relation rows.
- `values::Keys` becomes an overlay table.
- The planner's forks finish 0628 §1's overlays.

**Step 6: hand-offs.** Rayon collects and `spawn_in` jobs write into outputs sized first.

**Step 7: the default room goes.**
- Remove `rerail_vector::default_room`, `install_default_room` and `rows::install_run_room`.
- `HeapRoom` is for tests only.
- Every remaining `Vector::new()` and roomless map outside tests is given its class.
- The probe's count of roomless arrays reaches zero, and the earlier question (heap or run room for roomless arrays)
  no longer arises.

**Step 8: one operation on both sides.**
- One operation declares its memory (§3, "On the device") and runs from that one declaration on the CPU
  (UKL `vec`, frame) and in UKL's kernel form.
- First candidate: `rows::radix`, already shaped for the device. Then the join step.
- Metal runs wait until the hold on them is lifted. Until then this step is built and tested on the CPU backends.

## 7. Questions for the user

1. **Operation memory and `std::Vec`.** Inside an operation, is `Vec` allowed, since "what happens inside is
   unimportant", or does all operation memory go through the frame so that one mechanism counts it?
2. **A frame vec below the top that grows.** It moves within the frame, and the space it leaves is reclaimed only
   when the frame ends. The alternative is a list of segments, which has no move but its reads are not one slice.
3. **The interim.** Until step 7, roomless arrays still take the run room. Should they take the heap meanwhile
   (reverting 8f4fdb7293's default), or stay where they are?
4. **The probe.** Keep it, off by default, as the tool that measures each step, or remove it once step 0's baseline
   is taken.
