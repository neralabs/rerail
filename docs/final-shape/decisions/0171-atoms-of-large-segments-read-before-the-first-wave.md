# 0171: A store of large atom segments is read whole before the first wave; a lease taken again keeps "every atom held"

Status: built (2026-11-13). Amends decision 0111 (atoms found by their hash) where it meets decision 0104
(one read before the first wave, no SQL inside the waves).

## Intro

Every string the engine handles is an atom, a dense id (decision 0049). On disk the atoms are
segments, one per flush, each holding a run of consecutive ids. Before a run's first wave,
`Atoms::ready_for_run` reads a small store's atoms whole (up to 64 MB of text). Then a lookup
never reaches SQLite. A larger store is looked up by hash as the run needs atoms (decision 0111).
Strict mode (`RERAIL_SQL_STRICT`) refuses that lookup, because it is SQL inside the waves.

Every whole-repository run of the data-layer pack on a warm store aborted on it:

```text
rerail: RERAIL_SQL_STRICT: SQL inside the waves
  sqlite: SELECT id FROM atom_key WHERE h IN rarray(?1)
  ... Atoms::find ← Atoms::intern_all ← ContentStore::intern_atoms ← facts/vtab/syntax.rs (the batch's strings)
```

To dodge it, the E7 measurements (t36) ran with the tape cache off and a fresh store per
rule, so every run parsed the whole repository again. That was the wrong response: the
abort should have been fixed. This record fixes it.

## Two causes

**1. A lease taken again forgot that every atom was held.** Each phase of a run ends with
`run_over`. The next commit then gives the atoms' lease back, which clears `complete`, the flag
meaning "this process holds every stored atom". The next intern takes the lease again
(`acquire`) but never set the flag again. So every later phase looked up each new string in the
store's key, even on a new store whose atoms were all this process's own.

**Fix:** `acquire` reads where the database's ids end (as before), then sets `complete` to
`holds_all()`. If no other process wrote atoms meanwhile, this process still holds them all.
`ready_for_run` uses the same `holds_all()`.

Test: `a_second_run_still_holds_every_atom` in `content_store/atoms/tests.rs`.

**2. A store of a few large segments was looked up by key.** The data-layer store, after a
whole-repository run:

| `atom_segment` | |
|---|---|
| Atoms | 2,549,263 |
| Text | 154 MB in 7 segments; the 4 large ones hold 48–57 MB each |

A lookup reads its atom's whole segment (`read_atoms`). A run over the same files therefore
reads nearly all 154 MB anyway, one segment at a time, inside its waves. Reading the atoms whole
before the first wave costs no more, and it takes the SQL out of the waves.

**Fix:** `ready_for_run` also reads a store whole when it holds up to 256 MB of text
(`READ_WHOLE_LARGE`) and its segments average 1 MB or more (`LARGE_SEGMENT`).

Decision 0111's case is unchanged: the shared test store, 437 MB in 261,847 mostly tiny
segments, is still looked up by key, because there a lookup reads little.

## Checked

A 44-file corpus (the data layer's GraphQL package), run against a copy of the warm data-layer store
(`RERAIL_TAPE_CACHE=<copy>`, strict SQL):

| Engine | Result |
|---|---|
| Before the fix | aborts after 5 s: `SELECT id FROM atom_key WHERE h IN rarray(?1)` |
| After the fix | passes in 3 s; a second run passes in 1 s, with the same findings |

## What it costs

- Memory: up to 256 MB of atom text held from the first wave. A warm whole-repository run held
  it by its end anyway, one segment per miss.
- One read of the atom segments before the first wave, in place of one read per segment
  inside the waves.

## Not covered

A cold whole-repository run with the tape cache on can still stop on the writer's back-pressure
under strict mode. It queues about 295 MB of tapes and sources, against a 192 MB queue cap. That
was seen at load average 79, with the writer at `nice 19`. Decision 0138 had cleared cold runs on
a quiet machine. That is a separate item (t36).
