# 0384. A relation is one relation, written in place

Status: decided (2026-11-17), steps 1–3 built (2026-11-18). The user picked it over two other forms: relations of many parts with
an index per part, and nesting only the fixpoint's runs in blocks.

## Intro

The device binds a relation as one `rerail_blocks` relation in its block (decision 0325). Some relations were held
as many parts instead:

- an export, a part per chunk that gave rows;
- a fixpoint relation started from a kept one, the kept rows shared plus a tail of its own;
- a head the device wrote, a part per wave appended after the rows.

The CPU reads those parts where they lie. The device could not, so when it bound such a relation it first merged
the parts into a copy (`Rows::one`), kept while the rows were.

On a cold 300-file Metal run that copy was made 262 times, 56 MB in all, of relations of 2–16 parts:

| Where the rows came from | Merges | Copied |
|---|---|---|
| Exports and relations started from kept rows (`Life::Kept`) | 180 | 38 MB |
| Fixpoint relations with device-placed parts (`Life::Run`) | 82 | 18 MB |

The CPU binding made none.

This decision: **a relation is one relation.** Rows that join it are written into it once, and a reader finds it
whole. A relation is never a list of parts that a reader must merge.

## Steps

1. **An export is one relation** (built, Q172). When a chunk is filed, its derived relation is read where it lies.
   Each live row that no earlier chunk gave is written once into the export's relation, and the room for the whole
   chunk is reserved first. Repeats and rows dead in the chunk's relation are not written, so the export has no
   dead rows. The export holds none of the chunk's parts, which go when the chunk is filed.

   Measured at 300 files:
   - Metal: merges went from 262 (56 MB) to 157 (32 MB), and the profiled peak from 3.8 GB to 3.0 GB.
   - CPU: the findings are the reference's, in 18.3 s against 19.5 s. The peak is 1.79 GB against 1.85 GB.
     Filing costs 0.69 s against 0.62 s (`hold the export`).
2. **A relation started from another's rows goes on in their relation** (built, Q175). This amends the form first
   written here, a blocks delta of the base: a delta needs kernels that read a base and a delta as one relation
   (two descriptors, two indexes), and a continuation needs none.
   - Freezing a relation holds its tail as a prefix (`Part::cap`): the frozen rows read only their own. Its
     relation's writer is marked free at those rows (`Held::free_at`).
   - The first relation started from the frozen rows (`Rows::shared`) takes the writer, by one compare and swap, as
     its tail, and goes on writing in the same `rerail_blocks` relation. A reader binds it whole.
   - One writer at a time: a second relation started from the same rows while the writer is taken, or after rows
     were written past the prefix, starts a tail of its own. A taker dropped with nothing written gives the writer
     back (`Took`), and so does one frozen unchanged.
   - A prefix alone is bound as its relation: every reader of a bound relation reads the rows it is bound with, and
     its indexes are of those rows (`find_index`).
3. **The device writes a head into its relation** (built, Q175). The device writes a head's new rows past its rows,
   in room it reserves in the relation's block (`Part::room`), not into a part of its own appended after the rows.
   - The room is a loan (`Lent`): `push_fresh` claims the rows (`Rows::claim_tail`), or, beside rows derived on the
     host or into a lattice, reads them out and lets the next pushes reuse the room (`Rows::unclaimed`).
   - The next group of a stratum's rules writing the same head goes on after the last lot (`Lent::after`). A head with
     no rows yet gets a relation of its own (`Part::for_writes`), which becomes its tail (`Rows::append_part`), and
     the next lot goes on in it.
   - A loan is given back when dropped (a refusal), and claims are checked to come in order.

Measured on cold 300-file Metal runs over e2's corpus:

| | Merged copies | Copied |
|---|---|---|
| After step 1 | 147 | 28 MB |
| Step 2, the writer handed to the first only | 147 | 28 MB |
| Step 2, the writer free again when frozen unchanged | 99 | 16 MB |
| Step 3 too, a stratum's groups chained | 0 | 0 MB |

At 100 files the merges went from 88 (29 MB) to 0. The findings are the reference's: 5 of 5 Metal runs at 100
files, 3 of 3 at 300, and the CPU at 300. The strict 10-file outputs are byte-identical.

The instructions retired by a 300-file Metal run went from 797–823 G to 761–779 G, two alternating runs each, at load
16–22. The peaks are 2.2–2.9 GB on both binaries.

`Rows::one` still merges where a relation is of many parts. That happens when:
- a second relation is started from rows whose writer is taken;
- a tail's block has no room;
- `freeze` merges segments past `MAX_SEGMENTS`.

None happened in these runs. The profile lists each merge under "rows of many parts merged into one", and counts the
heads written into their own tail and the heads given a relation of their own.

## What it does not change

- The CPU readers read a relation's rows the same way whatever their source: one relation is a relation of one
  part.
- Exports are filed in chunk order on the run's thread, so an export's row ids, and the order its readers see,
  stay fixed for a given input.
