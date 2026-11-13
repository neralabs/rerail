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

- **Each node's columns at their place, as it is made:** its offset from its block's base
  at its push, its size and tail at its close.
- **Code ids at commit points.** Codes are numbered by first use, so an edit could
  renumber every node after it. Instead the parser commits after each statement,
  when its nodes are final, and the commit writes their code ids.
  Nodes past the last commit carry only their keys.
- **Edits touch only the tail.** A parser edits only the nodes it made last:
  a field set after a node, a node put before its operand (a wrap), nodes taken back.
  These are past the commit point, so they rewrite no code ids;
  the blocks from the edit get their bases again.
- **Every column is sized for the same node count.** A push is indexed stores,
  with one check for room; the image's bytes are reserved at the start.
- **The widths are guessed from the source's length** (2 bytes below 320,000 bytes for sizes,
  2 always for offsets and tails). Where a value does not fit, or 2 would have held a column
  written at 4, the file is parsed again at the widths it needs.
  The writer keeps the few nodes whose values need 4 bytes, so it knows without reading
  the columns. On the repository's 1,362 `.rr` files, none is parsed twice.
- **At the end the columns are moved into place** after the code column
  (the user's choice: the size column's place depends on the node count),
  and the header and code table are written.
- **One definition of the format.** The engine's constants are the crate's.

## Cost

Over the corpus (1,360 files, 8.80 MB, 1,166,154 nodes; instructions, cycles):

| The lean parser, whole | Instructions | Cycles |
|---|---|---|
| Writing the old node table (not the tape) | 362.4M | 69.3M |
| The tape, codes renumbered at each edit | 1,326.0M | |
| … SHA-256 by the CPU's instructions (`sha2`'s `asm`) | 1,074.4M | 178.0M |
| … code ids at commit points | 620.8M | 116.5M |
| … every column one capacity, indexed stores | 550.6M | 106.9M |

The renumbering had cost about one renumbering a node (1.2 million over 4.0 million rows).
What remains above the node table: the digest (about 9% of the time, 1.1 cycles a byte)
and the columns' stores, about 50 instructions a push and 40 a close.

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
