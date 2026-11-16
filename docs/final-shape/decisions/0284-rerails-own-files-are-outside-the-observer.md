# 0284: rerail's own files are outside the file observer

Numbered 0246 where it was written.

Status: built (2026-11-15). Amends decision 0105 (the journal) and the file observer (`content_store::observe`).

## Intro

The file observer answers what is at a path (its kind, digest, bytes, listing) and keeps those answers between
runs. Under a watched root it asks the system's journal what changed since the last run, and forgets the answers
under each path it names.

rerail writes files of its own under the roots it watches: a store inside the work tree, the store's WAL and lock
files, the observer's own store and cookies, the upkeep record, and the directory a run writes its findings into.
Every run writes them, so the next run's journal named them every time. The observer then forgot answers (the
listing of the directory holding them), moved and stored its cursor, and a warm run wrote to its store
(`RERAIL_WARM`: "the observer forgot answers the journal named changed").

The user asked that rerail ignore its own files in all observer behaviour (2026-11-15).

## Decision

rerail's own files are:

| What | Named by |
|---|---|
| The content store, and every file named after it beside it (`<store>-wal`, `<store>.files`, `<store>.files.cookies/…`, `<store>.runs.lock`, `<store>.upkeep`) | the observer, from its own store's path (`<store>.files`) |
| The cost statistics' file and the files named after it | `observe::own`, as the statistics start |
| A directory a run writes its outputs into, and what is under it | `Observer::own_output`, as the run makes it; kept as `own:<dir>` in the observer's store, so the processes after know it |
| Phase 0's scratch corpus and outputs (`$TMPDIR/rerail-phase-0-…`) | `observe::own`, as phase 0 makes them (decision 0250) |

For these, the observer:

- **keeps no answer.** `kind`, `digest`, `read` and `canonical` ask the system each time and record nothing;
- **lists none of them.** A directory's listing leaves them out, so writing them changes no listing kept;
- **takes no news of them.** Paths the journal names under a root are dropped when they are rerail's own, before
  anything is forgotten (`watch`) or a run record is checked (`changed_since`). A root whose only news is rerail's
  own files is unchanged: no answer forgotten, no cursor stored.

An own file is still answered, so a rule that reads one gets its bytes; only the keeping and the news change.

A file named after an own file (`<own>` followed by `.` or `-`) is its own too. Asking about an own file watches
nothing: no root above it is asked of the journal (decision 0250).

The outputs' record is written once, the first time a directory is named, and only under a root the store has a
cursor for: the journal of a later process reports only there. A directory named before its root is watched is
recorded at the run's end, if a root covers it by then. Elsewhere it is own for this process alone. Phase 0's
scratch outputs, one directory a run, were recorded each run. The observer's collection drops the records of
output directories that are gone.

## What this gives up

- **A corpus file named like a store** (`store.sqlite.md` beside `store.sqlite`) is treated as rerail's own: never
  kept, read each time. A cost only, never a wrong answer.
- **A run's outputs read as inputs** (a pipeline reading an earlier run's findings) are read each time rather than
  kept.

## Tests

- `rerails_own_files_are_outside_the_observer` (store, macOS journal): a listing leaves the store family and the
  outputs out, no answer is kept for them, and after a run's own writes the next run forgets nothing, stats nothing
  and stores no cursor; a run record holds across them.
- `collection_drops_what_no_run_asks_any_more`: an output directory still there is kept, one removed goes.
- `a_runs_output_directory_is_rerails_own` (engine): after a corpus run, its output directory is not in its root's
  listing.
- `an_output_directory_is_recorded_once_and_only_under_a_watched_root` (store): named before its root is watched,
  recorded at the run's end; outside any root, never recorded; the next process writes nothing again.
