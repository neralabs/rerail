# 0209: Statistics are one flat table, made in one batch, and keep a column's heavy values

Status: design (2026-11-14). Measured gap below; build order in §6. One question is open to the user (§5).

## Intro

The planner prices a join from each column's statistics. For each relation and column it has the rows, the
distinct values (*ndv*), and the sum of squared value counts (*sq*, whose ratio to rows is the column's fanout).
The user asked for two things (2026-11-14):

- **One batch.** All of a round's statistics are made in one GPU command launch, and in one batched CPU operation.
  That needs the planner's structure adapted, and the data behind statistics flattened.
- **Skew and high cardinality.** For some shapes on the critical path, more data is needed, especially for skewed
  columns and for columns of more than 64 distinct values. How should those be planned?

This decision answers both. Statistics become one flat table of numbered relations and columns. A round's
requests are made in one pass, on the CPU or the GPU. Each column that matters keeps its *heavy values*, its most
common values with their counts, so that a skewed join is priced from what it actually joins.

## 1. Where statistics are today

**The layout** (`plan::Stats`):
- per relation, per column, by name: rows, ndv, sq and sqd, in nested hash maps keyed by strings;
- frequencies of constants, fanouts, and delta rows in further maps keyed by (name, column);
- the planner's hot loop, which looks up an atom's columns, builds a string key for each lookup.

**The computation**, one relation after another:
- a request batch (decision 0123) that batches the asking, not the counting;
- the engine's sketches, kept as rows come (`InRel::kept_stats`): a 256-register HyperLogLog for ndv and a
  256-counter count sketch for sq;
- an exact count of each value only up to 64 values (`SMALL`), a dense window of 4,096, or a sparse map of
  1,024. Past those, no value's count is known.

**What it costs now.** Since kept statistics (0123 §5), a round mostly finds its statistics made. What is left:
- full counts when kept statistics are patched (0.32 s of an edit's 0.38 s, for `kind` and `docof`, decision
  0145);
- a cold run's lazy statistics (0.6 s of 1.6 s, decision 0117);
- statistics reads in estimates (most of 0.34 s per rule, decision 0132).

On the 215-file fixture, statistics are 0.15% of the run's own time. The cost is at scale, and it is the cost of
one relation at a time over string keys.

**What it misses.** The larger gap is accuracy, not time (§2).

## 2. The measured gap: skew past 64 values

Take the self-join seed of decisions 0206 and 0207, `call_ret(f, n) :- inf(f, n), …, inf(f, m), …, inf(f, k), …`.
It joins three atoms of `inf` on `f`, the function each node is inside. On the 215-file fixture:

| | |
|---|---|
| Functions (ndv of `f`) | 586, past the 64-value tier |
| Rows of `inf` | about 21,100, a mean of 36 a function, a largest of 707 |
| Largest 64 functions' share of rows | 62% |

Estimates of joins on `f`, as a share of their true size:

| Statistics | Two-way | Three-way |
|---|---|---|
| ndv only (uniform) | 0.19 | 0.02 |
| sq (kept today) | 1.00 | 0.51, from the tail's sq |
| Top 16 heavy values, uniform tail | 0.77 | 0.86 |
| **Top 64 heavy values, uniform tail** | **0.98** | **1.00** |
| Top 256 heavy values, uniform tail | 1.00 | 1.00 |

sq is a second moment, so it prices a two-way join exactly. A three-way join on one key needs the third moment,
and no kept statistic has it: the price was 56× low under ndv alone. That is why the planner priced the running
order as fine while another did a quarter of its work (0207).

64 heavy values a column make both exact here. They are the most data per column the planner needs for this
shape.

## 3. Decision: one flat table

All of a run's statistics live in one table (`plan::stats::Table`):

- **Relations** are numbered once per model: `RelId(u32)`. A relation's row holds its rows, its delta rows, its
  first column's number, and its width.
- **Columns** are numbered `ColId = first + c`. Their statistics are arrays indexed by `ColId`:

  | Array | Meaning |
  |---|---|
  | `ndv`, `sq`, `cube` | distinct values; sum of counts squared; sum of counts cubed |
  | `level` | how much is kept for the column (§4) |
  | `heavy` | (offset, length) of its heavy values in the arena |
  | `tail_rows`, `tail_ndv` | the rows and distinct values outside the heavy values |

- **The arena** holds every column's heavy values as two arrays, values and counts, each column's sorted by count.
- **Exact pair statistics** (decision 0203 §3) stay where they are, keyed by rule and atom signature.

A body's atoms are resolved to their `ColId`s once, when the body is prepared. The hot loop then reads array
slots, with no string built and no hash taken. `Stats`' maps become views of this table during the move, then go.

## 4. Decision: heavy values, kept as much as a column needs

Each column has a level. A column is raised only where a line needs it.

| Level | Kept | For |
|---|---|---|
| 0 | ndv, sq, cube (sketches) | every column |
| 1 | the top 64 heavy values, and the tail's rows and ndv | the join and filter columns of lines on the measured critical path (decision 0208) |
| 2 | the top 1,024, and a count-min sketch of the tail | a column whose top 64 leave over 20% of its rows in the tail, with its line still mispriced |
| 3 | exact pair statistics (0203) | a line still over 2× its price after level 2 |

**Raising a level.** The memo (0203 §6) raises a column when its line is on the measured path and either:
- its order measured over 2× its price, at a run or in a trial (0207);
- or the column is skewed past 64 values: ndv over 64, and sq · ndv / rows² over 2.

The level is kept with the line's note, so the next run makes the column at that level from the start.

**Keeping them up to date.** Heavy values are kept as a Misra–Gries summary of 2k counters. Summaries of two sets of
rows merge into one, so a delta's summary is merged into the kept one: no full count.

**Pricing with them.** For an equi-join of `a.x` and `b.y`:

- values heavy in both: Σ count_a(v) · count_b(v);
- values heavy in one only: their counts times the other's tail rows over its tail ndv;
- the tails: tail_rows_a · tail_rows_b / max(tail_ndv_a, tail_ndv_b), by containment.

A join of more atoms on one key multiplies the same way, value by value, so a three-way self-join is priced from
Σ count³ over the heavy values, plus the tail's share. A bound constant is priced at its own count where it is
heavy, else at the tail's rows over its ndv.

## 5. Open to the user: heavy values split the plan, or only its price?

Heavy values make the price right. They can also change the plan's shape. A rule can run as two orders: one for
its heavy key values, one for the rest. The engine already has the step this needs, a slot's membership in a set
(`Step::In`).

- **Price only.** One order a line, priced from heavy values. Simple, with nothing new in the engine. A line whose
  heavy and light values want different orders takes the order that is best on the whole.
- **Split plans.** Where a trial (0207) shows the best order differs between the heavy values and the rest, the
  line runs as two lines under one rule, split by `Step::In`. That means two specs, two notes and two kept plans.

My recommendation is price only first, then split plans where trials show a difference worth it. On the fixture,
price alone fixes the case measured (§2).

## 6. Decision: made in one batch

A round's requests (decision 0123) become one list: (relation, columns, level) per relation. They are made in one
pass.

**On the CPU** (`stats::make_batch`):
- one scan per relation, reading all its requested columns;
- for each row and column, the HyperLogLog, the count sketch for sq and cube, and the Misra–Gries counters at the
  column's level;
- the relations of a round spread over the pool;
- one write of the results into the table's arrays.

This replaces per-relation, per-column passes (`Deferred::statistics`, `Stats::measured`).

**On the GPU**, one command buffer for a whole round. It is built from the joins' kernels, as their fused sort
(`sort_all`, decision 0100 §9) is:

1. A job table, one job per (relation, column), each a segment of a resident column (device joins already keep
   relations resident).
2. A segmented radix sort of each job's values (`sort_init`, `sort_hist`, `sort_scatter`, per job).
3. Each segment's runs (`run_heads`, a scan, `run_compact`). A run's length is a value's count, so: ndv is the
   number of runs, sq the sum of lengths squared, cube the sum of lengths cubed.
4. The top-k runs by length per job (a segmented select on the run lengths): the heavy values, exact.

The whole round is one commit, and the host reads one output buffer. The GPU's heavy values are exact. The CPU's
are a summary, exact for the top values whenever their counts exceed the summary's error (rows / 2k).

**Where each runs.** The GPU batch runs where the round's relations are resident and the batch is past a size where
a launch pays, as the device joins decide (decision 0095). Otherwise the CPU's batch runs.

## 7. Build order

1. **The table and its ids.** Relations and columns numbered; ndv, sq and fan moved into the arrays; the hot loop
   reading slots. Planner tests unchanged.
2. **The CPU batch.** One pass per relation over its requested columns, with cube and level-1 heavy values. The
   kept statistics merge deltas' summaries.
3. **Pricing with heavy values** (§4), and the memo raising a column's level for lines on the measured path.
   Measured on the self-join seed: its price against its trials.
4. **The GPU batch**, as one command buffer, checked against the CPU's batch on the same relations.
5. **Split plans**, if the user wants them (§5).

## Open

- **O170, the move from `Stats`' maps.** Every reader of `Stats` moves to the table. Until then the maps are views
  over it, and the planner's tests must hold unchanged at each step.
