# Roll-up 69: final-rerail-shape (9a082a2457) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 75f61d7d5f. Theirs: final-rerail-shape at 9a082a2457. Git's base is virtual: the
merge bases are 39de0fce60 (theirs' pin in roll-up 53) and 6c2cce0fce (cost-classifier-metal-rf, which theirs
merged in 040084fdc7 and roll-up 68 brought into ours). Theirs brings six commits of its own:
- the width census (`RERAIL_WIDTH_CENSUS`): each range encoded, each relation's values counted by name, and the span
  each block was made in;
- t52's notes: the 10 GB of columns, what the sibling branches have, why each filter twin is made;
- 0165 §3, ported from 3d8f990a26: a local original read bounded is read in place, not copied;
- t52 step 4: an export reads its chunks' rows where they lie.

## Items

1. **Theirs' own merge is read as a merge.** Its inputs are in ours: 39de0fce60 by roll-up 53, 6c2cce0fce by
   roll-up 68. Its own lines (`/tmp/obs/evil.pl`) join 49f7195169's kept writer to cost-classifier's crate split:
   - a source's digest, its rows and `SourceRows` in `plan.rs`;
   - `kept_facts`' `flat`;
   - `of_structure` and `patchable` in statkeep.
   Ours has that mechanism in its own form: `plan/source_rows.rs`, the kept writer's `Picked` rows read where they
   lie, and ours' statistics, which roll-up 53 chose over `of_structure` (Q474, item 1). So theirs' merge text stays
   out, and every file theirs changed only through merges takes ours.
2. **The files of theirs' own commits are ours with those commits applied in order** (`/tmp/obs/byinvariant.sh`).
   Two hunks were rejected and placed by hand:
   - `facts/rows.rs`: the census's `made_in` (the span a part was made in) is a field of ours' `Held`, after its
     `ix_room` (0353 step 6). It is set in both constructors, the new part and the continuation. Ours' `Drop` and
     `Part::made_in` already read it.
   - `combining.rs`: the in-place reading (`Narrowing::unbounded`) joins ours' narrowing loop, which also passes the
     filter's `Shape`.
3. **The census's three new statics are in the global-state list** (`RANGED`, `BY_NAME`, `BY_SPAN`). They have the
   reason of ours' rows for the same census, after `SPLIT`. Theirs' list has none of the census's rows.

## Audit

`scripts/check/merge_audit.py` reports 48 findings:
- 45 on theirs' side: item 1's merge text in `statkeep.rs`, `kept_facts.rs`, `plan.rs`, `retain.rs`, `corpus.rs`,
  `compose/retained.rs` and `kept_strata/tests.rs`, and `combining.rs`' line without ours' `Shape`.
- 3 on ours' side: item 2's `combining.rs` loop.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean. The crate DAG, global-state and thread-state checks pass.
- 13 tests pass in rerail-corpus-prep, rerail-logic and rerail-setfix: theirs' two new ones
  (`an_export_reads_the_chunks_rows_where_they_lie`, `a_local_original_read_bounded_is_read_in_place_not_copied`),
  with the hiding's and the exports' tests.
