# t56: What a probe of the batch's relations costs, and the plan to cut it

## Intro

The per-file rules spend much of their time reading the batch's relations (`text`, `kind`, the matcher relations,
the views), one probe a row. The profile's join-step table charged `text / lookup on (n)` 7.4 s for 10.5 M rows on
`all` (709 ns a row), against about 150 ns a row for a probe of a derived relation, and three changes to the read
(t50 step 5's follow-ups, 2026-11-23/24) did not move that number. This note says why: most of that 709 ns is not the
read. It then sets the order of the work, each step with its check, its own measured component, and what follows
from each outcome.

## 1. What the step table measures

`setfix::StepClock` charges a step the ticks between the walk's step switches, scaled by the walk's CPU over its
span. Whatever a thread does inside a probe is the probing step's:

- **The batch's rows, made inside the first probe.** `Layered::base_len` (a `OnceLock`) asks `MadeRows::len`, which
  asks `Batch::complete`, which makes every document's rows as one group (`Batch::ensure`, decision 0494): the core
  rows, the annotations, the interned atoms and the matcher wave (`AstWave::matched`).
- **Another thread's make, waited for.** A probe finding the batch's documents claimed waits (`made.wait_for`), or
  runs the pool's next job (`rayon::yield_now`): any job, another chunk's joins included, its ticks the probing
  step's.
- **Lazy indexes.** `Relation::index` (a `OnceLock`) sorts a column at its first probe; other probes wait on it.

The macOS sample of `all`'s per-file window (m3, every thread, 94,152 stacks; counts inclusive):

| Under | Samples |
|---|---|
| `Batch::ensure` (the batch made inside a probe) | 4,637 |
| of it, the matcher wave (`AstWave::matched`) | 3,289 |
| of it, the core rows (`CoreRows::node`) | 319 |
| `Layered::base_len`'s `OnceLock` (the first probe, which makes it) | 3,088 |
| waits (`once::queue` waits, `__ulock_wait`, `psynch`) | 12,504 |
| `Relation::lookup_read` (the search and its reads) | 939 |
| of it, `Relation::between` | 649 |
| `Viewed::lookup_read` | 660 |
| `Layered`'s per-probe `Vec` (`spec_from_iter` under its `filter`) | 251 |
| `ColumnIndex` built inside a probe, waited on | 148 |

So `text`'s step is mostly the batch's make and the waits for it. The read is about a fifth of it.

**What this explains:**
- Writing the core rows in node order (so that `text` takes the in-place ordered search) took `text` from 709 to
  908 ns a row. The sort runs inside `make_rows`, which `text`'s first probe pays. That is not a read result.
- Writing the matcher relations in node order made `rr_ast_36` slower for the same reason.
- The two-level search of a column under key runs (d154b8984f) cut the lab's search from 230 to 204 ns. On `all` it
  was invisible under the make.
- The node-id hint (4b125c04ca) cut 0.4% of the run's instructions, the one change measured outside the step table.

**What the lab measures.** `a_text_reads_cost` reads after the make, hot (17,625 rows): `text`'s search 204 ns a
probe, a value read 9 ns, `node` (a view) 47 ns, a layered source 30 ns more. These are read costs. `all`'s are
colder (more rows, other threads), which step 1 measures.

## 2. The plan

Each step is judged by its own component, measured. When a component doesn't improve as predicted, the step is
sampled and its cause found before the step is kept or changed. A step is never reverted because the end-to-end
number did not move.

### Step 1: the step table charges reads only

The make, the waits and a lazy index build are each charged to their own entry, not to the probing step's. There is
one mechanism for every lazy make inside a probe: a thread-local "aside" that the clock switches to and back from
(`profile::aside(what, || …)`), called by `Batch::ensure` (made, and waited for), `Relation::index` and `Within`'s
solve. Check: the rows are unchanged, and the sum of the entries equals the walk's CPU, as before. Output: the
per-row cost of every input probe on `all`, read only. That table is the baseline for steps 3 to 5.

### Step 2: the batch is made as its own phase

Today the first probe of any batch relation makes every document's rows, and the other threads wait or run unrelated
jobs inside that probe. Step 1's figures say how often a chunk's make is partial (some documents) rather than whole.
- If a chunk's joins make every document anyway, make them before the joins start, as a phase, each document's
  rows on the pool (0494's groups already make documents as one group; `ast_wave::matched` already runs on the pool).
- If partial makes save work, keep them lazy, but a waiting thread runs the maker's own jobs, not any job.

t50 records two tries this step must not repeat ("The same run after the fixes", items 3 and 4). Making the
documents in groups as probes asked for them cost +411 G instructions over the whole make. Making groups side by side
(`complete`), with makers running pool jobs that read the batch, grew one run past 20 GB. So the phase makes the
batch whole, once, before any join reads it: no join runs inside it, so no read nests a make.

Check: the rows are unchanged, and the peak is no higher than before. Measured: the per-file window's wall time and
CPU, and the wait samples.

### Step 1, done (e71074b664)

One account per thread (`profile::ACCOUNTED`) holds what closed scopes (spans, walks) took. A walk's step subtracts
it, and the makes a probe can sit inside are spans (`profile::once`). On `all` (CPU, m3 at load 30), the rows were
unchanged:
- `text`'s line went from 709 to 537 ns a row.
- "batch: rows made" under the chunks is 12.9 s (25 makes, one a chunk), plus about 4.4 s more where a chunk's
  `measure rr_long_text_node` reads the batch first.

### Step 2, measured: no phase to make; the make's own work is the cost

Step 1's figures answer step 2's question:
- Every chunk makes its whole batch: 25 makes, one per chunk. Laziness saves nothing, and it costs nothing either.
- No thread waits on another's make: no "batch: waited for another thread's make" span. The profile's blocked
  waits total under 3 s of wall, none of them on the batch. The 12,504 wait samples in the earlier macOS sample
  were idle pool threads, not waits for the make.

So making the batch as a phase would remove neither CPU nor waits, and it is not done. The cost is the make's own
work, per chunk (CPU s, summed over the 25 makes):

| Part | CPU s |
|---|---|
| the matcher wave run (on the chunk's thread; 0.97 s more on the pool) | 6.92 |
| the matcher's captures (1.55 s on the pool) | 2.39 |
| node rows | 1.39 |
| string annotations | 0.84 |
| projected: identity maps, definition ids, granules | 0.84 |
| intern atoms | 0.60 |

Beside the make, each chunk counts the base relations the cross part reads (5.0 s: `docof` 1.3, `node` 1.2,
`node_order` 1.1, `text` 0.7, the nodes its exports hold 0.7) and measures its exports (2.2 s). These are t50's item 1,
moved from the stats phase into the chunks.

Step 2 is now the make's own work, largest first: the wave run, which runs on the chunk's thread, not by parts on
the pool.

### Step 3: one search for every relation, in its block

The batch's relations (`vtab::Relation`) search a heap `ColumnIndex` (sorted ids and a fence every 64). The derived
relations search setfix's sorted order (`runs::Run`: ids, fences, tags), given to their relation's block
(`Relation::try_add_order`, decision 0478 step 5b), where the device reads it too. These are two mechanisms for one
job, and the first isn't in its relation's block, as 0353 requires of an index. The batch's relations get the same
in-block order, built by the same code. `ColumnIndex` goes. Callers of `sorted_rows` read the order's ids. Check:
the rows are unchanged, and the vtab tests and `first_from`'s tests are ported. Measured: the lab's `text` search, and
step 1's `text` line. The algorithm is the same, so equal cost is expected: this step is for 0353 and for one
mechanism, and step 4 then speeds both kinds of relation at once.

### Step 4: a point probe reads its span's tags, not its rows

A bisection step reads a row by id: `Rows::get` finds the part, the chunk, the sealed test and the runs' high bits,
then makes one dependent read. A point probe makes about 8 of these: 6 in the fence's 64-id span, and 2 or 3 to gallop
to the end. Instead, the order keeps each id's 16-bit tag (the hash setfix's tagged runs already keep for the dedup
order, 2 bytes a row). A point probe then searches the fence, compares the span's 64 tags (one contiguous read) and
reads only the matching rows, which the probe reads anyway for their columns. A range probe keeps the bisection,
through a column resolved once a part (`ColRead`), not `get`. Check: the order's searches against a scan, in
`rerail-blocks` and setfix's run tests. Measured: the lab (`text`, `node`, a derived relation), then step 1's lines
for `text`, `kind` and `nameref`. Predicted: a hot point probe of about 60–80 ns (a fence search, one span, one
read), from 204 ns.

### Step 5: the layers above a probe allocate nothing

`Layered` collects a `Vec` a probe (its `filter`), and asks its parts per probe. Both are read in place, as
`lookup_read` is. Measured: the lab's layered minus bare (30 ns now).

### Steps 3 and 4, done (241c856165, aaaf1bb01c)

The orders' search is one module, `rerail_blocks::order`: the fences, tags, prints, their spans and `find`. setfix's
runs call it. The batch's relations keep one order a probed column in their block (`BlockVec`s: rows in value
order, none for a column pushed in order; fences; a print a row), searched by it. The prints were setfix's already
(derived orders are printed by default), so step 4 needed no new mechanism: the batch's relations had stayed off it.
- The lab's `text` probe: 209 to 60.9 ns, and 102.5 ns in a second run on a busier host. `node` stayed at 48–50 ns
  throughout.
- On `all` (m3 at load 104): the rows were unchanged, and `text`'s line went from 537 to 399 ns a row.

### Step 5, measured: small

With values admitted, a layered relation's probe asks its parts (`Values::parts`' load, the name's lookup, `Parts::of`):
30 ns in the lab, about 290 samples of a fresh 81,000-sample sample of `all`'s per-file window (head `aaaf1bb01c`, m3).
The `Vec` under `Layered` in the older sample was the trait's default `filter_eq`, a constant-filtered view's rows
found once per view (`View::filtered`), not per probe. Left as it is until it is a larger share.

### What the per-file joins spend now

The fresh sample's self samples under the joins' walk (`run_join_in_each::enter`, 21,247 inclusive):
- the join's own loop (`cands::candidates` 1,531, `enter` 1,256, `RowRef::at` 852, `Cands::any` 373,
  `Rows::each` 240, …): about 4,500;
