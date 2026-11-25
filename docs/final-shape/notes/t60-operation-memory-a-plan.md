# t60. Operation memory: where each scratch array belongs, and one way to ask for it (a plan)

A plan. The user approved it on 2026-11-25 ("go ahead"); its rules are decision 0687.

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
`working-data/m3/syms.py`, `scratch2.py` and `classify.py` (§5's classes).

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

### 5.7 The heap (step 0's survey)

`std::vec::Vec` appears at about 22,000 sites in `crates/`, tests included, and goes through the heap, not a room.
The first survey saw none of them. Step 0 extended the probe to the engine's global allocator
(`rerail_prof`'s `Sampled` calls `rerail_vector::probe::heap_alloc`, `heap_realloc` and `heap_free`), keyed by the
same sites.

Run `hp-3158` was the same cold PostHog run, on m1max under the probe: 29,930 findings, 326 s, 7.14 T instructions,
about three times an unprobed run.

| | Arrays | ≤64 B | <131 µs | Bytes at their ends | Makers |
|---|---|---|---|---|---|
| The heap | 622 M | 457 M (74%) | 469 M (75%) | 76 GB | 3,157 |
| The run room | 57 M | 47 M (81%) | 43 M (74%) | 52 GB | 499 |

By the makers' dominant class, the heap's 622 M allocations are:

| Class | Allocations |
|---|---|
| A | 439 M |
| B1 | 110 M |
| B2 | 0.25 M |
| C1 | 12 M |
| C2 | 36 M |
| D | 24 M |

**The run room's own cost is on the heap too.** Each roomless `Vector` costs, beside its ledger chunk:

| Heap allocations | Maker | Size | What it is |
|---|---|---|---|
| 53 M | `rows::run_room` | 24 B | `rerail_vector::default_room()` makes an `Arc<LifeRoom>` per array |
| 68 M | `region::Mutable::adopt` | 264 B | each region's owner, boxed |
| 7.6 M | `rows::placed` | 24 B | a third allocation per region |

That is two or three heap allocations per array, and it is why a `Vector` costs 39 to 46 ns where a frame vector
costs 4.6 ns (step 1's benchmark). Moving arrays to the frame removes all three.

The heap's own largest makers, the arrays in the operations themselves:

| Allocations | Maker | Size |
|---|---|---|
| 38 M | `rowcode::Fused::run` | 16 B |
| 19 M | `run_join_in_each::enter::flush` | 8 B |
| 30 M | `granules::number` | 156 B on average |
| 28 M | `View::find_keyed` | 40 B |
| 21 M | `Within::lookup` | 37 B |
| 17 M | `LongTexts::text_any` | 11 B: strings |
| 17 M | `Layered::sources_of` | 16 B |
| 11 M | `u8::fmt` ← `GuestRows::document` | formatted text |
| 8.1 M | `ExecutionArena::default` | 151 B, an arena made per root |
| 7.0 M | `containers_on` | 37 B |
| 6.9 M | `contained_on` | 40 B |
| 6.9 M | `FactsIn::children` | 38 B |
| 5.4 M | `Atoms::with_str` | 25 B |

In the dump, `heap` is the type of every heap row. `classify.py` gives each maker its class, as for the room's.

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

## 7. Choices the plan makes

1. **No `std::Vec` in the operations we write.** Everything an operation makes goes through the frame (or `Inline`).
   That gives one mechanism, measured by one count. It is the faster path, since a bump replaces `malloc`. And its
   layout is the one the device form of the operation declares. Third-party code keeps its own allocations.
2. **A frame vec that grows while it is not at the top moves within the frame.** The space it leaves comes back
   when the frame ends, so the waste is bounded by one operation. A list of segments would avoid the move, but its
   reads would no longer be one slice, and one slice is what the GPU-shaped code and the UKL layouts need. A
   reserve from a bound (§3) makes such moves rare.
3. **No interim change.** Until step 7, roomless arrays stay in the run room, as 8f4fdb7293 put them. Steps 2 and 3
   come first because they remove most of the arrays (class A, about 80%). With them gone, the run room's measured
   cost (0.18 T) shrinks before the room itself goes.
4. **The probe stays, off by default.** It measures each step, and it is committed with step 1. When it is off, its
   cost is one empty `Option` per region and one test in `default_room`, and step 7 removes that test.

## 8. Progress

### Step 0 (done)

- **The record** is decision 0687.
- **The probe covers the heap** (§5.7): `rerail_vector::probe`, called from the engine's global allocator, and
  allocation-free while off (one load per allocation).
- **The baseline** is run `hp-3158` (§5.7).

### Step 1 (done)

`rerail_vector::frame`, re-exported as `rerail_blocks::frame`, holds `op`, `Frame`, `FrameVec` and `InlineVec`. The
arena is a list of chunks per thread: 256 KB first, then doubling up to 16 MB, kept up to 64 MB.

Its tests:
- growth in place at the top, and a move below it;
- the frame's end, and an unwound inner frame, giving their bytes back;
- an outer frame's vector growing inside an inner frame, which goes to the heap;
- a vector dropped at the top giving its bytes back;
- a take larger than a chunk;
- an outer frame's take inside an inner frame refused;
- a split written by rayon workers, and stolen jobs' frames;
- `InlineVec`'s spill;
- doc tests that a `FrameVec` neither outlives its frame nor crosses threads.

The cost of a 32-byte array made, filled and dropped, in a release build on a farm Mac:

| | ns |
|---|---|
| `InlineVec` | 3.7 |
| frame | 4.6 (a bare take: 1.1) |
| `Vec` (jemalloc) | 11.4 |
| `Vector` in the heap room | 46 (the run room adds the ledger) |

`InlineVec` spills to the heap, not the frame. It may be made outside any operation, and an inner frame's end
would take a spill it made. So it holds any `Copy` value, slices too.

`op_on` (a frame over a dedicated scratch block, for a kernel to bind) waits for step 8, its first user.
`blocks::scratch` keeps the freeze's and the pack's temporaries until then.

### Step 2 (under way)

Changed so far:

- **`Source::lookup_each`** gives a lookup's rows one at a time, with no list. `Within` and `Layered` implement it
  natively, and their `lookup` is now a collection of it. setfix's `View::find_into` reads through it.
- **setfix's probe lists.**
  - `View::find_keyed_into` fills the step's own list (`LastProbe::ids`): the memo's, or one it let go
    (`Probed::put`, `spare_ids`), reused by the step's next probe. The memo shares it (`Probed::Ids` and
    `Cands::Shared` hold an `Arc`), so the clone per candidate list is gone.
- **`Within`'s containers and contained** fill a `FrameVec` in the frame of `Within::rows` (or the lookup's), which
  writes them into the pairs relation.
- **`FactsIn::children`** returns an `InlineVec<u32, 16>`.
- **Relation rows** (`Relation::from_rows`, `staged`, `Make`) are `Row = InlineVec<i64, 8>`, written with
  `rerail_facts_base::row!`: one per row on the stack, not one array each.
- **`VarSet`** is an `InlineVec<u64, 4>`.
- **rowcode.** A fused chunk's column and output lists (`Fused::run`) and its words (`flush`) are on the stack.
- **`Layered::node_read`** asks the base, then each part, in place: no list of `Arc`s made (`sources_of` is
  removed).
- **`rows::run_room` and `source_room`** give one shared `Arc` per life, not a new one per call. That is 53 M heap
  allocations: the interim run room (§7.3) costs the same without them.

Left for step 4, because each is a whole operation to frame: `granules::number` (about 40 working arrays, its
table placed in blocks at the end) and the parse writers.

The first part (the probe lists, `lookup_each`, `Within`, `children`, `Row`, `VarSet`, the shared room), on cold
PostHog on m1max with the probe off, against HEAD with the probe compiled in:

| | Instructions | User CPU | Wall | Peak | Findings |
|---|---|---|---|---|---|
| Before | 3.338 T | 319 s | 79.1 s | 7.16 GB | 29,930 |
| After | 3.215 T (−3.7%) | 308 s | 75.4 s | 7.17 GB | 29,930 |

## 9. One codebase with Typing-support and the hub (2026-11-25)

This branch merged Typing-support at 4d6ed1c0f8. That commit holds the hub (final-rerail-shape) at 67a14fbb5d, so
the merge brought in both lines.

The hub and Typing-support changed the same memory independently:

- **0342 amended twice (Typing-support, 2026-11-24).**
  - A stack per thread is the default room.
  - Large regions go to the thread's free-listed blocks.
  - The thread's long room (`maps::long`) holds what the census finds outlives its work.
  - `scratch::with` makes nested uses of that same stack.
- **The hub's 359dc17781** is 0684's change, made a second time. Its names are kept (0684, "Merged").

How 0687 sits with 0342 is recorded in 0687 ("With 0342's stack"). The work it leaves, in order:

1. **One stack per thread.** The frame's arena takes its chunks from `scratch::Stack`, each chunk a region with one
   header, and bumps without headers inside the chunk. The frame's bytes then lie in a block (`place_of`), the
   `OpScratch` class of §2 for any frame, so `op_on` is not needed. The bar is the step 1 benchmark: 4.6 ns for a
   32-byte array.
2. **One survey tool.** The probe (`RERAIL_PROBE_SCRATCH`) and 0342's census (`RERAIL_ANON_CENSUS`) count the same
   regions. The census takes the probe's heap counts and lifetimes, and the probe goes.
3. **Step 7 is reworded.** Each remaining `Vector` made with no room in an operation becomes a frame or inline
   array. One that outlives its operation goes in `maps::long`, or is data in a relation's block. The stack stays
   the default room, for everything else.
4. **Measure again on cold PostHog.** The run is the merged engine against `de8d639702`: instructions, peak and
   findings (29,930). 0342's own figures are for one-file edits of corpus0.

### 9.1 The re-baseline and the batches since (2026-11-25)

A binary from before the merge cannot run against the merged tree (phase 0's library changed with it:
`cross_file` is not lowered by the old binary). The merged engine (`de3b8fc8b3`, "mg") is the baseline, measured in
the same session as each batch, cold PostHog on m1max, CPU, with the probe off:

| | Instructions | User CPU | Wall | Peak | Findings |
|---|---|---|---|---|---|
| Merged (mg) | 2.698–2.704 T | 252.4–253.8 s | 64.4–67.6 s | 7.83–7.93 GB | 29,930 |
| Batches 1–2 (`cec68d61cf`) | 2.670 T (−1.0%) | 252.5 s | 65.3 s | 7.53 GB | 29,930 |
| Batches 1–3 (`151426b629`) | 2.656 T (−1.8%) | 249.7 s (−1.6%) | 63.9 s | 7.57 GB (−0.26 GB) | 29,930 |
| Batches 1–4 (`bf7c57a584`) | 2.644 T (−2.3%) | 248.9 s | 64.8 s | 7.22 GB (−0.68 GB) | 29,930 |
| Batches 1–5 (`6a9e50c610`) | 2.597 T (−4.1%) | 256.4 s (−2.6%) | 76.6 s | 7.28 GB (−0.58 GB) | 29,930 |

Batch 5's session ran on a busier host: its merged engine took 263.3 s and 76.8 s there, so its CPU and wall are
read against that run, not the range above.

The merged engine runs 0.5 T fewer instructions than this morning's step 2 (3.215 T): the hub's and
Typing-support's own work. Its peak is 0.76 GB more (7.17 GB then).

The merged engine's survey (`RERAIL_PROBE_SCRATCH`, 327 M arrays, 13,306 makers) ordered the batches:

1. **Step 4's operations** (`f79fad2426`). `granules::number` and the memo encoder (`encode_parts`) run in frames.
   Each writes its table once: the granule table is one region of 18 columns, not 18 arrays.
2. **Reads in place** (`cec68d61cf`). A walk reads node and long texts where they lie (`node_read_ref`,
   `Interner::with_name`): 17.15 M and 5.38 M arrays. A tape's digest is shown without a `String`
   (`tape_hex`, 11.1 M). A projection's 13 rank samples are one region.
3. **Run and chunk data** (`719c9f4ee5`, `151426b629`, and the plan search's).
   - A chunk's held rows stay one segment a chunk, in the chunk thread's long room. Before, one run-long array was
     grown by every chunk.
   - The cone rounds' answers and the planner's answer log are made in the long room.
   - A relation's column names are interned once per schema, and its columns' indexes and orders are one list.
     Before, each relation had `1 + k` strings and two boxes: 3.74 M + 2 × 1.07 M arrays.
   - A kept plan's search account is shared, not cloned at each of 3.65 M lookups.

4. **The survey's next makers** (`b4`'s survey: 242 M arrays against the merged engine's 327 M).
   - `values::Keys`: a shard is made on its first value, and the parts map is keyed by shared names.
   - The attribution reads each file's rows as one run (`by_file_of`), and a chunk's walked nodes are a segment a
     chunk in the long room. A shared value document is claimed by its first chunk, so a per-document set was wrong.
   - `distinct_upto` counts in an open table in a frame; `owners` borrows a document's list.
   - A parse's writer is reused across a thread's documents (`Writer::reused`, the `SPARE` slot, census `Thread`);
     TreeOps' writer too.
   - A relation's column index is boxed when first built.
5. **Hand-offs and copies made a call** (`a7398f3f8f` … `6a9e50c610`).
   - One hex for the engine (`prof::lower_hex`): the observer's digests made a string a byte (6.0 M).
   - A made group's column indexes are boxed when built (16 KB a group before).
   - One `ExecutionArena` a transducer run, cleared at each root (2.33 M).
   - A kept entry's row shapes are numbered once (2 × 2.37 M), and a frame's image goes into its block from a
     slice (`Entries::put_with`; 2.72 M + 2.39 M).
   - A string test finds its matcher by borrowed texts (2.76 M); a join's test reads its pattern into the
     interner's buffer, and `cat` tests emptiness in place (3.33 M).
6. **After batch 5** (`127229e53c` … `bd5cae4bfa`, not yet measured): a keyed probe walked once gives its rows to a
   closure (`each_keyed`, 2.7 M); a document found by its key makes no list (2.06 M); a memo entry's postings are
   named once a column and its covered rows written once (1.32 M).

The merge's red tests, fixed on the way:

- 0652's check now takes a demand set (`magic_…`) as the engine's own crossing (#932's case).
- Placement is as 0661 decided.
- The executor-coverage ledger is blessed.
- The state censuses follow the crate moves.
- The closure form refuses a component with a lattice relation.
- The opcode parity test knows `STATEMENT` (0650's recovery word).

Left, by the survey:

- `values::Keys::add` (5.44 M). The parts index is copy-on-write: each value admitted clones the parts map (a
  string per relation), the parts it adds to, and the shards it touches. It wants shards changed in place under the
  admit lock and read under theirs.
- The batch's `Vector<i64>` makers inlined into `make_rows` (7.46 M + 1.15 M), found by line in the next survey.
- The rayon collects (3.7 M).
- `scope::Builder::bind` (4.9 M, from the Python visitor's `bind_name`).
- `Interner::id` (1.62 M).
- The footprints: `chunk_held_rows` (the run's held rows, real data), `placed::place`, `build_in_wave_as`.
