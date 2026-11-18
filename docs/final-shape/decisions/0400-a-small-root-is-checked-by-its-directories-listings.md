# 0400 (was 0399): A small root is checked by its directories' listings, and the observer keeps its own journal of what changed

Status: decided by the user (2026-11-18, Q189's question 3, option (a): "use directory listing where possible to avoid
bombarding with individual stat calls"); built: step 1 (`content_store/observe/listing.rs`, `Observer::check_listed`),
step 2 (`journal::fsevents::start`'s marker, `Since::Behind`, `Observer::check_behind`). Amends decision 0105.
Evidence: t46.

## Intro

The observer answers what is at a path (a file's digest, a directory's listing) and keeps each answer with the
path's stat. A run must know which kept answers still hold. Decision 0105 asks macOS's journal, FSEvents, what
changed under each watched root since the last run, and trusts every answer the journal does not name.

t46 measured that this is not a guarantee FSEvents makes: a change made just before a process starts reaches its
stream only once fseventsd has written it to its log, which under load is later than the next run's start (12 of
40 trials on the M3 missed an edit, with the cookie in hand). Two branches hit it as wrong answers (bend's Q117,
benchmarks' Q54), and the test `the_run_after_a_cold_one_writes_nothing` is flaky on the M3 from it. The replay
itself also costs an edit 0.16–0.77 s of wall (Q18).

## Decision

1. **A root holding at most `LIST_MAX` kept answers (20,000; `RERAIL_LIST_MAX`) is checked by listing its
   directories**, not by asking FSEvents. Each directory that holds a kept answer is listed once a run, with each
   entry's attributes: on macOS by `getattrlistbulk` (one call returns many entries, no `stat` a file), elsewhere on
   Unix by `readdir` and `fstatat` relative to the open directory. A symbolic link is the one entry `stat`ed on its
   own, since an answer is of what the link names.
2. **An entry's answer holds when the listing gives the stat it was kept with** (dev, inode, size, mtime, ctime)
   and the racy rule allows it (recorded at least `RACY_NS` after the path changed). A directory's size is not
   compared: the bulk listing does not give it, and adding, removing or renaming an entry changes the directory's
   mtime and ctime.
3. **What does not hold is named exactly**: its own answer is forgotten, not what is under it. Every kept answer
   under the root is checked through its own directory's listing, so nothing under a named directory needs to be
   forgotten with it: a file added to `src/` names `src/`'s own answer (its listing), and every file in `src/` is
   still trusted. Under a directory that cannot be listed (gone, or not a directory), an answer that a path is
   missing still holds, and every other is named. A symbolic link whose target changed is named with what is under
   it and its canonical forms, which resolve through it.
4. **The observer keeps its own journal for these roots.** Each check that names something takes the next
   generation (`listing:gen` in the observer's store) and records each path named with it (`observed_changes`,
   one row a path, its latest generation), in the same transaction as the answers it forgets. A root's cursor is
   `L<generation>`; a run record made with it is answered by `changed_since` from the rows after it, after the
   root's check for this run. A check that names nothing writes nothing (a warm run's writes stay none).
5. **A larger root keeps FSEvents**, guarded (step 2, fil's check): every new stream is started after a marker file
   is written beside the cookies, and its first answer, its history's, is trusted only when the marker came in the
   history (fseventsd had logged every change before it). Else the answer is `Behind`, and the root is checked by its
   listings as above (`check_behind`): what they name is forgotten exactly and logged, and the root keeps the
   journal's cursor (the cookie's id; every change after it is told to the next replay). A run starts no stream
   again (`settle` 0): on the M3 the marker came in the history in 13 of 40 replays, and listing a root costs less
   than another replay. A test of the journal lets it be started again for up to 5 s, waiting on the same marker
   (one written afresh each time is as young each time, and as seldom logged). A run record made under a `Behind`
   replay's cursor is answered by a later replay only with its marker; else it is not answered (run again).
6. `RERAIL_JOURNAL=off` keeps its meaning: every path checked by its own `stat`.

## What it relies on

The file system's attributes, read now. A write changes the file's mtime and ctime; a create, remove or rename
changes its directory's. ctime cannot be set from user space, so a copy that keeps the mtime (`touch -r`, `rsync
-t`) is still seen. No ordering promise of a daemon is needed.

## Required behavior, asserted (a test each)

- R1 `a_small_root_is_checked_by_listings_not_stats`: a second run over an unchanged root makes no `stat` of a
  file, lists each directory once, and trusts every answer.
- R2 `a_listing_names_an_edited_file_and_no_other`: an edit (also one that keeps size and mtime) is named, and only
  it; a new file and a removed file are named, and their directory's own answer, not its siblings'.
- R3 `a_directory_that_cannot_be_listed_names_what_it_held`: a missing path under a missing directory holds; a
  removed directory names each answer under it.
- R4 `a_racy_answer_is_named_by_the_listing`: an answer recorded inside the racy window is not trusted.
- R5 `the_bulk_listing_gives_what_stat_gives`: for files (a compressed one too, on macOS), directories and a
  symbolic link, the listing's stat equals `std::fs::metadata`'s (directories: all but size).
- R6 `a_run_record_is_answered_from_the_observers_own_journal`: a record made at `L<g>` is told of what changed
  after it, over two later runs, and of nothing before it.
- R7 `the_run_after_a_cold_one_writes_nothing` passes on the M3 every time over 10 runs.
- R8 `a_replay_is_trusted_only_with_its_marker_in_the_history` (journal.rs): an edit made 0.5 s before a new
  stream (t46: missed by 6 of 10 replays that trusted the cookie) is named by every `Changed`, else the answer is
  `Behind`. Five runs on the M3 at load 33–37: 13 trusted, 27 behind, none missed; the observer's and journal's 33
  tests green every run, `rerails_own_files_are_outside_the_observer` (1 in 5 failing before) among them.

## What stays open

- **A replay filters by `id > cursor`** (`journal::since`). duscope found a history record carrying an older id than
  a change it holds. The marker shows the log was caught up; it does not show each record's id. Recorded, not
  fixed: a large root only.
- **A large root's records** are not answered across a `Behind` replay, and on a loaded host most replays are
  `Behind`: the cone rounds and `git_files` run again there. Every root of ep2 is below `LIST_MAX` (e142 started
  no stream); the data-layer repository's is not yet measured.

## Steps

1. Listings for small roots (1–4, 6), R1–R7.
2. The guard for large roots (5), R8.

## Measured

ep2 on the M3, profiling build, at f523157f2 (load 52–68), against e134 at 3b5f7f457, before 0400 (same state, same
edit query, `RERAIL_JOURNAL_TRACE=1`):

| run | wall | CPU | instructions | `before`, off core | `combine`, off core | rows (differing) |
|---|---|---|---|---|---|---|
| e134 (journal) | 5.47 s | 2.28 s | 29.4 G | 2.14 s | 1.46 s | 1,334 (0) |
| e142 (listings) | 1.98 s | 2.27 s | 29.3 G | 0.001 s | 0.058 s | 1,334 (0) |
| e143 (listings) | 1.92 s | 2.24 s | 29.3 G | 0.001 s | 0.059 s | 1,334 (0) |

- The wall falls by 3.5 s: e134 waited 1.0–1.4 s on each of three replays (the corpus, the rule docs, the typing
  programs). e142 and e143 start no stream (no journal trace line): each root is small enough to be checked by
  its listings.
- CPU and instructions are unchanged: listing the directories costs no more than the replays' bookkeeping did.
- e141, the first run after the binary changed (it moves each root from a journal cursor to an `L` cursor): 3.34 s
  wall, 2.76 s CPU.
- The engine's targeted tests (program_image, program_text, request_record, combined, memo::, atoms::,
  `the_run_after_a_cold_one_writes_nothing`): 53 passed, 1 ignored.

Not measured: the data-layer repository's 9,010 files (a warm run's listing cost at that size).
