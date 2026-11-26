# 0294. The cross-file part's strata off one thread: the work around a walk's morsels runs on the pool, a cut walk counts as one walk, and a walk going on from a cut reads its rows left in their order

Status: built (audited 2026-11-23). Decided (2026-11-16). Builds on 0278 §4 and 0279 §7 (morsels), 0292 §1, 0206 (cuts
and handoffs), 0203 (slow joins, exact statistics), 0187 (rows known sorted), 0112 (statistics). Measured in
notes/t40's follow-up.

## Intro

On all of PostHog the cross-file part is a chain of strata, each reading the one before it: `export_query__sup0`,
`export_query`, `rr_agg7__sup0`, `exports/resolve_export`, `sym_of__sup0` (`qn ⋈ resolve_export`, 2.2 M rows),
`sym_of` (6.4 M), `P__sup0`, `rr_diagnostic`, `violation`. Strata of one level already run at once (0113's board),
and their big walks already run in morsels (0279 §7, 0292 §1). Yet `sym_of__sup0` took 0.5–0.6 s, and only 0.4 s of
CPU of it was in its morsels. Spans around each step of its run gave, on its own thread:

| step | CPU |
|---|---|
| `scan_order`: the scan's rows sorted by the next step's key, before the morsels are cut | 0.19 s |
| `Fresh::settle`: the morsels' sorted runs merged, distinct, checked against held rows | 0.13 s |
| exact pair statistics (0203 §3), for a slow join that was not slow (§3) | 0.07–0.14 s |
| sketches and kept statistics of the new rows (`indexed_to_n`) | 0.06 s |

## Decision

### 1. Sorts, merges, key reads and statistics of many rows run on the pool

From 65,536 items (`runs::PAR_ROWS`):

- `merge_pieces` merges each level's pairs of pieces in parts cut by their outputs' places (merge path: a part's
  inputs are found by binary search of where its first and last outputs come from, `co_rank`), so a level spreads
  over the threads whatever its pieces' sizes, its last single merge too. A part puts out at least 8,192 items.
- The packed sorts (`sort_packed`, `sort_tupled`) sort parts at once and merge them so.
- `scan_order` reads the rows' key values in parts at once.
- `indexed_to_n` sketches each column, and `RelStats::delta` counts each column, as a job of its own.

Each gives what one thread gives: a merge's ties take the left piece's first, as one thread's; the packed keys hold
their id, so they are distinct; a sketch or a column's statistics take that column's values in the rows' order.
`runs::tests::merges_and_sorts_on_the_pool_are_one_threads` checks the merge against one thread's, ties and pieces
of very different sizes included, and the sort.

### 2. A walk cut in morsels counts as one walk

A limited walk in morsels shares its budget out: each morsel stops past its share. Each morsel's work counted its own
entry into the first step, and one it was refused, where its step counts, which the planner reads
(`profile::plan_work`), keep one entry for the walk. And the shares were rounded down. So a cut walk reported more
work than the planner counted, and stopped short of its check: the planner gave the line the few rows of work left
to its check as a budget, the walk went on and was cut again (`sym_of__sup0`: 100,008 rows of work, then 40). Now the
walk's work is its counts' plus the one step a cut walk is refused, as one walk's, and each share covers its
morsel's own entry, so the counts reach the budget. `cut_tests::a_scan_walked_in_morsels_…` checks both.

### 3. A cut is judged by its own work

At a seed's or a whole plan's cut, the line was judged slow when its work so far passed `slow_factor` times its
price for the rows its cut walk read. Those rows are the last walk's alone; the work was every walk's. After §2's
second cut, 100,000 rows of work were weighed against a price for 19 rows: the line was flagged slow, its exact
statistics taken (0.07–0.14 s on the critical path), and a search requested. The cut walk's own work is weighed now.

### 4. A walk going on from a cut reads its rows left in their order

The rows a cut left were kept as a set (`Rows0`'s bits). The walk going on sorted all of the scan's range by its key
again, then kept the rows left: `sym_of__sup0`, cut at each check (100 k, 200 k, 400 k, 800 k rows of work), sorted
2.2 M rows 7 times. The rows left now also keep the order the cut walk was to read them in (`Rows0::left`), and the
walk going on reads them so where one pass finds them in its key's order (the same plan); else it sorts them, not the
range. The cut test checks that every row left was read so (`left_read_in_order`).

### 5. A whole plan's exact statistics read its whole join

At a whole plan's cut (O168) the planner asks for histograms of the whole plan's steps, and `step_histograms` read
the probe's delta join, whose step at the same place is another atom: a key column past that atom's width panicked
(about one run in six; commit afe32582a). The caller says which join its order is.

## Measured

All of PostHog, profiling build, the M5 Max under load (the M3 was unreachable), the same 29,747 findings in every
run (the same set, node ids included):

| | before (load 16) | after (load 26) |
|---|---|---|
| the cross part's strata, first start to last end | 1.74 s | 1.07 s |
| `sym_of__sup0` | 0.47–0.61 s | 0.18 s |
| `sym_of` | 0.23–0.27 s | 0.09 s |
| `export_query__sup0` | 0.18–0.25 s | 0.12 s |
| exact pair statistics in the cross part | `sym_of__sup0`, 0.07–0.14 s | none |

## Open

- `Fresh::settle`'s linear passes after its merge (distinct, held check, merge with the settled rows) stay on one
  thread: 0.04 s for `sym_of__sup0`.
- A walk going on from a cut reads its rows' keys again to see they are in order (on the pool, 0.28 s of CPU over
  `sym_of__sup0`'s 7 walks); a walk whose plan did not change could skip the check.
- `sym_of` is 6.4 M rows where `P` reads those of one symbol (`builtins.len`, 29,772): the cross part derives every
  name's symbol, and writes them to the frames (`retain`, 0.57 s on the main thread). Demand would bound both.
