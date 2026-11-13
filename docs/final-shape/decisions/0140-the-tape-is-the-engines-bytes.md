# 0140: The parsers write the engine's tape, its bytes

Status: decided (2026-11-13). Built for the lean parser; tier 2 next.
Corrects 0136.

## Intro

**The problem.** The engine's tape is a byte string: the v9np image, format 11
(`code_rule_dsl::tape`, decision 0025). It holds a header, then five columns,
each padded to 16 bytes, then a table of codes:

```
header   magic, language, widths, version, source length and digest, five lengths
code     each node's code id, 2 bytes; codes numbered by first use
size     each node's subtree, itself counted          (2 or 4 bytes)
base     each 16 nodes' least start                   (4 bytes)
off      each node's start less its block's base      (2 or 4 bytes)
tail     each node's end less its last descendant's start  (2 or 4 bytes)
codes    kind and field, flags, alias, classes
```

The engine made a `.rr` file's tape from its parser's nodes in passes: sizes from the back,
codes folded and numbered, bases per 16 nodes, each column's width from its largest value.

Decision 0136 said tier 2 "writes the tape directly". It did not.
It wrote a table of 24-byte node rows (`NodeRow`), a format of its own.
The lean parser was then changed to write the same rows (`tape::Row`), and the two were
called the tape. A tape from either still needed the engine's passes.

**The decision.** A parser writes the image's bytes, byte for byte the engine's,
with no conversion after it (`rerail_tape_format::Writer`):

- **Each node's columns at their place, as it is made:** its code id at its push,
  its offset from its block's base, its size and tail at its close.
- **Edits touch only the tail.** A parser edits only the nodes it made last:
  a field set after a node, a node put before its operand (a wrap), nodes taken back.
  Codes first used at or after the edit are a suffix of the code table, numbered again
  over the nodes from there; the blocks from there get their bases again.
- **The widths are guessed from the source's length** (2 bytes below 320,000 bytes for sizes,
  2 always for offsets and tails). Where a value does not fit, or 2 would have held a column
  written at 4, the file is parsed again at the widths it needs.
  The writer keeps the few nodes whose values need 4 bytes, so it knows without reading
  the columns. On the repository's 1,362 `.rr` files, none is parsed twice.
- **At the end the columns are moved into place** after the code column
  (the user's choice: the size column's place depends on the node count),
  and the header and code table are written.
- **One definition of the format.** The engine's constants are the crate's.

## Tests

- `rr_syntax`'s `lean_tape_is_the_engines`: every `.rr` file and seven prefixes of each,
  the lean tape the reference parser's nodes through the writer, byte for byte.
- The engine's `lean_rr_tape_is_the_engines_image`: every `.rr` file and half of each,
  and three files whose columns need 4 bytes: the lean tape is the engine's own
  `encode_rr(…).to_bytes()`, byte for byte.
- `wide_tapes_are_parsed_again_at_their_widths`: a 70,000-byte string, a 70,000-byte
  comment, 20,000 facts.

## Next

Tier 2 (the interpreter, the JIT, ahead of time, Metal) writes the same bytes.
Its node table goes.
