# 0384. A relation is one relation, written in place

Status: decided (2026-11-17), steps 1–4 built (2026-11-18). The user picked it over two other forms: relations of many parts with
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

4. **A relation's indexes lie in its block** (built, Q176). The fixpoint keeps its sorted runs (decision 0112), and
   each run's arrays lie in blocks of relations, not on the heap: its ids, tags, fences and key prints
   (`BlockVec`).
   - An array goes in its relation's block (`Rows::home`, the tail's, else the newest segment's) when that has room
     past its relations' margin. Otherwise it goes in the open block of the relation's life, so an index does not
     take the room the block's tails grow into.
   - It is written once where it is computed (a run's sort, a merge), and grows as a vector does, in the same block.
     When dropped (a merged run, a dropped relation) its chunk goes back to the block's free list.
   - A chunk keeps what it asked, rounded up to an eighth of its class (`Block::try_alloc_chunk_fit`). The rest of
     the class goes back to the free list at once as smaller chunks. A class alone would keep up to twice the
     array, and in a pool slab no page of it goes back to the OS.
   - The sort's scratch keys (`sort_keyed`) and a printed run's prints in row order are working memory of one call,
     freed before it returns. The sort writes the tags into the run's array (`Extend`) and hands none back.

   - An array goes in its relation's block only while that keeps room for the relation's tail to double past it
     (`Home::grow`): an index never makes the relation it indexes move.
   - A `BlockVec` holds its first element's address (a block never moves), so a probe reads it as a vector's.

   Measured on cold 300-file runs, alternating with the step-3 binary at load 15–20:
   - 97% of the arrays (about 73,000) lie in their relation's own block, and the rest in the open block of its
     life.
   - CPU: instructions retired 974–1,002 G against 975–1,000 G, and sys time the same (7–9 s).
   - CPU peak: 1.61–1.64 GB against 1.50–1.64 GB, about 40 MB more on average, inside one run's spread.
   - Metal: 773–792 G against 775–777 G, and peaks of 2.20–2.25 GB against 2.23–2.32 GB.
   - The findings are the reference's: 5 of 5 CPU runs and 2 of 2 Metal runs at 300 files (the Metal ones before the address and room changes), and 5 of 5 Metal runs at
     100 files. The strict 10-file outputs are byte-identical.

   Two first forms cost more, and were changed before this was built:
   - A chunk's whole class kept the array: up to twice its bytes.
   - A walk from the `Arc<Facts>` to the block on every read cost 1.5% more instructions. Handing a class's rest
     back gave its untouched pages to the OS, about 2 s more sys time.

   Still on the heap: the fixpoint's dedup tables and lattice maps (under the heap sampler's floor at 300 files). The
   shapes of `ordered` (`Runs`, `TaggedRuns`, `Growing`) are the index-cost spike's (`RERAIL_INDEX_SPIKE`), not the
   fixpoint's.

5. **A lattice's key map is an index in its relation's block** (Q177). A lattice relation (decision 0124) holds one
   live row a key. Until this step, three structures held its keys:
   - the host's `Lat`, two `HashMap`s on the heap, from key to live row and from key to changes;
   - the device's `LatKeys`, a key table in device memory, rebuilt by every device fixpoint;
   - the device fixpoint's own copy of the head (`own.total`), copied whole into a new device relation every round a
     key was added, and read back at the end, row by row, into the host's fold, which folded it a second time.

   Now there is one key map: an open-addressed table in the relation's block (`setfix::keys::KeyMap`), a slot a key,
   three words: the key's live row (`u32::MAX` for none), its changes (for `widen`; its first value counts one) and
   its round (where the relation keeps levels).
   - Host and device hash a key by the same function (`key_hash`) and probe the same slots, comparing the key's
     columns in the relation's rows where they lie.
   - The host's fold (`Rel::fold`) probes the map, kills a key's old row and appends its new one (its readers see the
     delta), and points the slot at the new row.
   - The device folds into the head's relation itself, through the same map (`Batch::lattice_fold`, kernels
     `lat_hclear`, `lat_rehash`, `lat_merge`):
     - the head's relation is made at its first round (`Part::for_writes_with_room`), alone in a block with 256 MiB
       of address space past its rows (`LAT_ROOM`; memory only as it is written). Its rows (`Part::reserve`) and its
       map grow in that block, never moved. A head past it refuses its stratum;
     - a held key's row is updated where it lies;
     - a new key's row is written past the relation's rows, into the room reserved, and claimed after the round
       (`Part::written_more`);
     - the map grows (every live slot placed again, on the device) past half full;
     - a round that changed rows in place makes the relation forget its indexes (`Relation::forget_indexes`): an
       index built on it before no longer holds, and the next probe builds its own. Their room stays in the block
       unused.
   - No relation is copied a round, nothing is read back, and nothing is folded twice. A `shortlex_min` value (an
     arena text, decision 0307) is made its symbol where it lies. The store takes the relation as it lies with the
     map (`Fresh::place_folded`), each row at its slot's round, so a later CPU fold probes the map the device built.

   Requirements, each with a test:
   - R1. The host's fold keeps its results (the existing lattice tests: `flat_lattice_fixpoint_joins_copies_and_transfers`,
     `unbounded_lattices_run_only_with_widen`, `a_declared_lattice_replaces_the_join`,
     `shortlex_min_orders_symbols_by_length_then_bytes`).
   - R2. The key map lies in its relation's block, and no `HashMap` holds a lattice's keys (`a_lattices_key_map_lies_in_its_relations_block`).
   - R3. The device's fold gives the CPU's rows and levels, and the head's rows are placed, not read back
     (`a_leveled_recursion_on_the_device_has_the_cpus_levels`, its assertion on `placed` now true for the lattice).
   - R4. Nothing is read back: every head row the device fixpoint makes is placed (the report's `placed` is its
     `distinct`, asserted in R3's test for both heads). The read-back loop and the per-round copy of `own.total` are
     gone from the code.
   - R5. A CPU fold after the device's goes on through the device's map (`a_cpu_fold_goes_on_in_the_devices_key_map`).

   Built: the five requirements' tests pass, and a cold 100-file Metal run over e2's corpus gives the CPU
   reference's outputs (strict SQL). B-now has no lattices, so that run checks the set heads' path is unchanged.
   Left: the forgotten indexes' room is not given back to the block. A head that is probed by a key every round
   leaves an index's room behind each round, within its block's address space.

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
