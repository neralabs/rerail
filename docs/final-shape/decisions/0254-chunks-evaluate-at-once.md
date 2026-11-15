# 0254: A run's chunks evaluate at once, each on a connection of its own

Status: building (2026-11-15). Refines the chunked file-local part (`corpus::run_observed`, the chunk builder of
decision 0029) and the native engine's loaded inputs (`setfix::INPUTS`).

## Intro

A run cuts its files into chunks of about 12k–262k nodes. Each chunk's file-local program runs over its sources,
and the main thread files the chunk's rows: its exports, the totals the cross-file part plans with, and its memo
entries. Until now the main thread did all of it, one chunk after another. The pool parsed and batched the next
chunks meanwhile, but the evaluation itself was serial.

On PostHog (29,885 Python files, one rule, an M3 Max) the pool's threads were about 96% idle. 90–92% of their
on-CPU samples were rayon's idle loop (`wait_until_cold`, yielding), woken for the small jobs a component's strata
spawned. Each wakeup found little work. The main thread's local phase was 66 s: 35.5 s of runs, 24.3 s of guests,
3.4 s of filing. Its wait for the chunks was the run's length.

## Decision

### 1. A chunk's evaluation is a job; its filing stays on the main thread, in order

`process` is split in three steps:

- **Begin** (main thread). Replays the chunk's memo hits, measures its constants, and hands on a `Task`: the
  chunk's sources, documents, statistics and mix.
- **evaluate** (any thread). Runs the guests' rounds, measures and binds the sources, and runs the file-local
  program on the connection it is given. It reads the exports' rows out of the engine, then forgets the chunk's
  relations and inputs. It returns `Evaluated`. It reads only what the run made before the chunks, so it is `Sync`.
  The measurements made once per mix (`measured_once`, `guest_postings_once`) sit behind locks.
- **Finish** (main thread). Files an `Evaluated`: the totals, the stored base rows, the exports, the semi-join's
  rows and the memo entries.

`drain` keeps up to K chunks in flight as jobs (`a chunk evaluated`). It files them in the order they began.
While it waits for the front one it runs the pool's jobs (`help_until_as`). Filing in order keeps the run's
outputs, totals and memo entries the same as a serial run's.

K is `RERAIL_CHUNKS_AT_ONCE`, else the pool's threads less one (the main thread, which files), at most 8. A run
without the memo, and the cone's loads, evaluate one chunk at a time on the run's own connection, as before. A run
without the memo hands each export off while the run goes on (`Hooks::hand_off`). The builder keeps K+1 chunks
projecting (`in_flight`), so there is always a chunk ready for a free slot.

### 2. A chunk evaluates on a connection of its own

Each job takes a connection from a small pool, or opens one. It opens on the run's database, the way the cone's
connection opens: the store's atoms installed, `ready_native`, `x` attached when SQL first runs, and the run's
inputs from before the chunks bound (the repository's relations and the phases' constants). Its symbols are the
run's, found by the database's path. The connections are closed after the first drain.

### 3. The engine's loaded inputs are each connection's

The native engine kept the input relations it loaded (`INPUTS`) and the relations that change between rounds
(`CHANGING`) by the database's **path**. A run begins by forgetting its inputs (`run_native_inputs`). So a chunk
that began while another ran dropped that chunk's loaded input and loaded its own under the same key. The other
chunk's next stratum then read the new chunk's rows. On `posthog/` (5,413 files) the run gave 666 findings
instead of 7,080: 1,172 files lost all of theirs.

Both maps are now keyed by the connection's scope (`conn.at()`), the key the engine's derived relations already
used (`DERIVED`). A connection's close (`forget_scope`) drops its entries. The stored tables (`STORED`) stay keyed
by path. They are tables of the database, the same for every connection on it, so one connection's forgetting
only makes another read them again.

`setfix::inputs_tests::a_connection_reads_its_own_input_while_another_on_its_database_loads_one` reproduces the
interleaving. It fails with path keys (a reads b's row) and passes with scope keys.

### 4. The chunk size counts the chunks in flight

A chunk's growth in memory is measured as before, but it is divided by the nodes of every chunk evaluating at once.
The next chunk's size is chosen from that.

## Measured

`posthog/` (5,413 files, `len-_py.rr`, cold store, profiling build, an M5 Max loaded by other work, load average 27–39: the walls are rough, the CPU and instructions less so):

| | wall | user | sys | instructions | peak | findings |
|---|---|---|---|---|---|---|
| one at a time (`RERAIL_CHUNKS_AT_ONCE=1`) | 71.4 s | 48.9 s | 7.5 s | 513 B | 0.90 GB | 7,080 |
| 8 at once | 25.4 s | 45.0 s | 7.7 s | 491 B | 1.40 GB | 7,080 (the same set) |
| 8 at once, warm (all memo hits) | 5.7 s | 2.2 s | 2.0 s | | 0.33 GB | 7,080 |

All of PostHog (29,885 files, the same rule, cold store, profiling build, an M3 Max with load average 5–11 from
other work):

| | wall | user | sys | instructions | peak | violations |
|---|---|---|---|---|---|---|
| one at a time (`RERAIL_CHUNKS_AT_ONCE=1`) | 113.8 s | 234.1 s | 150.8 s | 2,989 B | 2.78 GB | 29,747 |
| 8 at once | 50.3 s | 242.0 s | 67.0 s | 2,645 B | 2.91 GB | 29,747 (the same set) |

System time halves: the pool's threads evaluate chunks instead of waking for a component's small jobs and
yielding.

## Open

- Peak memory rises with K: 1.40 GB against the 1 GB budget on `posthog/`. Each chunk in flight holds its sources
  and its engine relations. The chunk size scales with K (§4), but the K chunks' engine relations at their peak are
  not counted. A budget per chunk in flight (`RERAIL_RUN_MB` / K) is the next step.
- The main thread's Begin and Finish (memo replay, filing, memo publication) are serial. They bound the speed-up
  once K chunks keep the pool busy.
