# 0611. A filter every row passes is its relation

Status: decided and built (2026-11-22), t52 step 3. The user chose a check at run time over stopping at what the
pass can remove ("Build the run-time check").

## Intro

Hiding keeps a rule from reading an annotation as code (0165): every reader of a relation an annotation reaches
reads a narrow twin, `rr_narrow_<orig>`. Where the twin is the original's rows under the bound, `one_document`
makes it a filter (0575):

```
rr_narrow_rf_attr(c0, c1, c2, c3) :- rf_attr(c0, c1, c2, c3), c0 < B, c1 < B, c2 < B.
```

After the merge of `cost-classifier-metal-rf` and the port of 3d8f990a26, the reference program's large twins are gone
(t52). Six filters are left, on `all` about 30 MB: `rf_attr`'s (0.81 M rows), `rf_plain_attr`'s (0.60 M),
`rf_import_target`'s, `rf_owner_class`'s, `rf_qn`'s. No rule of the pass can remove them. Either the original's
folded annotation copy pairs a code node with an annotation's (`rf_attr(e, t, o, f)`: `e` and `o` an annotation's, `t`
the code definition around it), or the reader's only bounds are the twins it reads. Where an annotation exists the
twin and its original differ.

Where none exists, every row passes, and the twin is its original row for row: the doubling the user named ("check
for them and read the original when there are none").

## Decision

1. **A filter is a shape, not a name** (`plan::filter_of`). A rule `h(v0, …) :- x(v0, …), $vi < b, …` is a filter of
   `x`: the head the atom's distinct variables in order, every other literal one of them compared under a constant.
   Hiding's twins are filters, and so is any plain copy (`reference_edge(…) :- rf_edge(…)`, with no comparison).
   Its head is a plain relation (`plan::plain_filter`): not a lattice, whose rows are its groups' merges (`first(c,
   f, r) :- x(c, f, r)` under `shortlex_min` is each group's least, `first_tests`), nor a gate or a native operator.
2. **The executor checks the rows where they lie.** At a stratum of one relation whose one rule is a filter, and whose
   source the engine holds as rows, every live row of the source is read against the bounds. If all pass, the
   stratum is not run: the relation is held as its source (`hold_as_where_all`, then 0101's `hold_as`), the one
   `InRel` under two names, its rows, dead bits and indexes shared, nothing copied (0353). The source's statistics
   are the relation's. If one fails, the stratum runs as before. No run-level record is needed: the rows say whether an
   annotation reached them, in a chunk, in the cross part and in a memo hit's rows alike.
3. **Maintained, its change is its source's.** The same rows pass the same bounds: a source not moved leaves the
   filter unchanged; a source moved by a delta made in the run (`diffs`) gives the filter that delta; any other move
   leaves the filter with no delta, and its readers derive again.
4. **The issue board leaves a filter to the loop** (`Board::eligible`). Issued ahead as a job, it would derive the rows
   the loop then reads in place.
5. **A filter keeps its columns** (`ProjectAll`). Projected, it would hold fewer columns than its source and could not
   be read as it. Its source keeps them too, since the filter reads every one. Where an annotation exists, the
   twin holds the columns its readers do not read: on `all`, about 6 MB.

Not on the device (its strata bind their relations themselves), and not under kept strata (0077 §4), whose kept rows
are a stratum's own. Both derive filters as before.

## Measured

`all` (3,089 files, m1max, under the farm's other runs): rows and findings as before. The 7 `ref` rows and 1
`open_name` row that differ from W also differed on 2026-11-20. Bound as their sources in the cross part:
`rr_narrow_rf_attr` (811,392 rows), `rr_narrow_rf_plain_attr` (596,486), `rr_narrow_rf_import_target`, and copies
that are not twins: `reference_edge` as `rf_edge` (438,531 rows), `demand_rf_object` as `pre_198`. In chunks:
`rr_narrow_rf_owner_class` and plain copies of demand (`dm_rf_fn_follows_1` as `dm_rf_fn_seen_1`).

Columns released: 5,107 MB against 5,204 MB before; peak 12.5 GB against 12.8 GB.

Left with `rr_narrow_` in their names, and why:
- A chunk's `rr_narrow_rf_import_target` is no twin. The chunk derives no `rf_import_target`, so the twin is
  that relation's rules under the bound, with nothing beside it to double.
- `dm_rr_narrow_rf_attr_2` is a demand set: the columns a reader asks of the twin (3,600 rows), not a copy of rows.

## What it does not change

- What a rule sees (0058, 0165): a twin is still read wherever an annotation exists, and is the filter's rows there.
- The program: the filters are prepared as before (apart from 5), and the check is made per run of each.
