# 0275: a rule file's program, rederived until a key hits

Status: in progress (2026-11-15). Amends how a rule file's statements are read (decision 0062) and continues 0260.

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

## Tests

- `statements_from_the_tape_are_the_parsers` (engine, decision 0062): for every `.rr` file in the repository, the
  statements off the lean tape are the parser's own.
