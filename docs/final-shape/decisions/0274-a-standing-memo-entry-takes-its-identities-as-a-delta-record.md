# 0274: A memo entry whose rows stand takes its new identities as a delta record beside it

Status: built (2026-11-15, the user's choice at the django-bugs merge). Joins decision 0225 §2 (the per-file memo
holds each relation under its identity) with decision 0240 §4 (an entry is one image in a pack, its symbols by
content). Refines `content_store::frames::memo_pack` and `plan::corpus::memo`.

## Intro

A per-file memo entry holds a file's rows and lines, and the identities (`memo::COVERED`) of the relations it holds.
After a rule edit, a file whose entry lacks some identities is a partial hit. Its chunk runs only the cone of the
missing relations. When the cone's relations come out as the entry already held them, the entry's rows are still
right. Only its identity list grew.

The two branches stored entries differently:

- **0225 §2 (the hub).** An entry was two frames: its rows, and its outputs again (`local_out`). A partial hit whose
  rows stood wrote only the outputs frame, with its new identities, and the reader joined the two lists. A
  rule-body edit went from 431 G to about 202 G instructions.
- **0240 §4 (django-bugs).** An entry is one zstd output image appended to a pack, with symbols as strings and no
  second copy of its outputs. On posthog the memo went from 72 MB to 21 MB, and the writer's entry time from 3.4 s
  to 1.0 s.

Neither shape alone keeps both gains. Without the second copy, "write only what changed" has nothing to write to,
and the two frames bring back the copy and the id mapping that 0240 §4 removed.

## Decision

- **The entry is 0240 §4's.** It is one image in the pack, named by a row of `memo_entries`.
- **What stands is not written again.** An entry whose cone relations and guests equal the stored ones
  (`corpus`'s `standing`) writes only its identities (`memo::identities_of`). They are a small image of `COVERED`
  alone, appended to the pack like an entry (`MemoPut::over`).
- **The delta record lives on the entry's row.** It is located by four columns of `memo_entries`: `over_pack`,
  `over_at`, `over_len` and `over_raw`. `over_len` 0 means none. The record is written only where the stored
  entry has the same content and document key. Otherwise it is dropped, and the next run misses.
- **The reader takes both.** `Frame::projected` unions the record's identities with the entry's. A record that does
  not decode adds none, so its relations read as a partial hit again.
- **It goes with its entry.** An entry written again whole clears the record (the columns' defaults). Keeping a
  path's newest contents, collection (0227) and a dead plan's space delete the whole row, record included.
  Compaction copies the record next to its entry, and `memo_pack_bytes` counts it as named.

## Consequences

- An older store gains the columns as it opens (`schema::ADDED_COLUMNS`). An older binary ignores them and reads an
  entry as it was made: that is a partial hit, so correct but slower.
- The record holds identities only. A standing entry's other own rows and its lines are the stored ones by
  construction: the cone's relations compared equal, and everything outside the cone came from the entry.
- Tests: `frames::tests::what_is_kept_over_an_entry_is_read_with_it_and_goes_with_it` (store level) and
  `memo::tests::an_entry_holds_the_identities_it_was_made_under_and_no_others`.
