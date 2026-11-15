# 0240: The store's writer only writes, and no run waits for it before the process ends

Status: building (2026-11-15). Refines the write-back queue (decision 0102), the tape store (decision 0104) and the
per-file memo's frames (decisions 0146, 0212).

## Intro

A run's writes to the store go to one writer thread (decision 0102), behind the run. On PostHog (29,885 Python files,
one rule, cold, on the CPU) that thread spent 60 s of CPU, and the main thread waited for it for 60 s of the run's
487 s. Giving the queue more memory did not help: with `RERAIL_WRITE_QUEUE_MB=4096` the queue peaked at 439 MB and
never held a producer back.

The wait was a barrier. A run ends by waiting for every queued write, and phase 1 (`sources.rr`, which loads the
source files) is a run of its own inside the run. So the main thread stopped at phase 1's end, until 879 MB of tapes
and memo entries were in SQLite, though nothing read them from there: a queued write is read from memory until it is
written.

The writer's own time grew faster than what it wrote. Its memo entries went at 100 MB/s on 437 files, 39 MB/s on
5,413 and 10 MB/s on 29,885. A stack sample on 5,413 files (`tmp/light/w3/sample`) put its time in zstd compression
of tapes and sources (28%), inserts of memo frames into SQLite (28%), SQL parsed again at each call (6%), and the
memo's per-document bookkeeping.

## Decision

### 1. A phase's run leaves its writes queued

A run nested in another (`phases::in_phase`: phase 1, and every later phase's run) does not wait for the store's
writer at its end. The writer is one per store path, shared by the nested and the outer run, so the outer run's end
waits for the same queue and reports the first write that failed.

### 2. Sources and tapes are compressed where they are parsed

Taken from `sync-integration-hub-tape` (0c98766a5): the parse's worker compresses the source and the tape
(`ContentStore::source_and_tape_frames`), the queue holds the frames, and the writer only inserts them. A cache's
write is still dropped, not waited for, where the queue is full (decision 0104). The batch's and a small job's sizes
follow the cap in force (`cap_bytes`), not the default: under `RERAIL_WRITE_QUEUE_MB` they never committed early.
Its frame commit (dd8542774, frames encoded where they are queued) is superseded by decision 0212, whose frames are
queued encoded (`Encoded`). Its second join of the checkpointer is superseded by this branch's join as the queue
closes.

### 3. The store's statements are prepared once

`Connection::query_row` and `Connection::execute` parse their SQL at every call. The store's literal statements go
through the connection's statement cache (`connection::Cached`: `query_row_cached`, `execute_cached`), whose capacity
is now 256.

## Consequences

- `posthog/` (5,413 files), cold on the CPU, before and after:
  - the writer's CPU fell from 5.29 s to 2.0 s: tapes 1.63 s to 0.26 s, batches 42 to 9, memo entries 2.15 s to
    1.21 s;
  - phase 1's end no longer waits for the writer (0.9 s);
  - instructions fell from 566 B to 547 B. User CPU fell from 54.6 s to 31.2 s, but the load fell too (63 to 21).
- Tests: the store's (`rerail-store`, 204).
