# 0622. A relation's sealed chunks take the frozen columns' encodings

Status: steps 1 and 2 built (2026-11-22); steps 3 and 4 measured and not kept (below). t52 step 5: written before it was built, as t52's plan
asks. t52 named it 0605; it is numbered past the highest record in any worktree (0621).

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

## As built (steps 1 and 2)

- **Where it seals.** `Rows::seal` (`rerail_blocks::rows`) seals a relation's full chunks below its rows in a block with
  a free list. The engine calls it when it freezes a run's rows (`facts::rows::Rows::freeze`, `seal_segments`), for a
  segment that it alone holds, that continues no base and is not a device's. `RERAIL_SEAL_CHUNKS=off` leaves every
  chunk raw.
- **The tag** is bit 0 of the chunk's offset (`rows::SEALED`). `ColDir::get`, `low`, `lower_bound`, `read_col` and
  `release` decode or free a sealed chunk. `at`, the writers' address, asserts the chunk is raw, and so does a scan of
  raw chunks (`col_chunks_u32`).
- **CLIF.** `col_get` tests the bit and calls `col_sealed`, which decodes the five encodings in CLIF with no
  high multiply (the kernels' backend has none). `LINE`'s `(i * slope) >> 16` is split at the slope's low 16 bits.
  The kernels' `relation.h` is generated from it. Tests compare each reader with the host on sealed rows: the JIT
  (`the_jits_sealed_chunk_reads_are_the_hosts`), and the device (`the_device_reads_a_relation_as_the_host_does`),
  on a Linux host's device and on Metal (an M3 farm Mac).
- **Read at random, chosen for it.** A relation's column is read by row id, at random. The first build chose by size
  alone. It sealed 1.56 GB of chunks into 0.81 GB and lowered `all`'s peak from 3.23 to 2.96 GB, but cost 9% more
  instructions (1,842 G to 2,004 G, the same binary and host): the joins' reads. One random read, against about 5 ns
  raw, was 88 ns for `RUNS` (a search of its run starts), 22 to 24 ns for `LINE` and `GROUPS` (a decoder made per
  read), and 61 ns for a 64-bit column sealed to save a few percent. So:
  - `IntCol::get` decodes `LINE` and `GROUPS` from its own fields, as `PACK`.
  - A sealed chunk is chosen by `col::choose_for_reads`: `RUNS` only with at most 32 runs (`RUNS_FOR_READS`, a
    5-step search). `GROUPS` holds long runs about as small, and is read in three words. The kept facts keep
    `choose`.
  - A chunk is sealed only where it saves a quarter of its bytes.

  Then, on an idle host, a random read takes 1.5 ns (`CONST`), 3.5 ns (`PACK`), 4.2 ns (`LINE`) and 4.6 ns
  (`GROUPS`), against 5 ns raw: a smaller column is read from closer caches (`examples/sealed_reads.rs`).
- **On `all`, as built** (an M1 Max farm Mac, the same binary with `RERAIL_SEAL_CHUNKS=off` and on): 0.99 GB of chunks
  sealed into 0.34 GB. Peak 3.18 GB to 2.97 GB. CPU 181.8 s to 178.2 s. Instructions 1,828 G to 1,892 G (+3.5%,
  down from +9%). s1000's findings unchanged (`373cea39`), and every relation's rows the same.
- **Why the peak falls by 0.2 GB, not 0.65 GB.** A freed raw chunk's pages go back to the system (`free_chunk`), so
  the saving is real. But it is spread over the run. The peak is at the run's end, while the outputs are written and
  the store writer stages a gather's rows. Most relations sealed by then are gone (`RERAIL_HEAP_SAMPLE=1`'s account
  at the peak: 920 MB of relations' blocks, 1.3 GB of heap).
- **Step 3, measured and not kept.** Sealing a tail's chunks as its writer passes them (`extend_from_slice`) sealed
  the same 0.99 GB into 0.34 GB, only earlier. Peak 2.99 GB against 2.97 GB at freeze, with 1,929 G instructions
  against 1,892 G and CPU 199.0 s against 178.2 s: the tails are read sealed while they are written. So a chunk is
  sealed at its relation's freeze only.
- **What holds the peak** (2.97 GB at freeze): the cross part's `text` rows made again by the corpus batch (`copy
  (0353 step 3)`, 323 MB of heap); a kept block of `ask_p0`'s cone (472 MB); the cone's views and tables (273 MB);
  `rf_in_loop`'s heap (111 MB). The first is 0353 step 3's: `text` read on the tape for each node bound, not every
  document's node rows made (t50, "The cross part's `text`"). Built 2026-11-23 (t50, "Then the tape view"): the peak
  2,837 → 2,474 MB.

- **Step 4, measured and not built (2026-11-23).** A census in `Rows::seal` on `all` (m1max, a temporary aid): for
  each chunk it considers, its raw bytes, what it keeps now (sealed or raw), and what a dictionary of the chunk's
  distinct values would take (8 bytes each, and each row's index at ceil(log2 distinct) bits). Of 1,408 MB raw: kept
  893 MB; a dictionary alone 1,141 MB; the better of the two per chunk 775 MB. A dictionary would save 118 MB (13%)
  of the run's sealed volume, spread over the run. The peak saw 0.2 GB of step 1's 0.65 GB, since most sealed relations are
  gone by then, so its gain at the peak is a few tens of MB. Every reader (host, CLIF, the kernels' header) would
  pay a dictionary load per read. Kinds already pack to their range (`PACK`). Not built.

- **Its cost, measured again (2026-11-23, t53 §4).** On `all` at the head (m1max, the same binary): with
  `RERAIL_SEAL_CHUNKS=off`, instructions 1,644 G → 1,561 G (−5%), CPU 169 s → 165 s, the peak 2,433 → 2,586 MB
  (+153 MB), the rows the same. `IntCol::get` is the second busiest function in a sample of s1000's joins. So sealing
  buys 153 MB at the peak for 5% of the run's instructions; a dictionary would add a dependent load to the same reads.

## Test

For each step: a relation's rows read through every reader equal the raw layout's; the findings on `all` are
unchanged; and the census's "columns released" and the run's peak are noted against t52's figures.

## Why

- The peak is the chunks' columns at a fixed width. t52's census puts the frozen form at about half of the values'
  bytes, sorted.
- The reads stay O(1) arithmetic on words, the same on a GPU, where a column is read for its bandwidth.
- A layout read in place by every reader is 0353's rule. A packed copy beside the raw one would double what it set
  out to halve.
