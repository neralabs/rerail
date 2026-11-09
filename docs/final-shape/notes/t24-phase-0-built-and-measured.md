# t24. Phase 0, built and measured

## Intro

t23 proposed one loader, phase 0, that finds the corpus's files, knows which changed since the last run, and
answers the engine's file reads from the cheapest trustworthy source. This note gives what is now built on
`investigate-bend-vs-datalog`, how it is tested, and what it costs on two corpora:

- the data layer's pinned worktree (9,010 Python files, 135 MB; `2b1504adc`);
- the 84-file data-layer slice with typeshed (267 documents), under `calls_cx`, as in t22.

CPU is user + sys. The figures for phase 0 alone are the thread's own, from the benchmark's report; whole-process
figures are from `/usr/bin/time`. The machine was shared (load average 10–15 during these runs).

## What is built

**One observer for every read** (`content_store/observe.rs`).
- Every path the engine reads goes through it: rule files and their `#include`s, the corpus walk, the resolution
  rules' requests (`world.rs`: listings, reads, probes), and the phase records.
- Each answer is a `stat` (device, inode, size, mtime, ctime). A file is read only when its stat changed.
- Git's racy rule applies: an answer recorded within 2 s of the file's mtime or ctime is not trusted.
- A directory's listing is reused while the directory's own stat holds.
- **The validation scan.** Each run reads and hashes again one trusted file in 64 (`RERAIL_VALIDATE_EVERY`), a
  slice that rotates with each run. A file changed under the same stat is counted (`changed under the same stat`)
  and its new digest used.
- The observer's state is SQLite, beside the tape cache (`<store>.files`), written once after each run. Memory may
  be ahead of it, never behind.

**The Git provider** (`RERAIL_FILES=git`, `git_workspace.rs`, in process through gix).
- The corpus's files are HEAD's tree, less deleted files, plus staged and untracked files Git does not ignore.
  There is no walk. The same directories are skipped as in the walk (`.`-prefixed, `target`).
- A clean file, whose stat is its index entry's, is not read when its blob was seen before: the observer keeps
  each blob's sha256 (`git_alias`). A clone of a tree seen before hashes nothing.

