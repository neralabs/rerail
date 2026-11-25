# 0689. The cross part's `text` is read by node, and a relation made on lookup is read by the columns it is probed by

Status: decided (2026-11-25). It follows 0353 row 7 (a node's and a line's text read, not made a row each) and 0676
(the 4 re-export heads that cross files for real). It amends 0458 (the corpus batch's `text`) and 0364's recheck
pricing (Q208).

## Intro

0353 row 7 moved `text` off rows wherever a rule reads it at a node the per-file part binds. The cross part still
read `text` as rows: the corpus batch made every document's `text` rows, so that an edit's versions (`Before`, plus
and minus, decision 0458) could read them. On the 300-file bench the corpus batch made all 300 documents' rows, for 6
heads whose rules read the text of a node a cross-file relation binds: the re-export heads of 0676 (q4, q6, q13
`same_leaf`, PIE796's message) and two of p297's.

Each of those rules reads `text` at a node its rule binds. A node's text is one read of its document's tape
(`CoreRows::text_row`), so the rows are not needed. What the cross part needs is a relation that answers by node,
counts its rows, gives an edit's documents' rows for their versions, and is read whole only where something truly
reads all of it.

Making that relation exposed a gap in the planner. A recheck (0364 stage 3) runs a rule with its head's variables
bound. `named(x, l) :- used(c), text(c, x), line(c, l)` then has `x` and `l` bound. By its statistics `text` read by
its string gives a row or two, so the planner led with it. A probe of `text` has no index by string: the executor
built one by reading every row, which read every file's tape on a warm run. With `text` priced right, the next
choice was `line` by its line number, an index of every row of a view of the batch, with the same result. The planner
did not know which columns a relation can be looked up by.

## Decision

1. **The cross part's `text` is read by node** (`ProbedText`, `Projection::text_probed`). Where every `text` atom of
   the cross part's rules reads a node, or a `source` or `src_line` span, that a positive atom of its rule binds
   (`cross_text_by_node`, by the node columns of every derived relation, a cross-file one's too), the corpus batch
   makes no `text` row. Nor do its groups (`core_rr`) or `Extra` (the source's and lines' spans).
   - **A probe** reads one id's row as a group makes it: a node's `text_row`, a line's bytes off the source
     (`line_text`, as `LongTexts::text_any`). The texts are interned once and the rows are kept in the order read: a
     derived fact, written once where it is computed (0353, "Not copies"), as `Batch::tape_text` keeps them.
   - **Its count** is the kept statistics' total where the run gives it (`given_rows`, no document made, Q332), else
     counted on the tape.
   - **An edit's versions** read the changed documents' ids (`rows_of_documents`), each probed.
   - **A semi-join's rows** at the held nodes (0458) are those nodes probed (`for_rows_keyed`). The kept rows answer
     for the unchanged documents, as a made relation's do.
   - **Its statistics** are measured on the documents a made relation's sample takes (`sample_groups`), by probes.
   - **Its texts are the batch's strings** (`probed_text::strings`): a text test of them sees what it saw in the
     made rows.
   - **A read of every row** reads each document's ids in turn. Nothing in the cross part should need it.
2. **A relation made on lookup is read by the columns it is probed by** (`Stats::probed_by`). A source that makes its
   rows per value looked up (`Source::made_on_lookup`) is probed only by its `indexed` columns. In the cross part, a
   view of the corpus batch (`Source::syntax_batch`) is probed only by its indexed or sorted columns. A step that
   binds none of them reads every row, so it is priced as that: the relation's rows, each at its scan's cost
   (`Source::scan_cost`; a view's, one). Its source is asked nothing that makes a document: a view's estimate sampled
   documents, which made a run after an edit build 3 files (`r7`).
   - The recheck of `named` now leads with `used` and reads `text` and `line` by `c`.
   - `ProbedText` is made on lookup, probed by its node column, and its scan costs 64 derived rows a row.

## What this does not do

- **A text the cross part reads by value stays rows.** `text(m, "1")` with `m` free is a lookup by string. The split
  keeps the corpus batch's rows there, and the decision line says why.
- **The device.** No device read of `ProbedText` is written. The bench above ran on the CPU. A device stratum's read
  of the cross part's `text` is not measured here.
- **Chunk plans are not repriced.** Only the cross part registers its batch's views (`probed_by(…, true)`). The
  per-file part registers only sources made on lookup, through `lookup_costs`.

## Measured

The 300-file bench, on the M1 Max, CPU, one run each, against HEAD's run of the same day:

- 463 violations. All 661 output relations are identical, row for row, compared as sorted CSV.
- The decision line: "the cross part's: read by node, off the tape, by p297_rr_h_agg_…, q13_same_leaf, q4_same_leaf,
  q6_same_leaf, rr_diagnostic_33ab31c6…, rr_diagnostic_8dba1432…".
- CPU, the new binary's second run against HEAD's: cross 3.78 s (3.74 s), statistics 1.43 s (1.38 s), combine
  0.74 s (0.72 s), prepare 2.72 s (2.59 s). No phase changes measurably. The same binary's two runs differ by 0.3 s
  in cross alone (3.48 s, 3.78 s). The bench's corpus is 300 files, and this change moves where `text` is read, not how
  much is read.
- The new binary's first run took 2.47 s in combine: its spans were the 317 programs lowered ("a program's version in
  scope: its image, or lowered"), not taken as kept images, as a new binary's first run does. Compare second runs.

## Tests

- `tests_lang`:
  - `text_read_by_node_alone_is_the_rows_a_group_makes`: every node's and line's probe equals its made row, as do the
    count and a whole read.
  - `the_cross_parts_text_is_read_by_node_where_each_atom_reads_a_bound_node`: names the heads, and refuses a read by
    value.
- `tests_corpus`:
  - `a_text_read_by_node_alone_follows_an_edit_of_its_node_kept`: five edits, each against a cold run.
  - The warm tests and the made-batch tests (`r4`, `r7`) hold Q332 with the cross part probed.
- `tests_logic`:
  - `a_step_that_binds_no_column_a_made_relation_is_looked_up_by_is_priced_as_reading_every_row`: without
    `probed_by` the recheck leads with a read by a bound value; with it, `used` leads.