- an id's way to its place (decision 0640 B: `Projected::local_of` 786, `Projected::node` 784, `Ranks::select` 631,
  `Numbering::granule` 359, `Batch::row_of_node` 231, `Table::low_of` 206, `Batch::offsets` 188, the node hint 164,
  `Viewed::local_of_row` 177): about 3,500;
- sealed chunks decoded (`IntCol::get`): 1,360. About 1,000 of them are under `cands::candidates`: a derived
  relation's rows read by its probe's search and again by the step's read of the row. The batch's relations' share
  is now about 190.

These are the next levers, in the order of 0640's "What follows": the loop's per-candidate cost, then B.

### W against the head, same host (2026-11-24, m3, load 79, `all`, CPU, unprofiled)

| | CPU | wall | instructions | peak |
|---|---|---|---|---|
| W (`bin-prof`, 6970a3e539) | 50.4 s | 34.1 s | 491 G | 1,044 MB |
| head (`c1596b8fba`) | 151.9 s | 57.4 s | 1,579 G | 2,195 MB |

The findings' rows are W's (the head adds its debug relations). `tmp/refs/three/wvh.sh` runs both, one after the
other.

### Step 7: the join's own loop, measured by parts

The lab `setfix::loop_lab_tests::a_joins_candidates_cost` (an ignored test) runs three joins over derived relations
(sealed rows, the chains their waves kept), shaped like the step table's heaviest lines: a membership on two
columns, and a lookup by one column, with one and with eight rows a key. It prints each step's CPU a row, the
index path each probe took, a probe's reads (`runs::probe_reads`, counted in its first round alone), and the
one-row lookup taken apart on one thread: the order's search alone, `candidates`, the ids walked, and the rows read.

