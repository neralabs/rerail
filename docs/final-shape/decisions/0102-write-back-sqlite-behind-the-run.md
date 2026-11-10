# 0102: Write-back: SQLite is written behind the run, and waves go on from memory

Status: built (2026-11-10). It builds the writer that 0024 §5 planned ("a writer thread; reads see the
overlay and SQLite; the writer lags at most a wave; back-pressure from a cap").

## Intro

**The problem.** A corpus run moves from wave to wave: the chunks of the file-local part, one after
another, then the strata of the cross-file part. Each wave already reads its inputs from memory. But
between waves the run stopped to write to the store, on its own thread:

- each chunk's memo entries: each entry was copied, encoded and written, before the next chunk could run;
- the prepared program (made into facts, encoded and written, 3.2 s of the Python pack's cold run);
- the combined program (1.1 s), the planning choices (about 1 s), the kept statistics and the kept
  strata;
- the run's manifest.

None of these writes is read again by the run that makes it. They are for the next run, the next
process, or a SQL session. The run waited on SQLite for nothing.

**What changes.** The run hands each such write to the store's *writer* and goes on at once. The writer
is one background thread per store. It applies the writes in the order they were handed over. SQLite
becomes the durable copy that lags behind memory. It is never ahead of memory.

## Prior work consulted

- **ast-index-spike design index, §4 (statistics).** Planning statistics must survive a restart
  (D S124; SORA C7). The index has no design for a background writer: the spike wrote its store once per
  build, and nothing waited on it between waves.
  - **Agrees.** The kept statistics, choices and programs still survive a restart. They reach SQLite
    behind the run, and the run's end waits for them.
- **Lessons ledger row 10.** Measure the code that is actually running.
  - **Agrees.** The profile reports the writer's jobs, bytes and CPU in every run (11 jobs on the slice).
    So a measurement shows whether the queue was in use.
- **Ledger row 30.** A small or fresh store hides volume pathologies.
  - **Open.** These numbers are the slice's. The whole pack's run is the next measurement: its memo
    entries are about seven times the slice's, so back-pressure may engage there.
- **Decisions.**
  - 0024 §5 planned this writer. Its working set stays write-through, for tapes.
  - 0053: the manifest protects what a run names. It is still written after everything it names.
  - 0096: the statistics are one blob a run, now queued.

## The model

A write is a *job*: a closure over the values it writes, which it owns. A job names the keys it writes.

```rust
// native/src/code_rule_dsl/logic/plan/corpus/memo.rs, `remember_all`: one
// chunk's entries, moved into the job (no copy). Each key carries the entry,
// so a lookup gets it from memory until the writer has written it.
store.write_back(
    "the memo's entries",
    bytes, // held until written: counts toward the back-pressure cap
    keys,  // (queued(source, path, program), Some(entry)) for each entry
    Box::new(move |store| {
        // On the writer's thread: encode, then one transaction for the chunk.
        let results: Vec<_> = held.iter().map(|(source, path, e)| {
            (*source, path.as_str(), FactsResult { plan: set.plans[0], path_dependent: true, entry: encode(e) })
        }).collect();
        store.put_facts_results(SCHEMA, &set, &results).map(drop)
    }),
);
```

The model is consistent for the process:

1. **One writer, one order.** There is one writer for each store path in the process, and every handle
   of that store shares it. Jobs run first in, first out. So a job sees every write queued before it.
   The run's manifest is queued last, and it names only what is already written.
2. **Reads see writes.** A read of a key that a queued job writes gets the value the job left in memory
   (`pending`, used for memo entries). If the job left none, the read waits for that one job
   (`settle`). Any other read goes straight to SQLite and never waits.
3. **Bounded lag.** Past 192 MB queued, the next job waits for the writer to catch up. Memory never
   holds more than that unwritten.
4. **Durable at the end.** A run holds the writer from its start, and its last step waits for the queue
   (`written`). A write that failed is the run's error. The last handle of a store to close also writes
   the queue, so no queued write is lost when the process ends.

A failed write leaves the store as a cache that lacks one entry. The next run misses it and recomputes,
as if the write had never happened. The kept strata were always written on a best-effort basis: a busy
store leaves them unkept. They still are.

What a later write needs from an earlier one goes through the queue too. The prepared program's
documents get their source ids when they are written. The manifest must name those ids, so the
prepared job fills a list that the manifest job, queued after it, reads.

## What is queued

| Write | Who waited before | Now |
|---|---|---|
| memo entries, per chunk | each chunk, before the next | queued; moved, not copied; encoded on the writer |
| prepared program | the stats phase | queued; made into facts on the writer |
| combined program | combining | queued; checked and encoded on the writer |
| planning choices | each part's planning | queued (also kept in memory, as before) |
| kept statistics | the end of the file-local part | queued |
| kept strata | the cross-file part, every 4 M values | queued |
| run manifest and collection | the run, before the cross part | queued last; it starts collection |

Reads that settle their key first: the kept strata, the kept statistics, the stored choices, the
stored program and the combined program's manifest entry.

## Measured

On the slice: 8 programs over 486 files, cold runs on the CPU binding, alternated with the build
before this change, at load 19–20.

| | CPU, user + sys | wall | main thread, file-local part |
|---|---|---|---|
| before | 13.65 s, 14.74 s | 12.9 s, 14.2 s | 8.1 s, 9.0 s |
| write-back | 13.36 s, 13.64 s | 11.9 s, 12.4 s | 7.9 s, 7.9 s |

The writer did 11 writes of 43 MB in 0.57 s of its own CPU, beside the run. The main thread's
prepared-program write went from 0.21 s to 0.004 s. The memo entries are no longer copied before they
are encoded, so allocations in that step halved (394 MB to 193 MB).

The findings are identical cold, warm, and after an edit. After the edit, 485 of the 486 files were
read from the memo.

## What still waits on SQLite

- **The tape store of the guests' files.** On the slice this is 0.14 s on the main thread. A stored
  source's id is a SQLite row id, and that id goes on into the memo's keys and the manifest, so the write
  must happen before the id is known. Queuing it needs ids derived from content. Open.
- **Reads a new process needs.** The memo, the prepared program and the choices are read from the store
  by a process that did not make them. A long-lived session keeps all three in memory, so it reads none
  of them.
- **The strata the native engine declines.** They run in SQLite, so they read and write SQL tables by
  nature. This is one stratum on the slice, 0.05 s.

The statistics keeper's counting is about 0.9 s on the slice. It is in memory, and it does no reading
of SQLite on a cold run.
