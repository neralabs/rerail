# 0138: A frame's use is kept apart from its rows, and the WAL is checkpointed beside the writer

Status: decided and built (2026-11-12).

## Intro

**The problem.** A cold run of the 20-file corpus sometimes ended under `RERAIL_SQL_STRICT` before writing its
findings. Two of three such runs did. At the end of the cross part, its frames reach the store's writer as one
job of 289 MB. That is more than the writer queue's cap (192 MB) on its own. So the run waits whenever the memo's
entries (158 MB) are still queued, and the strict mode ends the run at that wait. The writer was behind because
two of its costs were in the wrong place:

- **Marking a frame used rewrote the frame's rows.** `frames` is a `WITHOUT ROWID` table, and a frame's encoded
  rows are its `data` column. SQLite rewrites a whole record to change one column, overflow pages included. So
  `use_frame_parts`, which sets `used` on the memo's frames, read and rewrote every memo entry it marked: 2.9 s of
  the memo's 4.1 s on the writer in a cold run. Reading `used` had the same cost, since the column comes after
  `data` in the record: `newest_by_path` walked past every entry's rows to read it.
- **Checkpoints ran inside the writer's commits.** SQLite's automatic checkpoint copies the WAL into the database
  and syncs it, inside the commit that crosses its threshold. The writer takes no job meanwhile: 3.0 s of a cold
  run's 7.7 s on the writer.

## 1. A frame's use, apart

`frame_part_used(space, rel, ident, part, used)` holds the run that last used each frame. It has the same key as
`frames`.

- **Writes.** Every frame has its row there: a frame put writes it in the same transaction, in batches of 300
  (three values a frame), and marking a frame used updates only it.
- **Reads.** A frame's use is read from there alone: the collection's scan, the memo's newest frame by path, and
  a relation's expired parts.
- **Deletes.** Each statement that forgets frames forgets their rows there in the same batch. A trigger would do
  it once per frame, a statement each, which the publication's bound on statements refuses.
- **Older stores and binaries.** `frames.used` is still written when a frame is put. A store made before this
  table gets its rows from `frames.used` once: the schema is applied when one of its objects is missing, and
  `INSERT OR IGNORE … SELECT … FROM frames` fills the new table (test: `a_store_made_before_the_frames_use_table_
  gets_it_from_its_frames`). An older binary reads `frames.used` and may collect a frame early, which is a miss,
  not a misread. So the change is additive and the schema floor stays (`schema.rs`).

A cold run's memo entries on the writer: 4.1 s to 0.95 s.

**Not done: the rows apart from the metadata.** A delta's head (decision 0117 step 4) still updates its base
frame's `digest` and `reads`, so the base frame's rows are rewritten. Moving `data` to a table of its own fixes
every such update. It changes `frames`' shape, so it raises the floor (a new store file). It is left until
something else needs a floor raise.

## 2. Checkpoints beside the writer

The writer's connection turns automatic checkpoints off (`wal_autocheckpoint = 0`). A checkpointer thread
(`rerail-store-checkpoint`), with a connection of its own, runs `PRAGMA wal_checkpoint(PASSIVE)` after the
writer's commits. Signals that come while it checkpoints count as one. In WAL mode a passive checkpoint neither
blocks the writer nor waits for it. The collection's checkpoint at the end of a run is unchanged.

## Measured

- **Cold run on the 20-file corpus** (n48, the first run after a rebuild, default cap, load 13): it passes, and
  the queue never fills. Before, it aborted at loads 14 and 26.
- **A one-file edit:** 115.8–119.0 G and 62 findings, the same as before. The main store's WAL is checkpointed
  between runs.
