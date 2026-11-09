# 0096: Planner statistics are kept in the store, maintained by the deltas that change the rows

Status: proposed (2026-11-09). Design only; nothing built. The approximate variant (last sections) is recommended over the exact one.

## Intro

The planner chooses join orders from statistics. For each relation it uses the row count. For each column it
uses four numbers: rows, distinct values, the sum of squared postings, and that sum within documents. A few-valued
column also gets each value's count, and the tree gets kind and edge counts.

Today a run measures these afresh from rows in memory:

- a memo hit replays its file's rows, partly so that they can be counted;
- a changed phase export is measured again whole;
- a reader's bound relation is sorted per column to measure it (`syntax::statistics`).

So a one-file edit pays for counting every file. This is the last O(repo) cost on the edit path, and it blocks
skipping an unchanged file's replay (the per-file cross store). Replay is the statistics feed.

This record keeps the statistics in SQLite next to the rows they describe. The same delta that changes the rows
also changes the statistics, in the same transaction. A run reads totals; it counts only the rows that changed.

## What the planner needs, and why every part can be maintained

Every statistic the planner reads is a function of per-value counts:

```text
n(v)    rows whose column holds v        (whole relation)
n_d(v)  the same, within document d

rows = Σ_v n(v)
ndv  = #{v : n(v) > 0}
sq   = Σ_v n(v)²                          // fanout of an equality probe = sq / rows
sqd  = Σ_d Σ_v n_d(v)²                    // the same, joins kept within a document
dfreq(v) = n(v)                           // few-valued columns only
```

Each one is updated in O(1) when a count moves from `a` to `b`:

```text
rows += b - a
ndv  += [b > 0] - [a > 0]
sq   += b² - a²                           // for ±1 steps: +2a+1 or -(2a-1)
```

`sqd` is a sum over documents, so a changed document changes only its own term. Kind and edge counts are plain
sums. No statistic needs a scan of unchanged rows. The anchor index already works this way: `value_frequency`
and `value_kind_frequency` are kept by triggers on `vidx` (spike D S241; `matcher/d_anchors/sql/schema.sql`).

## Decision

### 1. Three tables per store, beside the rows

```sql
-- The totals the planner reads: one row per (relation, column), at the version of the rows it describes.
CREATE TABLE stat_total(rel TEXT, col TEXT, v INTEGER,     -- v: the rows' version (export version, or epoch)
    rows INTEGER, ndv INTEGER, sq INTEGER, sqd INTEGER,
    PRIMARY KEY(rel, col)) WITHOUT ROWID;

-- Per-value counts, for value columns only (a node column's values never cross documents).
CREATE TABLE stat_value(rel TEXT, col TEXT, val INTEGER, n INTEGER CHECK(n > 0),
    PRIMARY KEY(rel, col, val)) WITHOUT ROWID;

-- One document's part: its additive terms, so replacing the document subtracts its old part exactly.
CREATE TABLE stat_part(rel TEXT, col TEXT, part INTEGER,     -- part: the granule / document
    rows INTEGER, ndv INTEGER, sq INTEGER, sqd INTEGER,
    PRIMARY KEY(rel, col, part)) WITHOUT ROWID;
```

How each kind of column is kept:

- **Node columns** (`d << 32 | index`). Values never cross documents, so all four numbers are sums of
  per-document parts. Only `stat_part` and `stat_total` are kept.
- **Value columns** (kinds, interned names, constants). `ndv` and `sq` come from global counts, so `stat_value`
  is kept. Only `sqd` comes from the parts.

Tree counts (kinds, edges, documents) get the same treatment as a relation `node` with parts.

### 2. Who writes them: the writer of the rows, in its transaction

Statistics are written where the rows are written, never by a separate pass:

