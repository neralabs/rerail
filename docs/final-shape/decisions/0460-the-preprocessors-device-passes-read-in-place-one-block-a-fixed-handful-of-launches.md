# 0460: The preprocessor's device passes read in place, in one block, in a fixed handful of launches

Status: accepted (2026-11-19); being built in the steps below. Revises 0448 (§2's memory, §3's
passes) and 0436 (R1, the tape's format). Numbered past the highest in any worktree (0459).

The user, on 0448 as built:

1. "I thought that the latest token indexes had a line index? Please verify."
2. "Look at the slabs and the linear GPU-compatible data structures in the rest of the
   worktrees. Make this use better and more compatible primitives."
3. "Why do we need so many launches? I was expecting a fixed number."

Then, on the plan: "use format 16, and merge whatever is needed from sibling branches to be
up-to-date before we start"; and "The setup should use barriers, etc to reduce the number of
launches to a fixed handful".

## Intro

0448's expansion gives the serial pass's tokens on 3,132 of the corpus's units. The user's
review found three things wrong with how it gets there, all about the engine's shared pieces.
The expansion uses none of them, and builds its own instead.

**The line index.** A format 16 tape carries its source's newline index (Elias–Fano,
`rerail_tape_format::newlines`). The reads written once in CLIF answer a byte's line from it
(`tape_line`, `tape_line_start`), on the CPU, in MSL and in UKL (`clif_tape_line`). The
preprocessor's tape was written as format 15 (`rerail_c_pp::tape::image`, `Writer::finish`),
which stores no lines. So 0448 counted each 4 KB block's newlines a word at a time, scanned the
counts, and climbed to a token's line itself (`px_line_blocks`, `px_newlines`, `px_line`). That
was a second line index. The evaluator also read the tape through a reader written by hand in
`pp_eval.metal`, not the CLIF reads (against 0310).

**The structures.** The engine has linear structures the CPU and the GPU share (0260, 0266,
0342, 0353), and 0448 used none of them:

| The engine's | 0448's |
|---|---|
| The slab residency: a tape placed once (`slabs::tape_room`, 0353 step 4a), a source admitted once (`admit_sources`), one block a batch reads (`gather`) | `Unit::load` copies each source and tape into a new buffer. That is a block copy, and 0353 does not list it |
| One GPU pool and budget: a `rerail_blocks::Block` leased within the budget (`lease_block_within`), with structures nested inside at offsets | About 80 `MappedBuffer`s a unit, outside the pool and the budget. Metal's 31 bindings forced two buffers to be merged |
| The tape's reads written once in CLIF (0310) | A hand-written v9np reader |
| UKL's library (0440 §5: `scan.h`, `radix.h`, `compact.h`, `group.h`, `memory.h`, `workqueue.h`), the same kernels on Metal, CUDA and OpenCL | Its own scan, segmented scan, sort and work queue in MSL (`pp_blocks.metal`, `work_queue.metal`) |

**The launches.** An evaluation took 118 to 176 launches. Counted from the encoder:

| What | Launches |
|---|---|
| The expansion's work queue | 6 (3 for each of its two phases) |
| The three radix sorts: 8 bits a digit; each digit a histogram, its scan's levels, a scatter | 80–90 |
| About 10 scans, 1, 3 or 5 levels each, chosen by the unit's capacity | 10–30 |
| 3 segmented scans, 5 levels each | 15 |
| Single passes | about 30 |

The expansion's 6 launches are fixed, because its work queue keeps the grid running. The rest
is one dispatch per level of a primitive. A dispatch boundary was the only way a level waited
for the level before it, since Metal promises no progress between threadgroups. The count varied
with the unit, because each primitive's number of levels was chosen by its capacity.

## Decision

### 1. The preprocessor tape is format 16, and the evaluator reads it where it is

- `rerail_c_pp::tape` writes format 16 through the tape crate's writer (`Target::F16`,
  `finish_16`: block-major, one pass, 0426), with its newline index. The tape's codes are fixed
  (its kinds and fields), so its language table is the crate's own static `f16::Lang`.
