# 0660. Metal at 512 MB gives the CPU's rows: chunks sharing the device each read what they wrote

Status: built (2026-11-23). t50 step 9. Numbered past the highest record in any worktree (0659).

## Intro

t50 step 9's test is the `all` run on Metal at the default budget, 512 MB, with every relation's rows the CPU's. It
failed in 17 of 18 runs, a different set of relations short each time. At 4 GB it was right, and so it was with one
chunk on the device at a time. t50 ("Step 9, narrowed") ruled out the residency, the wraps, the GPU overlapping two
command buffers, the screens and the freed chunks.

There was no single cause. The run was short through ten separate defects. Most of them are in the hand-off between
batches: chunks that share the device, a stratum's reruns, a head's placed lots. Each is fixed where its data lives,
and none by a copy (0353). The run is now right in every run: ten of ten at 512 MB on an M3 (`finding` 20,046,
`mn_class` 266,996, `dbg_rd` 480,367, `mn_dec_fn` 116,511, every export equal to the CPU run's). Its peak is about
3.5 GB, its wall about 33 s, and it exits 0.

## What a probe reads is what its builder wrote

1. **An index is found again by each batch that encodes a stage** (`Batch::index_again`). A stage that overflowed is
   encoded again in a later batch (`complete`'s rerun). It read its index through the first batch's slot, which that
   batch's finish may have given back as a duplicate (another batch built the same index meanwhile). The slot is
   then dead and its buckets gone. `dm_rf_param_site_1` found 0 rows of the CPU's 7,683.
2. **A taker binds the buffer its builder wrote through** (`InFlight::buffer`, `as_built`). A block that grows gets
   a new wrap (`Block::device_buffer`), which is another `MTLBuffer` over the same memory. Metal orders one command
   buffer's reads after another's writes only through a buffer both bind.
3. **An index whose view reads found no room in the 16z cache is not marked built** (`Building::misread`). Its
   buckets hold the dead entry's keys. A batch that took such an index fails as one whose builder the device failed
   does (`InFlight::misread`). `param`'s probe of a `child` view lost half a chunk's rows, every run.
4. **A batch that lost a submission fails** (`Batch::lost`). A command buffer whose launches failed to encode (a
   lease past the budget) is dropped uncommitted, with every stage encoded in it. Before, only the stage that met
   the error was refused, and the others finished with counts of work never run.
5. **A view is read in place only with its home** (`device_view`). With no home (the budget refused it), a reader
   made its rows from a chunk whose places were never filled. `docof` gave 326,776 rows of one document of 100.

## A head's rows, and a refused seed's

6. **A head's new rows exclude every lot placed for it** (`place`, `Fresh::lots`), not only the rows it holds. A
   stratum whose rules did not fit one submission places a lot per group, and the store takes each lot as it lies.
   `xd_raw_parameter_annotation` held about twice its rows in 13 of 36 chunks.
7. **A refused seed's aggregates and string joins get the CPU's index** (`JoinRule::indexed`), as its atoms do. Left
   with the device's unindexed one, `rf_dec_code`'s `min` found nothing in every chunk whose device run was refused.

## A document is found by its key through the directory's key order

8. **`ViewDir::by_key`.** The views' directory is in row order: the chunk's batch's documents, then each admitted
   value batch's (0431 §8). Each batch is in key order, but they are not in key order together. The kernels'
   lookups by key (`by_parent`, `by_has`, now `doc_of_key` in `join_stages.uglykl`) bisected the rows. A value
   whose key sorts below the chunk's documents was not found, and its parents had no children (`tp_ident` and
   `tp_list`, 9 rows each, at every budget). The directory now holds its documents' order by key, written once in
   its block where it is built (an index, 0353), and both lookups bisect it. The layout is `ViewDir`'s, in
   `view_reads.rs`, which writes `<ukl/reads/view.h>`.

## The run's end: a search's walk lists no candidates

9. **A probe's candidates are counted and drawn by position where the source holds them** (`Source::lookup_seq`,
   `View::find_seq`). Some runs spent 195 s after the run, others 36 s at an 18 GB peak, others 15 s. In each case
   the end-of-run drain of the join searches (0243, budgeted at 5% of the process's CPU) was in one walk step,
   past its budget:
   - `probe_size` read every candidate of a source's lookup to count them (0247 says it reads none);
   - `probe_sample` read a source's lookup whole and kept each match;
   - each listed the candidates, a file's every node or a kind's every node, to take their length.

   `lookup_seq` gives a lookup's rows by position: a view's document as one run of rows, a kind's nodes in the
   batch's own list (`by_kind`), a layered source's parts after its base's. By default, `lookup`'s rows. The
   decision is one mechanism: `probe_size` is the sequence's length, and `probe_sample` draws from it as from an
   index.

## The exit

10. **A count in a thread's teardown is dropped, not read** (`score::phases`, `exec::command_committed`). A batch
    held in a thread-local (an open wave) waits for its work when it is dropped in the main thread's teardown. The
    wait counts a phase in `score`'s thread-local, which may already be gone. That aborted one exit in four
    ("cannot access a Thread Local Storage value during or after destruction").

## Tests

- `a_childs_parent_walk_finds_the_values_admitted_below_the_chunks_keys` (device): `child` probed by parent over a
  chunk (keys 20, 23) and an admitted value (key 5), device rows against the CPU's. It fails with the row-order
  bisection.
- `a_lookups_rows_by_position_are_its_rows`: every source's `lookup_seq` against its `lookup`, bare and layered
  with a value admitted.
- `a_phase_counted_in_a_threads_teardown_is_dropped`: aborts (SIGABRT) with the old read.
- `a_refused_seeds_aggregate_gets_its_index`.
- The device crate's test binary registers what the engine's boot does (`device_metal::test_load`: carriers, core
  names, strict mode, languages). Without the carriers, `a_strata_seeds_on_the_device_are_the_cpus_rows_and_counts`
  panicked under nextest ("was the program booted?").
