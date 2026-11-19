# 0270: a lowered program's atoms are branded, and move to another store's

Status: decided and built (2026-11-15).

## Intro

Lowering a program for a run (`plan::lower`) turns its strings into atoms: a name becomes the store's atom for it,
a kind or a field the grammar's. Until now each became a plain integer (`Term::Int`, `Value::Int`), the same as a
number the rule wrote. Nothing could tell `$i > 2` from a test of the name whose atom is 2.

That mattered once prepared programs were to ship (decision 0264). A name's atom is the store's: a store numbers
strings in the order it meets them, so another project's store gives the same string another atom. A shipped
prepared program has to be moved to the reading store's atoms, and a move by position is not sound while a name
and a number look the same.

## Decision

1. **Branded atoms.** `Term::Branded(Branded)` and `Value::Branded(Branded)`, where `Branded { class, atom }`
   and `class` is `Name`, `Kind` or `Field`. Lowering makes one wherever it turns a string into an atom, and
   nothing else makes one. No pass can take one for an integer: it is another variant, and every reader names it.
   The −1 lowering writes for a kind or a field the grammar lacks stays an integer: it matches nothing.
2. **In the image, too** (format 5). A branded leaf (`kind::BRANDED`, its class in the node's flag bits) and
   branded fact cells (cell kinds 2 to 4). The run's in-place readers (`TermRef::Branded`, `Cell::Branded`) take
   the atom as the integer it is at run time, so execution is unchanged. The tree reader gets the brand back.
3. **The class is explicit**, because it cannot be told from the position: a constant in a comparison, or in a
   functor's argument, has no column to say it is a name. Kinds and fields are fixed by the grammar
   (`kind_atom`, `field_atom`) and never move; they are branded so that nothing confuses them with numbers either.
4. **Moving a program** (`plan::lower::relocate`): every branded name, in rules, `again` rules and deltas, and
   facts, is mapped through the prepared program's intern calls (`prepared_interned`), whose strings are interned
   in the reading store in one call. A name no intern call made refuses the move, and the run prepares again.
   `prepared::lookup` moves a program whose atoms are not the store's, makes its image again, and the run keeps
   the moved program in its store, so the next run reads it as it is.
5. **Printed forms.** The `.rr` printer and the program-text writer refuse a branded atom: a lowered program is
   kept as its image. With `RERAIL_PROGRAM_IMAGE=off` a lowered program is then not kept (it was a fallback).
   The program-as-facts form writes an atom as a `term`/`fact` row of kind `atom`, its class in the text column.
   The facts encoding writes value tags 2 to 4.

## What the readers do with a brand

The rule for every reader that the compiler flagged, and every partial match the review found: where a reader
treated an integer constant as a constant, a branded one is the same constant (planning, joins, SQL, statistics,
digests). A reader that tests for particular integers (`= 0 | 1` on a test's result) no longer matches an atom,
which it never should have. The digests of a term (`digest_term`) give a brand its own tag.

## Measured (2026-11-15)

The Python pack, one file, fresh project store, release, one thread, M3 Max:

| Run | Combine | Prepare | Whole run |
|---|---|---|---|
| No bundle | 9.5 G | 9.8 G | 38.9 G, 2.92 s CPU |
| The bundle, combined program only (0264) | 0.84 G | 9.3 G | 30.9 G, 2.29 s |
| The bundle with prepared programs, moved to the store's atoms | 0.84 G | 4.4 G | 25.6–26.0 G, 1.86–1.90 s |

Every output relation matches the baseline. That covers the three bundle runs, debug runs on a fresh store, an
edited one and a reused one, and a project store whose atoms were numbered apart first (another program ran on it):
there three prepared programs were shipped and moved. A second run on that store reads the kept program (6.24 G,
the warm row). Tests: `a_program_lowered_for_one_store_moves_to_another` (lowered for two numberings, the first
moved is the second; an unknown name refuses; the image keeps the brands), and 135 lowering, image, facts and rr
tests.

What preparing still costs with the bundle (4.4 G, about 0.3 s): compiling the text tests (0.14 s, made again from
their specs on reading), the memo's program and the components' plan (0.03 s), folding the guests' copies and the
tree's projection (0.04 s), reading the programs and assembling the split (0.03 s).

## Open

- `RERAIL_PROFILE` adds about 215 G instructions to this run (254 G against 39 G), outside every span; the s9
  engine from before this decision does too. Profiled runs' totals are not the run's.
- A profiled run that is fully warm (the same inputs on the same store) fails: `profile lifecycle: No such file or
  directory`. The s9 engine too.
