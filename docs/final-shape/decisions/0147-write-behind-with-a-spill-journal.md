# 0147: Write behind, as far as memory allows, with a spill journal past it

Status: draft (2026-11-11), an exploration. Only the measurement aids are built
(`RERAIL_WRITE_QUEUE_MB`, the writer's account by kind of write, the end-of-run drain line).

Numbered 0114 where it was written; renumbered 0147 in the hub (2026-11-13) because another record had 0114.

## Intro

**The problem.** A run hands every write to the store's writer thread and goes on (decision
0102). The writer runs the writes into SQLite behind the run. The queue between them is capped
at 192 MB. Past the cap, the next job waits for the writer: the run stops until SQLite catches
up. Under `RERAIL_SQL_STRICT`, that wait ends the run, since no wave may wait for SQL (0104).

At 20 files the cap is reached, and the run stops. The writer is not slow. Its work arrives in a
few large pieces: the memo's entries for all the files at once, the cross part's frames at once.
One piece is larger than the whole cap.

**What this decision proposes.** Let the writer fall behind by as much as memory allows. Past
that, write the queued rows to a journal file instead of waiting, and insert them into SQLite
later: in the same run when the writer is idle, or in the next process.

## What was measured

The 20-file corpus, cold, strict, with the cap raised to 4 GB (`RERAIL_WRITE_QUEUE_MB=4096`).
The debug-profile build; load average 23–52 on the machine.

| | |
|---|---|
| Result | completes, 62 findings (at the 192 MB cap it ends at the back-pressure) |
| CPU (user + sys), wall | 91.3 s, 72.1 s |
| Instructions retired | 1,003 G |
| Peak RSS | 4.57 GB (at 46 s, in the cross part, not the queue) |
| The writer's queue at its fullest | 415 MB |
| Left for the writer when the run ended | 1 job, 0 MB, drained in 0.35 s |

What the writer wrote, by kind (the main run):

| Kind of write | Jobs | Bytes queued | Writer CPU |
|---|---|---|---|
| frames (the cross part's kept frames) | 7 | 415.6 MB | 10.96 s |
| the memo's entries | 7 | 247.3 MB | 3.16 s |
| kept statistics | 2 | 18.9 MB | 0.03 s |
| everything else | about 1,800 | under 10 MB | about 2.5 s |

At 10 files the queue's peak is 0.2 MB and the writer's CPU 0.74 s: no cap is ever near.

So, with room in the queue, the waves do run far ahead of SQLite, and the writer catches up
while the cross part runs. The writer's 14 s of CPU on frames and memo entries is off the
run's path. **What the cap protects is memory**: the queued rows are held in memory until they
are written, and at 20 files that is 415 MB of a 1 GB budget.

## The design

### 1. The cap comes from the memory budget

The queue may hold as much as the run's memory headroom allows (the same headroom the chunk
size is sized from), not a fixed 192 MB. A job under 3 MB never waits (built: the stats write
of a few KB was held behind a 247 MB memo job, and freed nothing by waiting).

### 2. Past the cap, the rows go to a journal, not to a wait

The large writes are rows: frames and memo entries are each a list of
`(space, relation, part, digest, reads, rows)`. The writer already encodes the rows into one
block per frame (the `data` column). A journal record is that block, with its key:

```text
record = length, checksum,
         space (32 bytes), relation, ident (32 bytes), part,
         digest, reads,
         width, rows, types, data   -- the frames table's columns, as stored
```

When the queue is over its cap, the oldest row jobs are encoded and appended to a journal file
beside the store (`store.sqlite.journal`). The file is memory-mapped. Their rows in memory are
freed: each pending value becomes a reference into the map. A read of a pending frame decodes it
from the map, with the same decoder SQLite's rows use.

Appending is sequential, so it costs about what the encoding costs (which the writer pays today
anyway), and the pages it fills are the operating system's to evict, not the run's resident memory.

### 3. The journal is inserted later

- **In the run:** while the queue is under its cap, the writer takes journal records first, in
  order, and inserts them as it inserts any frame. A record inserted is marked done in the
  journal's header.
- **At the run's end:** the journal is flushed to disk (`fsync`) and the process exits. It does
  not wait for SQLite. Today the end of the run waits for the queue: a short wait at 20 files,
  but as long as the writer is behind at larger scales.
- **The next process:** before its one read (decision 0104), it reads SQLite's frames, then the
  journal's records not yet inserted, over them. Its writer inserts them first, and the journal
  is deleted once it is empty.

A crash loses nothing that was journalled: the next process replays it. A record cut short
fails its checksum and is dropped, along with what came after it. Those writes are recomputed,
as any lost write is (0024).

### 4. Only rows go to the journal

Small writes stay jobs in the queue: manifests, pins, choices, counters. They are closures today,
and they are small. Only the row writes (frames, memo entries, kept statistics, tapes and
sources) become records. That is a typed form of the job: `Record::Frames { space, puts }`, which
the writer inserts and the journal stores.

## What it does not fix

- **The memo's entries are large because of what they hold**, not because of how they are
  written. 247 MB at 20 files is mostly intermediate file-local relations that a cross rule reads
  (`base:within`, the `fl…_pyi019_*_binds_n` demand relations). Making them smaller is separate
  work: projecting or demanding the exports from the cross part (0099's next step).
- **The run's own peak.** It is 4.57 GB in the cross part at 20 files. The journal takes the
  queue's 415 MB out of resident memory, not the rest.

## Order of work

1. The cap from the memory budget, in place of the fixed 192 MB. Small jobs never wait (built).
2. Row writes as typed records (`Record::Frames`), the writer's insert unchanged.
3. The journal: append past the cap, references in place of pending rows, insert when idle.
4. The end of the run without waiting: `fsync` the journal, and replay it in the next process
   before its first read.
5. Measure at 20, 50 and 100 files. The gates are: strict runs complete; resident memory under
   the budget; findings unchanged; and the end of the run never waits for SQLite.

## Open questions

- Would the journal be enough as the store of frames, with SQLite holding only an index of
  `(key, offset)` into it? That would make the insert a few bytes a frame, but frames would live
  outside SQLite's transactions. Not proposed here.
- How much of the writer's 11 s on frames is the encoding, and how much SQLite's insert? If it
  is mostly the encoding, encoding on the producing thread (in the waves) would be the wrong
  trade, and the writer should keep it.
