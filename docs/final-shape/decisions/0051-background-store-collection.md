# 0051: The store collects in the background, keeping recent runs and a bounded history

Status: proposed (2026-11-06; designed, not built; revised the same day: bounded by size, not by age)

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

This decision gives corpus runs roots, keeps a bounded history of older runs, fixes the store's vacuum so space goes
back to the disk, and runs collection in the background. Atoms get a rebuild that only happens past a size limit.

What is kept is bounded by size, not by age. A first version kept what a run had used for 7 days, but a branch can
be churned whole many times a day (user, 2026-11-06: "we could end up with essentially unbounded GC"). On the full
corpus each churn adds about 100 MB, so a week of leases could hold gigabytes. A size bound holds whatever the churn
rate.

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

### 1. Each run records what it used, as a manifest

A corpus root is a canonical directory (`corpus_roots`). The store keeps a run counter, and each run of a root takes
the next number. The run records its **manifest**: the set of tapes it read or wrote, and its memo plan (§2).

```sql
-- Additive. A manifest's tapes are delta-varint tape ids, sorted: about 2 bytes a file.
CREATE TABLE IF NOT EXISTS corpus_roots (
    root_id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
) STRICT;
CREATE TABLE IF NOT EXISTS corpus_manifests (
    manifest_id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES corpus_roots(root_id),
    last_run INTEGER NOT NULL,      -- the store's run counter at the last run that used this manifest
    last_time INTEGER NOT NULL,     -- unix seconds of that run (only for the backstop, §3)
    plan_digest BLOB NOT NULL CHECK(length(plan_digest) = 32),
    digest BLOB NOT NULL CHECK(length(digest) = 32),   -- of the tape ids and the plan
    tapes BLOB NOT NULL,
    UNIQUE (root_id, digest)
) STRICT;
CREATE INDEX IF NOT EXISTS corpus_manifests_by_run ON corpus_manifests(last_run);
```

A run whose tapes and plan are the same as an earlier manifest of its root only updates that manifest's `last_run`
and `last_time`: one row. This is the usual warm run. A run with edited files writes a new manifest, about 20 KB on
the full corpus. Either way it's one statement at the end of the run, not one per file.

A tape is reachable while a kept manifest names it. The collector's tape query gets one more `NOT EXISTS`, over a
temporary table of the kept manifests' tape ids, filled at the start of each pass. A kept tape keeps its source, and
a source keeps its memo rows (they cascade from it).

### 2. A memo plan belongs to its runs

Today a memo plan includes the file's document key, so every file has its own plan. The key moves into the entry:
`lookup` compares it, and a different key is a miss, as before. The plan is then one digest per run: the engine
build, the grammar version and the file-local program. The manifest records it.

A plan is kept while a kept manifest names it. The collector deletes, in batches, the `facts_results` rows of the
plans no kept manifest names. A new index finds them without a scan:

```sql
CREATE INDEX IF NOT EXISTS facts_results_by_plan ON facts_results(plan_digest);
```

Only plans that some manifest has named are ever collected this way. The facts engine's own memo rows register no
manifest and are left alone.

### 3. What is kept: each root's last runs, and a history up to a high-water mark

A pass decides which manifests to keep, newest first by `last_run`:

1. **Protected:** the manifests of each root's last 2 runs. Switching branches and back never misses.
2. **History:** older manifests, kept while the history stays under a byte budget.
3. **Redundant:** a manifest whose tapes all appear in newer kept manifests holds nothing of its own, and goes.
4. **Backstop:** a manifest whose root hasn't run for 30 days goes, protected or not. That covers checkouts that
   are gone.

The history is measured as the bytes each manifest holds alone: the tapes and sources in it that no newer kept
manifest names. Walking newest to oldest, a manifest's own bytes are those of its tape ids not seen yet. The sizes
come from `length(frame)` of the tape and source rows, read once per pass.

The budget has two marks, so passes don't run on every write:

| Mark | Value | Use |
|---|---|---|
| high water | 512 MB | above it, a pass evicts |
| low water | 384 MB | eviction stops below it |

Eviction keeps manifests newest first while the history stays under the low-water mark, and drops the rest, never a
protected one. On the full corpus a whole churn is about 100 MB, so the history holds about four whole versions, or
many more partial ones. What the store keeps for corpus runs is bounded by the high-water mark plus the protected
sets. The protected sets are each root's current files: tapes are shared by content, so two checkouts of one
repository mostly share them.

