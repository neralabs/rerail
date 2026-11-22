# t46: What FSEvents guarantees, and what the observer may rely on

## Intro

The observer (decision 0105) asks macOS's file system journal, FSEvents, which paths changed under a root since the
last run, so that a run need not `stat` every file it knows. A new process starts a stream "since" the event id the
last run stored, waits for the replay of the journal's history, and then waits for a file it writes itself (a
cookie) to come back, which is taken to prove that every earlier change has been told.

Two sibling branches saw that proof fail on the same day (2026-11-18): bend's Q117 (the O125 replay, a step's
`rsync` missed, stale findings) and benchmarks-on-integration's Q54 (an edit step reading no components). This note
collects what Apple documents, what other journal users found, and what a probe on the M3 measured, before the
filesystem part is designed again.

## What Apple documents

From the `FSEvents.h` header and the File System Events Programming Guide (archived):

- **Event ids** "are monotonically increasing per system, even across reboots and drives coming and going". That
  is the only ordering promise.
- **History.** With a `sinceWhen`, "historical" events come first, then `HistoryDone`, then contemporary events.
  The service "will supply events that have happened after the given event ID". Nothing says when an event
  becomes part of the history.
- **The history is advisory.** "You should treat the events list as advisory rather than a definitive list of all
  changes to the volume"; "backup software should still periodically perform a full sweep". A disk changed by
  another system has its log discarded, and a new database gets a new UUID. A UUID that matches but an id lower than
  the stored one means a restore, a wrap or a purge: the stored state is invalid.
- **Granularity and coalescing.** The design is "passively monitoring a large tree of files"; it is "not designed
  for finding out when a particular file changes" (kqueue is). Events in a directory and its subdirectories close
  in time "may be coalesced", flagged `MustScanSubDirs`. A dropped event also sets `MustScanSubDirs`.
- **Root changes** come with event id 0, so no cursor can be taken from them.
- **Flushing.** `FSEventStreamFlushSync` delivers "every event that had already occurred" to this stream's
  callback. It says nothing of the persistent log another process's stream replays from.
- **Permissions.** A user is told only of directories it can reach by the ordinary permissions.

## What other journal users found

- **Watchman** (whose cookie the observer copied) states that on macOS, under load, events "may be received after
  the cookie file notification and FSEventStreamFlushSync returning", that FSEvents offers no ordering guarantee
  for this, and that query synchronization on current Apple platforms is unsupported. Its remedy is a settle
  period (the tree quiet for N ms). Watchman keeps its state in a running daemon; it does not replay history
  across restarts.
- **Git's fsmonitor** starts every daemon "since now" and keeps no history across restarts.
- **fil (issue 51)**: one update in seven, at load 9–20, missed files created about 4 s before it. Cause: a change
  numbered at or below the stream's start reaches the stream only through the history replay, which reads
  fseventsd's log, and fseventsd writes that log late under load. Their fix: write a marker before the stream
  starts, and accept the replay only when the marker comes back in the history; replay again for up to 2 s, then
  fall back to a full scan.
- **duscope (PR 185)**: a history record can carry an id older than a change it holds (fseventsd keeps one record
  a path in its buffer, its flags updated in place, its first id kept: the SANS forensics talk describes the same
  buffer). A filter on `id > cursor` lost a directory in 4 of 4 runs. Their replay uses `FullHistory` (0x80) and
  keeps every record of the chunk the cursor falls in.
- **notify (PR 4)** and **crate-agent (issue 234)**: events lost across a stream's stop and restart, intermittently
  (1 in 30–40 runs).

## Measured on the M3 (2026-11-18, load 31–60)

`scripts/profiling/fsevents_history_probe.py <dir> <trials>` (ctypes, no build; run it on the volume to test) runs, in a fresh directory each trial: write X; take a cursor from a
cookie seen on a live stream, then stop it (one process's run); change X and create Y (a new file, the control);
start a new stream since the cursor, as the next process does; write a second cookie and wait for it and
`HistoryDone`. Then it listens 2 s more, and replays from the same cursor 2.5 s later.

| | missed X and Y at the cookie | came later on the same stream | named by the replay 2.5 s later |
|---|---|---|---|
| without `FullHistory` | 7 of 20 (9 of 60 in a first run) | 0 | every one, in its history |
| with `FullHistory` | 5 of 20 (1 of 60 in a first run) | 0 | every one, in its history |

- **What fails is fil's case.** Y is a new file, so this is not duscope's old-id record; both files are missing.
  The cookie arrived and `HistoryDone` came in every failing trial: the observer's sync would have answered
  "complete".
- **The events are not lost to FSEvents, only to that stream.** They are in the history a moment later. They were
  not in fseventsd's log when the stream replayed it, and being older than the stream they never come live.
- **More misses when the edit is older at the stream's start** (0.5 s: 6 of 10 in the first run, against 1 of 10
  at 0 s). An edit just before the start is still told live; one that has left the live window but is not yet
  logged is in neither.
- **`FullHistory` does not fix it.**
- **fil's pre-marker detects every miss, and almost every success too.** The marker, written just before the
  stream, came in the history in 2 of 40 trials. On this host the log is almost never caught up when a run
  starts, so the check would replay again (or scan) nearly every time.
- **The replay itself took 0.33–0.45 s** (median by variant), the watch's wall in Q18.

## What the observer can rely on

- A change made **while a stream runs** is told to that stream (Watchman: possibly after a cookie, under load).
- A change made **before a stream starts** is told only once fseventsd has logged it. No API says when that is, and
  on a loaded host it is later than the next run's start. A cookie proves nothing about it.
- **The history is advisory** (Apple's own word): other systems, unclean shutdowns, purges and restores lose it,
  each detectable only partly (UUID, id lower than stored).
- **An id is not when a change happened**: a record may carry an earlier id than its latest change (duscope).
  Filtering a replay by `id > cursor` can drop changes; the observer does so (`journal::since`).
- **What is reliable**: `MustScanSubDirs`, the dropped and wrapped flags, `RootChanged`, the volume's UUID. The
  observer handles each (a named path is forgotten with what is under it; the others answer `Lost`).

So the guarantee the observer needs between processes, every change before the run named, is not one FSEvents
makes. What it gives is a hint that is right most of the time and wrong without saying so under load. Any design
that answers from it across processes needs a check of its own that does not come from FSEvents: a `stat` of what
the answer would trust, a settle period, or a long-lived stream that was running when the change was made.

## Sources

- Apple, `FSEvents.h` (CoreServices), and the File System Events Programming Guide: "Technology Overview",
  "Using the File System Events API" (developer.apple.com library archive).
- Watchman, "Cookies" (meta.github.io/watchman/docs/cookies).
- Git, `compat/fsmonitor/fsm-listen-darwin.c`.
- productdevbook/fil issue 51; kawadeomkar/duscope PR 185; Finityno/notify PR 4; aud-eos/crate-agent issue 234.
- SANS summit 2017, FSEvents forensics (the buffer: a path's record updated in place, written when the buffer
  fills).