At 50,000 keys every probe of the lab is a kept chain's (path 2), tagged by its lead's hash. A probe costs 35 ns of
search, 20 ns of `candidates` around it, and 30 ns to read its row, against 10 ns for that read by hand. The step
then read and compared the key's columns again, which the order's search had compared already.

**A (done): an order's leading key columns are exact.** A probe of an order whose leading columns are key columns
finds only rows that hold the key there (`cands::exact_cols`). Those columns are now neither read nor compared
again, as a view's exact columns were not. In the lab (warm round), the membership went from 117–139 to 93 ns, the
one-row lookup from 128–148 to 118 ns, and the eight-row lookup from 548–562 to 377 ns a probe (47 ns a row out, from
69). On `all` (m3, profiled): the rows were unchanged, and `kind / lookup on (c0)` went from 267 to 240 ns; the rest
moved little.

**Why `all` moved little: the lab's scale.** At a million keys the lab costs what `all` does (290–340 ns a probe),
and the search alone is 161 ns. A probe compares 7.4 rows and reads 17.5 tags, from 2 and 8:
- A 16-bit tag is shared by about 15 keys at that size, and its span is bisected: about 7 compares, each a sealed
  read of a far row.
- The scan sorts its rows by the next step's key (decision 0112's sorted batch), but a tagged order is sorted by
  the lead's hash: each probe's gallop from the last one's place crosses the array.

