# 0111: Atoms are found on disk by their hash; a process holds only the segments it reads

Status: built (2026-11-10). Amends decision 0049 §2 ("Opening reads the segments") and §3.

## Intro

Every string the engine handles is an atom, a dense id (decision 0049). On disk the atoms are
segments: a row per flush, holding the end offsets and the bytes of a run of consecutive ids.

Until now, a process that interned its first string read **every** segment. It rebuilt the
whole arena in memory, and a hash table over it, before it could give out an id:

```text
first intern in a process
  └─ Atoms::acquire → read_segments: SELECT … FROM atom_segment ORDER BY first   -- all of them
       └─ for each atom: check UTF-8, hash, insert into the table
```

0049 sized that for the full corpus: about 21 MB of text, read in a fraction of a second. The
store the tests share has grown well past that:

| `$TMPDIR/rerail-tape-cache.sqlite` (2026-11-10) | |
|---|---|
| File | 4.1 GB |
| `atom_segment` | 437 MB: 4,734,985 atoms, 408 MB of text, 261,847 segments |
| Segments of fewer than 16 atoms | 253,901 (a write transaction flushes what is pending, §4.1) |
| First intern, page cache warm | 4 s, about 1 GB resident |
| First intern, page cache cold | 58 s |

A lookup cost the size of the store, not the size of the answer. That is the defect this record
fixes. How the store got this big (atoms are never collected; tests leave their roots behind) is
a separate fix.

## Decision

### 1. On disk: a key from hash to id

```sql
-- One row per atom, written with its segment in the same transaction (0049 §4.1).
-- h is the atom's string hash (`hash_str`, 64 bits) as a signed integer.
CREATE TABLE atom_key (h INTEGER NOT NULL, id INTEGER NOT NULL, PRIMARY KEY (h, id)) WITHOUT ROWID, STRICT;
-- How far the key reaches: every atom below `upto` has its row.
CREATE TABLE atom_key_end (one INTEGER PRIMARY KEY CHECK (one = 0), upto INTEGER NOT NULL) STRICT;
```

- **String to id:** `SELECT id FROM atom_key WHERE h = ?`, then each candidate's string is
  read and compared. Two strings with one hash both have rows, so a hash collision costs one
  extra comparison and is never wrong.
- **Id to string:** `SELECT … FROM atom_segment WHERE first <= ? ORDER BY first DESC LIMIT 1`.
  This is the segment's primary key, one probe.
- **A flush** inserts its atoms' keys sorted by hash, so that one flush writes neighbouring
  leaves together.
- **A flush** writes keys only when the key reaches the segment's first id (`atom_key_end`).
  Otherwise the segments it does not reach are keyed later, as below.
- **An older store**, or segments a capture copied in without keys, gets keyed once, at a
  process's first lookup that misses in memory: the segments past `upto` are read one last
  time, and their keys are written in one transaction.
- **A store that cannot be written then** (read-only, or busy past the timeout) is not keyed.
  That process's lookups read every segment, as before, and the next process tries again.

What it costs to write: a flush of n new atoms into a key of m rows rewrites at most
min(n, m / 230) leaves. On the full corpus that is about 3 flushes of about 200 k atoms into
640 k rows (about 2.8 k leaves each, 34 MB in all), beside the segments' 29.6 MB. That was
0049's objection to a value index: 104 commits a run, each scattering over the leaves. With three
flushes a run it costs about as much as the segments themselves, and it buys lookups that do not
read the whole arena.

### 2. In memory: the segments a process has read

The mirror holds the segments it has read, not all of them, plus the pending atoms:

- A string looked up is found in memory, or by `atom_key`. Its segment is then read whole and
  kept, with its strings in the hash table. Atoms flushed together tend to be used together,
  so the segment is the unit of reading.
- An id's string is found in memory, or its segment is read and kept.
- `get`, which used to answer from memory only, asks the store on a miss as well. A miss in
  memory no longer means "not interned".
- The lease (0049 §5) no longer reads the other processes' segments. It reads only where the
  ids end, so that new ids continue from there.

### 3. In SQL: the `atoms` virtual table

- `atom = ?` and `value = ?` are §2's lookups.
- A range over `value`, or a scan, reads every segment, as before. Only tests ask for those.

## What the prior work says

- **Process-held state is rebuilt on attach** (history index, DH L3319–3343, S174/S178; 0049
  cites it). This record departs from it on purpose. The atoms are durable, so a process does not
  need to rebuild them. It reads what it uses, and its pending atoms are still the only state
  held only in memory.
- **Decision 0110** keeps a relation's indexes as sorted runs, merged when probes pay. The same
  shape was considered for `atom_key`: a run per flush, merged by rent-or-buy. But a string that
  is new (about 640 k on a first corpus run) has to be looked for in every run, so misses would
  cost a probe per run. SQLite's B-tree answers a miss in one probe.

## Order of work

1. `atom_key`, written by `write_pending`, and built for an older store. Test: after a flush
   and a reopen, a lookup finds each atom, and a hash collision (forced in the test) finds the
   right one.
2. The mirror as loaded segments plus the pending atoms. `get`, `with_str`, `value`, `lookup`
   and the virtual table read through it. Test: the existing `content_store::atoms` tests,
   including the crash and lease tests, and a test that a process reading one atom reads one
   segment.
3. Measure on the shared test store: the first intern's CPU (user+sys), wall time, and resident
   memory, against 4 s, 58 s cold, and about 1 GB.

## Measured (2026-11-10, a copy of the 4.1 GB store, load 48)

`probe_a_store` (ignored; `RERAIL_ATOMS_PROBE=<store>`) times one process's first lookup, the
string of id 4,000,000, and a new atom:

| | Wall | CPU (user + sys) | Resident |
|---|---|---|---|
| Before: first intern, warm / cold | 4 s / 58 s | | about 1 GB |
| The store's one keying (4.7 M keys) | 6.5 s | 3.4 + 0.7 s | 240 MB |
| Every later process: lookup, id, new atom | 4 ms each | 0.01 s | 15 MB |

The process holds 1 MB of atoms after these three: two segments, not 408 MB.
