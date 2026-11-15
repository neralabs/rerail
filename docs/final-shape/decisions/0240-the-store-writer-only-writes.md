# 0240: The store's writer only writes, and no run waits for it before the process ends

Status: building (2026-11-15). Refines the write-back queue (decision 0102), the tape store (decision 0104) and the
per-file memo's frames (decisions 0146, 0212). §4 supersedes the memo's frames (0146) and its numbering (0170).

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

### 4. A memo entry is one image in a pack, its symbols by content

A per-file memo entry (decision 0146) was a frame: typed rows of integers in the `frames` table, under relation
`local`, with its outputs written again as a second frame (`local_out`). Each row was padded to the widest
relation's width and at least ten values, and text was packed eight bytes to a value. A record averaged about 5 KB
in a `WITHOUT ROWID` table of 4 KB pages. Its symbols were the store's atom ids, so every read and write mapped them
(`numbering`: `to_store`, `to_process`), and every entry's ids were held before the waves (`hold_frame_symbols`).

Now:

- **Format.** An entry is one output image (decision 0239), compressed (zstd, level 1; `memo/image.rs`):
  - `r:<key>` holds the rows under each key, in their order and number (`Builder::rows_as_is`). The plan's symbol
    columns (`Syms`) are text; the other columns are integers.
  - Each output has `c:<name>` (its node columns) and `l:<name>` (its lines, one text each).
  - `#nodes` holds the document's node count.

  A symbol is stored as its string, and a reader takes its own atom of that string. So an entry reads the same in
  every process, forked or not, and nothing maps ids. Before a run's waves, the entries it reads have their strings
  interned (`memo::hold`), as their ids were held before.
- **Storage.** An entry's bytes are appended to a pack beside the store (`<store>.memo/<pack>.pack`). A small row in
  `memo_entries` finds the entry: space, part, path, content SHA-256, document key, pack, offset, length, and the
  image's length. A batch is one `write`, inside the store's write transaction, so writers in other processes take
  turns at the pack as they do at the store.
- **Failures.** A failed transaction leaves bytes that no row names. A row whose bytes did not reach the disk does
  not decode, and is a miss.
- **One copy.** The outputs' second copy is gone: a file read for its outputs alone reads its entry, and the
  entry's other relations are not decoded (`Found::projected`).
- **Keeping and collection.** A path keeps its newest four contents, ordered by where they were appended.
  Collection (decision 0227) forgets the rows of a live plan's space whose content no kept manifest names. A dead
  plan's rows go with its space.
- **Compaction.** Once the bytes that no row names are more than half of the packs, and the packs hold at least
  64 MB, compaction copies the named bytes to a new pack in one write transaction and removes the old packs
  (`compact_memo_pack`). A reader in another process that found a row before is pointed at an old pack, and misses.
- **Old frames.** The memo's old frames (`local`, `local_out`) are no longer preloaded. Collection deletes them from
  the live plans' spaces. The memo's schema and its plan digest moved on (8, `corpus-local/v4`), so no entry of
  either format is read as the other.

## Consequences

- `posthog/` (5,413 files), cold on the CPU, before and after:
  - the writer's CPU fell from 5.29 s to 2.0 s: tapes 1.63 s to 0.26 s, batches 42 to 9, memo entries 2.15 s to
    1.21 s;
  - phase 1's end no longer waits for the writer (0.9 s);
  - instructions fell from 566 B to 547 B. User CPU fell from 54.6 s to 31.2 s, but the load fell too (63 to 21).
- §4, on `posthog/` cold. Both builds are at the same merge (4779eb6dc), with and without §4:
  - The memo's entries take 21 MB in the pack, against 72 MB of frames (`local` 54 MB, `local_out` 18 MB). The
    images are 104 MB before compression.
  - The store file falls from 224 MB to 124 MB.
  - The writer's time on the entries falls from 3.40 s to 1.00 s. Its whole CPU falls from 4.57 s to 2.41 s.
  - The findings are the same (7,080).
- §4, the next run over the same store: every file (5,413) is read from the memo, and nothing is appended to the
  pack. The writer runs only the manifest's timestamp, a write a warm run may make. The run takes 3.2 s wall and
  29 B instructions, and finds the same 7,080.
- Open, found while measuring §4:
  - On `posthog/`, `attr_of` sometimes takes a cross product: `text($at, $a)` scanned before
    `field($n, 65541, $at)`, whose postings estimate is 0.1 rows a binding. That is 1.17 B rows and 56 s, against
    0.1 s. The build without §4 does it too (1,432 B instructions), so it came with the merges of
    de9122137/4779eb6dc. Not every run does it: one run of the §4 build took 652 B instructions, one 1,384 B.
  - `RERAIL_WARM` faults on the observer's journal when the run's roots are under a busy directory: the phase-0
    root in `$TMPDIR`, and the run's own outputs under the work tree. The memo writes nothing there.
- Tests:
  - the store's (`rerail-store`: `frames::` 11, `corpus_runs` 16, the column registry, the schema). Among them,
    `a_memo_keeps_a_paths_newest_and_its_pack_is_compacted` and `frames_are_kept_while_a_kept_manifest_reaches_them`,
    ported to the pack;
  - `memo::image::tests::an_entry_reads_back_as_made_its_symbols_by_content`, and the memo's other tests;
  - the engine's corpus tests that read the memo back: `value_documents_are_memoised_with_the_files_that_asked`,
    `cross_file_rows_are_memoised_per_import_component`, `corpus::phases::tests` (5), `corpus::session::tests`
    (15), `documents_seen_before_take_their_rows_from_the_content_memo`, and
    `a_corpus_runs_memoised_rows_bind_by_name_from_their_frames`. The last one damages b.py's entry in the pack
    now, where it used to damage its frame.
