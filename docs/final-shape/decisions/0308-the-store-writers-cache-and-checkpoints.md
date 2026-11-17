# 0308. The store writer's page cache holds a wave, and its checkpoints wait for 64 MB of WAL

Status: decided (2026-11-16). Follows 0314's measurements: after the arena's lock, the store's I/O was the largest
kernel cost of a cold PostHog run (`pwrite`, `fsync`, `pread`: about 24 thread-s). Builds on 0102 (write-back) and
the writer's checkpointer (`write_back::start_writer`).

## Intro

A whole-run sample (M3 Max) split the store's I/O by cause:

| what | about |
|---|---|
| the checkpointer, after every wave: the WAL copied into the database and synced | 10.8 thread-s |
| SQLite spilling a transaction's pages into the WAL before its commit (`pagerStress`) | 5.7 thread-s |
| commits writing their WAL frames | 3.2 thread-s |
| B-tree pages read back from the WAL as they changed again | 1.9 thread-s |

Every connection keeps a 1 MB page cache (`CACHE_KIB`), which bounds the readers, one per worker thread. The writer
is one connection, and a wave's transaction is larger than 1 MB. So it spilled its changed pages into the WAL
partway through. The memory map covers only the database file, not the WAL, so it then read those pages back
with `pread` when it changed them again.

## Decision

1. **The writer's connection keeps a 32 MB page cache** (`WRITER_CACHE_KIB`), so a wave's changed pages stay in
   memory until its commit. The readers keep 1 MB.
2. **The checkpointer copies the WAL only once it holds 64 MB** (`CHECKPOINT_WAL_BYTES`), and once more as the
   writer ends. A page that consecutive waves change is then copied and synced once per 64 MB of WAL, not once per
   wave. The final checkpoint means a later run reads the store through its map, not from the WAL.

## Measured

PostHog on the M3 Max, same 29,747 findings (fa0efeaed6ff). Whole-run `sample` counts:

| | before | after |
|---|---|---|
| spill (`pagerStress`) | 6,279 | 314 |
| `pread` | 5,168 | 2,005 |
| `fsync` | 6,003 | 4,716 |
| `pwrite` | 11,438 | 10,256 |
| checkpoint | 11,151 | 9,439 |
| commits | 3,890 | 8,171 (the pages that spilled are written once, at commit) |
| cold kernel CPU (`sys`) | 61–67 s | 52.5 s |
| peak | 2.63–2.66 GB | 2.65 GB |
| warm | 1.04–1.07 s | 1.01–1.08 s |

**The run's wall does not depend on any of this.** The writer runs behind the run (0102). A profiled run's writer
spent 14.8 s of CPU on its own thread in 5,466 transactions (one a wave): its commits 6.8 s, the memo's entries
3.3 s, new atoms 2.6 s. Nothing in the run waits for it, and the run's tail after its evaluation is 0.29 s. So this
decision saves CPU and kernel time on a busy machine, not wall. The checkpoint did not shrink much, because most
pages are written once: what remains is the volume, a 643 MB store written once into the WAL and once into the
database.
