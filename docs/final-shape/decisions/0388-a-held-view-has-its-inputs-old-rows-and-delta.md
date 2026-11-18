# 0388: A view the run holds of an input's columns has its input's old rows and delta

**Status:** built (the user's choice, 2026-11-17: "views get deltas", when the merge of 0364 stage 1 met Q37).

## Intro

Q37 (`6fe55cb12`) holds a rule that copies an input's columns as a view of the input, read in place, never kept:
`kind(n, k) :- node(_, n, k, _, _)` is no stratum that copies `node` in every run. Decision 0364's stage 1 removed the
counting supports. Before that, `kind` read its support `kind__sup0` and was kept and maintained like any other
relation. After it, `kind` reads `node` alone and is such a view.

A view keeps no rows. So in a session's edit the driver had no old rows for it and no delta. It derived `kind` again,
and every stratum that read `kind` was derived again too. The recursive stratum of
`a_cycle_closed_by_an_empty_relation_still_keeps_levels` (0090) was derived again instead of being maintained by its
levels.

```
kind(n, k) :- node(_, n, k, _, _).          // held as a view of node (Q37)
reach(a, a) :- side(a), never(a).            // never(n) :- kind(n, "no_such_kind").
// edit: node moved -> kind derived again (no delta) -> reach, side derived again
```

Decision 0090 already says what a view of one input is in a session (`RERAIL_SESSION_VIEWS=on`). Its old rows are the
view over the input's old rows, its delta is the view over the input's delta, and none of its rows are kept. A Q37
view is one row per input row: it keeps the input's key (`node`'s node, `doc`'s and `tape`'s first column) or every
column. So 0090's statement holds for it exactly.

## Decision

1. **One definition of a held view.** `maintain::held_views` lists the relations a run holds as views of an
   input's columns (`plan::input_view_shape`), each with its input and the input's column of each of its columns. The
   session's cut (`retain::cut`, Q37's "not kept") and maintenance read it.
2. **Maintenance takes a held view as 0090's view, whatever `RERAIL_SESSION_VIEWS` says.** In `input_views_over`, its
   stratum is its rule (the run holds it in place). Its input's change is its change. A stratum's own form (the
   driver's, `maintain::form`) writes the view's delta, `v__minus` and `v__plus`, as its rule over the input's delta,
   for the strata the driver forms after it.
3. **The kept part answers for a held view's old rows.** `Retained::set_views` gives the part the run's held views.
   `holds_old` and `has_old` are true for a view whose input's old rows are held. `old_source` is the input's old
   source with the view's columns picked in place (`retain::Picked`): no rows copied, none kept.

## Effect

In the test's edit, `kind`'s stratum is its view and its delta. `{reach, side}` takes its general form (0298), with
`kind` among its reads, and is maintained by levels. The targeted set on a farm host: 759 of 761. The two failures
predate this merge (Q101, Q102).