**The whole-run cutoff** (`corpus/reuse.rs`; t23 §4).
- A run's record keeps what it consulted, with digests:
  - the library files its text includes (`core.rr`, the `#include` closure, the engine's `components.rr`);
  - each document;
  - what its resolution rules listed, read, loaded or found missing.
- The record is keyed by the program texts with the root's settings, the options, the canonical root, the modes,
  the typing inputs, the engine build, and the variables that change outputs from outside the text
  (`RERAIL_SAMPLE_EVERY`, `RERAIL_SCOPE_GUARDS`). It also stores the outputs the run wrote.
- The next run under that key asks the observer about each consulted thing. If all are as they were, it writes the
  kept outputs and evaluates nothing.
- It is on for the command line's runs (`options.reuse`, or `RERAIL_RUN_REUSE=on`), off with
  `RERAIL_RUN_REUSE=off`. A run with T facts, typing declarations or a site directory, or that reads another
  phase's exports, keeps no record: it reads things the record cannot name.

## Tests

- `content_store::observe` (6):
  - a digest is reused across processes;
  - a racy file is read again;
  - the validation scan finds a change under the same stat;
  - a listing is reused until an entry is added;
  - listings round-trip through the store;
  - Git lists the worktree, and a clone reads no clean file.
- `corpus::reuse` (2): an unchanged run is kept, and its outputs are written again; an edited document, a new
  document, the root's settings (for a program that declares `env`) and another text each run it again.
- Unchanged: `git_workspace` 17, `corpus::phases` 1, `logic::rr::` 82, `memo` 39, `sql_executor::tests` 110,
  `tape_tests` 9, and the calls fixture (113 agree, 9 depart).

## Phase 0 alone on the data layer (9,010 files)

`corpus::tests::benchmark_phase_0`: the corpus's files, then every Python document's digest (9,024 with stubs).

| run | how files are found | stats | reused | hashed | from Git blobs | thread CPU | process user + sys |
|---|---|---|---|---|---|---|---|
| cold | walk (1,325 directories) | 10,349 | 0 | 9,024 (135 MB) | 0 | 0.64 s | 0.65 s |
| warm | walk, listings reused | 10,349 | 10,223 | 126 (scan) | 0 | 0.10 s | 0.11 s |
| warm, Git | Git index | 9,024 | 8,902 | 122 (scan) | 0 | 0.15 s | 0.29 s |
| first Git run, new store | Git index | 9,024 | 0 | 8,971 | 53 | 0.55 s | 0.71 s |
| fresh clone, store that saw the tree | Git index | 9,024 | 0 | **0** | 9,024 | 0.28 s | 1.29 s |

- **Warm with nothing changed: 0.10 s of CPU for 9,010 files**, of which the validation scan's re-reads are half.
- The cold run's wall time was 7.3 s, all of it the page cache filling: its CPU is 0.65 s.
- **A clone hashes nothing.** All 9,024 digests come from blob aliases. Its second run is the same: the files are
  younger than the racy window when first recorded, so their answers are not trusted yet, and Git answers again.
- The Git provider costs more than the walk when both are warm (0.15 s against 0.10 s of thread CPU). Each answer
  Git gives costs a `realpath` and an index lookup. The process also spends 0.1–1.0 s more outside the measured
  thread, which is gix reading the index and trees.
- So where the walk is warm, it is the cheaper provider. Git's place is a clone, a checkout or a new store, and
  the files Git ignores. In another worktree of the same repo, an ignored `tmp/` holds 127,171 Python files that
  the walk would read and Git does not list.
- A tree's first Git run, on a store that saw it through the walk, records no aliases: its answers were trusted,
  so nothing was read. A clone of it would then hash each file once. The aliases could be recorded from trusted
  answers too; that is a lookup per file per run, not done yet.

## The whole run on the 84 files

`calls_cx` with both memos on, on a copy of the slice, through the benchmark test with `RERAIL_RUN_REUSE`. CPU of
the main thread is the run's report; the process figure includes the test's own reading of the recording. The load
average was 10–46.

| run | cutoff | memos | CPU main thread | process user + sys | phase 0 | answers |
|---|---|---|---|---|---|---|
| prime (cold) | on, records | 0 found, 133 derived | 75.0 s | 86.7 s | 321 stats, 280 hashed, 34 listings | 6,299 agree |
| warm, no edit | off | 133 found | 14.5 s | 26.0 s | 314 stats, 305 reused | 6,299 |
| warm, no edit | **on: kept** | none run | **0.5 s** | 1.1 s | 315 stats, 307 reused, 8 hashed (scan) | 6,299 |
| comment added to `exceptions.py` | on: derives | 92 found, 41 derived | 42.0 s | 52.7 s | 352 stats, 341 reused, 11 hashed | 6,299 |
| the same again | on: kept | none run | 0.4 s | 1.0 s | 315 stats, 302 reused | 6,299 |
| `touch exceptions.py` | on: kept | none run | 0.4 s | 0.9 s | 315 stats, 309 reused, 6 hashed | 6,299 |

- **A warm run with nothing changed costs 0.4–0.5 s instead of 14.5 s.** The 315 stats cover the 84 documents,
  typeshed's 183 stubs and the requests' listings. A `touch` changes the stat and not the bytes. The file is read
  again, its digest is the same, and the run is kept.
- **Most of the 0.5 s is not phase 0.** `corpus::unchanged` parses and combines the whole program to learn its
  language and its phases. The stats and the record take milliseconds. Keeping those two facts in the record
  would leave the stats only.
- **An edit derives everything again.** A comment costs 42 s, because `exceptions.py`'s import component and the
  41 that reach it are derived again (t22's step 1). Typeshed's rounds run again too. The cutoff is all or
  nothing; partial reuse is below.
- **Peak RSS.** The report's peak is 762 MB for the prime and 355 MB warm. `/usr/bin/time` gives 1.46 GB and
  1.07 GB for the same processes, above the 1 GB budget. Both peaks appear with the cutoff off, so recording is
  not the cause. The report samples RSS at phase ends; the gap is a transient peak between samples. Open item O142.

## Against t23's order of work

| t23 step | state |
|---|---|
| 1. store the loader's answers and the read set, stat provider | built: the observer and the whole-run record |
| 2. `#include` resolution in P0r, the module graph as the prepared key | half: includes are checked by stat; the prepared key is still the flattened text |
| 3. the Git provider | built: listing and blob aliases; per file by index stat, not unchanged prefixes by tree id |
| 4. the validation scan | built, with its test |
| 5. events | not asked for |

What is not built:
- **Partial reuse.** A run where one document changed runs everything again, including typeshed's 12 rounds of
  requests. t23 §4's maintained P0 (the changed reads as a delta) is the next step. It needs P0's outputs as a
  phase's exports, so that later phases see a delta rather than a new world.
- **The module graph as the prepared key** (step 2), so that one rule edit re-prepares its dependants only.
- **Deletes.** A file that leaves the corpus leaves its observer entry behind (one row). The record of a run
  replaces the last one under its key, so records do not grow.