The marks are separate from the store's soft and hard limits (7 GB and 9 GB), which still apply to the whole store.

### 4. The store gives freed pages back to the disk

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

### 5. Collection runs in the background, in bounded passes

A pass is the existing `collect`: at most 256 rows per table, in one `IMMEDIATE` transaction, followed by a vacuum
step. Sections 1–3 add the manifests and the memo plans to it. Choosing the manifests to keep reads only manifests and
frame lengths, so it runs before the write transaction. Three places run passes:

- **The workspace server's maintenance thread** (`storage.rs`, which wakes every 30 s) runs a pass when any of these
  hold:
  - the store is above its soft limit (as today);
  - the corpus history is above the high-water mark;
  - a run has marked collection due.

  It takes the write lock without waiting (`immediate_transaction_no_wait`), and skips the pass when the lock is
  held.
- **A corpus run** marks collection due when it writes a new manifest. On a background thread it then runs passes
  until one deletes nothing or 200 ms have gone. They start after the run's manifest is written, so the run's own
  tapes are protected before any pass.
- **`rerail-engine gc <store>`** runs passes until the store has nothing to collect.

The store's state row records the run counter, the corpus history's bytes at the last pass and whether collection
is due. These are three new columns, added additively. A store under the high-water mark with nothing due runs no
pass at all, so an idle store costs nothing.

### 6. Races cost a cache miss, never a wrong result

A pass deletes only what no kept manifest or other root reaches, and only rows created before its generation (the
existing guard). A run's manifest is written only at its end. If a tape the run is using goes before then, because
it was older than the pass's generation and only an evicted manifest named it, the next lookup misses and the file is
parsed and evaluated again. A row being replayed is read in full before the replay,
so it can't disappear halfway. The memo's correctness never depends on a row staying.

### 7. Atoms: rebuilt past a limit, not collected

An atom's id is written into memo frames as a plain integer, in term postings and in the derivation graph. A
collector can't tell which atoms are live without decoding every frame against its program. So atoms are not
collected one by one. Instead:

- The store gets an atom epoch, and the memo plan includes it.
- When the arena passes 64 MB (2.5 times the full corpus's 26 MB) and no other process holds the atoms lease, the
  store starts a new epoch at open:
  - the arena is emptied, apart from atom 0;
  - the tables whose columns are atoms are cleared: `term_postings`, the term statistics and the derivation graph;
  - the memo's old plans become unused, and the plan collection in §2 deletes them.

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
- A comparable system's "all-or-nothing cache invalidation" (design index §17, LA 6, F3). Eviction is per manifest,
  oldest first, and never touches a root's last runs, so a collection never empties the memo as a whole. The atom epoch is the one all-or-nothing
  step, and it is bounded to a size trigger.
- "Incremental indexing allows no corpus-wide statistic" (design index, top findings item 10; S124). Collection removes rows whose
  roots lapsed. The cross-file statistics of later runs are rebuilt from the rows that remain, as in decision 0050.

## Order of work

1. Fix `auto_vacuum` for new stores, and convert existing ones in maintenance. Test: a fresh store reports 2; a pass
   that deletes tapes shrinks the file.
2. Manifests from corpus runs (§1). Test: after `collect`, a run's tapes and memo rows remain; a warm run writes one
   row; an edited file's old tape stays while its manifest is kept.
3. Memo plans per run (§2). Bump the memo's `SCHEMA` to 3. Test: a plan only an evicted manifest named is
   collected; the current one isn't.
4. Keeping and eviction (§3), with the marks as parameters. Test, on a small corpus with marks of a few hundred KB:
   - churn every file 20 times; the history stays under the high-water mark after each pass;
   - each root's last 2 runs never miss;
   - a root idle past the backstop is collected.
5. Background passes in the corpus run, the CLI command and the server's wake (§5). Measure on `tapes-full.sqlite`:
   the pass time with one evicted manifest, and the run's cost of writing its manifest (it should stay under 50 ms
   of a 0.93 s warm run).
6. The atom epoch (§7), when an arena first approaches its limit. Until then, an open item.

## Open

- The marks (512 and 384 MB) and the protected count (2 runs) are starting values. They should be revisited with a
  measured development workflow.
- The bound covers what corpus runs keep. The workspace server's snapshots keep their own roots, as before.
- Atoms are outside the byte budget (§7 bounds them separately), and so are the memo rows of other facts-memo users.