- 0436's R1 changes from "a v9np image" to "a format 16 image, read by the CLIF reads".
- The Bitlex grammar's tier 2 still writes format 15, as every tier 2 does until 0426 reaches it.
  Its tapes are compared with the lean writer's through the tape API (R4). The evaluator reads
  the lean writer's format 16.
- A unit's sources are the residency's (`admit_sources`), and its tapes are placed once in slab
  room (`tape_room`). A batch's kernels bind one gathered block and read each file through its
  `TapeView`. `Unit::load` and its copy go.

### 2. The reads written once

- The evaluator's kernels read the tape through the CLIF reads: `ukl/reads/tape.h` once they are
  UKL (§5), `rerail_clif`'s reads until then.
- `__LINE__` is the line of its token's origin: `tape_line` of that token's start, plus 1, from
  the tape's newline index. The climb through each expansion's end (`texp`) stays.
- 0448's line index goes: its passes and its scan.

### 3. The arena is one block

- A batch's arena is one `rerail_blocks::Block`, leased within the one budget
  (`lease_block_within`).
- Every store is at an offset in it, laid out once a rung: tokens, lists, arguments, the output,
  the scratch, and the primitives' partials. A kernel binds that block and the tapes' block, and
  reads each store at its offset.
- A rung is a larger block. A unit past its room still runs again at the next rung. A batch the
  budget refuses is `Error::BackendCapacity`, and the caller halves it (`pp_on::fitted`).

### 4. A fixed handful of launches: phases of a persistent grid, separated by barriers

- **The grid** is G groups of 256 threads (8 SIMD groups) that are resident together. G is the
  device's GPU cores times the groups a core holds at the kernel's threadgroup memory; on a Mac
  the cores are IORegistry's `gpu-core-count`, and with no count G is 8. Each group walks its own
  contiguous range of tiles, a tile a turn.
- **Within a group**, the threads synchronize with `threadgroup_barrier`, and SIMD groups use
  their scans and ballots.
- **Between groups**, there is a device-wide barrier, `ukl_grid_barrier` (`ukl/grid.h`, new in
  the library), with one form for each backend:
  - On Metal and OpenCL it is an arrive counter and a generation word in the block. Each group's
    first lane makes the group's writes visible, then adds to the counter. The last to arrive
    resets the counter and steps the generation. The others poll the generation, a bounded number
    of times (0027: no wait without a bound).
  - On CUDA it is a cooperative launch's grid sync.
  - When the bound is reached, the unit's `barrier` flag is set. Every phase reads the flag and
    stops, and the unit is refused. The host runs it again at G/2. As with `room`, that choice is
    made for the next run, so 0436's R5 (no host step between passes) holds.
- **A primitive is a few phases**, with a barrier between each and the next:
  - A scan: each group reduces its chunk; group 0 scans the G sums; each group scans its chunk
    from its offset.
  - A segmented scan: the same, with the head flags carried.
  - A radix digit: each group makes its chunk's histogram in threadgroup memory; the G × 256
    counts are scanned, digit-major; each group scatters its chunk in its order, so the sort is
    stable.
  - A single pass is one phase.
- Passes that do not depend on each other share a phase.
- A phase reads its count from the block, so there are no indirect arguments, and the block is
  the only thing sized by the host.
- **The expansion** runs on the same grid. Its queue's workers take items until the queue's
  count of active items is 0, a bounded number of turns. Its two phases are separated by a
  barrier.
- **Launches**, the same for every unit, whatever its size: the setup's kernel, the expansion's
  kernel encoded twice (the second finds the queue empty and ends at once), and the check. That
  is 4, or 3 where the setup and the expansion are one kernel.

### 5. The primitives are UKL's library

- The kernels are written in UKL over the library's group and memory primitives (`group.h`,
  `memory.h`), and the expansion becomes `pp_expand.uglykl`.
- The library gains two parts, each written once and tested on every backend:
  - the grid barrier;
  - the device-wide scan, segmented scan (add and max) and radix sort as phases of a persistent
    grid. Each is a device function that all G groups call, and reads its count from the block.
    The library's host plans today launch one dispatch a level, sized by the host.
- `pp_blocks.metal`, `pp_blocks.rs` and the MSL work queue (`rerail_metal::work_queue`, the
  preprocessor its last user) go.

## Merges before the work (2026-11-19)

