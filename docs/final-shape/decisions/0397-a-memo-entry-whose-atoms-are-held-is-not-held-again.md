# 0397: A memo entry whose atoms the process holds already is not held again before the waves

Status: built and measured (2026-11-18). Follows Q18's breakdown of an edit (e121, the work queue).

## Intro

Before a run's first wave, the memo lookup reads the entries of the files it finds unchanged, and holds their
symbols' atoms (`memo::hold`, `image::hold`). A wave that decodes an entry then finds each atom in memory, and asks
the store nothing (decision 0104).

An entry keeps each symbol as its string, so it reads the same in every process, forked or not (decision 0240 §4).
Holding an entry therefore reads every string it names and looks each one up in the process's atoms. On ep2 an edit
of one rule finds all 300 files unchanged. Their entries name 317,599 distinct strings. Collecting and
deduplicating them took 0.039 s, and looking them up 0.016 s, on every edit (e121).

Every one of those lookups found its atom. The process had read every stored atom before it (the atoms' lease,
decisions 0049 and 0171: "every stored atom held"), and an entry's strings are atoms the store holds. The process
that made the entry interned each string, and its pending atoms are written in the same transaction as the entry,
or before it. So the hold read 300 entries to learn nothing.

## Decision

1. **An entry records where its atoms end** (`image::encode_parts`). `#syms` gains a second row: one past the largest
   atom id its symbols had in the process that made it (the store's atoms handle, `made_under`). An entry naming no
   symbol records 0. An entry written before this has one row.
2. **A process knows below which id it holds every stored atom** (`Atoms::held_below`): `all_held`, the stored atoms
   it read or wrote. `None` once it is forked or maps its ids at the store (decision 0170), because its ids past the
   fork are its own, and with no store.
3. **`hold` reads no entry whose atoms end at or below that** (`image::hold`). The others are held as before: one
   that names an atom not yet stored here, one written before this decision, and every entry in a forked process.
   The profile says how many it read: "the memo's entries' atoms, before the first wave (decision 0397): N of M
   entries read".

The end is a hint for the hold alone. The entry is still decoded by its strings, in every process.

**Where the hint is wrong, the run is still right.** If the process that made the entry forked after making it,
its ids past the fork are not the store's, and the recorded end can be lower than the store's ids of its strings.
A reader may then skip an entry whose strings it does not hold. A wave that decodes it gives each such string a
provisional id (decision 0160), without SQL, and the id is checked against the store's when it is written; where
the store has the string under another id, the reader forks (decision 0170). That needs two processes writing one
store at once, the case those decisions were built for.

## Required behavior, asserted

- `memo::image::tests::hold_reads_no_entry_whose_symbols_are_held`, on a file-backed store:
  - an entry whose atoms are stored and held is not read (`hold` returns 0);
  - beside it, an entry naming an atom not stored yet is read (1), and is not once the atom is stored (0);
  - both entries decode by their strings, as made.
- `memo::image::tests::an_entry_reads_back_as_made_its_symbols_by_content`: an entry reads by its strings in
  another atom table, as before.

## Measured

ep2, an edit of one rule after a cold run, on the M3 (profiling build, load averages 24–26; e126–e129, sent with the
state's whole tree):

| | e121 (before) | e127 | e128 |
|---|---|---|---|
| process CPU | — | 2.21 s | 2.23 s |
| instructions | — | 28.6 G | 29.2 G |
| memo lookup: the entries' atoms held | 0.055 s (0.039 collecting, 0.016 lookups) | 0.017 s | 0.017 s |
| entries read by the hold | 300 | 0 of 300 | 0 of 300 |
| rows, rows that differ from cold | 1,334, 0 | 1,334, 0 | 1,334, 0 |

The cold run (e126) wrote every entry with its end, and the edits after it read none of them. The 0.017 s left is
the entries' blobs being split into their parts to find `#syms`; the hold itself finds nothing to do. The 0.04 s saved
is under 2% of the process's CPU, inside the noise of a host at that load (e121's breakdown recorded its threads'
CPU, not the process's, hence the dashes). With `RERAIL_COMBINE_TAIL=check` (e129) the
kept tail's check passes as well.

## What it does not do

- The atoms' lease still reads every stored atom in a new process (0.046 s on ep2, 215 MB allocated). Reading the
  atoms in place needs them stored with a hash index beside their segments: a change of the store's format, which is
  the user's call (as Q19's is).
- A new process still finds the 300 entries one by one (their digests, then their rows). The whole set could be one
  check that the file-local plan and every file's stat are the last run's.
