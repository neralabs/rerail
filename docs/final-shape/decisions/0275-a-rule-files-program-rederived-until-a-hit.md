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

## The whole-run cutoff's check, by the same cascade

The cutoff's check (`reuse::reused`) did two things again after every rule file was found as recorded: it listed the
rule files again (each phase program parsed, 54 ms of a 44-policy pack's warm run), and it combined the program again
to read a few of its parts (57 ms). Both are functions of what was just checked, so the record keeps them:

- **The include graphs.** The record holds a line of the texts' include graphs' digests (`graphs`). With every rule
  file as recorded, the same graphs list the same rule files, so the check compares this line and lists the rule
  files again only for a record without one. A phase program is looked for in the run's `phase_dir`, then among the
  engine's own: the places looked in before the one found are recorded as rule files with no digest, so a phase
  program written there later differs by its digest, as an edit does.
- **The program's parts the check reads** (`corpus::Summary`): whether it reads `declared_ret`, its settings, its
  JSON policies, its scopes and text globs, and its `env` facts. They are a line of the record (`program`, compact
  JSON), so the check reads them there and does not combine the program again. A record without one combines it.

A warm run of the 44 data-layer policies on `dl1` (`run-policies`, one pack): 4.1 G instructions and 0.31 to 0.35 s
user became 2.7 G and 0.14 to 0.15 s. Its 48 diagnostics are those of the cold run. `RERAIL_WARM=list` passes.

A batch reads each policy's header before its run (`tools::policy_groups`): its text parsed, its clauses checked,
its includes resolved and its libraries' declarations lowered, 44 times over the same libraries (183 ms of the warm
run's main thread). The header is kept the same way (`rr::text_header`): under the text's bytes, with the library
files its includes read as the entry's read record. The warm pack run went from 2.7 G to 1.9 G instructions.

The cutoff's report counted a workspace document twice: once by its path from the root (the run's `doc`) and once
by its absolute path (phase 1's `load`). It counts each document once now, as the run that made the record did.

Phase 0's own cutoff stays for now. Without it, phase 0's maintained run re-evaluates its cone with no change: 6.95 G
against 4.1 G on the same pack, and it reads the cost statistics (73 ms) because strata are evaluated.

## Tests

- `statements_from_the_tape_are_the_parsers` (engine, decision 0062): for every `.rr` file in the repository, the
  statements off the lean tape are the parser's own.
- `a_rule_files_program_is_found_while_what_it_read_holds` (engine): the entry is kept by the first read and found
  again, holds the program read and its files in reading order, misses once an included file is edited, and hits
  again once that file's bytes are put back (the stat moved, the sha holds).
- `a_handed_programs_hidden_form_is_kept_under_its_texts` (engine): a single program handed to its run is hidden on
  a miss, kept under its texts' digest, and read back as hidden.
- `a_phase_program_appearing_where_it_was_looked_for_first_differs` (engine): a phase program found among the
  engine's own is recorded where it was looked for first, with no digest; a file written there is read instead.
- `a_texts_header_is_found_under_its_bytes` (engine): a text's header is kept by its first read, found again as
  read, and another text's is its own.