| Branch | Merged? |
|---|---|
| final-rerail-shape at 7d8d2afa8 (UKL and the device layer, 0426, 0427, 0435) | Yes (9398ce1d7) |
| The hub at 5fbc02057 | Yes (ef1042656) |
| CUDA-support's 9f34a5480 (the label passes' runner on the lane) | Not merged: 14 hunks in the label passes' runner, which final-rerail-shape rewrote since (0447, 0449, 0459). It is for the line that holds both. These passes need none of it |
| Native-and-GPU-strata at a5c411cb8 (0445's row blocks, 0450) | Not merged: its new CLIF backend operations (coherent loads and stores, atomic `or`, compare-and-swap, fences) are written for the MSL backend, which UKL's replaced here. Merging means porting them to `clif/src/ukl.rs` and regenerating the reads, which is for the line that holds both. These passes use `Block`, the pool and the slabs, all already here |

## Requirements, each held by a test

| | Requirement | Test |
|---|---|---|
| R1 | The preprocessor tape is a format 16 image with its newline index, and its reads give the lean writer's nodes | `the_pp_tape_is_a_format_16_image` |
| R2 | The evaluator reads its files where the residency placed them: no copy of a source or a tape | a guard over `pp_eval.rs`; `the_names_pass_reads_the_tapes_in_place` |
| R3 | `__LINE__` is the tape's newline index's | `line_and_file_are_the_serial_passes` |
| R4 | One block a batch, leased within the budget: no `MappedBuffer` of the arena | a guard over `pp_expand.rs` and `pp_eval.rs`; `a_unit_past_its_room_runs_again_larger` |
| R5 | Each persistent-grid primitive equals a host reference from 1 to 10^6 elements, at G = 1, 8 and the device's G | the library's tests, on Metal and UKL's CPU backend |
| R6 | The grid barrier: 10,000 in a row, each seeing every group's writes before it; past residency, `barrier` and no hang | the library's tests |
| R7 | An evaluation is the same fixed handful of launches for the smallest and the largest unit | `an_evaluation_is_a_fixed_handful_of_launches` |
| R8 | The corpus's units still give the serial pass's tokens: 3,132, the same 3 refused | `the_corpus_preprocesses_on_the_device` (ignored) |

## Steps

1. This decision and its README row.
2. Format 16, the input in place, the CLIF reads, and the line index removed (§1, §2; R1–R3).
3. One block (§3; R4).
4. The grid barrier and the persistent-grid primitives in UKL (§4, §5; R5, R6).
5. The setup as one kernel and the expansion in UKL (§4, §5; R7, R8).
6. 0448's and 0436's sections brought up to date.

## Built

- **Step 2a** (c20304d20): `rerail_c_pp::tape::image_16` writes the preprocessor tape in format
  16, with its newline index. Its language table is the crate's static `lang()`: a code for each
  kind and field, tape tag 0, which no language's dialect takes. Every node's kind, field, start,
  end and subtree end, and every node's line, equal the format 15 image's and the source's
  newlines (R1, `the_pp_tape_is_a_format_16_image`). All 3,135 corpus files pass.
- **Step 2b and 2c** (this commit):
  - A unit is its documents in the slab residency. `pp_eval::placement` is a document's
    placement (`slabs::insert`): its tape, then its source, and the pp language's `DeviceCode`
    table, held once a slab under key `revision << 8` (tag 0). Its view is the tape's own
    (`f16::Placement::view`). `Unit::of` gathers the documents into one slab (`slabs::gather`)
    and lists their views, and the kernels bind that slab and the views (`PpFile` is
    `DocView`).
  - Writing a tape into its room is its loader's: the store's load (0353 step 4a) or, in the
    tests, `eval::tests::place`.
  - `Unit::load` and its copy are gone.
  - The kernels read each node through the CLIF reads (`clif_tape_number`, `start`, `end`,
    `size`, `line`), with `<ukl/reads/tape.h>` preprocessed for Metal
    (`pp_blocks::source`, `rerail_ukl::metal_source`). The hand-written v9np reader is gone.
  - `__LINE__` is `clif_tape_line` of its token's start, plus 1. `px_line_blocks`,
    `px_newlines`, the line index's buffers and its scan are gone (R3,
    `line_and_file_are_the_serial_passes`).
  - R2's guard is `the_evaluator_copies_no_tape`.
  - Results: the 13 evaluator tests pass, as do the 5 pp block tests. The corpus's first slice
    (`RERAIL_PP_COUNT=500`) gives 999 units the serial pass's tokens, with 1 refused for `#line`,
    as before. That slice still takes up to 146 launches, which steps 3–5 bring down to a fixed
    handful.
