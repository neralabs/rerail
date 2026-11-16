# 0105: The file system's journal says what changed; the observer trusts the rest

Status: built (2026-11-10). macOS: FSEvents. Elsewhere: Watchman when it is installed, else
stat mode.

## Intro

**The problem.** Every run checks each file it reads with a `stat`, and reads the file again
when the stat has changed. Phase 0 also does this for each directory it lists. On the workload
(237 Python files plus the typeshed they load), a run that edits one comment made 127–276 stats.
Each stat costs only a little, but a run that should take 0.1 s CPU does a few hundred of them.
That cost grows with every file a run reads, not with what changed.

**The rule.** Something outside rerail tells a run what changed since the last run. The run
re-checks those paths and trusts every other answer it kept.

On macOS that is the FSEvents history, which the OS's `fseventsd` keeps. Rerail runs no daemon or
watcher process of its own.

```text
run start   for each watched root:
              FSEvents.since(cursor kept for this root and volume)
              -> Changed { paths, cursor }  : forget those paths, trust the rest
              -> Lost { why }               : stat as before (stat mode)
run end     flush: kept answers and new cursors, in one transaction
```

## What is watched

- the corpus root;
- the rule library (`rr::library()`);
- each root the world may read (each directory a setting names, such as typeshed);
- for any other path a run reads, the stored root above it, or else its own directory. A later
  run then trusts it.

A root is asked once per run. A run nested in another run (phase 0's) keeps the outer run's
answer.

## One stream per root, kept for the process

A one-file edit asked the journal 45 times: each phase is a run of its own, and each run asked
each root again (the same root also came under two spellings, such as `/var` and
`/private/var`). Each question made a new FSEvents stream, replayed the history since its
cursor, and waited for a cookie. That came to 0.9 s of waiting with the machine quiet, and 2.6–5 s
when it was loaded.

Now the process keeps its streams (`journal::fsevents::LIVE`):

```text
process start   Observer::open: one stream for each stored root, from its stored cursor
                (a root under another is the outer stream's, from the older cursor);
                the replays run beside each other and beside preload
outermost run   begin (open 0 -> 1): next_epoch
a question      the stream that covers the root and started at or before the cursor:
                  first question of the epoch: write a cookie, wait for its event
                  answer: the events the stream kept with an id after the cursor
                none: a new stream for the root, from the cursor (one replay)
```

- **The cursor an answer gives is the cookie's event id.** Every change made before the cookie
  has a lower id and reached the stream before it, so a replay from that id misses nothing.
  The system's current id (`FSEventsGetCurrentEventId`, a call into `fseventsd` that took 0.15–
  0.2 s while the run wrote) is asked only where a stream is started or the answer is `Lost`.
- **Runs inside a run share its sync.** A change made after the outermost run's cookie is told to
  the next outermost run, and a later process replays it from the cursor stored. This is what
  asking once per run already meant.
- **Lost is per stream.** A stream that sees dropped or renumbered events, or keeps more than
  2^21 events, is dropped. The next question under its roots replays afresh. A process keeps at
  most 16 streams; the one used least recently goes first.
- `volume()` (`FSEventsCopyUUIDForDevice`) is asked once per device for the process. It was
  asked once per root per run.

Measured on the workload store `n43`, one-file edits, load 37–56:

| | questions | streams started | cookies | waiting |
|---|---|---|---|---|
| before | 45 | 45 | 45 | 0.9 s quiet, 2.6 s at load 100 |
| streams per process | 25 | 5 (one after another) | 9 | 0.61 s |
| + started at open, cookie id as cursor | 25 | 0 at question time | 6 | 0.07–0.29 s |

The instructions are the same with the journal on or off (325.5 G vs 326.7 G). The findings equal
the cold run's.

## What "trusted" means

- When a path has a kept answer under a root the journal trusts, that answer is used as it is:
  no stat, no racy-window re-check, no rotating re-hash.
- When a path has no kept answer, it is missing if its directory's kept listing does not hold
  its name. If that directory has no listing yet, it is listed once, and later probes use the
  listing.
- Each named path is forgotten together with its subtree and its parent's own row (the listing
  changes when an entry is added or removed). This happens in the same transaction as the
  cursor update.

## When the journal is lost

The answer is `Lost`, and the root is in stat mode for that run, when:

- there is no stored cursor;
- the root is on another volume (a cursor is kept under the volume's id);
- events were dropped (the history was purged);
- the cookie file written beside the store did not come back within 2 s.

In stat mode, every path that is not checked is dropped at flush, and the new cursor is stored.
The next run trusts the root again.

## Where it is kept

The cursors are rows of `observer_meta`, with key `journal:<volume>:<root>`. There is no table of
their own (`logic::facts_only`).

## Measured

Workload store `tc4`. Load average 30–110 (other worktrees were building).

| run | stats | trusted | CPU (user+sys) |
|---|---|---|---|
| comment edit, before | 127–276 | 0 | 1.1–1.25 s |
| comment edit, journal | 4 | 1187 | 1.0 s |
| unchanged, journal | 2 | 600 | 0.1 s |
| first run after the change (Lost) | 486 | 188 | — |

The findings are identical to the baseline (`n_loaded.csv`).

## Open

- Built since: Watchman (`RERAIL_JOURNAL=watchman`, or found on `PATH` off macOS). Its clock is
  the cursor, so a cursor is opaque text kept per root. A fresh Watchman instance has no history
  and is `Lost`. Watchman's history lasts only while its daemon runs; inotify and fanotify keep
  none, so without Watchman Linux is in stat mode. Windows (USN journal) is not built.
- Built since: a root is trusted only inside a run (`begin` to `flush`). A read outside every
  run checks by `stat`, because nothing marks when the files may have changed.
- Built since: canonical paths (`Observer::canonical`, kept as `canonical:<path>` records) answer
  `World::allowed` and the whole-run cutoff's checks with no `realpath` (comment edit: 475 → 1).
  Git's file list is kept with the journal's cursor taken before the status; a later run reads
  the status again only when a named path could change the list (a new or removed file, a
  directory, `HEAD`, the index, refs, config, info or an ignore file).
- A run whose edit makes the cone rerun as whole strata cost ~20 s CPU; three plans were the
  cost, and it is now 2.4 s (decision 0145).
