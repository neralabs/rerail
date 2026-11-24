# 0433: A listing names a directory's entries, not the directory; a racy answer is checked by its bytes

Status: built (audited 2026-11-23). Decided (2026-11-18, found by
`a_header_edit_asks_again_only_for_the_units_that_read_it`, red since the small roots were checked by listings).
Amends 0400 (R2, R4) and keeps 0105's naming: a named path is forgotten with everything under it.

## Intro

**Naming.** Between two runs the observer says which paths changed (decision 0105). A reader of a kept answer drops
it when a named path is the answer's path or above it: the journal names a rescanned directory, and everything under
it is then forgotten. For a file made or removed, the journal names the file; a reader of its directory's listing
catches it as its parent's entry.

**The listings.** A small root is checked by its directories' listings instead of the journal (0400). A directory
whose stat changed was named itself: its stat moves when an entry is added or removed. Named, it covered every answer
under it, and the cone's rounds (`still`) dropped each kept answer read there:

```text
ws/g.h is made                      # no unit looked at it
ws's stat changed: "ws" named       # covers ws/one.c, ws/two.c, ws/three.c, ws/h.h
every translation unit asked again  # 3 of 3, where 0 were wanted
```

And an answer recorded inside the racy window (a file written just before a run) was named by the next run's
listings whatever its bytes: the journal names a file only where it changed, so the unit that read a header edited
before the last run was asked again.

## Decision

1. A directory still a directory whose stat changed is not named. Its entries added or gone, against the listing its
   answer holds, are named, as a file event names them, and its own answer is forgotten unnamed (`listing_named`'s
   `stale`). A directory never listed held its kind alone, which holds. A directory gone or no longer one is named,
   as before.
2. A file whose stat is the answer's, but recorded inside the racy window, is read again: named only where its bytes
   changed, else recorded again with its read's time.

## Required behavior, asserted

- A file made in a directory asks again for no answer under it but the directory's listing:
  `a_header_edit_asks_again_only_for_the_units_that_read_it` (asks for none; a header edit asks for its unit alone).
- R2 `a_listing_names_an_edited_file_and_no_other`: a removed file and a new one are named (2), the directory's own
  answer forgotten with the removed file's (2), no sibling read.
- A racy answer whose bytes are as they were is read and not named:
  `a_racy_answer_with_its_bytes_unchanged_is_not_named`. R4 (a racy answer is not trusted by its stat) holds.
