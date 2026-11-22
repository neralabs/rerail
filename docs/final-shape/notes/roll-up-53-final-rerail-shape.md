# Roll-up 53: final-rerail-shape (39de0fce60) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at cd8b87d061 (roll-up 52, Q473). Theirs: final-rerail-shape at 39de0fce60. The merge
base is 10669c8c95. Theirs brings three commits:

- the kept writer reads rows where they lie, and a structure's statistics are kept (49f7195169);
- no kept put holds its rows in a heap vector (ec4b749e40): `KeptWhat::Flat` becomes `Picked`, a block of the put's
  own or rows lent where they lie;
- a width census of the run's columns (`RERAIL_WIDTH_CENSUS`).

## Items

1. **A source's structure statistics: one mechanism, ours'.** Both sides keep them for the next run (Q332, R7).
   - Ours' `RelStats::of_statistics` gives them as a kept relation's, which a changed document's rows patch.
   - Theirs' `of_structure` holds the structure's exact sums, unpatchable (`patchable`, stored as format 4), and
     answers an edit from the structure again. With the documents pending, that builds every one: the warm edit's
     cost Q332 removes.
   So ours' form stays, and theirs' `shape`, `of_structure`, `patchable` and format 4 go. The probe paths'
   `k.patchable()` tests go with them, since every kept statistic is patchable.
2. **Theirs' picked puts on ours' new puts.** The puts ours added since the base are written as blocks
   (`BlockFlat`), as theirs' rule requires: the reader record (decision 0401 step 8), the observer's facts (t33), the
   slice's kept rows, a prepared program's facts and the Neural Engine's models. `put_idents` reads a record through
   `KeptPicked::fill`. `stages` is both sides' test: no reader record, and rows or picked rows.
3. **`source_rows` is the planner's.** Theirs' `retain::digest`, `each_row` and `SourceRows` are read by `plan.rs`
   and `compose::retained`, which are rerail-planner's, under the corpus. They move to `plan/source_rows.rs`, and
   `retain` imports them.
4. **What ours' split moved.** Theirs' `kept_source` re-export is ported to rerail-setfix's facade, and the view
   test's `Picked` check to `setfix/retain_tests.rs`. `retain`'s guest rows are written through theirs' writer.
   `memory.rs` keeps both sides' additions (ours' ceiling and readings, theirs' census).
5. **Found:** theirs' `kept_strata` test helper still passed six arguments to the five-argument `flat_put`, so it
   did not compile at theirs' tip either. It now gives a block of one row.

## Audit

`scripts/check/merge_audit.py`'s lines are items 1-4: theirs' `shape` lines gone, ours' `Flat` puts rewritten, the
moved items.

## Checks

On spark2, CPU, and on one platform only:
- `cargo check --workspace --tests` is clean, and `check_crate_dag.py` passes;
- rerail-store `kept_facts` 12, rerail-setfix `statkeep` 5 and rerail-tests-logic `warm_tests` 7 all pass.