- **Step 3** (this commit): an evaluation's stores are in one block, its arena
  (`pp_blocks::Arena`).
  - The passes' structures (`Front`, `Defs`, `Vis`, `Exp`, and the scans and sorts inside them)
    declare their stores in it (`zeroed`, `from_slice`): each a `Buf`, an offset in the layout.
  - The arena is then leased once, before any pass is encoded. It is a
    `rerail_blocks::Block` from the GPU pool, within the one budget (`lease_block_within`). A
    refusal is `BackendCapacity`.
  - A pass binds each store as the block's buffer at its offset (`Bound`,
    `checked_set_buffers_at`). Indirect arguments are read at their store's offset.
  - The unit's views are a store of the arena (`Front::files`), and its documents are the
    gathered slab.
  - A rung is a new, larger arena.
  - The 31-binding limit still holds while each store is its own binding. The column merges
    it forced go with step 5, where a kernel binds the arena once and reads each store at its
    offset.
  - R4's guard is `the_arena_is_one_block`. All 14 evaluator tests and 5 block tests pass.
- **Step 4** (this commit): `ukl/grid.h`, in UKL's library.
  - `ukl_grid_barrier`, bounded at `UKL_GRID_SPINS` polls. Past the bound it sets the grid's
    failure word, and every later barrier returns false.
  - The persistent grid's primitives, each a device function every group calls, its count read
    on the device:
    - `ukl_grid_scan_n` and `ukl_grid_scan_exclusive_u32`;
    - `ukl_grid_seg_scan_n`, add and max;
    - `ukl_grid_sort_u64`, a stable LSD sort. Its passes are the library's own tile bodies
      (`body/radix_tile.h`), included again over coherent rows: the body gains
      `UKL_RADIX_SPACE`.
  - Reads of other groups' writes go through `UKL_COHERENT`.
  - Tests, on Metal (`rerail-gpu`'s `library_tests`), at 1, 8 and 32 groups:
    - `grid_barriers_see_every_groups_writes`: 10,000 barriers, each round reading another
      group's writes (R6);
    - `a_grid_past_residency_fails_its_barrier`: 4,096 groups of 1,024 fail in about 0.4 s and
      do not hang (R6);
    - `the_grid_scan_equals_the_host` and `the_grid_segmented_scans_equal_the_host`, 0 to 10^6
      values, a count past the buffers' lengths ignored (R5);
    - `the_grid_sort_is_stable_and_equals_the_host`, 0 to 10^6 keys, 8 to 64 bits (R5).
  - Results: the library's other 17 tests still pass, and `grid.h` stands alone on every backend
    and as OpenCL C 1.2 (`rerail-ukl`'s tests, under cargo).
  - Not yet as decided:
    - R5's "the device's G" is tested at 32 groups, not at the device's resident count. That
      count comes with the setup kernel in step 5.
    - CUDA uses the same counter form: the device layer has no cooperative launch.
    - The tests ran on Metal only, not on UKL's CPU backend.
- **Step 5a** (this commit): the evaluation on one persistent grid, four launches for every unit.
  - `pp_grid.metal`:
    - `pp_setup`: the front, the definitions, the visibility and the top level, to the queue's
      seed, as phases with a grid barrier between them;
    - `px_expand`, encoded twice: the queue's first phase, then the top level's rollback,
      sizes and places, then the queue's second phase. A stage word lets the second launch
      take up a queue whose workers all gave up waiting;
    - `pp_check`: the refusals.

    Every pass of `pp_eval.metal` and `pp_expand.metal` is now an inline body over a grid-stride
    loop, and `px_work` is `px_worker`, run by every SIMD group of the grid.
  - The scans, segmented scans and sorts are `ukl/grid.h`'s. `pp_blocks.metal` keeps only the
    searches. `Scan` keeps only its total, and `Sort` keeps only its other buffers and
    histograms. `SegScan`, the plan kernels, the indirect arguments and `Grid::Indirect` are gone.
  - The kernels bind the arena once, with the slab and the layout. `SLOTS` names every store,
    and `source()` turns it into the MSL's `S_*`. `Lay` holds a run's offsets and words, and
    each structure writes its own slots (`Front/Defs/Vis/Exp::lay`). The 31-binding limit no
    longer applies.
  - `LAST` stops `pp_setup` after the front, the definitions or the visibility, for the tests
    of those stages.
  - The groups' sums are sized by the run's groups. A failed barrier sets `X_BARRIER`. The
    host sees it in the grid's failure word and runs again at half the groups (`fewer`), down
    to 1.
  - Tests:
    - `an_evaluation_is_a_fixed_handful_of_launches` (R7): an empty unit, a unit of 800
      definitions and uses, and a unit that runs again at a larger rung, all at 4 launches. A
      grid of 4,096 groups fails its barrier, runs again, and gives the same output at 256
      groups.
    - All 16 evaluator tests pass, and the 2 block tests pass.
    - The corpus in three slices (R8): 999 + 1,199 + 934 = 3,132 units equal the serial pass,
      the same 3 refused (`#line` twice, a call across a stretch's end once), every unit at 4
      launches. Before this step a unit took 118 to 176 launches.
  - Not yet as decided:
    - G starts at 32 (`GROUPS`), not the Machine's resident count. 256 groups of 256 held here
      (an M5), so 32 is conservative. The count from IORegistry's `gpu-core-count` is still to
      come.
    - `pp_expand` is still MSL, not UKL (step 5b).
    - The GPU time, first slice (999 units, 2.34 M tokens), from the command buffers' GPU
      intervals (`Expanded::gpu_seconds`), on an M5 at load average 32 to 40: 2.24 s in all,
      a median of 1.67 ms a unit, and 16.8 ms for the slowest. A small unit's time is mostly
      its barriers: about 100 a run, at 32 groups. Step 4's code was not timed, so there is
      no comparison against it; that would take a second build of step 4.
- **Step 5a's GPU time, taken apart** (2026-11-19, M5, the machine at load average 32 to 72 but
  its GPU idle). The first slice's 2.24 s is about 2,000 times what 2.34 M tokens should cost.
  The measurements are ignored tests: `where_an_evaluations_gpu_time_goes`,
  `the_grid_sort_alone_is_fast` and `what_a_barrier_costs`. A stop word (`STOP`) ends
  `pp_setup` before phase k, so the difference between two stops is that phase's cost.
  - **What a step of the grid costs:**

    | Step | Cost |
    |---|---|
    | Empty command buffer | 11 µs |
    | `ukl_grid_barrier` | 2.8 µs at 1 group, 3.8 µs at 32 |
    | Device fence | 0.8 µs |
    | Threadgroup barrier with device scope | 0.4 µs |
    | Dependent load | 0.2 µs, coherent or not |

  - **`ukl_grid_sort_u64` costs about 35 µs a digit whatever its count:** 154 µs for 3 keys or
    1,000 at 32 bits, and 275 µs for 3 keys at 64 bits. Each digit runs a whole tile's rounds
    and about 8 grid barriers. The setup sorts 17 digits, about 600 µs of the fixed cost.
  - **A two-line unit costs 1.3 ms, the same at 1 group as at 32.** It is a fixed latency: about
    130 grid barriers, the sorts' digits, and chains of dependent loads. None of it is work.
  - **A unit of 15 K nodes in one file costs 4.3 ms at 32 groups.** About 0.6 ms of that is
    `pp_counts`/`pp_dir_counts` and another 0.6 ms is `pp_dirs`: each is one thread a file,
    walking the whole tape serially (40 ns a node). The queue's two phases take 2.5 ms.
  - **The corpus runs one evaluation a unit:** 999 command buffers one after another, each
    paying the fixed latency. 0448 §2's "one arena a batch of units" is not built: `expand`
    takes one unit.
  - So the causes, largest first:
    1. no batching: the fixed latency is paid 999 times, not once;
    2. a per-evaluation latency of about 1.2 ms, from barrier count and per-digit sort cost
       rather than from work;
    3. serial per-file walks, which scale with a file's nodes on one thread;
    4. the queue's phases on large units.
- **Step 5c: batches, and the grid's latency cut** (user, 2026-11-19: "please do all of these").
  - **Batches** (`expand_many`): one arena and one run of the grid for many units.
    - `Layout::join` lays the units out one after another: their files, directives and stretches,
      with each unit's slots past the last unit's highest.
    - `visible()` sees only rows from its unit's first slot on (`FLOORS`, a search by slot),
      plus the built-ins' rows at slot 0. A row of an earlier unit sends the walk straight to
      the built-ins, so another unit's rows never spend the walk's bound.
    - `Unit::batch` reads the units' documents in the one slab they are in. It refuses documents
      in more than one slab, because gathering them would copy them (0353).
    - A batch splits in halves when its documents span slabs, when the budget refuses its arena
      (at 500 units the arena is about 505 MB, against the 512 MB Metal budget), or when the
      device refuses it. A refusal is therefore a lone unit's.
    - Each unit's output starts at its first stretch's first top token's place, read from the
      two scans already stored (`Expanded::unit_out`).
    - Test: `a_batch_is_its_units`. Eight units define, undefine and use the same names
      differently, one redefines `__FILE__` before another uses it, and one is refused. Each
      unit gives its own serial answer, and the refused unit is refused alone.
    - The first slice: 999 units equal, in 4 runs of about 250 units. GPU time is 74 to 84 ms,
      down from 2,242 ms.
  - **The grid's primitives** (`ukl/grid.h`, `ukl/group.h`):
    - `ukl_grid_scans` runs up to 4 scans over shared barriers, and `ukl_grid_scan_n` is the case
      of one.
    - Up to `UKL_GRID_SMALL` (4,096) values, a scan or segmented scan is group 0's alone: one
      barrier, not three.
    - The sort of one tile (up to 1,024 keys) is group 0's rank sort in threadgroup memory: one
      barrier, against 5 a digit.
    - A larger sort first finds the bits that vary (an OR and an AND of the keys, one phase), then
      passes over only the digits that hold them. A last pass moves the rows where the caller
      reads them (`ukl_grid_sort_in_input`).
    - `ukl_tg_or` and `ukl_tg_and` are new in `group.h`.
    - Tests: the library's grid tests, with 64-bit sorts of keys that vary in two digits and in
      one; 17 tests on Metal, and `rerail-ukl`'s 8 under cargo.
  - A two-line unit's evaluation drops from 1,368 µs to 637 µs.
- **Step 5d: no thread walks a file.** The front's `walk_dirs` (one thread a file, its items in
  order) is gone, with `pp_dir_counts` and `pp_dirs`. Each row's fields come from passes of a node
  a thread, and scans and sorts between them:
  - a row is a node of a directive's kind;
  - its depth comes from the groups' +1/−1 events, scanned;
  - its rank among rows, groups and arms, from their flags scanned in one `ukl_grid_scans`;
  - the arm it is read under is the latest arm at its depth before it: a search in the arms'
    keys (depth, place), sorted. A row's group node is likewise the latest group one depth less;
  - `text_before` is the text tokens' rank (the text map moved from the visibility into the
    front);
  - the guard's shape comes from per-row marks (atomics by file) and a per-file pass.

  `the_directives_pass_gives_the_spikes_tables` and every evaluator test pass. The corpus:
  3,132 units equal, the same 3 refused. GPU time for the three slices: 52, 42 and 45 ms.
