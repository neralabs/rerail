# 0170: A process whose atoms fork from the store's maps its ids at the store

Status: decided (2026-11-13), with provisional ids (0160, `RERAIL_ATOMS_PROVISIONAL=1`)

## Intro

Every string the engine handles is an atom: a dense id (`Sym`) shared by the processes that open one store, and
written into its rows. Decision 0160 gives ids out without a lease: a process takes the next id from its own end,
and checks it against the store's when its atoms are written. When two processes start at once, both give out the
same ids to different strings, and one of them stores first. The other has forked.

0160's trial stopped there: the forked process dropped its writes and refused its next run. Two to three of four
concurrent cold runs failed that way. This record keeps the forked process going: it keeps its own ids in memory,
and maps them to the store's wherever a row crosses the store's boundary.

## The decision

1. **The fork.** A process forks when it finds, inside the writer's transaction (which holds SQLite's write lock),
   that the store's atoms end past where this process's stored ones did, while some of its own are pending. It says
   so once:

   ```
   rerail: atoms: another process stored atoms 1..2387 while this one held 2362 (decision 0170): mapping this
   process's ids to the store's from here
   ```

   Ids below the fork point (`fork`) mean the same in both. Past it, the process's ids and the store's differ.

2. **The map** (`content_store/atoms/map.rs`). Two directions, past the fork point only:
   - local to store, when a row is written. Each local atom is looked up in the store by its key (decision 0111)
     and verified by its string. A string the store does not have is appended to the store, in segments of at most
     64 MB, at the store's end. The ids of segments appended but not yet committed are held as open, and dropped
     if their transaction rolls back.
   - store to local, when a row is read. A store id this process does not know is read (just that string, from the
     segment that holds it) and interned locally.
   The lookups happen in the writer's transaction or when rows are read, never in the waves.

3. **What is mapped.** A write job says what its rows hold (`write_back::Syms`):
   - `Mapped`: rows with symbol columns, mapped inside the job. Frames (`b's'` columns), and the memo's entries
     (their symbol columns, known from the program's declarations, `memo::register_syms`).
   - `None`: no atoms in the rows (tapes, plans, programs, automata, statistics of files, pins, the pending atoms
     themselves).
   - `Unmapped` (the default): rows that may hold a `Sym` in an untyped column. A forked process drops them, as a
     cache's misses (`DIVERGED_TEXT`). Today: the planning choices and the kept statistics by constant
     (`stat_blob`, which also reads none when forked).
   So a job that does not say is never written with the wrong ids. A new job holding atoms has to type its columns
   to be kept across a fork.
   - The derivation tables (`name_bindings`, `derivations`, `derivation_inputs`), written in their own transactions,
     map their atom columns themselves, and their reads map back. The tree counts a run keeps are bound there.
   - Refused once forked, as an error and never a wrong id: the `atoms` virtual table (its ids are this process's,
     the tables it joins hold the store's) and the prefilter's `term_*` rows.

4. **Reads.** A row read from the store carries the store's atoms (`Data::Encoded { store }`), and is mapped to
   local ids when decoded. Snapshot rows (this process's own) are not mapped.

5. **The writer** retries a batch that finds the store busy, with backoff, until the job's deadline. Mapping holds
   the write lock longer than a plain write; a writer that failed `busy` lost the run's results.

## Measured

Four strict runs at once over four parts of posthog (hogql, models, api, temporal), one fresh store, profiling
build, load 80:

| | 0160's trial (twice) | this |
|---|---|---|
| runs that ended with their solo findings | 1 of 4, 3 of 4 | 4 of 4 (three forked) |
| wall time of the four | 17.2 s, 19.4 s (failed runs end early) | 27.1 s |

Findings: hogql 1,629, models 494, api 1,057, temporal 1,063, as alone. The store after: 1,461,017 atoms, each
string once, no gaps in the ids.

A second pass over a store two forked runs (models, temporal) wrote: both read the mapped frames and memo, and end
with the same findings, in 9.4 s against 13.4 s cold.

The first version read each candidate's whole 64 MB segment for every string it looked up, holding the write lock
for seconds while the other processes' writers waited at `BEGIN`. A lookup now reads each segment once per batch
and slices it.

## Tradeoffs

| | 0160 trial: drop and refuse | map at the store (this) | one id from the string (64-bit hash) |
|---|---|---|---|
| concurrent cold runs | most fail | all finish | all finish |
| cost alone | none | none (no fork, no map) | 8-byte ids, no dense arrays |
| cost when forked | a failed run | a lookup per symbol of rows crossing the store | none |
| invariant to keep | none | every stored `Sym` is in a typed column, or its job is `Unmapped` | none |

## Departures and open points

- The four parts' atoms together (90 MB) exceed `READ_WHOLE` (64 MB), so a later run over that store is not
  `complete` and looks atoms up in its waves; under `RERAIL_SQL_STRICT` it aborts. That is 0160's open point, not
  the map's.
- Dropped when forked: the planning choices and the kept statistics by constant. Typing their columns keeps them.
- Refused when forked: SQL over the `atoms` table (the prefilter's gates, the vocabulary) and the `term_*` writes.
  None runs in these runs; a forked process that needs them fails. Answering the table in the store's ids maps them.
- `fact_value_stats` (the facts database's value statistics, `val ANY`) is written on its own connection, outside
  this map. It is empty in these runs.
- No guard yet checks that every column holding a `Sym` is typed. The default `Unmapped` is what keeps it safe.

## Prior work consulted

- 0049 (the lease), 0104 (no SQL inside the waves), 0111 (atoms found by key), 0146 (the memo), 0160 (provisional
  ids and the translation it called for).
- ast-index-spike indexes: interned ids were process-local, with no shared store to map against.
