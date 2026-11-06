# 0051: The store collects in the background, from roots each run renews

Status: proposed (2026-11-06; designed, not built)

## Intro

A corpus run keeps three things in its persistent store (`RERAIL_TAPE_CACHE`): each file's source and tape
(decision 0049), each file's memoised rows (decision 0050), and the atoms the rows name (decision 0047). Nothing
ever removes any of them. Four kinds of rows pile up:

- **Every edit** adds a source, a tape and a memo row for the new content. The old ones stay.
- **Every engine build** writes a new memo plan for every file (`RERAIL_ENGINE_BUILD_ID` is in the plan).
- **Every grammar or tape-format change** writes a new tape for every file.
- **Every new string** adds an atom, and atoms are never given out again.

The store already has a collector (`ContentStore::collect`, `reclamation.rs`). It deletes, in bounded batches, rows
that no root reaches. The roots are workspaces, snapshots and object pins. But a corpus run registers no root, so
the collector would delete everything it stored. And nothing runs the collector outside the workspace server,
which runs it only above the 7 GB soft limit.

This decision gives corpus runs roots, collects dead memo plans, fixes the store's vacuum so space goes back to the
disk, and runs collection in the background. Atoms get a rebuild that only happens past a size limit.

## Where the bytes are (full corpus, 2026-11-06)

`tapes-full.sqlite`, after the full corpus's cold and warm runs and a few engine builds (135 MB):

| Table | MB | Rows |
|---|---|---|
| `tape_objects` | 63.1 | 8,957 |
| `source_objects` | 33.2 | 8,957 |
| `atom_segment` | 28.4 | 103 segments (25.8 MB of string bytes) |
| `facts_results` (the memo) | 2.2 | 18,020: two plans × 9,010 files |

From these, the cost of each kind of change:

| Change | What it adds |
|---|---|
| One file edited | about 11 KB (a 3.7 KB source and a 7 KB tape) and one memo row |
| One engine build, then a run | about 1.1 MB of memo rows, one per file |
| A new grammar version | about 63 MB of tapes |
| New strings | their bytes, on disk and in every process's memory (the arena is mirrored in RAM) |

Engine builds are the growth that matters in development: a few hundred builds is a few hundred MB. Grammar changes
are rare but large. Atoms matter for RAM more than for disk.

## Decision

### 1. A corpus run renews pins on the tapes it used

Each run pins every tape it read or wrote, under owner `corpus:<canonical root>`, with a lease of 7 days. A pinned
tape keeps its source, and a source keeps its memo rows (they cascade from it). The existing collector already
honours pins (`object_pins`), so this needs no new reachability rule.

Renewal is cheap because it is skipped when the lease is fresh. The upsert changes a row only if its lease ends
less than 6 days from now:

```sql
-- One row per tape; a run on an unchanged corpus rewrites only pins older than a day.
INSERT INTO object_pins(object_kind_id, object_id, owner_string_id, lease_until)
VALUES (1, ?tape, ?owner, ?now_plus_7d)
ON CONFLICT(object_kind_id, object_id, owner_string_id)
DO UPDATE SET lease_until = excluded.lease_until
WHERE lease_until < excluded.lease_until - 86400;
```

The pins are written by the worker in the transaction that batches the chunk's tape lookups, so a tape is pinned
before the run hands it on. An edited file's old tape is no longer renewed, and it expires after 7 days. Those days
cover switching branches and back. An old grammar's tapes expire the same way.

Pins are per root, so two checkouts of the same repository keep each other's shared tapes alive. When one stops
running, its pins lapse.

### 2. A memo plan records when it was last used

Today a memo plan includes the file's document key, so every file has its own plan. The key moves into the entry:
`lookup` compares it, and a different key is a miss, as before. The plan is then one digest per run: the engine
build, the grammar version and the file-local program.

A new table records each plan's last use, and a new index lets plan rows be found without scanning:

```sql
-- Additive: other users of the facts memo register no plan here, and their rows are never touched.
CREATE TABLE IF NOT EXISTS corpus_memo_plans (
    plan_digest BLOB PRIMARY KEY CHECK(length(plan_digest) = 32),
    last_used INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS facts_results_by_plan ON facts_results(plan_digest);
```

A run updates its plan's row once. The collector deletes, in batches, the `facts_results` rows of plans last used
more than 7 days ago, then the plan's row. Above the soft limit the age is 1 day.

### 3. The store gives freed pages back to the disk

`initialize` sets `auto_vacuum = INCREMENTAL` on a new store, but after `configure_connection` has already switched
the file to WAL. SQLite then ignores the setting:

```text
$ sqlite3 a.db "PRAGMA journal_mode=WAL; BEGIN EXCLUSIVE; PRAGMA auto_vacuum=INCREMENTAL; CREATE TABLE t(x); COMMIT;"
$ sqlite3 a.db "PRAGMA auto_vacuum;"
0
$ sqlite3 b.db "PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; CREATE TABLE t(x);"
$ sqlite3 b.db "PRAGMA auto_vacuum;"
2
```

Every store examined reports 0, so the collector's `PRAGMA incremental_vacuum(128)` does nothing. Freed pages are
reused, but the file never shrinks. The fix has two parts:

