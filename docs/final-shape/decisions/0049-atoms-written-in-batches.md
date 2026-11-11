# 0049: Atoms are stored as segments of the in-memory arena, written with the rows that use them

Status: accepted (2026-11-06; amended the same day with the segment shape and crash recovery; built and measured,
with the lease as file locks, §5)

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
A store's mirror also carries the store's identity (`store_identity`, random when the store is
created). Opening a store or a connection to it checks that identity. A store deleted and created
again at the same path, by any process, gets a mirror of its own instead of inheriting the old
store's numbering.
The planner's symbols keep a mirror from run to run, so a run does not read a large store's atoms
again. The mirror holds no file open after the store's last handle closes. That handle also closes
its threads' connections, so the file can be deleted and the store created again. A mirror made
after the store was written reads the stored atoms before the planner's SQL uses them.

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
   atom. This is checked whenever atoms are interned. This bounds what a crash loses. A flush is now a sequential append, so this is cheap: on
   the full corpus, about 3 appends.
3. **At the end of a run, when the store is closed, and when another process asks for the lease**
   (§5).

The prefilter's `Counts::flush` stops inserting into `atoms` itself. It interns its terms through
the mirror like every other writer, inside the store transaction that §4.1 covers.

### 5. Other processes: a lease

Pending ids are safe only while no other process gives out ids. The lease is an exclusive file lock
beside the database, `<db>-atoms.lock`.

- A process takes the lock before it gives out its first pending id, and then reads the segments
  other processes wrote. It gives the lock back after the flush that leaves nothing pending.
- A process that wants to give out ids and finds the lock held takes a shared lock on
  `<db>-atoms.want`, then waits for the lease, up to 60 seconds (then `Busy`). The holder looks
  for that shared lock when it interns, at most every 50 ms. If it is there, a flush is due: the
  holder flushes and gives the lease back.
- The operating system releases a file lock when its process dies. A dead holder's lease is
  therefore free at once, with no heartbeat or staleness rule. That is safe: by §4, nothing
  durable names the dead holder's pending ids, so they can be given out again. (SQLite in WAL
  mode needs one host anyway.)

(This replaces the first draft's lease row with a heartbeat: taking a row needs a write
transaction, which can't be opened from inside another one, and a crashed holder would have
needed a timeout.)

## Crash and restart

What is on disk after a crash at any point:

- **Segments up to `stored`, and only rows that refer to ids below it.** A flush shares its
  transaction with the rows that use its atoms (§4.1), and a SQLite commit is atomic in WAL mode. A
  crash during a flush therefore rolls back the segment and those rows together. No committed row
  can name an id whose string is missing.
- **No lease.** The dead process's file lock was released by the operating system.

What is lost:

- The pending atoms: at most §4.2's bound (256k atoms, 16 MB, 30 seconds).
- The in-memory work that used them, which died with the process anyway.

On restart:

1. **Open** reads the segments and rebuilds the mirror. It holds exactly the committed atoms.
2. **The lease** is free: the operating system released the dead process's lock.
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

## Measured (2026-11-06)

Built as above. On serverplan-quality over the 9,010-file corpus (104 chunks), with machine load 56–58:

| | Before (bulk-tuned rows) | Now |
|---|---|---|
| Atom commits per run | 104 | 3 segments (639,214 atoms) |
| Bytes written for atoms | about 200k pages (800 MB) | 29.6 MB (about 7k pages) |
| Interning in the batch worker | 44% of the batch phase | 0.19 s of 3.7–4.0 s |
| Batch phase (worker CPU) | 8.8 s before any tuning | 3.7–4.0 s |
| Instructions retired, whole run | 415e9 | 387–394e9 |
| Worker waits for the planner run | yes (`hold`) | no (`hold` removed) |

The replay's totals are unchanged (3988 273 228 0, 1638 95 55 0, 2350 178 173 0). The tests in
`content_store::atoms::tests` cover:

- a crash in a child process: only pending atoms are lost, their ids are given out again, and the
  lease is free at once;
- the lease passing between two processes;
- a refused commit;
- SQL over pending atoms.

The crash-recovery tests of the push path (`facts::observe::tests::recovery`) now read the atoms
from the segments on disk.

## Expected effect (the estimate before building)

| | Now | Expected (A) |
|---|---|---|
| Atom commits per full-corpus run | 104 | about 3 |
| Pages written for atoms | about 200k (800 MB) | about 6k |
| Size on disk | about 2.7× the text | about 1× |
| Interning's share of the batch phase | 44% | under 5% |
| Worker waits for the planner run | yes (`hold`) | no |