| Where rows change | Delta at hand | Statistics written |
| --- | --- | --- |
| Phase `keep` | plus/minus rows per export, per granule (SQL) | `stat_value` ± per row, `stat_part` replaced for each changed granule, `stat_total` patched; `v` = the export's new version |
| Memo record of a file | the file's rows, already in hand when it is derived | its part (and its value counts) stored with the record |
| A document leaving or entering the run | its stored part | its part subtracted or added |

The phase case is pure SQL over the delta that `keep` already computes. It runs as one `INSERT … ON CONFLICT DO
UPDATE` per table, grouped by value, and is O(changed rows).

The memo case is different: its rows are not in SQLite. So the file's part is stored with its memo record. This
is a few integers per column, plus a value-count list for each value column, which is no larger than the file's
distinct values. A run then adds and subtracts parts for the files whose digests changed. A memo hit is never
read for its statistics. This is what makes the per-file cross store possible: an unchanged file is neither
replayed nor counted.

### 3. Delayed readback: versions, not timing

The decided model is that SQLite is durable, its writes may lag, and memory may be ahead. Statistics follow the
model with no new rule:

- **Same transaction.** A row delta and its statistics delta commit together. Whatever SQLite holds, its
  `stat_total.v` names exactly the rows it describes. A lagged write lags both together.
- **Memory is ahead.** The session holds the totals in memory, as the reader's sources are held today (`phased`:
  rel → version → source and statistics). It patches them with the same delta it hands the reader. The planner
  reads memory. SQLite is read only by a cold session, or after a restart.
- **The version decides.** A session compares the version it holds with the store's `v`. If memory's version is
  newer, memory is used. If they are equal, either may be used. If the store is newer (another process wrote),
  the store's totals are read. The digest of a reader's memo key already folds in the export versions, so the
  statistics are covered by the same key.
