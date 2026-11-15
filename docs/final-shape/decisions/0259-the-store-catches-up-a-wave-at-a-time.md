# 0259: The store catches up a wave at a time, and the write-behind is a spool on disk

Status: stage 1 built (2026-11-15). Stage 2 (the spool, §5) is replaced by decision 0260 §8: the journal is a
file-backed delta block. Drafted as 0255 (renumbered: 0255–0258 are taken in
other worktrees). Supersedes decision 0102's commits by size and idleness (a batch closed at half the queue's cap,
`BATCH_IDLE`), and, with the spool, its in-memory cap; settles decision 0147's journal. Keeps decision 0240 (the
writer only writes; a phase's run leaves its writes queued) and 0024 §5 (the store may lag; what it lacks can be
made again).

## Intro

A run's writes to SQLite go to one writer thread behind the run (decision 0102), so no wave waits for SQLite. The
writer committed whenever its batch reached half of the queue's 192 MB cap, or the queue was idle for 500 ms.
Nothing marked where a wave ended. So the store's facts caught up at points unrelated to the run: a crash, or a
reader in another process, could see half a wave.

The user asked for the store to be consistent wave by wave (2026-11-15):

> We also want the waves to record facts into the DB so that it can be queried in SQL and used as a source of
> truth, but writing is slow so we queue up the writing work and let it catch up. [...] Each wave should be a
> single transaction; the goal is that the SQLlite database be consistent in its facts and catch up atomically
> wave by wave.

Asked how a wave's writes should be held while it runs:

> I want the write-behind on disk, efficiently encoded in deltas, in a memory mappable linear structure in the same
> format as it's stored in memory, and done one batch at a time including commits and WAL. And then the cap can be
> put way higher and we can only stop when it's clear that the db will never catch up or the disk is getting full.

Asked where a wave ends, the user chose each chunk, the cross part and the run's end, "but make it easy to change".

A survey of the other worktrees (2026-11-15) found no spool built anywhere. Decision 0147 designed a journal used
only past the memory cap, in a record format of its own. Decision 0240 §4 appends memo images to a pack beside the
store.

## Decision

### 1. A wave is the run's

A run's writes are waves of its own (`write_back::run_waves`, at the start of `run_observed`): a thread-local,
`WAVES`, names the run, and the run's jobs on other threads carry it (`run_state`, **Carried**). A job joins its
run's wave still being written. The wave waits in the writer, unsent, until a boundary of its run seals it.
Concurrent runs in one process (tests, a session beside a phase's run) share a store's writer, and each seals only
its own wave.

A run writes to several stores (the store, the tape cache, the observer's). A boundary seals the run's wave in the
writer of every store (`end_wave`). Each store takes its part in a transaction of its own: two SQLite files cannot
commit as one.

A job queued outside a run is a wave of its own.

### 2. The store takes one wave a transaction

The writer takes the sealed waves in the order they were sealed. A wave is one transaction
(`ContentStore::in_one_transaction`), each job a savepoint in it. A wave that finds the store busy, or that a job
forked (decision 0170), is rolled back whole and run again. No commit is made by size or idleness.

The WAL is checkpointed beside the writer after each wave (`PASSIVE`, its own connection): it copies only what is
committed, so only whole waves.

A job that must commit before it goes on (`write_back_alone`: the manifest, which collection reads on another
connection) seals the run's wave before it and is a wave of its own, in transactions of its own.

### 3. Where a wave ends is one table

| Boundary | Where | Seals |
|---|---|---|
| `Before` | before the chunks: the combined program, the prepared sources, the plan holds | yes |
| `Chunk` | each chunk's `process` (tapes, tree counts, memo entries); a load in the cone is a chunk | yes |
| `ConeRound` | a round of the cone | no |
| `Cross` | the cross part's end (`statements.enter("write")`): kept strata's frames, retained rows, rounds, statistics | yes |
| `End` | the run's end (`written`, and the end of `run_waves`): cross memo, manifest, atoms, kept plans | yes |

`write_back::seals` is the one place that says which boundaries seal a wave. Changing where waves end is an edit to
that function.

### 4. What waits

- `written` (the run's end) seals the run's wave and waits for every wave sealed so far. A nested phase's run seals
  at its end and does not wait (0240 §1).
- A read of a queued key gets the value the job left in memory (`pending`), as before. A read that waits for its
  job (`settle`: the kept statistics) seals the wave that holds it first; otherwise it would wait for the run's own
  boundary.
- Back-pressure waits for the writer only while it has sealed waves to take. The wave still being written is never
  waited for, since only its boundary seals it. A cache write (`write_back_unless_full`) is still dropped at half
  the cap.
- A handle's own transaction no longer commits the writer's open batch first (`yield_writer`, gone): the writer
  holds a transaction only while it takes a sealed wave.

### 5. The spool (stage 2)

A wave's writes become rows, not closures. Each kind of write is a relation of typed columns, named for the table
it writes and how. A sealed wave is one file of a 0239-style image: flat regions of fixed-width items, mapped where
it is read. Blobs are stored as they are already encoded: a frame's columns as `cross_rows`' zigzag varints of
differences between rows, a memo entry or a tape compressed. So the bytes on disk are the bytes in memory, and a
read ahead of the store (`pending`) reads the mapped file.

The spool is a directory beside the store (`<store>.spool/`), one file per sealed wave, written atomically (beside
its name, then renamed). The writer takes the files in order. Each wave's transaction also moves the store's
`applied_wave` row to the wave's number, and then the file is removed. A file whose wave is not past `applied_wave`
was applied already and is removed unread. A file that does not check is a wave not written: it is removed, and a
later run makes its work again.

The cap becomes the disk and the writer's pace. A producer waits only when the store's disk has less free than
`max(2 GB, 5%)`, or when the writer cannot catch up (the spool's bytes over its measured rate exceed the run's time
so far, with at least 30 s of backlog).

The image format lives in the logic crate (`logic/plan/output.rs`), above the store's. The spool's image moves down
to the store crate, or a layer below both.

## Staging

1. **Built.** Waves per run; `end_wave` at the boundaries; one transaction a wave; no size or idle commits;
   `yield_writer` gone. Waves are held in memory, as closures, until sealed.
2. The spool: the image's byte column, the spool's files, `applied_wave`, the writer taking files. Each kind
   converted to rows, the largest first (frames, memo entries, tapes), each conversion removing its closure.
3. The disk and pace limits replace the memory cap; `RERAIL_WRITE_QUEUE_MB` goes.
4. A guard test: no `write_back` call queues a closure.

## Consequences

- A 10-file run (one Python pack rule, a cold store) wrote 19 jobs in 4 transactions: before, its one chunk, the
  cross part, the end. A 49-file run wrote 123 jobs in 15. A warm run over the same store gives the same findings.
- Until the spool, a wave's jobs stay in memory until sealed. The large ones (frames, memo entries) share their
  rows with what the run holds anyway (the frames' snapshot holds the queued frames), so a long cross part holds
  little more than before. The writer no longer starts on a wave's first jobs while the wave runs.
- Open: whether the spool's waves should replace 0240 §4's pack for memo entries (one file of the entry's bytes,
  not two). 0147's question, whether the journal should be where frames are kept, is answered no: the store keeps
  them, and the spool holds them only until the store does.
