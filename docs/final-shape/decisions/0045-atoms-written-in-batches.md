# 0045: Atoms are stored as segments of the in-memory arena, written with the rows that use them

Status: proposed (2026-11-06; amended the same day with the segment shape and crash recovery)

## Intro

Every name, path and string in the engine is an atom: a dense integer id. A process keeps all of
them in memory (a mirror): the strings back to back in one byte arena, an end offset per atom, and a
hash table from string to id. The database keeps them too, so ids survive the process.

Today the database copy is a row table plus an index:

```sql
-- One row per string, and the string again in the index: about 2.7x the text on disk.
CREATE TABLE atoms (atom INTEGER PRIMARY KEY, value TEXT NOT NULL) STRICT;
CREATE UNIQUE INDEX atoms_by_value ON atoms(value);
```

A run store with 203k atoms (6.7 MB of text) measured: table 8.3 MB, index 9.8 MB, and almost
nothing else. A corpus run builds its files in chunks and commits each chunk's new strings before
the chunk's rules run: 104 commits for 640k strings on the 9,010-file corpus. Each commit
rewrites about 1,900 index leaves, because new strings land all over the index. That is about 200k
pages, roughly 800 MB, written per run (open item O3; notes T10 §1b).

The per-chunk commit exists because the process has two mirrors that only meet through the table:

- the store's (`content_store::atoms::Interner`), which interns a chunk's strings on the batch
  worker;
- the planner's symbols (`logic::native::setfix::Interner`), which learns them by reading the
  table's tail.

A thread-level lock (`atoms::hold`) also makes a planner run the table's only writer, so the worker
waits for it.

This record makes the in-memory arena the shape on disk too. One mirror is shared per database, and
SQL reads atoms through it. Atoms are written in a few large appends, always before or with
anything that refers to them.

## Decision

### 1. One mirror per database, shared

A process holds one mirror per database file, found by canonical path. The store and the planner's
symbols both use it, behind one reader–writer lock. An id is given out under the write lock, so no
two writers can give out the same id. `hold` goes away, and the worker no longer waits for a
planner run. A failed planner run's symbols stay in the mirror: an unused atom is harmless.

The mirror keeps a watermark, `stored`. Ids below it are in the database; ids from it on are
**pending**.

### 2. On disk: segments of the arena

```sql
-- Atoms first .. first + n - 1: n end offsets (u32, little-endian, relative to the segment)
-- and their bytes. Segments are contiguous: each one's first is the previous first + n.
CREATE TABLE atom_segment (first INTEGER PRIMARY KEY, ends BLOB NOT NULL, bytes BLOB NOT NULL) STRICT;
```

- A flush appends one row: the pending range, copied from the arena. Its pages are written once,
  in order. There is no index to maintain.
- Opening reads the segments in order and rebuilds the arena and the hash table. It checks that
  the segments are contiguous and the strings distinct, and fails as corrupt otherwise (as
  `catch_up` does today).
- Storage is about 1× the text. For the full corpus, about 21 MB of text, or about 6k pages written
  per run against 200k (A).

### 3. In SQL: `atoms` is a virtual table over the mirror

The SQL that reads `atoms` keeps its text: vocabulary fill, prefilter lookups, statistics and
derivations. `atoms(atom, value)` becomes an eponymous virtual table over the shared mirror, in
the style of decision 0008:

- `atom = ?` is an array lookup;
- `value = ?` is a hash probe;
- a range on `value` (the vocabulary's prefix ranges) uses a sorted order of the ids, built the first
  time it is asked for and extended as atoms are added.

SQL sees pending atoms as well, so no reader has to flush first.

The derivation tables' `REFERENCES atoms(atom)` go: a foreign key can't point at a virtual table.
§4's rule replaces the check, in the write path.

### 4. When atoms become durable

Every atom becomes durable. The rules are:

1. **With anything durable that refers to it.** Every write transaction on the database
   (`immediate_transaction` and its `_once` and `_no_wait` forms, and a planner run's
   `flush_symbols`) appends the pending segment first, in the same transaction. The atoms and the
   rows that use them commit together, in one commit. A file meant to survive a crash counts too:
   once open item O4 makes spill files recoverable, a spill flushes first. Today spill files are
   discarded after a crash, so they don't count.
2. **When enough is pending:** 256k atoms, or 16 MB of text, or 30 seconds since the first pending
   atom. This bounds what a crash loses. A flush is now a sequential append, so this is cheap: on
   the full corpus, about 3 appends.
3. **At the end of a run, when the store is closed, and when another process asks for the lease**
   (§5).

The prefilter's `Counts::flush` stops inserting into `atoms` itself. It interns its terms through
the mirror like every other writer, inside the store transaction that §4.1 covers.

### 5. Other processes: a lease

Pending ids are safe only while no other process gives out ids. A single row,
`atom_lease(pid, host, heartbeat, wanted)`, says who may.

- A process takes the lease, in a short transaction, before it gives out its first pending id. It
  refreshes the heartbeat at each flush, and at most every 5 seconds while it holds the lease.
- A process that wants to give out ids sets `wanted`, then waits, up to its busy timeout. The holder
  checks `wanted` at each chunk boundary, with one read. If it is set, the holder flushes and gives
  the lease up.
- A lease is taken over when its holder is gone. That is known at once on the same host, because the
  pid no longer exists. Otherwise it is known when the heartbeat is more than 30 seconds old.

## Crash and restart

What is on disk after a crash at any point:

- **Segments up to `stored`, and only rows that refer to ids below it.** A flush shares its
  transaction with the rows that use its atoms (§4.1), and a SQLite commit is atomic in WAL mode. A
  crash during a flush therefore rolls back the segment and those rows together. No committed row
  can name an id whose string is missing.
- **The lease row, naming the dead process.**

What is lost:

- The pending atoms: at most §4.2's bound (256k atoms, 16 MB, 30 seconds).
- The in-memory work that used them, which died with the process anyway.

On restart:

1. **Open** reads the segments and rebuilds the mirror. It holds exactly the committed atoms.
2. **The lease** names a pid that no longer exists on this host, so it is taken over at once. On
   another host it is taken over after the heartbeat goes stale.
3. **Ids from `stored` on are given out again.** They may now name different strings than before
   the crash. That is safe because nothing durable refers to the old meanings. In-memory
   structures that did refer to them were lost with the process.
4. **The work is redone.** The strings that were pending are interned again when their sources are
   read again, and become durable by §4's rules. A corpus run starts over anyway: it recreates its
   store.

Ids already depend on the order in which strings are first seen. A crash changes that order for the
lost range only, and no finding depends on an id (the findings contract is keys).

## What the prior work says

- **Process-held state.** The spike lost planning state that was held only in Python, so a
  reopened store planned differently. Its rule is that process-held state (interners, frequencies)
  is rebuilt on attach, with a fingerprint check (history index, DH L3319–3343, S174/S178).
  Pending atoms are exactly that kind of state. Open rebuilds the mirror from the segments, and
  checks that they are contiguous and the strings distinct.
- **Ids that depend on order.** In the spike, stored bytes depended on batch order because kinds
  were interned at first sight (history index, S130). Here kinds and fields are grammar codes, not
  atoms (decision 0009), and name ids already depend on order. This record doesn't change that.
- **Decision 0008.** Base relations are served to SQL as virtual tables. `atoms` follows that.
- **Decision 0009.** Strings exist only in `sym_text`, `sym_atom` and `sym_cat` over the mirror.
  It is now one mirror, shared.
- **Decision 0024 §6.** It sets 1 MB of page cache per connection. That is unaffected: atoms are no
  longer read through SQLite's pages.
- **Decision 0026.** Nothing is stored per node beside the tape. The segment row is per flush.
- **Open item O3's closure note.** It proposed "intern ahead … and write the atoms with the first
  layer that uses them". That is §4.1. The lease replaces the reserved range, because ids stay dense.

## Order of work

1. One shared mirror for the store and the planner's symbols, and `hold` removed. Test: the worker
   and a planner run intern at the same time and get distinct ids.
2. The `atoms` virtual table over the mirror, with the readers unchanged. Test: the vocabulary's
   and prefilter's existing tests.
3. `atom_segment`, the watermark, and the flush in the write transactions, at the thresholds and at
   close. `Counts::flush` through the mirror. A schema migration copies an existing `atoms` table
   into one segment. Tests:
   - a write transaction commits its pending atoms with its rows;
   - reopening gives the same ids;
   - a crash simulated after an uncommitted flush leaves no dangling ids.
4. The lease, and its takeover from a dead holder. Test: two connections acting as two processes.
5. Measure on the full corpus: commits, pages written, CPU and instructions retired for the batch
   phase. The replay's totals must stay identical.

## Expected effect

| | Now | Expected (A) |
|---|---|---|
| Atom commits per full-corpus run | 104 | about 3 |
| Pages written for atoms | about 200k (800 MB) | about 6k |
| Size on disk | about 2.7× the text | about 1× |
| Interning's share of the batch phase | 44% | under 5% |
| Worker waits for the planner run | yes (`hold`) | no |