**B: the probes come in the order's order.** A scan feeding a tagged order sorts its keys by the lead's tag first, so
the gallop moves forward a short way each probe.

**C: an order prints its leading key columns.** Its prints cover its first `p` columns (the shortest key set it
serves), not the whole key or nothing, so a span's rows are found by their prints and a longer key bisects only
within them. This reaches the kept chains and the dedup order, which were never printed, and the ranged orders.

Predicted, at a million keys: the search from 161 to about 50 ns. Each is measured alone, in the lab at both
scales and then on `all`.

**B was in place already.** `scan_order` sorts a scan's keys by the lead's tag first when the next order is tagged
(`key_cmp`). The lab's single-thread search had sorted its keys by value: 139.5 ns a probe, against 101.5 ns by tag
(1M keys). Both read 7.4 rows a probe: the cost is the compares, sealed reads of rows far apart, not the tags, which
lie next to each other.

**C (done): an order prints its lead, and a whole-key order its key.** `rerail_blocks::order::within` searches a span
by the prints of the order's first `printed` columns where the key holds them, and bisects a longer key within their
span alone. The relations' orders (`Rel`'s first, `Rel::want`'s) print their lead (`Index::led`); a derived input's
own order prints its key columns, a ranged one's too (`Index::printed_on`). A byte a row an order.
- The lab at 1M keys: rows compared a probe 7.36 to 1.02 (one-row lookup), the search by tag 101.5 to 33 ns
  (predicted about 50). The steps: membership 253 to 188 ns, one-row lookup 292 to 202, eight rows 612 to 425 a
  probe.
- On `all` (m3, profiled; c21 at load 7 against c20 at 58): the same rows; 1,664 to 1,632 G instructions; the peak
  2,368 to 2,345 MB. The step lines this change does not touch (the views') fell 10–20% with the load alone; past
  that, `kind / anti-join` 377 to 292 ns, `kind / lookup on (c0)` 240 to 188, `nameref / lookup` 205 to 134,
  `rf_expr / lookup` 1,185 to 553. `kind / membership` 221 to 181 is within the load's effect.
- Open: `text`'s line rose 385 to 688 ns in c21, whose chunks ran more at once (wall 31 s against 81). Its search is
  the same call as before (one column, printed on it), so something outside the read lands on its step at that
  load. Looked at next.

### Step 6: `wstart` priced again

The answer that `wstart`'s rows are cheaper stored than inlined priced its readers' reads by the step table. It is
redone with step 1's read costs and reported, whichever way it goes.

### Not in the plan

- Writing the batch's relations in node order. The in-place search it allows costs as much as step 4's. It is
  revisited only if step 4 measures short, with the sort's cost counted in the make.
- More micro-changes to `Rows::get` before step 4: it leaves `get` out of a point probe's search.
