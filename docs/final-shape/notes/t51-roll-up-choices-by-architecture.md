# t51. The roll-ups' choices, read by architecture

## Intro

Roll-ups 19 to 38 merged each branch's work into this one. At each conflict the records (Q380-Q453) say which side
was kept, but seldom why, in terms of what each side's design holds true. The user, 2026-11-21: "Merge conflicts are
not about code. They are about architecture."

This note goes back over twelve of those choices. Each section says:

- what each side was doing, and the requirement or problem behind it, from the commits and their tests;
- how the two combine, keeping both sides' content and staying as close to main as the content allows;
- what the merged code should be, the fixes that takes, and the tests that check it.

Five of the twelve need fixes (items 2, 3, 4, 6, 10). Seven were right, or had already been done in ours' form; each
of those says why, so it is not opened again. Item 2 is a measured regression: theirs' warm tier fails on this tree.

| # | Roll-up | The choice | Verdict |
|---|---|---|---|
| 1 | 36 | benchmarks: ours' planner, corpus, label passes and launch kept | Right: theirs' work is here in ours' form |
| 2 | 38 | a semi-join's statistics beside its input's frame | **Wrong**: every warm run after an edit builds every document |
| 3 | 38 | `UNOWNED_ROOM`, 64 MiB a relation made outside a run | **Wrong fix**: the open block should be the machine's |
| 4 | 32 | the grid's join yields nowhere; the CPU's grid is one group | **Half done**: production grids do not join |
| 5 | 22 | a file's record lanes one threadgroup | Right: it was 64 lanes, one group, before |
| 6 | 35 | main's decompositions and main-thread record not ported | **Wrong**: divergence from main kept for no reason |
| 7 | 34 | the radix sort's rank groups and virtual threads | Right: two axes, not two mechanisms |
| 8 | 21 | the CPU launch's scheduler fields and resident groups | Right: one struct, each field one job |
| 9 | 25 | cost-classifier's `demand_` lead removed | Right: ours has no form suffixes; Q59 measured |
| 10 | 38 | host files by `doc_of`, others by `hit_document`; the identity file | **Two paths for one thing** |
| 11 | 37 | Cranelift's return area on every host | Right: read only where results spill |
| 12 | 26 | rr-align's deferred joins over ours' roll-up 24 port | Right: theirs is a preparation pass (0281) |

## 1. Roll-up 36, benchmarks-on-integration: ours' planner, corpus and device files kept

**Theirs** (31 commits, 0d2e62b954..87de426f02) is decision 0353's device rows and Q203's device memory:

- Row 5: tape, `ast_depth` and `node_order` read in place on the device, not uploaded (133dcd51b8), and a `has` axis
  probed by one column read off the tape (6fdbc1eb28).
- Rows 9 and 10: a recursive set head grown in its block (41e3362e32), a device-written head alone in a block with
  index room (d83d7d2bf8). Theirs tried the same for every relation of the run's (0ad49fcbdb), measured it worse,
  and went back (87de426f02: copies 917 MB against 780 MB).
- Q203: join scratch given back when a thread's device strata end (5fc8408201), and what threads keep counted
  (`shared::Kept`: the scratch, and the resident relations).
- 0518 (0549 here): the Metal budget is the overall budget by default.
- 0442 for pool jobs: a window's stats and reads (aca7162655) and a labelling's text rows (fec2490bf3) run in the run.
- The planner: a witness cut past a bound head takes only a step joined by a bound variable (960374749f). A dict's
  keys and values, led through two unjoined `unparen` scans, ran past 100 s on 300 files.

**Ours**:

- 0450 step 3 (04a2cf4cc5) removed the device's cache of relations. A view's indexes live in its own block, and
  every upload is per stratum. That cost hogql 74-82 B instructions against 73-76 B, and was taken for one owner of
  every byte (0487).
- Ours' planner prices by witnesses and has no head-bound cut at all.
- Ours runs pool jobs in the run through `profile::on_cores`, `carried` and `run_state::Carry`.

**What the merge did.** Each piece of theirs is here in ours' form:

- The device rows are in ours' UKL join kernel (`vread` cases 9-11, `has_member`, `head_room` and `ALONE_ROWS` as
  at theirs' tip). The run-wide alone blocks theirs reverted were not taken.
