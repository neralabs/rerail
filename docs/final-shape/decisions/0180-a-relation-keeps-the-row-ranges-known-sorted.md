# 0180: A relation keeps the row ranges known to be in an order, and its runs take them in place

Status: built (2026-11-13), `setfix/runs.rs` (`Sorted`, `Ids`, `Part`, `Index::add_run_in`) and `setfix.rs`
(`Rel::push_fresh` records the ranges; `set_delta`, `index_rest`, `want`, `based` and a derived input's order use
them). It narrows decision 0112's orders: a run no longer always lists its ids.

## Intro

An order (decision 0112) is a list of runs, and each run used to be the ids of its rows, sorted by the order's
columns: 4 bytes a row, and a sort to make it.

But a relation's rows are often written in an order already. A wave's new rows are checked against the relation
as one sorted batch (`Fresh::settle`): they are sorted by the relation's first order, then appended in that order.
Sorting them again to make the first order's run, and listing their ids, repeats what the writer knew.

Measured on the O125 replay before this decision (`RERAIL_SPIKE_PRESORTED`, 21ca21160):

| | runs (wholly in order) | rows | rows already in the order |
|---|---|---|---|
| component orders | 8.5k (7.9k) | 9.4 M | 96% |
| derived-input orders | 610 (131) | 4.3 M | 9% |

## The metadata

A relation (`Rel`, and the `InRel` it is moved into) keeps `sorted`: row ranges `lo..hi`, each with the columns
of the order its rows are in (`cmp_vals`: tagged when the columns are every column).

- **Who records it:** the writer that knows. `push_fresh` appends a wave's batch sorted by the first order, and
  says so. A range that starts where the last one ended, in the same order and not before its last row, joins it.
- **Why it stays true:** a relation's rows are only appended. Deleted rows stay in place, marked dead, and readers
  skip them. A relation started from a kept one (`based`) shares its rows and takes its ranges.
- **What it implies:** rows in the order on columns `c` are in the order on any prefix of `c`, untagged. A tagged
  range (every column, sorted by the leading value's hash first) implies only its own order.
- **Checked:** in debug builds, `note` asserts the rows of each range it is given are in order (the engine's
  plan, session, corpus and waves tests and a debug O125 replay ran with it).

```text
rows   0 ......... 900 .......... 2000 ....... 2600 ..... 3000
sorted [0..900 full] [900..2000 full] [2000..2600 full] [2600..3000 on (0, 1)]
order (0, 1, 2), tagged: every range applies        order (0): only 2600..3000
```

## Runs in place

A run's ids are now one of:

- `Ids::List`: ids sorted, 4 bytes a row, as before;
- `Ids::Rows(lo, hi)`: the rows `lo..hi` themselves, known to be in the order: no list, no sort.

`Index::add_run_in` takes the parts of its rows known sorted (`Sorted::within`): those become runs in place, the
rest are sorted, and the pieces merge into one run. A single piece in place stays in place. Tags (a tagged order),
fences and prints (a printed order) are kept as before, read from the rows in turn.

A probe searches positions (`split_at_pos`), reading the id at a position (`Ids::get`: listed, or `lo + i`), so
both forms answer alike. A key's candidates are `Part`s: an id slice, or a row range (`Found::ids` iterates
either). Two runs in place that meet in order (the first's last row not past the second's first) merge in place:
tags and prints joined, no ids listed.

## Measured

O125 replay, `--profile profiling` build, unprofiled, steps 06–08, `RERAIL_KNOWN_SORTED=off` against on, three
alternating rounds; "same as full" in every run.

**Instructions retired** (G), cross part:

| | cold | 06 | 07 | 08 |
|---|---|---|---|---|
| off | 9.77, 9.70, 9.69 | 4.26, 4.27, 4.25 | 6.59, 6.58, 6.63 | 7.78, 7.69, 7.79 |
| on | 9.40, 9.41, 9.39 | 4.34, 4.24, 4.19 | 6.42, 6.49, 6.47 | 7.75, 7.59, 7.75 |

The cold run retires 0.30 G (3.1%) fewer; step 07 about 0.1 G; 06 and 08 within their noise.

**The runs' builds** (`RERAIL_SPIKE_PRESORTED=time`, `add_run_in` wall time, whole replay), two rounds:

| | component orders | derived-input orders |
|---|---|---|
| off | 105.0, 109.3 ms | 85.8, 90.9 ms |
| on | 27.2, 36.4 ms | 75.7, 96.3 ms |
| rows taken in place | 8.98 M | 0.34 M |

- Component orders: their builds cost 70% less. 8.98 M of their 9.4 M rows are runs in place: 36 MB of id lists
  never made over the replay; the cold run's 3.5 M (14 MB) stay off the kept relations' orders as they are held.
- Derived-input orders: little changes. Their probes key on columns other than the rows' order (decision 0186:
  9% of their rows are in the probe's order, and fewer are known so).

Peak RSS (the replay samples it every 2 s) varied 660–960 MB within each mode; it does not resolve the change.

## Tests

- `runs_of_rows_known_sorted_answer_as_sorted_runs` (`runs.rs`): an index made with the known ranges and one made
  without hold the same ids, tags, prints and fences, and answer alike (`find` over all and over deltas, `find_at`,
  groups, the dedup test, and after a merge), for the full-row order (tagged), prefix orders (one and two columns),
  printed ones, and partial knowledge (half a wave's range told). Runs in place that meet merge in place. It fails
  when a tagged range is taken to imply a prefix order.
- Debug builds assert each recorded range (`Sorted::note`); setfix 37, logic plan 105; engine plan 54, session 25,
  corpus 110, waves 53; a debug O125 replay.

## Open

- Other writers could record what they know: a merge-join's output, a scan of a sorted order, a source's sorted
  columns (`sorts_rows`, which a view already uses).
- Other readers could use it: sorting a scan for the next step's probes (`scan_order`), column statistics, merge
  joins.
- A relation reordered (`Index::reorder`) sorts its runs again: its ranges are not consulted there yet.
