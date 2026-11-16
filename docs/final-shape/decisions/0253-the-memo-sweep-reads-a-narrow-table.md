# 0253: the memo sweep reads a narrow table

Status: superseded by decision 0240 §4 (2026-11-15, at the hub merge into investigate-bend-vs-datalog): a memo
entry is a row of `memo_entries`, with its content digest in its own column, and the sweep reads that table, so
`frame_part_used` has no `content` column. Was built (2026-11-15). Amends decision 0227 (reachability) and the frames' schema (`frame_part_used`).

## Intro

The store keeps each file's memo entry as a frame: a record in `frames`, holding the entry's rows inline. The
collection that decision 0227 runs keeps an entry while a kept manifest names its content. That content's sha256 was
in the frame's `reads`, so the sweep read every memo frame's record. The records hold the rows, so the sweep read
nearly the whole table, on every collection.

Decision 0227 judged that cheap enough for now. The user asked for it to be fixed (2026-11-15).

## Decision

`frame_part_used`, the narrow table beside `frames` that each frame already has a row in, gets a `content` column:

| Frame | `content` |
|---|---|
| A memo entry (reads: its path and key, then its content's sha256 in four words) | the 32-byte sha256 |
| Any other frame | empty |
| A row put before the column | NULL |

It is written with the frame's row there, and again when a delta changes a frame's reads.

The sweep reads `space, rel, ident, part, content` from `frame_part_used` alone. A NULL row (an older store's) is
filled from its record's reads, once, in the same transaction. The column is added to an older store as it opens
(`ADDED_COLUMNS`); this store has no migrations beyond that.

## Measured

On a local store of 121,222 frames (`frames` 714 MB, `frame_part_used` 11 MB), with the file cache cold:

| Read | Time |
|---|---|
| Every frame's `reads` (the old sweep's read) | 1.28 s |
| Every `frame_part_used` row | 0.14 s |

With the file cache warm, both are under 0.03 s.

## What this gives up

- 32 bytes more for each memo entry, in a table read only when frames are put and collected.

## Tests

- `the_memo_sweep_reads_the_content_digests_alone` (store): after the records' reads are blanked, the sweep still
  collects by the stored digests. A row with no digest gets one from its record and is collected by it.
- `frames_are_kept_while_a_kept_manifest_reaches_them` and `an_older_store_gains_the_added_columns`, unchanged.