- The scratch return is `in_device` and its `DEPTH`.
- `budget::metal()` defaults to the overall budget.
- Theirs' 0442 jobs are ours' `Carry::here` and `carried`.
- Theirs' planner test, `a_cross_product_past_a_bound_head_is_not_priced_as_one_witness`, is in `plan/tests.rs`
  and passes on ours' witnessed pricing (Linux, 2026-11-21). No cut is needed.
- Theirs' `Kept::Resident` count has nothing to count here, since ours has no resident relations.

**Verdict.** No fix. The record's "ours kept" overstated what was dropped; this section replaces it. 0450 step 3's
per-stratum uploads are a known cost of ours' design, measured when it was made, not a merge regression.

**Tests**: the witness test (passes); the device tests on a farm Mac when Metal is allowed again.

## 2. Roll-up 38: a semi-join's statistics beside its input's frame (a regression)

**Theirs.** Q332 makes a warm edit build only the changed document (b7e2c7818d, 59516331e6, 10eb137dcf, 8ee6684b65,
3142635422, d4aa849d18). On ep2 a warm edit went from 35.9 G and 371 MB to 13.8 G and 131 MB. d4aa849d18's first
point:

> A semi-join's base relations' statistics were measured off every document each run. They are kept and patched as
> every base relation's are; the statistics written beside their inputs' frames for that are gone.

**Ours.** Q396 (ec5501a7da, babad81e57, cf497c6219, 5e2a9418d7) counts a semi-join's statistics at the nodes the
exports hold (`keptstats::SemijoinCounts`, walked by document through `for_rows_keyed`), not every node's. That took
hogql's cold run from 99.7 to 84 B. Those statistics are kept beside the input's frame (`semijoin_stats`) and read
back while the input's digest is the same (`kept_input_stats`).

**What the merge did.** It kept ours, and so put back exactly what theirs removed:

- A semi-join's relation never takes kept statistics (`kept = if semijoin.contains(&rel) { None }`).
- It is not in the documents' table's statistics (`base_stats_out`).
- So the corpus batch has no count of its rows (`give_totals` leaves it out).

