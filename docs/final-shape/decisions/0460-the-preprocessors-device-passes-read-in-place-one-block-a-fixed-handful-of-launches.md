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
| R8 | The corpus's units still give the serial pass's tokens: 3,132, the same 3 refused | `the_corpus_expands_on_the_device` (ignored) |

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
