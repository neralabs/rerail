# Roll-up 62: benchmarks-on-integration (b5fe5fcaa6) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at dddf53350b. Theirs: benchmarks-on-integration at b5fe5fcaa6. The merge base is
ae63210afe. Theirs brings six commits of its own and no merges (decision 0353 step 3, the measure's rows, the batch's
relations block):
- core.rerail's places (`line`, `end_line`, `col`, `docof`) are the `Loc` view's for every node, token and end point.
  Only spans are rows (`places::Places` binds the view and those rows as one relation). `rr_long_text_node`'s
  statistics are counted on the tape (`LongTexts::statistics`);
- pair statistics and the export's set in blocks (`facts::rows`). The heap sampler names a thread by its system name;
- a within axis's looked-up rows and the node view's kind index in blocks;
- a recursive component's cached sort is found again each round;
- a stage run again after an overflow finds its indexes again (`Batch::again`): a given-back one read no rows.

## Items

1. **The place rows**: ours (0612, roll-up 61) places every node, token and end point through one closure
   (`LineTable::place`, a guest's in its host). Theirs' gate (`node_places`: not in core.rerail's shape, where the
   `Loc` view holds them) is on that closure's three switches, and on `docof`. Ours' node-text loop stays its own.
2. **rerail-device-metal's test binary has one load hook**: ours', in `code_rule_dsl/device_metal.rs`.
   - Theirs added a second hook in the crate's `lib.rs`. It registered the run's carriers again, so the binary
     panicked as it loaded ("registered twice").
   - What theirs' hook added is ours' now: strict mode on every connection, the core names, and Windows' load
     section.
   - Theirs' global-state row for its `LOAD` goes with it.
3. **Theirs' `an_index_built_twice_is_found_again_where_it_is_kept`** opens an evaluation
   (`LaunchScope::enter`), as ours' device tests do. Under 0601, a submission outside one is the hard failure, and the
   test met it.
4. **The rest merged clean**: `facts/rows.rs`, `places.rs`, `within.rs`, `setfix`, `exports.rs`, the profiler. The
   one hunk in `plan.rs` reads ours' histogram's `get`. The sampler's `SYSTEM_NAMES` row is in the global-state list.

## Audit

`scripts/check/merge_audit.py` reports 11 findings:
- 2 on ours' side: item 1's switches and item 2's doc line.
- 9 on theirs' side: item 1's `rows_*` form, and item 2's removed hook and its row.

The landed-lines and lost-lines checks find only item 1's lines. No file changed only through merges.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean, and the global-state and thread-state checks pass.
- 48 tests pass:
  - theirs' new ones: the long texts on the tape, the system thread name, the id set, the histogram, the memory
    profile, and the index built twice;
  - tests-lang's and tests-logic's place, end-line, lines and `Loc` tests;
  - rerail-device-metal's `views_made_on_the_device_are_the_hosts_rows`.
