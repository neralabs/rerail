# 0439: The frames are retired, and the kept facts are read through SQL

Status: decided (2026-11-19, 0401 step 9). Follows 0401 (F5, F6), 0438 (step 7), 0104 (no SQL inside the waves) and
0353 (blocks are never copied). Supersedes the frames of 0104, 0117 (confirmations, retags, deltas), 0138 (use rows),
0227's collection of maintained frames, 0295 §4 (frame segments) and 0302 (`hold_frames`).

## Intro

**Frames.** Until step 9, the engine kept what a program's cross-file part derived twice: as frames, one row of the
`frames` table per relation and part, its columns zigzag varints of their differences; and, since step 6, as kept
blocks (0401). Step 7 (0438) made the next run bind the blocks where they lie. Nothing read a frame after that, but
every run still wrote them.

**Step 9** takes the frames out, and makes the kept blocks readable from SQL in their place (F5):

```text
before                                   after
run end   -> frames rows + kept blocks   run end   -> kept blocks only
next run  -> binds the blocks            next run  -> binds the blocks
SQL       -> SELECT ... FROM frames      SQL       -> SELECT ... FROM kept_rows(space, rel, ident, part)
```

## Decision

1. **The engine writes and reads no frame.** The keep (`retain::write_kept`, `write_input_kept`) puts kept entries
   only: no frame, no levels part, no retag or delta frames, no hold rows. What a relation's next run compares is the
   entry's digest and reads (0438 item 5). The engine-side names say what they are: `InKept`, `Retained::bind_kept`,
   `write_kept`, `kept_tests`.
2. **The store has no frame machinery.** `content_store::frames` is gone. What remains of it is the store's state this
   process holds (`content_store::held`): the snapshot of each space's kept-blocks row and the per-file memo's
   admitted entries, read before the first wave (0104). The relation key and stored digest are `KeptRel` and
   `KeptStored`. `cross_rows::encode_windows`, which only frames used, is gone.
3. **An old store loses its frames when it is opened.** The seven frame tables are `schema::RETIRED_TABLES`: SCHEMA
   no longer makes them, and an open that finds one drops them all and removes `<store>.frames`. This raises no
   floor. An older binary makes the tables again (its SCHEMA's `IF NOT EXISTS`), and this build drops them again at
   its next open: a cache's rows lost.
4. **The sweep collects memo entries only.** The upkeep job is `MemoSweep` (`sweep_memo`, `memo_sweep_wanted`,
   `collect_unreachable_memo`). It keeps a live plan's entry while a kept manifest names its content (0227). A
   space's kept blocks are collected by their roll-up (0401 F4, step 8).
5. **The kept facts read through SQL (F5).** Two eponymous virtual tables are on every store connection, beside
   `atoms`:
   - `kept_entries(space, rel, ident, part, digest, rows, types, reads)` is each space's directory (or one space's,
     with `space = ?`). `rows` is the live rows' count.
   - `kept_rows(space, rel, ident, part)` gives one entry's live rows: `id`, `level`, `c0` … `c31` (NULL past its
     width).

   A cursor steps over the live ids in the mapping and reads each value where it lies (`View::get`). Nothing is
   collected into rows (0353). A row is left out when the run killed it (the entry's dead bits) or its relation
   dropped it. The space rows come from this process's snapshot, else from the store's last commit. An `s` column
   holds the store's atom ids, as `atoms` answers them, so the two join.

## Required behavior, asserted

- F-R5: `the_kept_relations_read_through_sql_equal_their_views`: entries, digests, types, reads and live rows'
  counts equal the view's; each entry's rows equal its live rows; a killed row is left out; a column past the width
  is NULL; an entry not kept has no rows.
- `a_store_of_before_loses_the_retired_tables_and_their_segments` (item 3).
- `a_run_that_begins_stops_a_sweep_of_many_small_deletes`, now over 3,000 memo entries (item 4).
- `atoms_another_process_stored_past_this_ones_end_are_mapped_after_a_fork`: a kept entry written after a fork holds
  the store's ids in its block (the frame's half of the test, ported).
- The step-7 tests (0438), unchanged, now with no frame behind them.

## Not copies (0353)

- `kept_rows` reads each value in the mapping. Its cursor holds its space's mapping and its place, not the rows.
- `kept_entries` holds each entry's key and counts, which are metadata, not rows.

## Measured

The 57 data-layer rules over corpus0's first 50 files (`tmp/run50.sh`), with the profiling build, at load 36 on the
M5:

| leg | wall | instructions | peak |
|---|---|---|---|
| cold | 7.5 s | 212.4 G (231.0 G with the frames) | 712 MB |
| warm | 0.1 s | | |
| edit (a function added) | 1.7 s (2.0 s at step 7) | 32.8 G (34.6 G with the frames) | 547 MB (765 MB with the frames) |
| warm after the edit | 0.1 s | | |
| cold of the edited tree | 7.3 s | 196.0 G | 686 MB |

All 589 outputs are equal across the edit, the warm run after it and the edited tree's cold run, with 48 findings
each. The store holds no frame table and no `.frames` (18,687 frames and 39 MB before).

## Open: the blocks are larger than the frames were

0401's step-6 measurements set a condition: before the frames are retired, the copies must be no larger than the
frames. In this tree they are not. The kept blocks take 211 MB on disk over 20 spaces, against the frames' 39 MB. A
block's columns are 8-byte words with their indexes, where a frame's were varints, and one space's base is 121 MB of
it. A space also keeps the delta it replaced until its next write, which sweeps it (0401 F2), because a process still
reading the previous commit can open its files: one replaced delta per space at most, 28 MB here.

The format that meets the condition is built on a sibling branch: the user's "kept facts are packed chunks, appended
to a chain of mapped files" (numbered 0402 there, decided 2026-11-18, steps 1–7 built). It replaces 0401 F1, F2 and
F6's copy and keeps F3–F5. This tree retires the frames ahead of it, because no reader was left (0438) and writing
them cost 18 G instructions a cold run. Merging that work here is what meets the condition. Its kept-relation reads
take this decision's tables (F5) as they are.
