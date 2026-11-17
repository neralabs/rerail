# 0329. A projection of `node` that keeps its key is held in place

Status: decided (2026-11-16), working on the user's "wstart (14.8 s) and kind (7.6 s) … Guests: 18 s". Refines
0104's rule for native runs ("a projection of an input is a stratum like any, run from the input where it is held")
for one case: 0141's projections of `node`.

## Intro

The lowering derives `kind` from `node` (decision 0141):

```text
kind(n, k) :- node(_, n, k, _, _).
```

`n` is `node`'s key: one row per node. So `kind`'s rows are `node`'s rows, one each, in two of its columns. A native
run still ran the rule as a stratum. Every chunk scanned `node`'s view, made 96 M head rows, settled them and
sketched them. On a cold PostHog run that cost 7.9 s of join CPU in the program's chunks and another 6.2 s in the
guests' (56 M rows), about 14 s in all.

## Decision

1. A stratum whose one rule is a projection of the bound `node` (one positive atom, each head variable once in it)
   and keeps `node`'s column `n` is held as a view of `node`'s source in those columns (`setfix::hold_view`,
   `Run::node_projection` and `hold_projection`). No row is made, and readers probe the source in place. `docof` over
   nodes qualifies too. A maintained run (a session's, 0117) still runs the stratum.
2. Its statistics are what the stored relation's sketches gave: each column's distinct values, the squares of their
   counts, and exact counts (`syntax::node_column_counts`). They are counted from the batch without reading its
   rows: `n` is the key, a document's count is its nodes, and a kind's count is one code read a node. Exact, because a
   derived relation's counts are read as exact (0319).
3. A strand does not take the stratum (`strand_reads_with`): it is held on the run's thread, before the readers'
   jobs.

## Measured (M3 Max, PostHog, 29,885 files, cold; load 20-45)

| | Before (g1) | Held, statistics approximated | Held, exact counts (v5) |
|---|---|---|---|
| Join CPU (profiled) | 78.8 s | 72.7 s | 62.9 s |
| `kind`'s strata | 7.9 s joins + 6.2 s in the guests | 0 | 0.8 s (the counts) |
| `wstart` | 14.8 s | 14.1 s | 9.6 s |
| Profile CPU in all | 223.8 s | 221.1 s | 214.5 s |
| Cycles (profiled) | 826 G | | 809 G |
| Cold runs, unprofiled | 191-200 s user | | 175-183 s user, 29.4-30.2 s wall |
| Findings, cold and warm | 29,747, fa0efeaed6ff | same | same |

Statistics matter as much as the hold. Taken from `node`'s sampled column statistics, the held `kind` made
`pre_115` and `param` plan worse (instructions 2,575 -> 3,408 G). Counted with `of_kind`, a scan of the tape a kind,
the counts took 12 s. Counted from `Facts::histogram`, the tape's tokens and the nodes a batch leaves out were
counted too (111 kinds where the rows hold 78).

## Not done

- Inlining `kind` into its readers (0097's choice, 48082beb9) stays priced. Unpriced in every module, instructions
  went 2,558 -> 4,734 G.
- A projection of another input, or one that drops `node`'s key, still runs: its rows need a dedup.

## One mechanism with Q37 (2026-11-17)

cost-classifier-metal-rf built the same thing at the same time (work queue Q37, 6fe55cb12 to 4085f6120): any rule that
copies a bound input's columns and keeps its key (`input_view_shape`: `node`'s `n`, `doc`'s and `tape`'s first column,
or every column) is held as a view, never a stratum, never kept, and a session counts it neither run nor clean. Its
measurement: read in place through the source, each reader's row went back through it (the row's document, a kind's
dialect), 8.4 s against the copy's 6.0 on fixtures/python, so it reads the columns once into flat rows and sketches
them as a stratum's relation is sketched.

The merge keeps one mechanism, Q37's: its detection, its handling in sessions, retain's cut and the kept choices.
This decision is one of its two read forms, chosen by the source:

- **In place** (this decision), where the source answers the view's probes from its own index and counts its columns
  without reading its rows: a batch's `node` (`Source::sorted_rows`, the field by child off the tape, fdf391f11;
  `syntax::node_column_counts`). The cost Q37 measured was the per-row document search those slices removed. On a
  cold PostHog run, `kind` cost 0.8 s held so, against about 14 s copied.
- **Flat rows** (Q37's `hold_view`), for every other source.

Either way the view's statistics are a copy's: exact counts in place, sketches over the flat rows. `node_projection`
and the run loop's own dispatch are gone; `RERAIL_COPY_VIEWS` derives every view as a stratum, both forms included.

## A probe of a kind reads the batch's kind index (2026-11-17)

The held `kind` answered a probe by its code (`kind($r, #k)`, `$r` unbound) by asking each document's tape for the
nodes of each of the code's raw codes (`Facts::of_kind`, a scan of the tape), once per value probed. With exact
counts, a kind no file holds counts 0, so the planner probes it first: `assign_name`'s delta (27 rows) then probed it
27 times per chunk. On a 10-file cold run that took 20 s of the strata's CPU per stratum (`assign_name`,
`p290_name_at`), and the run 53 s against about 5 s before.

Now the batch keeps `node`'s rows grouped by kind (`Batch::kinds`): each node's kind read once, as the column reads it
(`Dialect::kind`), placed by a counting pass into one array of rows with a sorted key array and offsets. A probe reads
its kind's run (a document's part of it by its row range), and the column's exact counts (2. above) are the runs'
lengths, from the same pass. The same strata now take under 0.8 s, and the findings are the same.