- **Missing means measure once.** A store without the tables, or a relation without a `stat_total` row, is
  measured from its rows once and the result is written (a backfill, as the anchor index's upgrade does). The
  answer is not to recompute on every run.

### 4. What stays measured in the run

Relations derived inside a run (setfix `measured`, `obs` fanouts) are measured by the run that makes them. They
are not stored here until derived relations have deletes in the engine core (0094 §4's open point). Then the same
identities apply to their deltas. Constants' postings (`Stats::memo`) are point lookups into `stat_value`.

### 5. Reading: the existing `stat_*` reader

The planner already reads statistics from SQL (`Stats::read`, `stat_meta`/`stat_col`/`stat_kind`/`stat_edge`/
`stat_val`, T's schema). `stat_total` and `stat_value` are served under that schema as views. One statement reads
a run's relations, as the spike's planning reads one batch (D S150, S241). The session's in-memory totals fill the
same `Stats` fields through `Totals::apply`, which stays as the single conversion.

## What this agrees with, and departs from

- **Agrees with the spike §4: no corpus-wide statistic.** The spike allows per-file statistics and running
  aggregates (`_accum_kind_freq`), and requires that statistics survive a restart (D S124; SORA C7). This
  record is exactly per-file parts plus running totals, persisted.
- **Agrees with the lessons ledger #4 and #5.** There is no ANALYZE and no temp-table statistics. The numbers
  are ours, exact, and kept by delta.
- **Agrees with the anchor index (D S241).** Its frequencies are kept by triggers in the rows' own transaction.
  Here the writer writes them explicitly rather than by trigger, because the rows of a phase export are encoded
  per granule and a trigger would decode them again.
- **Departs from today's code.** `Totals::add_rows` measuring replayed rows, `Own::measure` per chunk mix, and
  `syntax::statistics` on bound exports all become backfill-only paths.

## Cost

- **Space.** Per relation and column, one total. Per document, one part row. Per value column, one row per
  distinct value. On the 9,010-file repo this is well under the rows themselves, and inside the 10 GB disk budget.
- **Time per edit.** O(changed rows) to patch, plus one read of the run's totals (a few hundred rows).
- **Measured so far** (struct plus reader, 84 files). Interning and measuring bound exports cost about 0.06 s
  before the export deltas. A reader edit now runs in 0.33–0.47 s in a session, and memo replay is in it. This
  record removes the counting from replay. Step 3 then removes the replay itself. Neither is measured yet.

## Order of work

1. Write the tables in phase `keep` from its plus/minus. Serve the reader's bound exports from them, with no
   `syntax::statistics` on an export.
2. Store a part with each memo record. Sum the parts in the session, and stop counting replayed rows.
3. Build the per-file cross store (rows in SQLite, keyed by file and program). An unchanged file is not replayed.
4. Derived relations, once the core has relations with deletes.

## Approximate counts: an exploration

Planner statistics only have to be good enough to choose a join order. They are pure overhead: no finding
depends on them, and the memo key does not include them (`memo::plan_digest` hashes the build, the grammar,
the node-id scheme and the program, and nothing else). So the question is what the cheapest structure is that
keeps the estimates within a small factor, supports deletes, and is bounded in size.

### What exact costs, measured

The struct phase's exports over 84 files plus typeshed hold 245,681 cells across 9 relations and 26 columns.
Exact per-value counts for them are 89,705 `(rel, col, value)` rows, so the statistics would be 37% of the data
they describe. They also grow with the repository. The skew is extreme: in `binds.c3` one value of 4,891 holds
3,137 of 11,534 rows, and that one value is almost all of `sq` (fan 864.6). Any structure has to get the heavy
values right and may be rough on the tail.

### The candidate: two linear sketches per column

A *linear* sketch is a fixed array of counters. Each value updates a few counters by +1 on insert and −1 on
delete. The sketch of a multiset is the sum of its values' updates, so:

- **Deletes are exact.** Removing a row undoes its insert to the bit. After any order of inserts and deletes
  the counters equal those of the final multiset. The sketch is history-independent, as "two loads equal one
  load" asks (spike D S124).
- **Parts add and subtract.** Two documents' sketches sum to the sketch of both. A document's minus is the
  negation of its old sketch.
- **The size is fixed** whatever the repository's size.

The two sketches:

```text
sq  (fanout = sq / rows): AMS / Count-Sketch.
    d rows of w signed counters; value v adds ±1 (sign from its hash) at column h_i(v) of row i.
    sq ≈ median over i of Σ_j t[i][j]²            // unbiased per row; the median tames outliers
    A heavy value lands whole in one counter, so it is never sampled away.

ndv: levelled counting bitmap (an L0 sketch).
    level(v) = trailing zeros of hash(v), so level ≥ L is a 2^-L sample of the distinct values.
    Each level has m counters; v adds ±1 at slot g(v) of its level.  // counters, not bits: deletes work
    ndv ≈ 2^L · (−m · ln(empty/m)) at the lowest L whose levels ≥ L leave ≥ 30% of slots empty.
```

`rows` stays an exact counter. `sqd` is exact too: a document's term `Σ_v n_d(v)²` is computed from its rows
when they are in hand, and subtracted from its old rows when it changes. A node column's `ndv` and `sq` are also
exact sums, because its values never cross documents.

### Accuracy on the same exports

The simulation is in `docs/final-shape/notes/0096-sketch-sim.py`. For each column it inserts every row, deletes
a third of them and inserts that third again. The sketch must come out as if built once. The ratio is the
estimate divided by the exact value:

| Sketch | Size per column | Fanout, worst ratio | ndv, worst ratio |
| --- | --- | --- | --- |
| AMS w=64 d=3; ndv m=64 | 768 B + ≤ 1.3 KB (≈10 occupied levels × 128 B) | 1.25× | 1.35× |
| AMS w=256 d=3; ndv m=128 | 3 KB + ≤ 2.6 KB | 1.12× | 1.41× |
| ndv from rows²/sq (no sketch) | 0 | — | 442× |

The ndv sizes count only occupied levels. The array has 32 levels, but those above about log₂(ndv/m) are
empty and are stored sparsely. The third row is the free option, which assumes uniform postings. It rules
itself out: skew is the normal case here.

Fanout errors of 1.1–1.25× and ndv errors of 1.4× are below the granularity at which these plans differ. The
join orders that matter differ by 10–1000× in fanout: compare `binds.c2` at 3,942 with `binds.c1` at 14.4. This
has not been measured on the planner itself. §"Validation" below says how to measure it.

### What stays exact

Some estimates are not served well by a sketch:

- **Few-valued columns (dfreq).** A column keeps exact per-value counts while it has at most 64 distinct
  values (kinds, flags, `def.c1`, `param.c1`). That is at most 64 rows, and the planner reads them value by
  value.
- **The program's constants (`Stats::memo`).** A constant compared in a rule is a point query. A Count-Sketch
  point query errs by about √sq/w, which can swamp a rare constant. So the constants the planner registers are
  a watch list with exact counts. A new constant costs one SQL count over its relation's rows when first
  registered. That is rare: the program changes, not the data.

### Exact versus approximate

| | Exact (`stat_value`) | Approximate (sketches) |
| --- | --- | --- |
| Size | grows with distinct values: 37% of the cells here | fixed: ≤ about 2 KB per column |
| Cost per changed row | one B-tree upsert per value column; a 0↔1 test for ndv | d + 1 counter updates in memory |
| Write to SQLite | one row per changed value, at random keys | one blob per changed column, rewritten |
| Cold run (every row new) | builds a table about a third of the data | fixed work per row; nothing grows |
| Deletes | exact | exact (linear), and history-independent |
| Memory in a session | one map entry per distinct value | fixed |
| Accuracy | exact | 1.1–1.4× on the measured exports |
| Few-valued columns, constants | for free | an exact side table (≤ 64 values; watch list) |
| Determinism | yes | yes, with fixed hash seeds |
| Restart (delayed readback) | rows tagged with a version | blobs tagged with a version; the same rule |
| Complexity | trivial | two small structures plus a switch at 64 values |

What decides it: the statistics are overhead whose only job is ranking join orders. Exact counts pay a cost
that grows with the repository, at a precision the planner does not use. The sketches make that cost constant.
The one thing exact gives for free, point counts of particular values, is kept exact where the planner asks for
it.

### What changes in §1–§3

- `stat_value` keeps only few-valued columns (≤ 64 values) and the watch list.
- `stat_total` gains a `sketch BLOB` holding the AMS and ndv counters.
- `stat_part` goes. A document's minus is computed from its old rows, which are always at hand. The phase store
  holds them per granule. A memo record holds the file's rows and is read once, for the changed file only. That
  is O(changed), and no per-document statistics are stored. This holds for exact counts as well.
- The writer updates the sketches from the delta rows in memory (the session holds them, memory ahead). It
  rewrites the changed columns' blobs in the same transaction as the rows. The version rule of §3 is
  unchanged.

### Validation (before it is built)

Two checks, both cheap:

1. **Planner sensitivity.** Add a debug knob that multiplies each `fan` and `ndv` by a deterministic per-column
   factor in [1/1.5, 1.5]. Compare the chosen plans (`RERAIL_DUMP_STATS`, plan dumps) with the exact run, on
   the struct reader and on the gate's converted rules. If the plans are unchanged, or change with no change in
   CPU, the sketch sizes above are enough.
2. **Accuracy at scale.** Run the simulation over base facts (node rows, names) of the 9,010-file repository,
   where columns have far more distinct values than these exports.

## Recommendation

Approximate, with an exact side for few-valued columns and the watch list:

- AMS w=64 d=3 for `sq`;
- a levelled counting bitmap, m=64, for `ndv`;
- exact `rows` and `sqd`.

Keep the exact design as the fallback if the validation shows the planner is sensitive at 1.5×.