- **Step 5e: fewer barriers.** The files' first nodes and the node total are the views' counts,
  which the host holds, so the front's count pass, its barrier and its scan go: the bases are
  written into the block with the layout. Independent scans share their barriers through
  `ukl_grid_scans`: the front's row, group and depth scans (one), its text and arm scans (one), and
  the top level's `pscan` with `bscan` (one). Measured warm on the M5, load average 14 to 38:
  - a small unit takes 182 µs of GPU time (it was 188 µs after step 5d, 1,368 µs before 5c);
  - a large unit at 32 groups takes 2.20 ms (it was 2.21 ms after 5d, about 4.0 ms before);
  - the corpus slices take 36, 41 and 44 ms (52, 42 and 45 ms after 5d; 2,242 ms for the first
    slice before batching).

  The large unit's time is now mostly two phases of the top level (phase 32 at 265 µs, phase 21
  at 72 µs), not barriers.
- **Step 5f: the two slow phases were sorts.** After 5e, a large unit's setup took 539 µs, and
  two phases held 337 µs of it: the paren closes' sort (264 µs) and the definitions' rows sort
  (73 µs). Measured alone (`the_grid_sort_alone_is_fast`), the old sort was slow in both of its
  paths:
  - the one-tile path ranked every row on group 0 alone, n² compares on 256 threads: 1.5 ms for
    1,000 keys;
  - a grid digit gave each group whole tiles of 1,024 rows, so 1,600 rows kept 2 of the 32 groups
    busy: about 46 µs a digit.

  The changes:
  - **Few rows rank over the whole grid.** For `n <= UKL_RADIX_TILE` and at most a row a thread,
    each group holds the rows in threadgroup memory. Each SIMD group ranks a row at a time, its
    lanes taking a share of the compares each and `ukl_sg_sum` adding them. A row's lane keeps
    its rank. Where the rows are sorted in place, one grid barrier comes before they are written.
    1,000 hashed keys: 73 µs at 32 groups, from 1,494 µs.
  - **`ukl_grid_sort_u64_from(…, low, bits, …)`**: the rows are already in the order of their low
    `low` bits, so only the digits above them are passed. The closes are placed in position order
    (their scan), so their sort passes the depth's digits alone (`low` 24).
  - **The depth's bias is 2^15 + 128,** not 2^15, so the depths near 0 (−128 to 127) differ in
    one byte, and a closes sort passes one digit, not two.

  The library's sort test adds the few rows' path at 1, 8 and 32 groups (200 keys) and two sorts
  given `low` (900 keys, low 32; 70,001 keys, low 24).

  On the M3, a large unit at 32 groups now takes 401 µs of setup: the closes sort takes 58 µs, the
  rows sort 28 µs. On the M5 under load, the setup went from 539 to 230 µs. The corpus is
  unchanged: 3,132 units equal, the same 3 refused.

  **What is left is the expansion.** It is 3.4 of the large unit's 3.8 ms on the M3, and 9.6 of a
  250-unit batch's 12.7 ms (`RERAIL_PP_STAGES=250`, sent to the host with its corpus slice:
  `RERAIL_PP_CORPUS`).
