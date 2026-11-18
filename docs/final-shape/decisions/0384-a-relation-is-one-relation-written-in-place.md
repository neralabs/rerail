# 0384. A relation is one relation, written in place

Status: decided (2026-11-17), step 1 built. The user picked it over two other forms: relations of many parts with
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

1. **An export is one relation** (built, Q93). When a chunk is filed, its derived relation is read where it lies.
   Each live row that no earlier chunk gave is written once into the export's relation, and the room for the whole
   chunk is reserved first. Repeats and rows dead in the chunk's relation are not written, so the export has no
   dead rows. The export holds none of the chunk's parts, which go when the chunk is filed.

   Measured at 300 files:
   - Metal: merges went from 262 (56 MB) to 157 (32 MB), and the profiled peak from 3.8 GB to 3.0 GB.
   - CPU: the findings are the reference's, in 18.3 s against 19.5 s. The peak is 1.79 GB against 1.85 GB.
     Filing costs 0.69 s against 0.62 s (`hold the export`).
2. **A relation started from another's rows is a delta of it.** Its tail is a blocks delta whose ids continue
   after its base's (`Relation::base_rows`), and the device reads base and delta as one relation, as it already
   reads a kills delta. Most of the 157 merges left are of two parts: the shared rows plus a tail.
3. **The device writes a head into its relation.** It claims rows in the head relation's own tail, with one atomic
   add (`rerail_blocks::rows::Rows::claim`), and writes them there, not into a part of its own appended after the
   rows (decision 0325 step 4).

After step 3, `Rows::one` merges nothing and goes. The profile lists any merge it makes until then, under "rows of
many parts merged into one".

## What it does not change

- The CPU readers read a relation's rows the same way whatever their source: one relation is a relation of one
  part.
- Exports are filed in chunk order on the run's thread, so an export's row ids, and the order its readers see,
  stay fixed for a given input.
