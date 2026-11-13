# 0160: Provisional atom ids, without the lease (a trial)

Status: trial (2026-11-13), behind `RERAIL_ATOMS_PROVISIONAL=1`; the default is still the lease (0049)

## Intro

Every string the engine handles (a name, a path, a node's text) is an atom: a small number (`Sym`, u32) that
stands for it. Atoms are dense (0, 1, 2, …), shared by every process that opens a store, and written into the
store's rows: a frame's string columns, the per-file memo, statistics. Two processes must never give one id to
two strings, or two ids to one string, or the rows they write mean the wrong thing to each other.

Decision 0049 keeps that by a lease: an exclusive file lock beside the store. Only its holder gives out new ids. A
run holds it from before its first wave to its end, and gives it back when another process asks.

The Django practice rules report (bug 5) saw parallel runs sharing a store end in `content store writer is busy`.
One run also aborted under `RERAIL_SQL_STRICT` on `SELECT first + length(ends) / 4 FROM atom_segment`: a run that
gave the lease back took it again inside its waves, and taking it reads where the store's ids end. That is a
database operation in the wrong place.

This trial gives out ids without the lease. They are provisional: given out from this process's end, and checked
against the store's when they are written. This record says what that changes, what it measured, and what the
full design needs.

## The lease's failure, measured

Four strict runs at once, one store, over four parts of posthog (hogql, models, api, temporal: 437 to 635 files
each), profiling build. Alone, hogql runs in 8.1 s.

| | lease (0049), twice | lease, once more (load 70–80) | provisional (this trial), twice (load 70–80) |
|---|---|---|---|
| runs that ended with findings | 0 of 4, 0 of 4 | 0 of 4 | 1 of 4, 3 of 4 |
| the others | 3 `busy` after the 60 s lease wait, 1 killed at 120 s | 4 aborted by the strict mode: `acquire` reads the store's end inside the waves | diverged |
| wall time of the four | 120 s (killed) | under 10 s | 17.2 s, 19.4 s |

The runs that ended have the findings they have alone (api 1,057; hogql 1,629; models 494).

The lease's runs did not wait for each other: they deadlocked. Samples of the four processes show:

- across processes, a lock-order inversion: one process holds the lease, and its writer waits for SQLite's write
  lock; another holds SQLite's write lock (a write job interning inside its transaction) and waits for the lease;
- within a process, the main thread sleeps in `Atoms::acquire` holding the `lease` mutex, while its writer, which
  has just committed, blocks in `Atoms::committed` on that mutex.

Nothing moves until the 60 s `LEASE_WAIT`, then the waiters fail `busy`.

## What the trial does

- **Ids without the lease.** A string not in memory gets the next id from this process's end. Outside the waves
  it is looked up in the store first (by its key, decision 0111). Inside a run, with every stored atom held
  (`complete`), nothing is read: a string not in memory is new as far as the run knows.
- **Checked when written.** A segment is written inside the writer's transaction, which holds SQLite's write
  lock, so the check and the write are one step across processes. The store's end equals this process's end, or
  this process's own last segment (`wrote`): the ids are the store's, written as they are. This is the common
  case, and it costs nothing more than before. Past it, with none pending here, this process's end moves up.
- **A divergence.** Past it, with some pending here: another process stored other strings at the ids this one gave
  out. This process is diverged. Its writes behind the run are dropped, as a cache's misses
  (`DIVERGED_TEXT`). It refuses its next run's start (`ready_for_run`), so it never reads rows naming the other
  process's ids with its own meanings. A wrong answer is never given; a failed run is.

## What the trial shows

1. **No deadlock and no read inside the waves.** Only SQLite's write lock serializes processes now. A run never
   waits for another's run.
2. **Divergence is the common case for concurrent cold runs.** All four started from an empty store and stored their
   first atoms within milliseconds of each other ("another process stored atoms 1..2388 while this one held 2387").
   One to three of four diverged, most in phase 0, and a diverged process cannot use the store for its later
   phases. So the check alone is not the design. A process must be able to carry on after another stored atoms
   first.
   - The same strings in the same order are not a conflict, and a check that adopts such a prefix was tried. It
     adopted nothing: each run interns strings of its own early (its rule file, its output's paths), so the
     sequences part at once. It was taken out.
   - Inside the waves the trial reads nothing. A run started on another thread while one is in its waves reads no
     end and no atoms (`hazards::inside_waves`). When the writer moves this process's end past another's atoms,
     it reads those atoms too, behind the run, so `complete` still holds.
3. **The full design is a translation at the store's boundary.** A diverged process keeps its own ids in memory
   (they are consistent within it). The store keeps its own. Between them is a map for the ids past the point
   they parted: local to stored when a row is written, stored to local when a row is read. A string the store has
   under another id maps to that id, and a new one is appended. Every place that writes or reads a `Sym` in a
   stored row goes through it:
   - frames, the memo (0146): their columns are typed (`b's'` for a symbol), so the map applies to those columns;
   - core facts kept from tapes, statistics kept by constant, anything else holding a `Sym` in an untyped
     column. Each needs its type made known, or it is not kept across a divergence.

## Tradeoffs

| | lease (0049) | provisional + translation (this, finished) | ids from the string (a 64-bit hash) |
|---|---|---|---|
| a process waits for another | yes, for the lease (deadlocked here) | no; only SQLite's write lock | no |
| database work inside the waves | taking the lease back reads the end | none | none |
| one id per string, everywhere | yes, by construction | yes, at the store; locally after the map | yes, unless two strings hash alike (checked when written) |
| cost when one process runs | none | none (the map is empty) | ids are 8 bytes, the arena is keyed by hash, not dense |
| cost when processes race | they serialize, or deadlock | a map lookup per symbol column of rows written or read past the divergence | none |
| what has to change | the deadlocks fixed: never wait for the lease holding SQLite's write lock, never sleep holding the `lease` mutex | every stored `Sym` column typed, and mapped at write and read | every `Sym` user that indexes by id (dense arrays) |

Pros of provisional ids: no process waits for another, and a run's waves never touch the database. A single run
pays nothing. The cost appears only when runs race, and then only on the rows that cross the store's boundary.

Cons: the map must cover every stored `Sym`. A column that holds one but is not typed as one is a silent wrong
answer after a divergence, so the types become an invariant, needing a guard of their own (a check that every
column of `Sym`s is typed). Until then, the divergence check is what keeps it safe.

A cheaper change for the corpus runner's case: decision 0131's open item, keying the default store by the corpus
root and not the working directory. Parallel runs over different repositories then share no store, and so
contend for no lease.

## Departures and open points

- The trial's diverged process fails its next run; the finished design maps instead.
- A store whose atoms exceed `READ_WHOLE` (64 MB) is not `complete`, so a run looks atoms up inside its waves,
  lease or not. One posthog/hogql run (9 MB of source) left 485,289 atoms, 77 MB, in the store: 60 MB of it is
  strings over 100 bytes, the texts of whole modules, classes and functions. So the second run over the same store
  read atoms in its waves. The texts came from long-text twins computing `@rr_node_text` before the join that drops
  their rows; 0167 orders the value after it, and the store holds 28 MB.

## Prior work consulted

- 0049 (one interner, the lease), 0104 (no SQL inside the waves), 0111 (atoms found by key), 0114 §4 (reserved
  atoms of components), 0131 (the default store's key).
- ast-index-spike indexes: interned ids were process-local there; no shared store, so no cross-process allocation.