**Measured.** Theirs' warm tier (`--profile warm`, 16 tests, Linux, CPU) has one failure, the run after an edit:
`r7_a_run_after_an_edit_makes_no_document` builds 3 of 3 documents. `RERAIL_STATS_KEEP_TRACE` shows why: on the
third run every base relation's rows are counted by kept statistics except `docof`, the semi-join's ("not counted:
no kept statistics").

**The requirements.** Both stand:

- Ours: a cold run counts only the held rows, since the whole view is computed from the tape (1 s on hogql).
- Theirs: a warm run makes no unchanged document, and reads no row of one to get the relation's count.

**Merged shape.** A semi-join's statistics are base statistics like the others, kept per document in the documents'
table and patched by the changed documents. They are counted at the held nodes, not every row:

- `SemijoinCounts` counts into a `statkeep::RelStats` (per document, as `documents_scanned` keeps them) at the held
  nodes. A cold run pushes it to `base_stats_out` under the relation's tag, as `base_counted` does.
- A warm run patches it as `base_counted` patches the others: the changed documents' old held rows out, their new
  held rows in. The new held rows are the edited documents' export rows, which the run has.
- The batch's row space for the semi-join's relation is its view's rows, not the held rows. It comes from the
  documents' kept view counts (0494: `base_counts`, "3 of 3 documents' views' counts kept"), so `give_totals`
  covers it with no statistics.
- `semijoin_stats`, `kept_input_stats` and the frame's statistics for a semi-join go, as in theirs. ours' cold
  counting stays.

**Tests**:

- `r7_a_run_after_an_edit_makes_no_document`: it now names the reads that made documents in its message. It must
  pass.
- The whole warm tier: theirs' gate for Q332, which roll-up 38 never ran.
- A new test, `a_semijoins_statistics_are_kept_per_document_and_patched_by_an_edit`: over a program whose cross part
  is a semi-join, a cold run's statistics are the held rows' (as `SemijoinCounts` counts them), and a one-file
  edit's are equal to a cold run's over the edited corpus, with `made <= 1`.
- Q396's `syntax` and keeper tests, and theirs' `a_copy_of_any_inputs_columns_is_held_in_place`.

**Done** (e870c85994, and the statistics fix with item 3's commit).

- `corpus.rs`'s `held_counted` counts a semi-join's statistics at its held nodes, by document, into a `RelStats`. They
  are kept in the documents' table under their own name (`held_stats_name`), so the batch's row space stays the
  view's (0494). On both routes (layered and batch), statistics kept of the same documents stand, and no document is
  walked. R7 passes.
- An edit counts them again over the documents the run holds, and makes none. It does not patch them by the changed
  documents. This follows from what a semi-join is: a node is held when another document's export names it, so
  editing file A changes which of file B's rows are held. The held rows are not a per-document sum, and the changed
  documents' old and new rows cannot patch them. The R4 and session tests now name this.
- `SemijoinCounts`, `KeptInput::stats`, `kept_inputs`' `stats_of`, `write_kept`'s and `write_input_kept`'s
  `input_stats`, and `holds_stats` are removed, as in theirs. This leaves one mechanism.
- The new test is `keptstats::a_semijoins_held_rows_counted_by_document_are_their_measure`. It found a fault in
  `statkeep`: rows keyed by a document key (`doc32` 0) form one group across documents in `syntax::measured`, but
  `RelStats` summed their squares by document. So counting a document out took out less than it had put in (`kind`'s
  sq 98 against 50). Each column now keeps that group's counts by value (`Column::zero`, an `Exact`) and moves
  `zero_sq` by each square's change, so counting a document in or out is exact. The stored format is now 3; format 2
  had the error baked in.

## 3. Roll-up 38: `UNOWNED_ROOM`, a 64 MiB block for a relation made outside a run

**Theirs** (d4aa849d18): outside a run, `profile::local::<Open>()` is a fresh value each time. So every relation
made outside a run leased a block of a run's size, 16 GiB of address space. A session's 1,000 one-document batches
exhausted the address space (mmap refused at 256 TiB). Theirs gave such a relation a block of its own room:
`2 * need + MARGIN`.

**Ours** (0384, 0491): a relation's tables lie in its block. Its key map, pending rows and runs grow there, and the
block tests check it:

- `keys::a_relations_pending_rows_lie_in_its_block`;
- `keys::a_lattices_key_map_lies_in_its_relations_block`;
- `runs::a_relations_runs_lie_in_its_block`.

With theirs' room these failed. The tail outgrew its block, moved (`tail_moved`), and left its tables in the old one.

**What the merge did.** It made the room at least 64 MiB, so the tests' relations never move. That is a size chosen
to hide a structural fault, and it holds only while a relation stays under 64 MiB.

**The fault**, as both sides' evidence shows it, is that outside a run nothing owns the open block. Decision 0442
gives every piece of state an owner (the run, else the machine). The context crate says so: `local` is "outside a
run, a value of its own that nothing keeps", while `machine` is "outside a run: `outside_a_run`'s machine's".

**Merged shape.**

- `rows.rs`'s open blocks are the run's (`local::<Open>()`) inside a run, else the machine's (`machine::<Open>()`).
  A session's work between runs shares the machine's open blocks, as a run shares its own: 1,000 batches take one
  block's address space, not 1,000.
- The relation's tables stay in its block because the block is big, as in a run. The `unowned` branch and
  `UNOWNED_ROOM` go.
- A tail that must move still leaves its tables behind, inside a run too, when a run's block is full. That is a
  second fault: `Part::moved_room` must carry the key map and the pending rows, or refuse the move and copy them as
  a relation's index copy does (`DevIndex::moved`). Fix it with the first, and test it with a block made small.

**Tests**:

- The three block tests, with no size floor.
- Theirs' `tree_walk_file_edits_retain_the_walk...` session test (1,000 documents).
- A new test, `relations_made_outside_a_run_share_the_machines_block`: 1,000 small relations made outside a run take
  one block (`Live`'s count).
- A new test, `a_moved_tail_takes_its_tables`: a relation in a block sized to force `tail_moved` still finds its
  pending rows and key map in its new block.

**Done.**

- `rows.rs`'s `open_blocks()` returns the run's open blocks inside a run and the machine's outside one.
  `UNOWNED_ROOM`, the `unowned` branch and `lease_room`'s use for it are removed.
- The tables follow their rows. `RowData::lies_in` (and `Rows::home_is`, which counts no references) tells a table
  that its relation's rows have moved to another block. `RowSet::reserve` and `KeyMap::reserve_at` then place it
  again in the new block at its next placement, at its own size.
- I chose that over `Part::moved_room` carrying the tables. The tables belong to setfix's `Rel`, not to the facts
  layer's `Part`, and every move (`grow`, `moved_room`, a later run copying a kept relation) is caught where the
  table is next written. The device binds a key map by its own block (`device.rs`'s `LatMap { home: m.facts() }`),
  so a table not yet placed again is still read correctly.
- Tests:
  - `relations_made_outside_a_run_share_the_machines_block`: 1,000 relations in one block.
  - `a_moved_tail_takes_its_tables`: a run's rows copied into the next run's block take the pending rows' table and
    the key map with them, and every row and key is found there.
  - The three block tests pass with no size floor, as does theirs' 1,000-document `tree_walk_file_edits` test.
  - rerail-logic: 570 of 570. rerail-facts: 110 of 110. The warm tier: 16 of 16. Clippy is clean.

## 4. Roll-up 32: the grid's join, and the CPU's grid of one group

**Theirs** (decision 0540 §3, 7a6bf5498b): a persistent grid is the groups that are running, found on the device.
Each group takes a ticket, the first closes the count after a window (`UKL_GRID_WINDOW`), and the host no longer
guesses the resident count. On theirs' CPU backend groups ran one at a time, so its grid was one group, by design.

**Ours**:

- Roll-up 21 took Go-C's resident groups, so the CPU shim runs `resident` groups at once as coroutines. 0527 §3 adds
  32 threads a group.
- Ours' barrier never resets its arrivals, and its poll yields on the CPU.

**What the merge did.** The join yields nowhere, so on ours' CPU, whose groups take turns at a yield, the first
group's window ends before any other joins: the grid is one group. Only the library tests call `ukl_grid_join`
today. The preprocessor's grids (`pp_grid`, `pp_probes`) do not join: their size is still the host's guess, which
0540 §3 replaced.

**Merged shape.** Every persistent grid joins (0540 §3), on every backend:

- The CPU backend knows its resident count exactly: the launch carries `resident`. Its join closes when that many
  have joined (`last == min(dispatched, resident)`), with no window, and may then yield as ours' barrier does.
- A GPU keeps theirs' window.
- `pp_grid` and `pp_probes` take the join's fourth control word and drop the host's guess.

**Tests**:

- `a_grid_past_residency_is_the_groups_that_run` on the CPU backend, with yields in the join.
- A new test, `the_cpus_grid_is_its_resident_groups`: a dispatch of more groups than `resident` has `resident`
  groups in its grid, not one.
- The pp grid tests on the CPU (`RERAIL_GPU=cpu`), small cases only (the user's order of 2026-11-20). On Metal, the
  mini first and one test at a time, when Metal is allowed again.

**Not a duplication**: the work queue's ring (ours, 0486 §10) and its budget (theirs, 0540 §4) are different jobs,
a queue's storage reused and a worker's share of a launch. Both stay.

**Done.**

- `grid.h`: where a backend knows its resident count (`UKL_GRID_RESIDENT`, the CPU's `__ukl_cpu_resident()`, the
  launch's `resident` clamped as `__ukl_cpu_run` clamps it), the first group closes the count once that many have
  joined, with no window. Its polls yield, bounded by `UKL_GRID_SPINS`. The groups waiting for the count to close
  yield too, so on the CPU the first group runs again to close it. A GPU keeps theirs' window.
- `Device::resident_groups` gives the CPU's exact count (`compute_kernel_cpu::resident_groups`). It had given none,
  so the library tests never ran a CPU grid at its residency.
- `pp_setup` (`PP_GRID_BEGIN`) and the probes (`pb_sort_alone`, `pb_barriers_alone`, `pb_pool`) join their grids. The
  grid's stage words move past the join's ticket word (`PG_STAGE` 4, `PG_STOP` 5). The pool probe's takers are the
  dispatch's threads, taken in turn by the grid's. The probes' control words are four.
- The host's count (`pp_arena::groups`) stays as the count the grid is dispatched at, not the grid. A dispatch at the
  resident count closes the join when its last group joins, so it waits out no window; past it, the join finds the
  groups that run. `fewer`'s halving is left for a barrier whose bound is reached, which residency no longer causes.
  `Expanded::joined` reports the groups that joined (`Front::joined`, the grid's word 3).
- `an_evaluation_is_a_fixed_handful_of_launches` held 0468's rule, that a grid of 4,096 groups fails and runs again at
  half. It now holds 0540's: it runs once at 4,096 dispatched, fewer join, and the output is the same.
- Tests (CPU backend: `RERAIL_GPU=cpu` on Linux, and on the M3 with no Metal):
  - `the_cpus_grid_is_its_resident_groups`: 128 groups dispatched, 32 joined.
  - rerail-gpu: 26 of 26.
  - rerail-metal's pp tests: 9 of 9.
  - rerail-c-directives' evaluation tier (`--profile second`, 1 s each): 31 of 31.
  - Clippy is clean on rerail-gpu, rerail-device, rerail-metal and rerail-ukl.
- The tier's first run on the M3 timed out 27 tests at 1 s. That is the CPU kernels' first compile into their cache:
  one test took 8.7 s cold and 0.28 s warm, with the same instructions retired. That is the first-launch cost, not a
  slow test.

## 5. Roll-up 22: a file's record lanes one threadgroup

Listed as a possible cap on parallelism. It is not one. `RECORD_LANES` was 64, equal to `THREADS`, before the merge
(f56a1df498); the merge added a const assert that says so, and a threadgroup barrier before the refusal word is
read. A file's lanes were already one group. No fix.

The coverage gate's `replace/3` failure on the CPU device (Q445) is in ours' `lit_replace_txt`, not in theirs'
dropped text kernels: it stays with the deferred CPU failures.

## 6. Roll-up 35: main's decompositions and main-thread record not ported

**Main.**

- #915 (54e31a8fbb) split the functions over main's complexity ceiling that its scrub touched:
  - `block_program.rs`: `check_supported`, `fit`, `fitted`, `lanes_and_states`, `sinks_and_tickets`, `dispatch`,
    `executor_pipeline`, `interpreter_blocks`, `program_row`, `push_steps`, `push_outputs`, `bindings`,
    `launch_until_done`, `read_back`;
  - `runtime.rs`: `reserve`, `trace_large`, `count_driver_refusal`;
  - `metal_program_backend.rs`: `pack_batch`, `upload_batch`, `dispatch_batch`, `collect_results` and nine more.
- #929 (756ebc4ce1) records the main thread's `ThreadId` when the pool takes it in, so a run's main wait knows it on
  every platform.

**Ours.** These files were rewritten for UKL (0440) and 0442's machine-held state (`DeviceState`, the reservations
of Q203). Ours' `is_main_thread` is `pthread_main_np` or `gettid == getpid`, and false elsewhere. On Windows,
`in_a_job` therefore takes the main thread for a job, and its wait runs the pool's jobs instead of sleeping on its
bell.

**What the merge did.** It kept ours whole "while Metal is off". That reason does not hold: the decomposition is a
change of structure, not of what runs on the device. Every later merge of main conflicts in these files again. That
is the divergence the user asks to keep small.

**Merged shape.**

- Each function main split is split the same way in ours, under main's names, wherever ours still has that
  function. Ours' bodies stay ours: UKL launches and `DeviceState`. A function ours removed (the `MappedBuffer`
  paths, `untouched_*`) stays gone and is named in the record.
- The main thread is the machine's: `Machine::here` records its `ThreadId` in the machine (0442, no global), and
  `in_a_job` compares with it. That is main's behaviour on Windows, with ours' owner.

**Tests**:

- The metal crate's tests on the CPU backend and the packed backend's tests (no change of behaviour expected; both
  compile on Linux).
- `rr::tests::a_run_on_two_threads_ends`.
- A new test, `the_machines_main_thread_sleeps_on_its_bell`: on the machine's main thread `in_a_job` is false on
  every platform. Run it on a Windows host too.
- The next main merge's conflicts in these files are the measure: there should be none past ours' bodies.

**Done** (the main thread: e7dc15faf5; the decompositions: this item's commit).

- The main thread. `rerail_context::MainThread`, the machine's `OnceLock<ThreadId>`, is set in `Machine::here`.
  `is_main_thread` is true on the run's machine's main thread on every platform. libc's `pthread_main_np` and
  `gettid` are gone, and so is the crate's libc dependency. `the_machines_main_thread_sleeps_on_its_bell` passed on
  Linux and on win1, and rerail-accel passed 88 of 88.
- `block_program.rs`. `run_batch` is main's sequence: `check_supported`, `BatchForms::new`, the slabs, then
  `FileTable::new` and `dispatch`. `dispatch` calls `BatchBlocks::new` (`lanes_and_states`, `sinks_and_tickets`),
  `executor_pipeline` (`interpreter_blocks`, `InterpreterTables::new`, `program_row`, `push_steps`,
  `push_outputs`), `bindings`, `launch_until_done` and `read_back`. `fit` and `FileRow::fitted` are main's too.
  Ours' bodies are kept:
  - `check_supported` is `missing_form` and `ready()`, not `METAL_DEVICE`;
  - the kernel is UKL, `lane::kernel` / `lane::kernel_fixed`;
  - `bindings` binds by name (`Pass::buffers`), not by index;
  - the repeated-launch count is the run's (`with_local::<RepeatedLaunches>`).

  `BatchBlocks` declares its blocks in reverse lease order, so they go back to the pool in the order the old locals
  did. Main's `bytes_of`, `command_error`, `objc_msg`, `sel_register` and `batch_helpers` are Metal's Objective-C
  calls, which UKL replaced (0440). They stay gone.
- `runtime.rs`. `BufferReservation::acquire` calls `reserve` and `trace_large`, as main's does. `reserve` keeps
  ours' one account (Q203): the bytes bound to admissions (`bound_bytes`), and a draw from this thread's admission
  (`draw_reserved`). Main's counts the leases alone.

  `count_driver_refusal` is main's profile of a driver refusal: `capacity.metal_pool_driver`, the driver's,
  pool's and accelerator's MB at the refusal. Ours had lost it. It is back at ours' one refusal site,
  `HostMapped::over`. Main's two call sites were `MappedBuffer`'s, which ours removed.

  Main's other names here are `MappedBuffer`, the private uploads, Metal pipelines and `metal_execution_guard`.
  Ours replaced them with `HostMapped`, the pool's blocks, the kernel store (0395) and `evaluation_scope`, and they
  stay gone.
- `metal_program_backend.rs`. Ours' body is in `execute_packed_once`, run by 0334's `restarting`. It is split under
  main's names:
  - `pack_batch`, `print_batch_error`, `assign_batch_workspace`, `lease_workspace`, `execute_on_device`;
  - `upload_batch`, `resident_doc_table`, `upload_documents`, `copy_scheduler_state`, `print_workspace_profile`;
  - `dispatch_batch`, `collect_results`, and `MetalWorkspace::count_buffer_sizes`.

  Ours' differences are kept:
  - `assign_batch_workspace` takes the lanes;
  - `lease_workspace` holds 0334 §9's halving of the lanes on a refused workspace, which main does not have;
  - `resident_doc_table` is `DocTable::new` (the slabs and the 16z cache), not main's `one_block`;
  - `dispatch_batch` is a `lane::Command` that binds by name and uses the other slabs and the cache.

  Main's `device_and_queue`, `pipeline` and `untouched_path_buffers` / `untouched_result_buffers` /
  `untouched_results` have no counterpart: ours has no queue (`lane::device`) and zeroes `HostMapped` blocks.
- Fixed on the way: under test builds, `DeviceState`'s `pool` and `peak` were dead on macOS, because a test's
  ledgers are `test_pool`'s. They now carry their readers' cfg. The relocation tests' `FAR_*` constants are now
  `#[cfg(test)]`, as their only reader is.
- Tests (the M3, `RERAIL_GPU=cpu RERAIL_DEVICE=cpu`, no Metal):
  - clippy over rerail-metal, rerail-engine and rerail-compiler, all targets, is clean;
  - rerail-metal: 25 of 25;
  - the engine's packed and block-program tests: 182 of 182;
  - `extract_equals_reference_on_every_executor` and `lookback_fallback_equals_reference` (`run_batch`, both
    executors, against the reference): 2 of 2.
- `a_run_on_two_threads_ends` takes 20 s, against about 1 s on main. The time is the library's cold phase 0 under
  `RERAIL_TAPE_CACHE=off`: 2.45 s with a warm store. Starting from the template copy still took 16 s, so that is
  not the cause.

## 7. Roll-up 34: the radix sort's rank groups and virtual threads

These are two axes, not two mechanisms:

- Ours' rank groups (255eebdc63) handle a SIMD group narrower than 32: digits read by 32 threads where the hardware
  group is narrower (Intel's 8 and 16).
- Theirs' virtual threads (0527 §3) handle a CPU group of fewer threads than the tile, each thread a coroutine:
  `UKL_RADIX_VT_MAX`, 8 on the CPU.

The `#error` refuses virtual threads without matched SIMD groups. No backend builds that pairing: virtual threads
are on the CPU only, whose SIMD groups are matched. No fix.

## 8. Roll-up 21: the CPU launch's fields

`__ukl_cpu_launch` has ours':

- `sched` and `sched_bytes`: the scheduler's storage, the host thread's;
- `tls_key`: the host's one pthread key, since roll-up 34, no thread-local in a library (0440 step C).

It also has theirs' `resident` (groups run at once) and `pad`. Each field has one job, and the host's `Launch`
matches it field for field. No fix.

## 9. Roll-up 25: cost-classifier's `demand_` lead removed

Theirs' `demand_first` forced three things to lead its rule:

- `demand_x`;
- a pair cache's small side, by `shape_base(head)`;
- a deferred join's request for a maintenance support head.

Ours:

- Q59 (`demand_bounds_lead`) measured a forced `demand_x` reading 171 k rows of `lambda_arg` where a constant's lead
  read 229. Ours keeps a demand as a bound, not a forced lead (`own_demand`).
- Ours removed the forms' suffixed relations (0364 R1, 74d820a807), so `shape_base` has nothing to strip.

The merge's choice follows both. No fix.

Left open on both sides: these roles are found by relation-name prefixes (`rr_demand_agg_`, `rr_pair_cache_`,
`demand_`). The rewrite that makes a demand relation should mark it on the version, and the planner should read
the mark. That is not this merge's to fix, and it needs a decision of its own.

## 10. Roll-up 38: two ways to make a hit's document; the identity file

**Theirs** (c07c8ab5c7, Q332): a memo hit's document is made when a rule first reads it (`syntax::Input::Pending`),
through `hit_document`.

**Ours** (0512, 695f7aa21f): a file read for its programs is shown as a file, its tree a guest of it
(`Made::File`). Its programs are made at its parse and kept with its tape (0431 step 6).

**What the merge did.** Host files go through ours' `doc_of`, made at once; every other hit goes through theirs'
`hit_document`, pending. That gives one document two paths. The split is there because `guests_of` reads a host's
guests when the batch is built.

**Merged shape.** One path:

- `hit_document` makes a host file's document pending too, its guest list taken from the memo entry. The entry
  keeps its requests (1ba2536e5e), and the guests need no rows.
- `guests_of` reads that list, so the host is pending like any other document.
- `doc_of`'s host branch goes.

**Tests**:

- 0512's tests (a file shown as a file, its tree a guest).
- The partial-hit tests.
- `a_warm_edit_makes_the_changed_documents_rows_alone` over a corpus with a `.rerail` host file.

**Done.** The tests showed the two paths had made different documents, not only made them at different times. Three
faults, all fixed:

1. **The cold corpus batch held host files unsplit.** A chunk split a host file into text and tree for its own
   projection. It handed the corpus batch `kept_docs`, a clone taken before that split, plus the programs, whose host
   is the tree's key. So the batch held each file under its own key with the tree's nodes, and no host for its
   programs. The split now happens once, where `Builder::take` files a document into its chunk (`Builder::split`). The
   chunk, its projection and the corpus batch hold the same text and tree, and the projection's second split is gone.
   Visible in the rows: a fresh run reported a script's call at the script's line (`foo` at 1), not the file's (3).
   0512 says the file's.
2. **A warm run made no host file's programs.** Theirs' `hit_document` and ours' `doc_of` both made the text and
   tree. Nothing made the programs, so a warm run's rows lost every call in a host file. This failed on HEAD too.
   `hit_document` now makes a host file's whole set: the text, the tree, and the tree's programs (`Guests::programs`,
   seeing what the chunk sees). It is the one maker for the eager hits, the pending hits and an edit's old documents.
   `doc_of` is one call of it, and its host branch is gone.
3. **A host file was never pending.** It is now pending as its text, its tree and its programs, each its own
   `syntax::Pending`, all made by one shared make on the first read of any. The batch knows each guest's host, file
   and kind before any is made. `GuestOf`, what the batch reads of a guest, is a made one's `Guest` or a pending one's
   `Hosted` (`Pending::with_guest`). Its place is read only from the made document.

   Those three come from the documents' table, not the memo entry, which this item's plan named. The table already
   keeps every document's digest and size by key. Its row now keeps, for a host file's guest, the host, the file,
   whether it is the tree, and its tape tag (`retain::KeptGuest`, the content word 3). A table without them (an older
   store) leaves the host file eager.

Fixed on the way: `Batch::ensure` found the documents a read needs under one lock and claimed them under a second. Two
reads side by side could both find a document unclaimed, and the second made it again and set its group aside.
`reads_side_by_side_make_each_document_once` failed 13 times in 30. It now claims under the lock it found them
under, and passes 30 of 30.

Tests (spark2, `RERAIL_GPU=cpu`):

- New: `a_warm_run_builds_no_unchanged_host_files_documents`. A JavaScript run over three HTML hosts and a script: the
  warm run builds and parses none, and its rows are a fresh run's, at the files' lines.
- New: `a_warm_edit_of_a_host_file_reads_as_a_cold_run`. The edit's rows are a fresh run's, the edit's call at its
  file's line.
- The warm tier: 18 of 18.
- 0512's, the guests' and the partial hits' tests: 50 of 50.
- `plan::corpus`, `vtab::syntax` and `rr::tests`: 422 of 422.
- Clippy over the engine is clean.

Found, not fixed: the warm edit still makes the unchanged host files when it plans, for the statistics of `kind`, a
view of `node` held in place (`InPlaceView`). `node`'s statistics are patched by the edit, 74 rows to 78. The kind
totals (`Own`, from the hits' kept rows and the 0096 keeper's arithmetic) still sum to 74. `view_stats_held` needs them
to agree, so it counts the view off every document. The Python warm edit tests do not hit this.

**The identity file** (`repository-identity.ts`) is theirs' whole now (0497: "a bare repository is a folder"). Ours
had no decision against it, and theirs' tests are the catalog's. Its one call of `runGit(..., { output: "bytes" })`
is ours' `runGitBytes`. On the M3, after it, `outside-git.test.ts` has one failure left, "a plain folder while
GIT_DIR names a re…". That and the catalog's other failures are open with roll-up 38; most report a missing native
addon although the addon loads.

## 11. Roll-up 37: Cranelift's return area on every host

rr-align (9e651c48bc) set `enable_multi_ret_implicit_sret` because System V's two return registers refused rr's
tier 2 on x86-64. Ours sets it in the one place every JIT module's flags come from (`rerail_clif::jit::flags`).
Cranelift reads the flag only where results spill past the registers: `isa/aarch64/abi.rs`'s check is in the spill
branch, and refuses otherwise. So it changes no function that compiled before, on any host, which is theirs' own
measurement too ("aarch64 Linux is unchanged"). No fix.

## 12. Roll-up 26: rr-align's deferred joins over ours' roll-up 24 port

Ours' port (roll-up 24) made the deferred joins a keyed step of `preparation::front`. rr-align's makes them a
`ProgramPass` (`DeferJoins`), one of 0281's preparation passes, as every other rewrite of a whole program is ("the
preparation pass (decision 0281): …" in every run's decisions). Theirs is the architecture's form; ours was a step
beside it. Taking theirs was right. No fix.

The CPU device failures found at roll-up 26 (the C directives' device reads, `a_files_start_on_metal_is_the_interpreters`,
`replace/3`) are with theirs' kernels unchanged. They stay open as the user deferred them.

## What the roll-ups' records lacked

- Each record named the side kept, not the requirement each side met. Items 1, 5, 7, 8, 9, 11 and 12 were sound but
  read as arbitrary.
- Items 2, 3, 6 and 10 were decided by what compiled or passed:
  - in 2, a test of ours (the semi-join statistics) and none of theirs: the warm tier was never run;
  - in 3, a size that makes a test pass;
  - in 6, a reason that does not apply;
  - in 10, the shortest join of two paths.
- Every roll-up from now runs both sides' gates over the merged tree before it is committed: the warm tier, the
  maintain tier, and the tests each side's commits added.
