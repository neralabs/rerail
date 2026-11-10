# 0096: Planner statistics are kept in the store as deletable sketches, maintained by the deltas that change the rows

Status: accepted as the design (2026-11-09), settled by the spikes in §"Evidence". Amended the same day (§2–§4):
statistics live in memory and are written with the wave SQLite commits, as blobs, never a row per value. Steps 1
to 4 are built for a session (`logic/plan/statkeep.rs`, `corpus/keptstats.rs`, `corpus/retain.rs`; "Order of
work").

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
So this record keeps them approximate, in a fixed size per column, in memory beside the rows they describe. The
delta that changes the rows patches them too. SQLite keeps them durable as it keeps the rows: it commits
consistent waves as fast as it can, and a wave's statistics are part of it. A run reads totals, and counts only
the rows that changed.

## Decision

### 1. What is kept, per (relation, column)

| Statistic | How | Exact? |
| --- | --- | --- |
| `rows` | a counter | exact |
| `sqd` (within documents) | a counter; a changed document's term `Σ_v n_d(v)²` is subtracted from its old rows and added from its new ones | exact |
| node column `ndv`, `sq` | counters: a node column's values never cross documents, so both are sums of per-document terms | exact |
| value column `sq` | an AMS sketch: 3 rows × 64 signed counters | ≈, within 1.5× measured |
| value column `ndv` | a levelled counting bitmap: 64 counters per occupied level | ≈, within 1.7× measured |
| few-valued column (≤ 256 distinct) | each value's count, in memory and in the relation's blob | exact |
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
256 drops its map, and it comes back only when the relation is counted again from its rows (§4's backfill), so
a column on the boundary does not flap. Accuracy only differs between the two states, and plans held at 4×
(§"Evidence"), so this is history-dependent only where it does not matter.

A node column is told by its values, not its declaration: a column whose every value names its row's document
(`value >> 32`, as `syntax::measured` reads a row's document) never shares a value across documents, so its
`ndv` and `sq` are sums of per-document terms and exact. One document's value naming another moves the column to
its sketch for as long as that document's rows are counted.

### 2. Where they live: memory, and one blob per memo plan in SQLite

The general model holds: everything that must be fast is in memory, and SQLite keeps up as best it can by
committing consistent, durable waves. **No statistic is ever written, read or counted per row in SQLite.**

- **In memory** (`statkeep::RelStats`): per relation its row count, and per column the counters and sketch of §1
  and the exact map while it is small. A session holds them beside the sources they describe; patching them is a
  few nanoseconds per changed row per column (§"Cost").
- **In SQLite**: one blob per memo plan, `stat_blob(plan, blob)` in the content store: the files' contents it
  counts (path, stored source, document key) and each base relation's `RelStats` (`Keeper::encode`). A run
  writes it once, when it counted a file in or out; nothing is written per row or per value. The blob names the
  memo entries it counts by their content, so it agrees with the entries whatever the timing of the writes, and
  a later process patches it by the files whose content changed.

```sql
-- One row per memo plan: the base relations' statistics, and the files' contents they count.
CREATE TABLE stat_blob(plan BLOB PRIMARY KEY, blob BLOB NOT NULL) WITHOUT ROWID;
```

  A phase export's statistics are held in memory only. A process without them loads the export's rows from the
  phase store whole anyway, and counts them as it interns them; keeping them durable pays once the export's
  source is itself kept (step 3).

No per-document statistics are stored. A changed document's minus is computed from its old rows, which are at
hand when it changes: the session's memo holds each file's rows (`memo::Entry`), and the phase store hands on an
export's plus and minus. Where the old rows are not at hand (the memo dropped them), the relation is counted
again from its rows once, which is the backfill path.

### 3. Who patches them: whoever has the delta

| Where rows change | The delta | What is patched |
| --- | --- | --- |
| A file's rows of a base relation (memo hit, or its file-local part run again) | its old rows (the entry the last run counted) and its new | its terms subtracted, then added (`RelStats::document`) |
| Phase `keep` | plus/minus rows per export | the export's `RelStats`, patched; written in `keep`'s transaction |
| A document leaving or entering the run | its rows (its memo entry) | its terms subtracted or added |

The session keeps, per relation, which content of each document its totals count, so an edit touches only the
documents whose content changed.

### 4. Delayed readback: contents, not timing

- **Memory is ahead.** The planner reads memory. The blob is read by a process without a session.
- **The contents decide.** The blob counts named contents. A file whose content is the one counted is not
  counted again; one whose content changed is counted out by the old content's memo entry and in by the new.
  A blob written before a crash still names what it counts, so it is never wrong about it, only behind.
- **Missing means counted once.** A store without the table, a blob that does not decode, or an old entry the
  memo no longer holds: every file is counted in again from the rows the run reads anyway, and the result is
  written. This is the backfill, as the anchor index's upgrade does.

### 5. Reading

The planner reads `Stats`, filled from memory: `RelStats::statistics` gives the same `(column, rows, ndv, sq,
sqd)` that `syntax::measured` gives, so every reader of a measured relation reads a kept one unchanged. T's SQL
statistics (`Stats::read`) stay for T's own relations.

### 6. What stays measured in the run

Relations derived inside a run (setfix `measured`, `obs` fanouts) are measured by the run that makes them. They
move here once derived relations have deletes in the engine core (0094 §4's open point). The same sketches then
apply to their deltas. serverplan-quality's `a34441da4` (merged at `d89eb3147`) already takes a derived relation's
statistics from the postings the engine kept while deriving it (`InRel::posted`), with no sort. Its per-value
posting lengths are the counts a sketch is fed, so a maintained derived relation can patch its sketch from them.

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
| Where they live | in memory; one blob per memo plan in SQLite, written with the run, never a row per value (§2) |
| Delayed readback | the blob names the contents it counts, so it is behind at worst, never wrong; memory ahead (§4) |

## What this agrees with, and departs from

- **Agrees with the spike §4: no corpus-wide statistic.** The spike allows per-file statistics and running
  aggregates (`_accum_kind_freq`), and requires that statistics survive a restart (D S124; SORA C7). These are
  running aggregates kept by delta, persisted.
- **Agrees with the lessons ledger #4 and #5.** There is no ANALYZE and no temp-table statistics. The numbers
  are ours, kept by delta.
- **Departs from the anchor index (D S241) in where counts are kept.** Its frequencies are rows patched in the
  rows' own transaction. Here nothing is patched in SQLite: memory is patched, and a wave writes one blob.
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
| Blob written in the wave's transaction | 2.8 µs (4.4 µs as a commit of its own) |
| Today: a full re-measure (`syntax::measured`'s sort) | 2.1–7.4 ms, 10–37 ns per row of the relation |

What an edit costs: say 1,000 rows changed in each of 30 sketched columns.

| Mechanism | Per edit |
| --- | --- |
| Sketches in memory, one blob per relation per wave | 30 × 2,000 × 3.2 ns + 3 µs ≈ 0.2 ms |
| Exact counts in SQLite (rejected: per row) | 30 × 2,000 × 760 ns ≈ 46 ms |
| Today (each 200k-row column measured again) | 30 × 2–7 ms ≈ 60–220 ms, growing with the repository |

The exact in-memory map is cheaper per row than the sketch: 1.3–3.1 ns against 3.2–4.9 ns, because its
hot entries sit in cache. But it holds every distinct value: 31,179 or 200,000 entries per column here, and
2.5M for the struct workload. To survive a restart it is written to SQLite at 760 ns per changed row, or
measured again from the rows, which is O(repo). The sketch's per-row cost is a few nanoseconds on rows that
change anyway, and its write is one small blob per column. So the choice of §1 stands.

The key-like column's ndv was estimated at 0.75× (150,828 of 200,000), inside the 4× the planner tolerates.
Node columns are exact (§1) and are not sketched.

## Order of work

| Step | What | State (2026-11-09) |
| --- | --- | --- |
| 1 | A phase export's statistics counted once, then patched by each version's plus and minus | built (`8da5e1b01`): `PhasedSource` keeps its `RelStats` |
| 2 | A base relation's statistics patched by file: a file's old rows out (by the content counted), its new in; replayed rows not counted again | built for memoised runs, in a session (`e388adcc3`) and over the tape cache, durable as one blob per memo plan (`5f7def8b8`) |
| 3 | An unchanged file is neither replayed nor counted: the session keeps the base sources and exports, and patches a changed file's rows in | built, in a session: the keeper counts each export row and each base relation's held row by the files holding it (`exports::Counted`), and the files' tree counts; a changed file's old rows out and new in, a gone file's out. An unchanged hit's rows are not held again (`58e74ad23`); a file is counted in one pass a column (`5c9635187`) |
| 4 | Derived relations: a maintained relation's statistics patched by its delta rather than read from its postings | built, in a session (`02d67b604`): the engine's maintained relation gives its delta from the one it started as (`InRel::delta_since`), which patches the `RelStats` kept with it |

Measured on the O125 replay (calls.rr over real history, a session, load 65–140), per one-to-four-file step:

| Cost | Before | After steps 1–2 | After step 3 |
| --- | --- | --- | --- |
| Base relations measured (`measure the base rows held`) | 0.56 s | 0.08–0.09 s (making the sources) | the same |
| Memo hits' rows measured again (`measure <rel> (memo)`) | about 0.6 s | none | none |
| Kept relations' `r__old` measured (`bind and measure kept/measure r__old`, step 4) | 0.25–0.37 s of the 0.58–0.88 s bind | the same | 0.08 s for the 18 not maintained, and 0.02–0.22 s patching the 264 maintained by their deltas (step 4) |
| The memo replay (`local/memo replay`) | 1.3–1.5 s | 0.45–0.52 s (after the feeders' merges), 0.43 s of it holding the hits' export rows | 0.002–0.04 s: the counted exports, base rows and tree counts installed |
| Counting a changed file out and in (`memo remember`) | | 0.48–0.68 s for a 13,300-row file, over 207 relations | 0.79 s with its export rows counted too, four hash passes a column; 0.29–0.70 s in one pass |
| The step | 27–40 s, nearly all of it the cross-file evaluation | the same within the load's noise | 20.5–24.3 s, 16.5–22 s of it the cross-file part |

Step 3's numbers are O125 steps 1–2 (2026-11-09, load 80–115), and step 4's steps 1–3 (load 40–140); each step's
outputs equal a full run's, with the counted rows and with them given up (the hits held from the memo instead).

The gate's replay over commits patches 1–3 files a step from the kept blob; its CPU is the same with the keeper on
and off (alternated, load 90–117), as its statistics were a small share. Every step compared with a full run
gave the same outputs (O125 steps 1–2; the struct phase spike, E1–E3).

What steps 3 and 4 need:

- **Step 3, as built.** The keeper counts a session's exports by file (`exports::Counted`: each row with how
  many files hold it), alongside the statistics, by the same old-content lookup. A memo hit whose content it
  counted is not held again; the run's exports are the counted ones, installed once every file was seen (before
  the cone of requests' digests, or before the statistics). The cone starts from the counted rows less those of
  the files it has not seen yet (gone, or loaded only last run); its loads' rows are held as before. A keeper
  that gives up mid-run (a chunk not attributed to its files, a memo entry gone) holds the skipped hits from the
  memo, as the replay did, and counts every file in again next run. Only a session keeps them: a process over the
  tape cache would count them in each run for nothing. `RERAIL_COUNT_EXPORTS=off` turns it off.
- **Step 3, the rest.** The base relations' held columns are kept as counted rows too, by the files holding
  them, and so is the sum of the files' tree counts: an unchanged hit adds nothing, and the run installs them
  (0.004 s). A file is counted in one pass a column: each row's value goes into its document's map, and the
  sums are read from the maps (`one_pass_counts_as_the_four_passes_did` checks it against the four passes).
- **Step 4, as built.** A maintained relation starts as the kept one less its overdeleted rows (`Rel::based`):
  in place, its first rows the old ones with those marked dead, or compacted from the survivors when too many
  are dead (DRed's overdelete is often more than an eighth of a relation at O125). Either way it records the
  relation it started as, so `InRel::delta_since` gives the rows it removed and added at the cost of the change
  and a pass over the dead bits. A session keeps each such relation's `RelStats` (`retain::Patched`) with the
  counts of the constants rules test it by. They are counted from its rows the first run it is maintained, and
  patched by its delta each run after. A row's delta is not whole documents, so a column's `sqd` would read as
  `sq`; a maintained relation's statistics keep a sketch of its (document, value) pairs for it
  (`RelStats::paired`, within 1.5× in `a_paired_relation_patched_by_rows_keeps_its_sum_within_documents`). A
  relation a run did not derive again keeps its statistics as it keeps its rows.
- **The first count.** A relation not yet kept by delta is counted from the engine's postings when it is first
  bound as `r__old` (`RelStats::from_postings`, a hash a distinct value rather than a row), in place of being
  measured. One the engine kept no index on (no later stratum of the cold run read it, so its indexes went) is
  measured as before, and counted from this run's relation, rebuilt with its indexes. At O125 step 01 that is
  0.54 s for 250 relations, 0.31 s measuring 32 and 0.20 s counting 19 of them: about 0.45 s more than measuring
  alone, against about 1 s before. A survivors rule now starts in place when its delete set is a SQL table
  (`kind` and `docof`, whose overdelete reads an input's delta): their 142 k rows were copied row by row each
  step (0.20–0.23 s; now 0.03–0.07 s).
- **The input deltas.** `__plus` and `__minus` are measured each run, in proportion to their rows: 0.27 s at O125
  with preorder node ids, where an edit renumbers the file and its deltas run to about 75 k rows a side, and
  0.002–0.010 s with per-definition ids. A file count can't give them, as it counts the whole file, old and new,
  not the rows that differ. The fix is at the source: per-definition ids (decision 0090, "why a session's
  maintenance deletes so much").
- **The program's analysis** (pruning, lowering, splitting, 1.3 s) is kept by the process and read back by the
  store (decision 0073). A process serving two stores now keeps it for each, so it no longer prepares the program
  again on each switch (O125 with `O125_FULL_EACH`).

The measurement aids stay: `RERAIL_STATS_SKEW`, `RERAIL_STATS_SKEW_LOG` and `RERAIL_STATS_SHADOW`. Each step's
claim is checked by running them again.
