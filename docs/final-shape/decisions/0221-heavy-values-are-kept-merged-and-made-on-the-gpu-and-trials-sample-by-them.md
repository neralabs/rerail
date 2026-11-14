# 0221: Heavy values are kept with notes, merged from changes, made on the GPU; trials sample by them

Status: built (2026-11-14). Closes O171 and O172 of decision 0216: a trial's sample drawn apart from a skewed
key's heavy rows (`Run::heavy_parts`); raised columns kept with their line's note in the store (`rr_join_raised`);
a relation's change merged into its heavy values (`HeavyCol::merge`, `InRel::mark`, `delta_since_mark`); and the
GPU batch of decision 0209 §6 (`rerail_accel::gpu::join::column_stats`).

## Intro

Decision 0216 made a column's heavy values by a full pass over its relation, when a checked line raised the
column. It left four things open (2026-11-14), and the user asked for them to be addressed:

- a trial's sample, drawn at a stride, mostly missed a skewed key's heavy rows (O171);
- a relation that changed was counted again in full;
- a raised column was forgotten at the process's end;
- the GPU batch was not built (O172).

## Decision

### 1. A trial samples a skewed key's heavy rows apart (O171)

At a line's trials, where its key has heavy values (`Run::heavy_parts`), the rows its trials draw from
(`setfix::trial_ids`) are split in two:

- rows whose key is one of the 64 heavy values;
- the rest.

The key is either a column of the lead atom, or reached through the one atom that holds both it and a variable of
the lead: `inf(f, m)` from `kind(m, return)`.

**The sample.** A quarter of the sample, at least 2 rows, comes from the heavy part, and the rest from the other,
each at a stride (`setfix::trial_over` over the rows named). Each order runs over both parts, each part under half
the trial's cap and its own cutoff. Its work per row is the two parts' means, weighted by the parts' rows. An order
is cut if either part was.

**Tried and dropped:** half the sample from the heavy part, each part under the whole cap. The running order's
heavy part passed its cap, every order was cut, and none could win.

### 2. Raised columns are kept with the line's note

A note keeps the columns its line raised (`Note.raised`). The memo's store entry has a third relation,
`rr_join_raised`, beside its notes and exact pairs (decision 0203 §6). A process that reads the notes raises
their columns, so a stratum's first plan makes them (`heavy_for` before seeds and probes).

A trial raises before `put_tried` notes its line, so `raise` makes the note; `put_tried` sets its rule after.

### 3. A change is merged, not counted again

A column keeps up to 128 counters (`table::KEPT`, twice the 64 reported, as a Misra–Gries summary's 2k).

**What is kept of the relation.** Its mark, as the heavy values were made over it (`InRel::mark`): its address,
rows and dead bits. The planner does not hold the relation itself, which may be large.

**When it changed** (`InRel::delta_since_mark`, as `delta_since` but from a mark), the change's rows are counted
per value and merged (`HeavyCol::merge`):

- a kept value's count moves by its change, so it stays exact where it was;
- a value the change adds that was not kept comes in at what it added;
- the 128 largest are kept, and the rest are the tail;
- `err` is the most a value left out held.

sq and cube move value by value, a tail value's taken at the tail's mean. Where no change is known (a relation not
started from the marked one), the column is counted again.

**When they are current** (`Run::heavy_current`): made or merged over the relation as it is now. A relation the
engine does not hold is current within a tenth of its rows, as before.

### 4. The GPU batch (0209 §6)

`column_stats(cols, k)`: every column of a round in one command buffer.

- For each column, the joins' device radix sort with its runs (`encode_sort`). A run's length is a value's count.
- `col_stats`, one threadgroup a column, writes each run's length, and reduces their count, squares and cubes.
- A second radix sort of the lengths, and `col_top`, gives the `k` longest runs' keys and lengths.
- The host reads `3 + 2k` words a column.

It is exact, as the CPU's batch is.

**Where each runs** (`table::heavy_batch`): on the GPU from 1 M rows of a round (the columns' rows summed), where
there is a device. Otherwise on the CPU. `RERAIL_STATS_GPU=on` or `off` chooses. Off macOS, the CPU always runs.

## Measured (2026-11-14, debug build, strict)

**Tests.**
- `accel` `gpu::tests`: the device's statistics of a skewed wide key, a narrow one, one value and none, in one
  command buffer, are the exact counts.
- `plan::table`:
  - the GPU batch makes the CPU's heavy values;
  - a merged change counts a kept and a new value exactly, and prices a three-way self-join within 2% of a full
    count.
- `plan::choices`: a note's raised columns are read back from the store, and raised in the process.
- Planner 120 pass, engine 50.

**The two batches, warm** (one column, half its rows on 600 values; debug, so the CPU's hashing is unoptimized):

| Rows | GPU | CPU |
|---|---|---|
| 1 M | 24.9 ms | 154 ms |
| 8 M | 204 ms | 1,448 ms |

**Smoke** (the self-join seed of decisions 0206 and 0207; 545 findings in every run):

| Sample | Rows of heavy keys / the rest | The winner's work per row, estimated | Share of the line's 874 |
|---|---|---|---|
| At a stride (0216) | 9 rows, unsplit | 492 | 0.56 |
| By heavy values | 2 of 75 / 7 of 252 | 593 | 0.68 |

- The trials' work went from 39,874 to 30,256 rows.
- The line's work was 385 k, unchanged: the same winner.
- `RERAIL_STATS_GPU=on` makes `inf.0`'s heavy values on the GPU, with the same trials and findings.

## Open

All three addressed by decision 0223 (2026-11-14); the first stays open as O173.

- The estimate is still under the line's work. Each part's sample has few rows, and a heavy row's work varies
  with its function's size. A larger heavy share needs a larger cap; neither is tuned.
- The GPU batch uploads the columns from the host. Columns the device joins already hold could be read in place.
- `col_stats` is one threadgroup a column; a column of tens of millions of rows wants it spread.
