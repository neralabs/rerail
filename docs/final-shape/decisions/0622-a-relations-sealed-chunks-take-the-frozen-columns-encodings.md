# 0622. A relation's sealed chunks take the frozen columns' encodings

Status: decided, not yet built (2026-11-22). t52 step 5: written before it is built, as t52's plan asks. t52 named
it 0605; it is numbered past the highest record in any worktree (0621).

## Intro

The engine has two layouts for a column of integers:

- **The working relation's columns** (`rerail_blocks::rows::ColDir`): each a run of chunks, rows `2^min_log` and
  then doubling, at 4 or 8 bytes a value. A 4-byte column may hold the low 32 bits under key runs, the high bits a
  run each (one a document, 0494 step 7a). Every reader reads it in place: `ColDir::get`, CLIF's
  `col_get`/`col_at`/`col_high` (0325), the JIT, and the kernels' `relation.h`.
- **The frozen columns** (`rerail_blocks::col::IntCol`): a column written once in the fewest bits it needs. It is one
  of `CONST`, `PACK` (frame of reference), `RUNS`, `LINE` (a slope and a bit-packed residual) or `GROUPS` (128 values
  a group, each with its own minimum and width, 0456 K2), chosen by exact size (`col::choose`). Only the kept facts
  use it (`kept.rs`, `packed.rs`, `ranged.rs`).

t52's census measured what the working relations would hold in the frozen encodings. On `all`, ranges of 4,096
rows: 4.2 GB as written, 2.15 GB sorted, against 10.1 GB of columns then. Since t52 steps 1–4 and t50's fixes, the run
peaks at 3.2 GB. Most of that is chunk relations' columns at 4 or 8 bytes a value, whose values span far less than
that.

## Prior art, read first-hand

MLDB's `plugins/tabular` (`frozen_column.cc`, `frozen_tables.h`, `frozen_tables.cc`, read 2026-11-22):

- Formats are chosen per column, at its freeze, by the smallest estimated size. No cost function: `bytes < bestBytes`.
  The formats are direct, table (dictionary, `indexBits = bitsToHoldCount(uniques)`), sparse table, integer and
  double.
- `FrozenIntegerTable`:
  - Default: offset = min and slope = 0, in `bitsToHoldRange(max - min)` bits.
  - A non-decreasing column instead takes slope = `(last - first) / (n - 1)` and offset = `min - maxNegOffset`,
    where that needs fewer bits. Each residual `v[i] - (offset + trunc(i * slope))` is bit-packed.
  - A read is `trunc(i * slope) + residual + offset`: one multiply and one bit extraction, O(1).
- The format is fixed per column, and each column covers one row range. Its size estimate for the integer table
  leaves out the slope ("TODO: calculate with slope").

`IntCol` already has these, and `GROUPS` beside them (MLDB has no per-group width). What MLDB has that the
engine's working layout lacks is the step this record decides: **its columns are frozen, and a read decodes them where
they lie.**

## Decision

1. **A chunk is sealed once full.** A relation's chunk that its writers have passed (its rows claimed and written,
   `Rows::len` past it) is sealed. Each of its columns is encoded by `col::choose` into an `IntCol` in the same block,
   and the chunk's offset in `ColDir::chunk[k]` points to it. A tag in the offset's low bits says which: the raw
   chunk, or its `IntCol`. The raw chunk's memory goes back to the block's free list (`free.rs`). A chunk still being
   written stays raw.
2. **One layout, read in place.** There is no second copy and no packed shadow (0371's mistake, and 0353). A sealed
   chunk is the column's only bytes. Each reader decodes it where it lies:
   - `ColDir::get` on the host;
   - CLIF's `col_get` (0310: the reads written once, so the JIT and SQLite's virtual tables follow);
   - the kernels' generated `relation.h`.

   All of them branch on the chunk's tag, then do `IntCol`'s arithmetic.
3. **A node id column keeps its fields.** Key runs (0494 step 7a) stay the document's part. Below them, the low 32
   bits (slot and index) are what `col::choose` encodes: `LINE` or `GROUPS` for ids in canonical order, `PACK` for the
   others.
4. **Relative columns are a later step.** A column stored as another's difference (column 0's document for every
   node column) is not taken here. It is measured after step 1 of the build, by the census, before it is decided.
5. **Dictionaries later too.** Atoms and kinds as a dictionary per chunk (MLDB's table) follow once steps 1–3 are
   built and measured. `IntCol` has no dictionary encoding yet.

## Building it (t52 step 6), each step measured by the census on `all`

1. Seal and encode the chunks of a frozen relation (`relation::freeze`): the canonical order's columns, whose values
   are sorted and small.
2. Every reader decodes a sealed chunk. Each is tested against the raw reads on the same rows: host `get`, CLIF
   `col_get` (JIT and interpreter), and the kernels' header (one filtered run on a farm Mac).
3. Seal a chunk relation's full chunks as its writers pass them, not only at freeze.
4. Dictionaries for atom and kind columns.

## Test

For each step: a relation's rows read through every reader equal the raw layout's; the findings on `all` are
unchanged; and the census's "columns released" and the run's peak are noted against t52's figures.

## Why

- The peak is the chunks' columns at a fixed width. t52's census puts the frozen form at about half of the values'
  bytes, sorted.
- The reads stay O(1) arithmetic on words, the same on a GPU, where a column is read for its bandwidth.
- A layout read in place by every reader is 0353's rule. A packed copy beside the raw one would double what it set
  out to halve.