- **New stores:** set `auto_vacuum` on the empty file before `journal_mode`.
- **Existing stores:** convert once with `PRAGMA auto_vacuum = INCREMENTAL; VACUUM`. This rewrites the file, about
  1–2 s for 135 MB, and needs room for a copy. It runs in maintenance when no other connection is open: the atoms
  lease (decision 0047 §5) shows that.

After a pass, `incremental_vacuum` returns up to 4,096 pages (16 MB), not 128 (0.5 MB). At 128 pages per 30 s wake,
a grammar change's 63 MB would take over an hour to return.

### 4. Collection runs in the background, in bounded passes

A pass is the existing `collect`: at most 256 rows per table, in one `IMMEDIATE` transaction, followed by a vacuum
step. Section 2 adds the memo plans to it. Three places run passes:

- **The workspace server's maintenance thread** (`storage.rs`, which wakes every 30 s) runs a pass when any of these
  hold:
  - the store is above its soft limit (as today);
  - the last pass was over an hour ago;
  - a run has marked collection due.

  It takes the write lock without waiting (`immediate_transaction_no_wait`), and skips the pass when the lock is
  held.
- **A corpus run** marks collection due when its renewal found pins or plans to replace. On a background thread
  it then runs passes until one deletes nothing or 200 ms have gone. They start after the last chunk's memo writes,
  when the worker thread is idle. The pin step has already run, so the run's own tapes are pinned before any pass.
- **`rerail-engine gc <store>`** runs passes until the store has nothing to collect.

The store's state row records the last pass's time and whether collection is due. These are two new columns,
added additively.

### 5. Races cost a cache miss, never a wrong result

A pass deletes only what no pin reaches, and only rows created before its generation (the existing guard). If a
run's tape or memo row is collected anyway, because another process's pass ran before the pin landed, the next
lookup misses and the file is parsed and evaluated again. A row being replayed is read in full before the replay,
so it can't disappear halfway. The memo's correctness never depends on a row staying.

### 6. Atoms: rebuilt past a limit, not collected

An atom's id is written into memo frames as a plain integer, in term postings and in the derivation graph. A
collector can't tell which atoms are live without decoding every frame against its program. So atoms are not
collected one by one. Instead:

- The store gets an atom epoch, and the memo plan includes it.
- When the arena passes 64 MB (2.5 times the full corpus's 26 MB) and no other process holds the atoms lease, the
  store starts a new epoch at open:
  - the arena is emptied, apart from atom 0;
  - the tables whose columns are atoms are cleared: `term_postings`, the term statistics and the derivation graph;
  - the memo's old plans become unused, and section 2 collects them.

The next run is cold, and the search index is rebuilt by the existing backlog. This is deliberate: it trades one
cold run for a bounded arena, and it happens rarely. Renumbering atoms in place would avoid the cold run, but every
blob that names an atom would have to be rewritten under an exclusive lock.

## What the prior work says

- "An incremental load must return its dead rows, or it fills the disk" (design index §20; S134, D L748-764). In
  the spike, copy-on-write filled a 4 GB disk at batch 350, with 639 MB held against 40 MB live. A VACUUM every 25
  batches held 4.6 times live (history index §2.19(a)). Section 3 is that lesson for this store: freed pages must
  actually be returned.
- "Key memoized analysis by what it is a function of" (history index §2.16(d)). Moving the document key from the
  plan into the entry keeps it in what the memo checks. The plan then names only what is common to a run, which is
  what makes a plan's last use something that can be recorded.
- A comparable system's "all-or-nothing cache invalidation" (design index §17, LA 6, F3). Pins expire per tape and
  plans per program, so a collection never empties the memo as a whole. The atom epoch is the one all-or-nothing
  step, and it is bounded to a size trigger.
- "Incremental indexing allows no corpus-wide statistic" (design index, top findings item 10; S124). Collection removes rows whose
  roots lapsed. The cross-file statistics of later runs are rebuilt from the rows that remain, as in decision 0050.

## Order of work

1. Fix `auto_vacuum` for new stores, and convert existing ones in maintenance. Test: a fresh store reports 2; a pass
   that deletes tapes shrinks the file.
2. Pins from corpus runs (§1). Test: after `collect`, a run's tapes and memo rows remain; an edited file's old tape
   goes once its lease has passed (with `now` given).
3. Memo plans per run, with last use (§2). Bump the memo's `SCHEMA` to 3. Test: a plan unused for 8 days is
   collected; the current one isn't.
4. Background passes in the corpus run, the CLI command and the server's wake (§4). Measure the pass time on
   `tapes-full.sqlite` with one dead plan, and the cost of renewal on a warm run. The renewal should stay under
   50 ms of a 0.93 s warm run.
5. The atom epoch (§6), when an arena first approaches the limit. Until then, an open item.

## Open

- Pins per root keep 7 days of every root that has run. A machine with many short-lived checkouts could hold many
  copies of the same tapes' pins, but not of the tapes: tapes are shared by content.
- The soft-limit age of 1 day for plans is a guess. It should be revisited with a measured development workflow.
