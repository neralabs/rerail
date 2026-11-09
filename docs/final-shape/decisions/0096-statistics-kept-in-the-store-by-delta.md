# 0096: Planner statistics are kept in the store, maintained by the deltas that change the rows

Status: proposed (2026-11-09). Design only; nothing built.

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

## Open

- Whether a value column of very many distinct values (interned names) should keep exact `stat_value` rows or
  a linear sketch. An AMS sketch estimates `sq` and supports deletes. Exact counts are the default until a
  measurement says otherwise.
