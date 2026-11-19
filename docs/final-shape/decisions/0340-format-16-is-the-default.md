# 0340. Format 16 is the run's default tape format

Status: decided (2026-11-17), by the user: "Switch to format 16". Decision 0293 kept format 15 as the default until
0156 was decided. This decision makes format 16 the default. `RERAIL_TAPE=15` and `RERAIL_TAPE=16z` still choose
the other formats.

## Intro

A run stores and reads its documents' tapes in one format (`formats::Format::of_run`). Until now that format was 15,
unless `RERAIL_TAPE` asked for another. Format 16 (0156, 0293) holds the same tree in fewer bytes. Its parent, its
subtree end and its children are cheap to find in place. The device's label passes read it faster at s1000
(671 ms against 781 ms for format 15, t41) once each pipeline is built for the batch's format.

## What changes

- `RERAIL_TAPE` unset or empty now means `16`:

  ```
  RERAIL_TAPE      the run stores
  (unset), 16      format 16      (was 15)
  15               format 15
  16z              format 16 in fixed LZ4 blocks
  ```

- Some languages have no format 16 table (`f16::lang`): C#, CSS, HTML, Java, Markdown and Markdown's inline
  grammar (0293, "Not done"). Their documents keep format 15, stored under version 16. The profile counts each
  such language once (`tape.keeps_15.<language>`). The stderr line is printed only when the run set `RERAIL_TAPE`,
  so a run that asked for nothing is not told about a default it did not choose.
- A store keys tapes by format, so a store written by an earlier run holds no format 16 tapes. The first run after
  this change parses every file again.
- Tests:
  - The tests of format 15's columns and bytes (`tape::tests`, `tape::resident::tests`, the slabs' in-place test)
    pin their thread to format 15 (`formats::this_thread_stores`, `as_run`).
  - Every other test runs as the product does, in format 16.
  - A test that stores a run's bytes in a store keys them by `formats::stored_version()`, not
    `TAPE_FORMAT_VERSION`.

## Not decided here

0156's open tradeoffs (T1–T7) are still open. This decision only makes format 16 the run's default.
