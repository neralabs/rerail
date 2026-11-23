# Roll-up 61: sync-integration-hub-tape (88bf7a54e1) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at bcf6da6634. Theirs: sync-integration-hub-tape at 88bf7a54e1. The merge base is
3c058416f0. Theirs brings five commits of its own and no merges:
- decision 0612: a file's lines have one definition, its language's (`Terminators::of`), and every place is read off
  the `LineTable`. `src_line_lf`, `tape_line` (the relation), `split_line` and `lf_line` are gone. The `Loc` views read
  the table's words in their slab (`lines_word`), and `ViewDoc` has no copy of the lines. The data-layer pack's
  overlays and the lint tool-pack's publish files follow;
- a maintain mode's line starts where it is cheapest (`Stats::lead_free`, a delta's share of its relation's probe);
- a relation measured by rows alone prices its probes' misses (its rows bound its distinct values);
- the docof removal join's test, and 0563's text.

## Items

1. **The planner is split in ours (crate split 5j).** Theirs' three commits on `logic/plan.rs` were applied to ours'
   `plan.rs` in order. The hunks it rejected were applied to `plan/search.rs`, where ours keeps `Stats`, the line
   search and the probe pricing. All of them applied:
   - `lead_free` and its three initializers;
   - the source orders and the prepared plans kept off a lead-free line;
   - the delta's share;
   - the measured rows bounding a column's distinct values;
   - 0612's `src_line` arity.
   The maintain pricing stayed in `plan.rs`. Ours keeps `past_floor` in `plan/again.rs`, so theirs' path
   (`corpus::maintain::past_floor`) follows it.
2. **The generated reads were written again from the merged CLIF** (`view_reads.rs`, `reads.rs`): `body/view_reads.h`
   and `reads/view.h` changed, and `ids.h` and `relation.h` did not.
   - Theirs' `view.h` was its older unfactored layout. Ours' writer also makes `body/view_reads.h`, and both now
     carry 0612's `clif_lines_word` and drop `clif_loc_line` and `ViewDoc.lines`.
   - `tmp/runs/write-reads.sh` now prints every header that names `reads.rs` as its generator. It had printed only
     `view.h` and `ids.h`, so the first copy left the bodies stale and the kernels' setup failed.
3. **`device_chunk.rs`**:
   - Ours reads the lines where they lie through `home()`, on the host too, and refuses them only for a device that
     cannot reach them.
   - Theirs' guest places (`Locs::place_lines`) choose the table.
   - Theirs' rule that the `Loc` views, which now read the lines' words, are refused with the lines is ours'
     condition on both.
4. **`device_metal/Cargo.toml` is ours.** Ours already depended on rerail-clif for the store's text cache. Theirs'
   copy of the same dependency landed beside it and made a duplicate key.
5. **`base_tests.rs`**: theirs' outputs, one fewer with the `tape` relation gone, with ours' scrubbed file name
   (`docs/graph_query.rst`, 54e31a8fbb).
6. **The scrub's rule on theirs' new files**: one pack file cited a branch with its owner's prefix. It names the branch
   alone now.
7. **The decisions index**: 0612's row after ours'.
8. Theirs' remaining uses of `tape_line` are the CLIF tape read, not the relation. Ours has more tests of it
   (`tape_format/src/f16.rs`).

## Audit

`scripts/check/merge_audit.py` reports 163 findings:
- 151 on ours' side: the rewritten generated bodies (138), and theirs' hunks over ours' lines in `search.rs` (item 1,
  9). The rest are `device_chunk.rs` (item 3) and `base_tests.rs` (item 5).
- 12 on theirs' side: items 1 (the `past_floor` path), 2 (theirs' unfactored `view.h`), 3, 4, 5 and 6.

The landed-lines check finds each of theirs' added lines as merged, or accounted for above. Its `plan.rs` lines are
in `search.rs`. The lost-lines check finds no line of ours dropped but items 3 and 5's.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean.
- Theirs' new tests pass (15), with rerail-clif's `reads_are_these`:
  - rerail-lang's `lines` tests;
  - rerail-tests-logic's `a_maintain_mode_line_starts_where_it_is_cheapest`,
    `a_probe_of_a_relation_measured_by_rows_alone_prices_its_misses`, `a_finding_s_places_moved_are_maintained`,
    `base_tests`' three line tests and `a_files_lines_are_its_languages`.
- Over the written reads, the kernels' setup builds. rerail-device-metal's `views_made_on_the_device_are_the_hosts_rows`
  and `a_views_index_is_in_its_block_for_every_thread` pass on the CPU device.
