# 0184: A wave's batch, a scan's order and a re-sorted order merge what is known in order

Status: built (2026-11-13), `setfix.rs` (`Fresh::offer`, `Fresh::settle`, `scan_order`) and `setfix/runs.rs`
(`sort_pieces`, `merge_pieces`, `Sorted::within_key`, `Index::reorder`). It extends decision 0180 past an order's
runs, to the engine's other sorts.

## Intro

Decision 0180 gave a relation the row ranges known to be in an order, and an order's runs took them in place. Three
other sorts still sorted rows that were already in order:

1. **A wave's batch** (`Fresh::settle`): the rows rules derive for a relation, sorted by its first order for the
   dedup test. A join that scans rows in order often emits its rows in order too.
2. **A scan for the next step's probes** (`scan_order`): a scan of 256 rows or more is sorted by the key the next
   step probes with, so those probes come as one sorted batch (decision 0112).
3. **An order sorted again on other columns** (`Index::reorder`, when a new key set extends its chain).

Measured on the O125 replay before this decision (`RERAIL_SPIKE_PRESORTED`):

| sort | sorts (wholly in order) | rows | rows in order, stretches of 64+ |
|---|---|---|---|
| wave batches | 7,069 (3,502) | 9.7 M | 36% |
| scans for the next probe | 1,062 (363) | 2.3 M | 38% |
| orders sorted again | 93 (28) | 0.17 M | 4% |

## What each now knows

**A wave's batch: its writer records it.** `Fresh::offer` compares each row it is given with the one before, in
the first order (two rows at hand, one comparison), and notes where the order breaks (`breaks`). `settle` then
takes each stretch between breaks of 64 rows or more as sorted, sorts the rest, and merges the pieces. Rows given
with no order known (`Fresh::add`: the device's rows) mark the batch `lost`, and so do breaks more than one in 64
rows (past the first 64): it is then sorted whole, as before.

**A scan: the scanned relation's ranges.** The next step's key is the scan's columns that bind it, in the key's
order (constants hold no order), tagged by the first when the next order is tagged. `Sorted::within_key` gives the
scan's rows known sorted by that key (`implies_key`: an untagged range gives any untagged prefix of its columns; a
tagged range only a tagged key on its prefix). A scan wholly known sorted is not sorted: its rows stay a range, in
order already. Otherwise the stretches between the known ranges are sorted and every piece merged.

**A re-sorted order: its rows' ranges.** A run over rows `lo..hi` (a wave's run, or waves' runs merged) is made
again from those rows (`Index::run_over`), its parts known sorted in the new order taken in place (decision 0180).

## The merge

`merge_pieces` merges sorted pieces lying one after another: adjacent pieces two at a time, a pass at a time,
through one buffer the length of the sequence. Each item moves about log(pieces) times and is compared once a pass;
one piece moves nothing. `sort_pieces` sorts the stretches between known pieces (each alone), then merges.

```text
positions  0 ......... 300 ..... 340 .............. 900 ..... 1000
known      [0, 300)              [340, 900)
           sorted as is  sorted   sorted as is     sorted
merge      ((0..300 + 300..340) + (340..900 + 900..1000))       two passes
```

## Measured

O125 replay, `--profile profiling` build, unprofiled, steps 06–08. `RERAIL_KNOWN_SORTED=runs` (only decision
0180's runs use the ranges; the batch does not compare its rows) against on, three alternating rounds; "same as
full" in every run.

**Instructions retired** (G), cross part:

| | cold | 06 | 07 | 08 |
|---|---|---|---|---|
| runs only (0180) | 9.47, 9.42, 9.41 | 4.12, 4.13, 4.22 | 6.40, 6.40, 6.41 | 7.59, 7.59, 7.60 |
| on | 9.25, 9.25, 9.25 | 4.23, 4.21, 4.12 | 6.36, 6.34, 6.34 | 7.56, 7.49, 7.54 |

The cold run retires 0.19 G (2.0%) fewer; steps 07 and 08 about 0.05 G; 06 within its noise. With decision 0180,
the cold cross part went from 9.69–9.77 G to 9.25 G.

**What was taken as known** (whole replay, `RERAIL_SPIKE_PRESORTED=1`):

| sort | rows | taken as known | note |
|---|---|---|---|
| wave batches | 9.56 M | 3.21 M | 93% of the rows in order in long stretches |
| scans for the next probe | 1.88 M sorted | 0.64 M | about 340 scans not sorted at all (722 sorts left of 1,062) |
| orders sorted again | 0.17 M | 0 | their rows' ranges are in the first order, not the new one |

The third gains nothing on this replay: a re-sorted order's columns lead with a key set's columns, and the ranges a
wave writes are in the first order (every column). It costs one lookup of the ranges a run.

## Tests

- `pieces_known_sorted_merge_into_the_sort` (`runs.rs`): `sort_pieces` gives the full sort's order with no pieces,
  one covering all, pieces at either end, many short ones, and ties; each item once.
- `a_waves_rows_in_order_are_merged_not_sorted` (`setfix.rs`): batches with stretches in order, rows out of order
  between, repeats and rows held, checked in batches as they come, leave the relation the same rows in the same
  places as batches sorted whole; an order re-sorted on other columns then answers alike. It fails when a known
  stretch is taken one row too long (debug builds assert every recorded range, decision 0180).
- setfix 39, logic plan 105; engine plan 53, session 25, corpus 110, waves 53 (debug: the ranges asserted).

## Open

- A re-sorted order could keep the order it is sorted into as a range of its own only if the rows were moved,
  which they are not (rows are appended only, decision 0180).
- A scan of a base source (a view) knows nothing yet: its source's sorted columns (`sorts_rows`) are an index, not
  the rows' order.