- **The encoders are labelled.** `Enc::run` set no label, and the launch budget's record reads
  each dispatch's label, so `nsstring_as_str` was given nil: undefined behaviour that a cargo test
  catches (Relax's does not). Each encoder now takes its kernel's name.
- **Step 5b: the evaluator is UKL, launched through the lane (§5).** `pp_eval.metal`,
  `pp_expand.metal`, `pp_grid.metal`, `pp_blocks.metal` and the MSL work queue
  (`work_queue.metal`, `rerail_metal::work_queue`) are gone:
  - The passes are `pp_eval.uglykl`, `pp_search.uglykl` (the searches), `pp_expand.uglykl` and
    `pp_grid.uglykl` (the kernels), and the tests' probes are `pp_probes.uglykl`.
  - The kernels are `DEFINE_KERNEL2(pp, …)`. The grid's take the arena, the slab and the
    layout as rows, and their threadgroup memory as a `SHARED` row (`PpTg`). The probes take
    their scalars as `ARGS` (`PpProbeArgs`).
  - The passes are written in UKL's dialect: C types, `ukl_atomic_*` over plain words,
    `ukl_sg_*`, `ukl_min` and `ukl_max`, `ukl_ctz32`, and pointers where MSL had references. Every
    function that reaches a SIMD-group operation takes `__UKL_PARAMS` (15 of the expansion's).
  - The expansion's queue is `<ukl/workqueue.h>`, the same protocol. An argument's list is made
    by `ukl_atomic_cas`, a strong exchange, where MSL had a weak one in a loop.
  - `pp_blocks.rs` is `pp_arena.rs`: the arena, the layout and the one source, which the kernel
    store now preprocesses for the device's backend. `Enc` holds a `lane::Command`, and binds rows
    by name at their offsets in the arena, the arena's buffer now the device layer's
    (`Block::device_buffer`). The slab is bound as its `device_buffer`. R9's one command is the
    lane's.
  - Tests: `the_evaluator_is_ukl` (no MSL left in the source; UKL's queue and grid; launched
    through the lane), and `the_source_compiles` (every kernel; `RERAIL_PP_DUMP` writes the
    preprocessed source). All 18 evaluator tests and the 3 arena tests pass. The corpus: 3,132
    units equal, the same 3 refused, 4 launches each.
- **The evaluator is OpenCL C 1.2 too (§5).** `the_source_is_opencl_c_1_2` preprocesses the
  source for OpenCL, its SIMD group emulated at width 32, and runs clang's OpenCL C 1.2 front end
  over it (0440 §3.5's gate). It found what Metal's C++ front end accepted:
  - MSL's references into device memory (`device PxList& L`): 15 functions. They are pointers now.
  - One `__UKL_ARGS` placed inside the wrong call: Metal's `__UKL_ARGS` is empty, so only
    OpenCL's front end could catch it.
- **G is the device's resident count (§4, R5).**
  - `Device::gpu_cores` reads IORegistry's `gpu-core-count` from the entry whose ID is the
    `MTLDevice`'s registry ID. Other devices give no count.
  - `Device::resident_groups(threads, local)` is the cores times the groups a core holds, each
    core taken at its least: one group of the device's largest (`max_threadgroup`, 1,024 threads,
    so 4 groups of 256), and `maxThreadgroupMemoryLength` of threadgroup memory over the kernel's
    own (`ComputeFunction::threadgroup_bytes`, from its pipeline).
  - `pp_arena::groups()` is that count at the grid kernels' threadgroup memory (the larger of
    `pp_setup`'s and `px_expand`'s), and `GROUPS_UNKNOWN` (8) with no count. `GROUPS = 32` is gone.
  - On the M5 Max: 40 cores at 12,432 bytes a group, so 2 groups a core and G = 80
    (`the_grid_is_the_devices_resident_count`).
  - The library's grid tests run at 1, 8 and 32 groups and at the device's count for each kernel
    (`grid_sizes`): 40 to 160 groups on the M5 Max, all passing.
  - All 18 evaluator tests pass at G = 80. The corpus: 3,132 units equal, the same 3 refused, 4
    launches each.
  - Measured, not a check: the three slices' GPU time is 168 ms at G = 80, against 147 ms at
    G = 32 (M5 Max, load average 28 to 37 both times). Most units are small, and a barrier costs
    more with more groups.
- **The grid primitives run on UKL's CPU backend too (R5, R6).**
  - Before this, the CPU ran a launch's groups one after another, so a grid barrier at more than
    one group could not pass there.
  - The shim (`ukl_cpu_shim.h`) now runs up to the launch's resident count of groups at once. Each
    is a coroutine with its own scheduler, threads' stacks and threadgroup memory, and takes the
    launch's next group when its own ends.
  - `ukl_poll_yield()` (`backend.h`) suspends a thread that polls another group's progress: the
    grid barrier's poll, and the work queue's idle polls. Once its whole group polls, the group
    yields to the next. On a GPU it is nothing.
  - On the CPU a poll is a round of every other group, so `UKL_GRID_SPINS` there is 1,000.
  - The host gives the resident count (8,192 threads at once over a group's threads, at most 64
    groups), and stacks and threadgroup memory for each.
  - `RERAIL_GPU=cpu`: the five grid tests pass at 1, 8 and 32 groups, and the 4,096-group grid
    fails its bound in 1.4 s without hanging. The library's other 18 tests pass on the CPU, all
    23 on Metal, and `rerail-ukl`'s 8 under cargo.
  - The CPU run of the grid tests takes 123 s (10^6-key sorts on coroutines).
- **G is capped at 32 (the user, 2026-11-19: "just cap it").** `pp_arena::groups()` is the
  device's resident count at most `GROUPS_MOST` (32). A preprocessor unit is small, and each
  barrier costs more with every group. On the M5 Max G is 32: the corpus at 3,132 units equal, 4
  launches each, 149.6 ms of GPU time over the three slices (load 15 to 20). The library's grid
  tests still run at the uncapped count.
