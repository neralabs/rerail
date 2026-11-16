# 0275: a rule file's program, rederived until a key hits

Status: built (2026-11-15). Amends how a rule file's statements are read (decision 0062) and continues 0260.

## Intro

A warm run with nothing changed still read its policy from scratch. It built the include graph with phase 0, parsed
every library file, lowered the program, printed it, and hid and merged it. Only after that did it look up its
prepared program. On `dl1` that was about 60 to 70 ms of the main thread's samples, out of about 170.

The user asked for two things (2026-11-15):

- Use the lean `.rr` parser, which is fast now.
- Check for the stored program as cache rules do: rederive each step, and stop at the first stored step that hits.
  The keys go from cheapest to dearest: the files' stat (inode, ctime, mtime, size), then their sha, then the lowered
  program.

## The lean parser reads rule files

- `rerail-rr-syntax` is built at `opt-level = 3` in debug builds, like every other lean parser (`Cargo.toml`). At O0
  it was the only lean parser on a run's path that was left unoptimized.
- A rule file's statements come off the lean parser's tape (`rr::stored_statements`), not off the tape the content
  store keeps for it. Decision 0062 kept that tape so that a file read before was not parsed again. With the lean
  parser optimized, the parse costs less than the lookup and the decode of the stored facts: those were 10 ms, plus
  4 ms to open the store, against a parse that no longer shows in the samples.

Measured on `dl1` (warm, cutoff off): 1.5 to 1.7 G instructions became 1.4 to 1.6 G, and user CPU went from 0.15 to
0.10 s. `rr::read_file` went from 74 ms of the main thread's samples to 29.

## The program, found by the first key that hits

A run's program is a chain of derivations: the files' bytes, then their statements, then the lowered program, then
the hidden one. Each step is rederived as a cache rule's output is, and the chain stops at the first stored step
whose key still holds:

1. **The files' stat, then their sha.** `rr::read_file` keeps the program it lowers as an entry of the combined
   programs' kind (`combined.rs`), under the rule file's canonical path and its bytes' sha (`rule_file_key`), with
   the read record: every file it read or probed, in reading order, with its sha (or none, for a miss). The entry
   is taken while each file still reads as recorded (`LibraryFile::unchanged`). The observer answers by the file's
   stat (inode, ctime, mtime, size) while that holds, with no read, and by reading and hashing the bytes when the
   stat moved. So a file touched but not changed still hits, by its sha. A hit returns the lowered program from its
   image (decision 0173) and skips phase 0's include graph, the parse and the lowering.
2. **The lowered program.** On a miss, the files are read (by the lean parser) and lowered again. The lowered
   program, printed, is the key of the next step: its texts' digest (`texts_digest`), which `combine_keyed` already
   kept a pack's combined program under. A single program handed to its run (`run_read`) is now kept there too
   (`combine_stored`), so an edit that lowers to the same program (a comment) hides nothing again.
3. **The prepared program** is keyed as before, by the texts' digest and the facts the run adds to its program.

A run names the entries it read in its manifest, as it names its combined program, so collection keeps them while
it keeps the run. Only the outermost run does: `read_file` notes each entry on its thread (`READ_BEFORE`), and the
outermost run takes them. A hit writes nothing: a warm run under `RERAIL_WARM=list` passes.

Measured on `dl1` (warm, cutoff off): 1.4 to 1.6 G instructions became 0.9 to 1.1 G with the first step, and 0.8 to
0.9 G with the second. User CPU is 0.08 s. `rr::read_file` is 13 ms of the main thread's samples: the store's open
(4 ms) and the entry's lookup and decode (5 ms).

## Tests

- `statements_from_the_tape_are_the_parsers` (engine, decision 0062): for every `.rr` file in the repository, the
  statements off the lean tape are the parser's own.
- `a_rule_files_program_is_found_while_what_it_read_holds` (engine): the entry is kept by the first read and found
  again, holds the program read and its files in reading order, misses once an included file is edited, and hits
  again once that file's bytes are put back (the stat moved, the sha holds).
- `a_handed_programs_hidden_form_is_kept_under_its_texts` (engine): a single program handed to its run is hidden on
  a miss, kept under its texts' digest, and read back as hidden.
