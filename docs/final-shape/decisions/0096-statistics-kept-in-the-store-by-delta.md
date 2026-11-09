# 0096: Planner statistics are kept in the store as deletable sketches, maintained by the deltas that change the rows

Status: accepted as the design (2026-11-09), settled by the spikes in §"Evidence". The sketch and the measurement
aids are built (`logic/plan/sketch.rs`). The tables, their writers and their readers are not built yet.

## Intro

The planner chooses join orders from statistics:

- for each relation, its row count;
- for each column, the distinct values (`ndv`), the sum of squared postings (`sq`, whose ratio to the rows is an
  equality probe's fanout), and that sum within documents (`sqd`);
- for a few-valued column, each value's count;
- for the tree, kind and edge counts.

Today a run measures these afresh from the rows in memory:

- a memo hit replays its file's rows, partly so that they can be counted;
- a changed phase export is measured again whole;
- a reader's bound relation is sorted per column to measure it (`syntax::measured`).

So a one-file edit pays for counting every file. This is the last O(repo) cost on the edit path, and it blocks
skipping an unchanged file's replay (the per-file cross store), because replay is the statistics feed.

Statistics are pure overhead: no finding depends on them, and no cache key includes them (`memo::plan_digest`
hashes the build, the grammar, the node-id scheme and the program). They only have to rank join orders well.
The spikes below show that the planner keeps its plans when every fanout and distinct count is off by up to 4×.
So this record keeps them approximate, in a fixed size per column, in SQLite beside the rows they describe. The
delta that changes the rows changes them too, in the same transaction. A run reads totals, and counts only the
rows that changed.

## Decision

### 1. What is kept, per (relation, column)

| Statistic | How | Exact? |
| --- | --- | --- |
| `rows` | a counter | exact |
| `sqd` (within documents) | a counter; a changed document's term `Σ_v n_d(v)²` is subtracted from its old rows and added from its new ones | exact |
| node column `ndv`, `sq` | counters: a node column's values never cross documents, so both are sums of per-document terms | exact |
| value column `sq` | an AMS sketch: 3 rows × 64 signed counters | ≈, within 1.5× measured |
| value column `ndv` | a levelled counting bitmap: 64 counters per occupied level | ≈, within 1.7× measured |
| few-valued column (≤ 256 distinct) | each value's count (`stat_value`), as the planner's `dfreq` reads it | exact |
| the program's constants (`Stats::memo`) | a watch list: each constant a rule compares a column with, counted exactly | exact |
| tree: kinds, edges, documents | counters per kind and per edge | exact |

The sketches (`logic/plan/sketch.rs`):

```text
sq:  AMS / Count-Sketch, D=3 rows of W=64 signed counters.
     A value v adds ±n (sign from its hash) at counter h_i(v) of row i.
     sq ≈ median over rows of Σ counter².    // a heavy value lands whole in one counter: never sampled away
ndv: a levelled counting bitmap, M=64 counters per level.
     level(v) = trailing zeros of hash(v), so the levels ≥ L hold a 2^-L sample of the distinct values.
     v adds n at slot g(v) of its level.     // counts, not bits: a delete takes them out again
     ndv ≈ 2^L · −M·ln(empty/M), at the lowest L whose levels ≥ L leave ≥ 30% of the slots empty.
```

Both are linear: a row's insert adds to counters, and its delete subtracts the same amounts. That gives three
properties:

- **History-independent.** After any order of inserts and deletes, the counters equal those of the final
  multiset. This is the spike's "two loads equal one load" (D S124), and it is tested:
  `a_sketch_is_that_of_its_final_rows_whatever_the_order_of_inserts_and_deletes`.
- **Mergeable.** Two sketches add to the sketch of both, and a part is taken out by subtracting it.
- **Fixed in size.** At most 768 B plus 257 B per occupied level, and at most 5.7 KB measured on the gate corpus.
  The hashes are a fixed mixer (splitmix64), so a sketch is the same in every process.

A column moves between exact and sketched by its current distinct count: exact at most 256, sketched above. The
sketch is kept always, because it is cheap. The exact map is kept while it is small. A column that grows past
256 drops its map. Once it falls back to 128, its map is rebuilt from its rows by one SQL count. The hysteresis
keeps a column on the boundary from flapping. Accuracy only differs between the two states, and plans held at 4×
(§"Evidence"), so this is history-dependent only where it does not matter.

### 2. The tables, beside the rows

```sql
-- One row per (relation, column): the totals the planner reads, and the sketch, at the rows' version.
CREATE TABLE stat_total(rel TEXT, col TEXT, v INTEGER,      -- v: the rows' version (export version, or epoch)
    rows INTEGER, sqd INTEGER,
    ndv INTEGER, sq INTEGER,                                -- exact for node columns; estimates otherwise
    sketch BLOB,                                            -- AMS + occupied levels; NULL for node columns
    PRIMARY KEY(rel, col)) WITHOUT ROWID;

-- Exact counts: few-valued columns (≤ 256 distinct), and the watch list's constants.
CREATE TABLE stat_value(rel TEXT, col TEXT, val INTEGER, n INTEGER CHECK(n > 0),
    PRIMARY KEY(rel, col, val)) WITHOUT ROWID;

-- Tree counts, by kind and by (parent kind, child kind).
CREATE TABLE stat_tree(kind INTEGER, parent INTEGER, n INTEGER, PRIMARY KEY(kind, parent)) WITHOUT ROWID;
```

No per-document statistics are stored. A changed document's minus is computed from its old rows, which are
always at hand when it changes. The phase store holds them per granule (`export_rows`). A file's memo record
holds the rows of its file-local part, and is read once, for the changed file only. Tree counts are the
exception: the old tape may be gone. So each document's kind and edge counts are kept with its tape record. That
is a list bounded by the grammar's kinds, not by the file.

### 3. Who writes them: the writer of the rows, in its transaction

| Where rows change | The delta | What is written |
| --- | --- | --- |
| Phase `keep` | plus/minus rows per export (SQL, already computed) | sketches and counters patched from the delta; `v` = the export's new version |
| A file's file-local part, run again | its new rows (in hand), and its old rows (its memo record) | the same, as a minus then a plus |
| A document leaving or entering the run | its rows (memo record, phase granules) and its tape record's tree counts | its terms subtracted or added |

The sketches are patched in memory, by the session, from the delta rows. The changed columns' `stat_total` rows
are rewritten, one blob each, in the same transaction as the rows. The cost is O(changed rows × 4 hashes), plus
one small row write per changed column.

### 4. Delayed readback: versions, not timing

The decided model is that SQLite is durable, its writes may lag, and memory may be ahead. The statistics follow
that model with no new rule:

- **Same transaction.** A row delta and its statistics delta commit together, so `stat_total.v` names exactly
  the rows it describes. A lagged write lags both together.
- **Memory is ahead.** The session holds the totals and sketches in memory, as it holds the reader's sources
  (`phased`: rel → version → source and statistics). It patches them with the delta it hands the reader, and
  the planner reads memory. SQLite is read only by a cold session, or after a restart.
- **The version decides.** If memory's version is newer, memory is used. If the store's is newer (another
  process wrote), the store's totals are read. The reader's memo key already folds in the export versions.
- **Missing means measured once.** A store without the tables, or a relation without a `stat_total` row, is
  measured from its rows once, and the result is written. This is a backfill, as the anchor index's upgrade does.

### 5. Reading

The planner already reads statistics from SQL (`Stats::read`, T's `stat_meta`/`stat_col`/`stat_val` schema).
`stat_total`, `stat_value` and `stat_tree` are served under it as views. One statement reads a run's relations,
as the spike's planning reads a batch (D S150, S241). The session's in-memory totals fill the same `Stats`
fields through one conversion (`Totals::apply` today).

### 6. What stays measured in the run

Relations derived inside a run (setfix `measured`, `obs` fanouts) are measured by the run that makes them. They
move here once derived relations have deletes in the engine core (0094 §4's open point). The same sketches then
apply to their deltas.

## Evidence (2026-11-09)

All runs used the debug build at commit `5caae5e4b` plus the spike's measurement aids. CPU is user plus sys. The
machine load was 27–211 and is given per run.

### Spike 1: how wrong may the statistics be? (`RERAIL_STATS_SKEW`)

`RERAIL_STATS_SKEW=f[:seed]` multiplies every fanout (`sq/rows`, and `sqd/rows`) and every distinct count the
planner reads by a fixed per-column factor, log-uniform in [1/f, f]. Row counts, few-valued counts and constants
are left exact, as §1 keeps them. With `RERAIL_STATS_SKEW_LOG`, each search is also made with the true
statistics, and the skewed order is priced with them. The log line is the regret: the skewed order's work over
the best order's, both by the true statistics.

Workloads, all cold (a fresh tape cache per run):

- **Rules over the gate corpus's 1/20 sample** (452 files, 1.50M nodes): the four converted rules with a
  cross-file part (grammar-callable-constructors, serverplan-quality, async-sync-boundary,
  plan-scope-ownership), plus error-handling.
- **The struct phase spike** (`spike_phases_struct_over_the_corpus`, 84 files plus typeshed): its reader plans
  over phase exports, and each edit is compared with a full run.

| Workload | Skew | Searches | Same order | Regret p99 | Regret max | Total planned work |
| --- | --- | --- | --- | --- | --- | --- |
| rules, 1/20 | 1.5× (seed 1) | 842 | 78.1% | 1.065 | 1.53 | ×1.000 |
| rules, 1/20 | 1.5× (seed 2) | 837 | 91.3% | 1.028 | 1.08 | ×1.000 |
| rules, 1/20 | 2× | 843 | 78.1% | 1.066 | 1.53 | ×1.000 |
| rules, 1/20 | 4× | 843 | 77.2% | 1.256 | 1.95 | ×1.000 |
| struct spike | 4× | 7,191 | 83.7% | 1.154 | 1.90 | ×1.000 |

The orders that change are those of small rules. The largest regret on the rules is 35 rows of work against 18.
On the struct spike it is 28,900 against 15,212, for one rule (`sym($f, @cat($q, $x))`). The struct spike's
checks against full runs passed under 4× skew. The rules' violations were the same under every skew.

Real CPU on the rules, cold, alternating, with no regret searches, at load 59–81:

| Run | Plain | 4× skew |
| --- | --- | --- |
| a | 42.2 s | 40.6 s |
| b | 43.4 s | 41.9 s |

No cost is visible at 4×, within the noise. The file-local planner already runs on guessed distinct counts for
its batch sources (`make_plan`: `d` 300, `kind` 200, `idx` 20, and so on) and does well. That points the same
way.

### Spike 2: how close are the sketches at scale? (`RERAIL_STATS_SHADOW`)

`RERAIL_STATS_SHADOW=<file>` sketches every column the run measures exactly (`syntax::measured`, and the cross
part's `Totals::apply`), and logs both. These are columns over 256 distinct values, the ones §1 sketches:

| Run | Columns sketched | Rows up to | ndv up to | Fanout worst | ndv worst | Sketch max | Exact entries instead |
| --- | --- | --- | --- | --- | --- | --- | --- |
| serverplan-quality, full gate corpus (9,024 files, 28.2M nodes; 184 s CPU, load ~75) | 30 | 198,239 | 113,255 | 1.46× | 1.41× | 5.7 KB | 600,251 |
| grammar-callable-constructors + async-sync-boundary, 1/4 (2,256 files, 7.1M nodes) | 42 | 171,284 | 85,391 | 1.26× | 1.39× | 5.4 KB | 609,152 |
| struct spike (84 files + typeshed, every edit) | 524 | 184,404 | 59,793 | 1.39× | 1.70× | 5.1 KB | 2,530,837 |

The logs are read by `docs/final-shape/notes/0096-regret.py` and `0096-shadow.py` (`T=256`), and the tuning below is `0096-ndv-tune.py`. Earlier, in Python over the struct exports (`0096-sketch-sim.py`): fanout 1.25× and ndv
1.35×.

The worst errors, 1.5× on fanout and 1.7× on ndv, are well inside the 4× the planner tolerates. The ndv
estimator was tuned on synthetic sweeps of 70 to 113k distinct values, 8 seeds each:

| M | Level choice | Worst | Mean abs. log error |
| --- | --- | --- | --- |
| 64 | lowest with ≥ 30% empty (kept) | 1.68× | 0.154 |
| 64 | the level nearest half full | 1.41× | 0.152 |
| 128 | lowest with ≥ 30% empty | 1.40× | 0.091 |

M=128 halves the error at twice the levels' size. Given the 4× margin, M=64 stays. Raising it is a constant
change if a workload ever needs it.

### Settled

| Question | Answer |
| --- | --- |
| Exact or approximate? | approximate for value columns over 256 distinct values; exact for the rest (§1) |
| Fanout | AMS, 3 × 64 |
| Distinct values | levelled counting bitmap, M=64 |
| Deletes | linear sketches: exact and history-independent |
| Per-document parts | none, but each document's tree counts kept with its tape record (§2) |
| Delayed readback | version-tagged rows, written in the rows' transaction, memory ahead (§4) |

## What this agrees with, and departs from

- **Agrees with the spike §4: no corpus-wide statistic.** The spike allows per-file statistics and running
  aggregates (`_accum_kind_freq`), and requires that statistics survive a restart (D S124; SORA C7). These are
  running aggregates kept by delta, persisted.
- **Agrees with the lessons ledger #4 and #5.** There is no ANALYZE and no temp-table statistics. The numbers
  are ours, kept by delta.
- **Agrees with the anchor index (D S241).** Its frequencies are kept in the rows' own transaction. Here the
  writer patches them rather than a trigger, because the rows of a phase export are encoded per granule.
- **Departs from the anchor index's exactness.** The anchor index counts each term exactly, because it picks the
  rarest anchor by them. The rule planner needs exact counts only for constants, which the watch list keeps.
- **Departs from today's code.** `Totals::add_rows` measuring replayed rows, `Own::measure` per chunk mix, and
  `syntax::measured` on bound exports become backfill-only paths.

## Cost

Measured 2026-11-09 by `docs/final-shape/notes/0096-statbench` (release, the engine's `sketch.rs` by path; CPU of
the thread, best of 7, load 8). Columns of 200,000 rows; an edit takes 1,000 rows out and puts 1,000 in.

| Per changed row, per column | Sketch | Exact, in memory | Exact, SQLite upsert |
| --- | --- | --- | --- |
| Zipf column, 60k values (31,179 present) | 3.2 ns | 1.7 ns | 760 ns |
| key-like column (200,000 values) | 3.2 ns | 3.1 ns | — |
| few-valued column (40 values; kept exact) | 4.9 ns | 1.3 ns | — |

| Per changed column | Cost |
| --- | --- |
| Estimates read (`sq`, `ndv`) | 0.21–0.26 µs |
| Blob encode and decode | 0.9–2.7 µs (2.0–5.6 KB) |
| Blob written in the rows' transaction | 2.8 µs (4.4 µs as a commit of its own) |
| Today: a full re-measure (`syntax::measured`'s sort) | 2.1–7.4 ms, 10–37 ns per row of the relation |

What an edit costs: say 1,000 rows changed in each of 30 sketched columns.

| Mechanism | Per edit |
| --- | --- |
| Sketches | 30 × 2,000 × 3.2 ns + 30 × 2.8 µs ≈ 0.28 ms |
| Exact counts in SQLite | 30 × 2,000 × 760 ns ≈ 46 ms |
| Today (each 200k-row column measured again) | 30 × 2–7 ms ≈ 60–220 ms, growing with the repository |

The exact in-memory map is cheaper per row than the sketch: 1.3–3.1 ns against 3.2–4.9 ns, because its
hot entries sit in cache. But it holds every distinct value: 31,179 or 200,000 entries per column here, and
2.5M for the struct workload. To survive a restart it is written to SQLite at 760 ns per changed row, or
measured again from the rows, which is O(repo). The sketch's per-row cost is a few nanoseconds on rows that
change anyway, and its write is one small blob per column. So the choice of §1 stands.

The key-like column's ndv was estimated at 0.75× (150,828 of 200,000), inside the 4× the planner tolerates.
Node columns are exact (§1) and are not sketched.

## Order of work

1. Write `stat_total` (with sketches) and `stat_value` in phase `keep` from its plus/minus. Serve the reader's
   bound exports from them, with no `syntax::measured` on an export.
2. Patch them from a file's old and new local rows when its file-local part runs again. Keep each document's tree
   counts with its tape record. Stop counting replayed rows.
3. Build the per-file cross store (rows in SQLite, keyed by file and program). An unchanged file is neither
   replayed nor counted.
4. Derived relations, once the core has relations with deletes.

The measurement aids stay: `RERAIL_STATS_SKEW`, `RERAIL_STATS_SKEW_LOG` and `RERAIL_STATS_SHADOW`. Each step's
claim is checked by running them again.
