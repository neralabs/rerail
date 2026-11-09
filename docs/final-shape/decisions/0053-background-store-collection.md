# 0053: The store collects in the background, keeping recent runs and a bounded history

Status: accepted (2026-11-06; bounded by size, not by age; built and measured, apart from the atom epoch in §7)

## Intro

A corpus run keeps three things in its persistent store (`RERAIL_TAPE_CACHE`): each file's source and tape
(decision 0051), each file's memoised rows (decision 0052), and the atoms the rows name (decision 0049). Nothing
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
the next number. The run records its **manifest**: the sources it read, the grammar version and tape format of their
tapes, and its memo plan (§2). The manifest records sources, not tapes, because a file read from the memo is never
parsed, so the run knows its source but not its tape.

```sql
-- Additive (content_store/schema.rs). The sources are delta-varint ids, sorted: about 2 bytes a file.
CREATE TABLE IF NOT EXISTS corpus_manifests (
    manifest_id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES corpus_roots(root_id),
    last_run INTEGER NOT NULL,        -- the store's run counter at the last run that used it
    last_time INTEGER NOT NULL,       -- unix seconds of that run (only for the backstop, §3)
    grammar_version INTEGER NOT NULL,
    tape_format INTEGER NOT NULL,
    plan_digest BLOB NOT NULL,
    digest BLOB NOT NULL,             -- of the sources, the tape version and the plan
    sources BLOB NOT NULL,
    UNIQUE (root_id, digest)
) STRICT;
-- What the kept manifests name: (source, grammar version, tape format).
CREATE TABLE IF NOT EXISTS corpus_live (
    source_id INTEGER NOT NULL, grammar_version INTEGER NOT NULL, tape_format INTEGER NOT NULL,
    PRIMARY KEY (source_id, grammar_version, tape_format)
) STRICT, WITHOUT ROWID;
```

A run whose sources and plan are the same as an earlier manifest of its root only updates that manifest's
`last_run` and `last_time`: one row. This is the usual warm run. A run with edited files writes a new manifest,
about 20 KB on the full corpus, and adds to `corpus_live` only the sources the root's previous manifest didn't
have. Either way it happens once, at the end of the run.

The collector honours `corpus_live`:

- its tape query keeps a tape whose source, grammar version and format are live;
- its source queries keep a live source.

The table is durable, so every collector of this build respects corpus runs, the workspace server's included. A
source keeps its memo rows: they cascade from it.

### 2. A memo plan belongs to its runs

Today a memo plan includes the file's document key, so every file has its own plan. The key moves into the entry:
`lookup` compares it, and a different key is a miss, as before. The plan is then one digest per run: the engine
build, the grammar version and the file-local program. The manifest records it.

A plan is kept while a kept manifest names it. The collector deletes, in batches, the `facts_results` rows of the
plans no kept manifest names. A new index finds them without a scan:

```sql
CREATE INDEX IF NOT EXISTS facts_results_by_plan ON facts_results(plan_digest);
```

Only plans that some manifest has named are ever collected this way (`corpus_plans`: `live` is 0 for a plan no kept
manifest names). The facts engine's own memo rows register no manifest and are left alone.

### 2a. Every memo writer's old rows are collected (2026-11-08)

Three writers put memo rows under sources that live on, so the source's collection never took their old rows:

- **A prepared program** (decision 0073) is kept under its program text's source, which every run of that program
  names. Its pointer's plan names the engine build, so each build added a row and the old ones stayed. A run's
  manifest now names these plans too (`record_corpus_run_naming`, table `corpus_manifest_plans`): they are live
  while a kept manifest names them, and dead after, as the run's memo plan is.
- **The slice's memo** (the workspace server's waves) has no manifest. Each wave that writes or reads under a plan
  holds it for a day (`hold_corpus_plan`, renewed at most hourly). A lapsed hold makes a keep decision due, so the
  next pass marks the plan dead and its rows go, with no new run to prompt it.
- **Rows from before this decision** (corpus memo schemas 2 and 3, `RETIRED_CORPUS_MEMO_SCHEMAS`) were never under a
  recorded plan. Passes sweep them in bounded batches until one finds none (`corpus_swept`).

The test `old_memo_rows_are_collected_while_their_source_lives` checks all three, with the source still live.

### 3. What is kept: each root's last runs, and a history up to a high-water mark

A pass decides which manifests to keep, newest first by `last_run`:

1. **Protected:** the manifests of each root's last 2 runs. Switching branches and back never misses.
2. **History:** older manifests, kept while the history stays under a byte budget.
3. **Redundant:** a manifest whose sources all appear in newer kept manifests (with their tape version) holds nothing of its own, and goes.
4. **Backstop:** a manifest whose root hasn't run for 30 days goes, protected or not. That covers checkouts that
   are gone.

The history is measured as the bytes each manifest holds alone: the tapes and sources in it that no newer kept
manifest names. Walking newest to oldest, a manifest's own bytes are those of its sources not seen yet, with
their tapes. The sizes come from `length(frame)` of the tape and source rows, read once per decision.

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
- **Existing stores:** convert once with `PRAGMA auto_vacuum = INCREMENTAL; VACUUM`. This rewrites the file and
  needs room for a copy: 3.75 s for the full corpus's 135 MB. `rerail-engine gc` does it. The server doesn't, since
  it can't tell that no other process holds the store.

After a pass, `incremental_vacuum` returns up to 4,096 pages (16 MB), not 128 (0.5 MB). At 128 pages per 30 s wake,
a grammar change's 63 MB would take over an hour to return.

The pragma also has to be stepped to the end. It frees a page per step, and the collector ran it in
`execute_batch`, which freed none. With `auto_vacuum` on, the s200 churn below still held 505 free pages (2 MB) in a
9.5 MB file. The same pragma in the `sqlite3` shell freed all of them.

