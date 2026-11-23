# 0438: A kept relation is bound where it lies, and kept as the run holds it

Status: built (audited 2026-11-23). Decided (2026-11-18, 0401 step 7). Follows 0401 (F3, F6), 0353 (blocks are never
copied) and 0104 (no SQL inside the waves). Supersedes the snapshot check `admission_matches` and the frames' reads of
the per-stratum keep (`kept_strata`).

## Intro

**Kept facts.** Between runs, the engine keeps what the cross-file part derived, so the next run can skip the strata
whose reads did not change. Since 0401 they are kept as blocks: each space (a program's part) has a base file and one
cumulative delta file, and an entry holds one relation's rows, its dead rows' bits and its levels.

**Binding.** Step 7 makes the next run read those entries where they lie, in the mapped file, instead of decoding a
frame into new columns. A run binds one commit of a space's blocks (`Bound`) and holds a relation over the mapping:

```text
run starts           -> Bound::of(space): the base and delta named by the space's row, mapped once
stratum is clean     -> entry.rel(atoms): an InRel whose columns point into the mapping (no copy)
stratum runs         -> reads the bound rows; rows it adds go to a tail of its own (the mapping is read-only)
run ends             -> each relation's entry put (its rows lent whole, dead bits and levels beside them)
```

Building it showed six places where a relation's state was lost between one run and the next, each of which made the
next run derive again what it had kept, or lose rows.

## Decision

1. **One commit a run.** `Bound` is a space's base and delta as one commit, mapped once and cached per (store, space,
   row). A put made after a run bound its space is not read by that run. The space rows are held with the frames'
   snapshot, the store's state this process already holds (`Snapshot::kept`, no map of their own): read at the
   store's preload, under the snapshot's lock, and replaced by the writer as each write commits. So a
   binding inside the waves asks no SQL (0104); the strict run failed when it did.
2. **Rows pushed past a bound relation are kept.** `Rows::opened` continued its last segment's relation in its block,
   or else moved every row into a tail (`moved_all`). Two faults:
   - `moved_all` left `absorbed` at 0, so the segments stayed beside a tail that held their rows too. A maintained
     relation lost the rows pushed after it was moved: the edit run dropped one `rr_cx_component` row.
   - Over a mapped (read-only) segment, moving copied the kept rows out of the mapping (28 MB on 50 files). Now a
     read-only segment stays where it lies, and the rows pushed after it go to a tail of their own.
3. **An entry is written from the rows, dead ones too, with the dead bits beside them.** `InRel::in_block` wrote a
   relation's kills into its block. A relation bound over the read-only mapping can take none, so it went unkept, and
   the next run derived it again. `kept_rows` now lends the rows as they are (`InRel::rows_in_block`) and the dead
   bits apart, as the entry's format keeps them.
4. **A relation with no block is kept flat.** An empty relation, or a view over a source, has no block of its rows.
   It was left unkept, and its stratum ran again in every next run: a phase's unchanged strata, 12 of 19.
   `retain::kept_put` keeps such a relation's live rows as a flat entry (the derived fact, written once).
5. **A retag keeps its rows' digest.** A relation handed off as the run went is put with its rows and digest then.
   At the run's end it is retagged with its reads, and a retag carries no digest of its own (`kept_reads`: 0). When a
   wave gathered both, the retag's 0 replaced the digest, so every reader of the relation ran again in the next run.
   `KeptGather` now keeps the earlier put's digest and takes the retag's reads.
6. **A cell reused within a process binds the space's current commit.** K4's cells carry a part's frames from one run
   to the next in the same process. `Retained::rebind` binds the space's current commit, and keeps only the derived
   and input metadata that commit still has.
7. **The corpus root has one name.** `run_handed` canonicalizes the root once. Named by an alias (`/tmp`, `/var` on
   macOS), the root was read by the alias, while the observer's other roots and the run's keys used the canonical
   name. A cold run then watched the directory above the root (the store's own), and the warm run watched the alias:
   a cursor written in a warm run.

## Not copies (0353)

- The bound relation: columns over the mapping, read in place.
- A flat entry for a relation with no block (item 4): a derived fact written once, where it is kept.
- The inputs' granules written by the per-stratum keep (`kept_strata::keep`) stay flat entries until step 9 makes
  them lent rows. They are marked in the code.

## Required behavior, asserted

- `rows_started_from_a_bound_relation_read_the_rows_pushed_after_it`: the mapped segment stays, and the pushed rows
  are read once (item 2).
- `a_tail_moved_past_its_rows_holds_every_row_once` (item 2).
- `a_run_reads_one_commit_of_the_kept_blocks`, `an_entry_put_after_the_run_bound_its_space_is_not_read_by_it`,
  `a_derived_entry_replaced_after_the_run_bound_its_space_is_not_seen`,
  `an_input_granule_replaced_after_the_run_bound_its_space_is_not_seen` (item 1).
- `a_kept_input_is_bound_where_its_granules_lie` (F-R1).
- The kept-facts gather test: a retag keeps this wave's digest (item 5).
- `fresh_and_restored_programs_have_identical_relation_identities` (items 3 and 4: a restored phase runs none of its
  strata), `a_new_process_maintains_the_cross_file_part_from_the_kept_rows` (item 3),
  `the_run_after_a_cold_one_writes_nothing` (items 5 and 7), `a_header_edit_asks_again_only_for_the_units_that_read_it`
  (item 7).

## Measured

The 57 data-layer rules over corpus0's first 50 files, with the profiling build (`tmp/run50.sh`), at load 31 on the M5.

| leg | wall | |
|---|---|---|
| cold | 9.6 s | 927 MB peak |
| warm | 0.1 s | |
| edit (a function added) | 2.0 s | 823 MB peak; 2.2 s profiled before |
| warm after the edit | 0.1 s | |
| cold of the edited tree | 8.4 s | |

F-R2: all 589 outputs of the edit, the warm run after it and the edited tree's cold run are equal (48 findings each).
F-R3: a run that changes nothing leaves the facts blocks and their rows as they were.
