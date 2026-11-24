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

Check: the rows are unchanged. Measured: the per-file window's wall time and CPU, and the wait samples.

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

### Step 6: `wstart` priced again

The answer that `wstart`'s rows are cheaper stored than inlined priced its readers' reads by the step table. It is
redone with step 1's read costs and reported, whichever way it goes.

### Not in the plan

- Writing the batch's relations in node order. The in-place search it allows costs as much as step 4's. It is
  revisited only if step 4 measures short, with the sort's cost counted in the make.
- More micro-changes to `Rows::get` before step 4: it leaves `get` out of a point probe's search.