### 5. Collection runs in the background, in bounded passes

A pass is the existing `collect`: at most 256 rows per table, in one `IMMEDIATE` transaction, followed by a vacuum
step. Sections 1–3 add the manifests and the memo plans to it. Choosing the manifests to keep reads only manifests and
frame lengths, so it runs before the write transaction. Three places run passes:

- **The workspace server's maintenance thread** (`storage.rs`, which wakes every 30 s) runs passes for up to a
  second of each wake when either of these hold:
  - the store is above its soft limit (as before);
  - corpus collection is due.
- **A corpus run** marks collection due when it writes a new manifest. A warm run also resumes collection that remains due.
  A thread of its own then runs passes while
  the cross-file part runs, until two in a row find nothing or 200 ms have gone, and the run joins it before it
  returns. The manifest is written first, so the run's own sources are live before any pass. `RERAIL_CORPUS_GC=off`
  skips the passes. `RERAIL_CORPUS_HISTORY_MB` sets the high-water mark; the low-water mark is three quarters of it.
- **`rerail-engine gc <store> [--history-mb n]`** converts an old store (§4), decides, and runs passes until two in a
  row find nothing.

Two in a row, because a pass keeps the rows created in its own generation. Its successor deletes them.

`corpus_state` records the run counter, the history's bytes at the last decision and `due`:

| `due` | Meaning |
|---|---|
| 0 | nothing to do |
| 1 | a new manifest: a decision is due |
| 2 | decided: passes go on until one finds nothing |

A store with nothing due, under the soft limit, runs no pass at all, so an idle store costs nothing.

### 6. Active readers defer collection

A generation check does not protect an active run. Before it publishes its manifest, a collector can delete a source
whose id the run already holds. Publishing memo results with that id then fails a foreign-key check.
Reusing an id after deletion also makes an unprotected lookup unsafe.

A corpus run now holds a shared file lease before its first source or stat lookup. It releases the lease after
publishing memo rows, stats, and its manifest. A standalone cached parse holds a lease through tape publication.
Readers can run together. Each collector tries an exclusive lease before deciding which manifests to keep or deleting rows.
If a reader holds a lease, the pass returns without clearing `corpus_state.due`. A later run or maintenance pass resumes it.
Continuous overlapping readers can delay collection until a gap.

The lease uses a stable sidecar at the database's canonical path, so symbolic aliases and separate processes share it.
The sidecar must not be unlinked while the store is in use. Closing its handle or exiting the process releases the lock.
There are no persistent reader pins to expire after a crash. All processes sharing the cache must use this protocol.
Lease release also explicitly unlocks the file. A subprocess can inherit its descriptor before exec; closing only the parent's
handle can leave that inherited lock held until exec or process exit.

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
  roots lapsed. The cross-file statistics of later runs are rebuilt from the rows that remain, as in decision 0052.

## Built and measured (2026-11-06)

| Step | Where | Tests |
|---|---|---|
| `auto_vacuum` before WAL; `incremental_vacuum` stepped; conversion | `content_store/connection.rs`, `reclamation.rs`, `corpus_runs.rs` | a new store reports 2; an old one converts |
| manifests, `corpus_live`, the decision, passes | `content_store/corpus_runs.rs` | decision rules on synthetic manifests; 20 churns of 8 files under a 64 KB mark: history under the mark after every pass, the last 2 runs never miss, the first version collected, fewer than 64 free pages |
| memo plan per run, key in the entry (memo schema 3) | `logic/plan/corpus/memo.rs` | `an_entry_round_trips_and_keys_are_stable`; the replay |
| the run's manifest and background passes | `logic/plan/corpus.rs` | the measurements below |
| `rerail-engine gc` | `bin/rerail-engine.rs` | on `tapes-full.sqlite` |
| the server's wake | `workspace/storage.rs` | the storage tests |

Follow-up validation (2026-11-07): 226 corpus, rule-language, SQL executor, and content-store tests pass together in parallel,
twice, with four ignored tests and the separate syntax census excluded. Collection stays enabled.
New tests cover concurrent readers publishing memo rows and roots, pending collection resuming on a warm run,
symbolic aliases, process exit, and inherited lock descriptors.

Measured (load 50–60):

- **s200 copy, every file edited before each of 6 runs, 3 MB high-water mark** (one version is about 2 MB). The
  store holds at 3 manifests (2 protected, 1.7 MB of history), 600 tapes and 7.5 MB, with no free pages. There
  were 4 findings each run.
- **Full corpus, 100 rules, a warm run:** 12.32e9 instructions with collection, 12.28e9 with `RERAIL_CORPUS_GC=off`.
  Recording the manifest costs under 0.4%.
- **The full-corpus store, made before 0053 (135 MB):** `gc` converted it in 3.75 s. The run's 8,957 tapes and memo
  rows were kept.
- **A rebuilt engine** makes a new plan, so its first run is cold and writes a new manifest. The previous plan stays
  live while its manifest is among the root's last 2 runs.

The replay is unchanged (3988 273 266 0), and the `content_store` tests pass (105).

## Open

- **The atom epoch (§7) isn't built.** It waits until an arena first approaches 64 MB. The full corpus holds 26 MB.
- ~~Memo rows from before 0053~~: swept (§2a).
- **An older binary's collector** can ignore `corpus_live` or active reader leases. Shared-cache safety requires updated readers and collectors.
- **The marks** (512 and 384 MB) and the protected count (2 runs) are starting values. They should be revisited with a
  measured development workflow.
- The bound covers what corpus runs keep. The workspace server's snapshots keep their own roots, as before.
