# 0140: The parsers write the engine's tape, its bytes

Status: decided (2026-11-13). Built for the lean parser, and for tier 2's
interpreter, Cranelift and Rust ahead of time (docs/parser-generators/37).
Corrects 0136.

## Intro

**The problem.** The engine's tape is a byte string: the v9np image, format 15
(`code_rule_dsl::tape`, decision 0025). It holds a header, then five columns,
each padded to 16 bytes, then a table of codes:

```
header   magic, language, widths, version, source length, five lengths
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

## In the hub (2026-11-13)

Written on a branch whose base was format 11; the hub was at 15 (12: named-only at rest and no
stored source digest, RER-131; 14 and 15: implied tokens and keywords never written). The
crate states format 15 (`rerail_tape_format::FORMAT_VERSION`, its `header` offsets, no digest),
so the writer writes the hub's images. The engine's `encode_rr` reads the lean parser's image
as it is (no `finish`, no `Pending`), and the statements are read off the same image
(`rerail_tape_format::decode`, `rr_syntax::tape::nodes`), as a stored tape's are (decision 0062).
The statement parser's tree through the engine's passes (`packed_rr_tape`) is the tests' oracle.

In the hub the header holds no digest (format 12 on), so the writer computes none: the
SHA-256 rows below are this branch's, before the merge. The engine's in-memory digest of a
parsed source (`TapeImage::source_digest`) is made apart (`encode_rr`).

## Cost

Over the corpus (1,360 files, 8.80 MB, 1,166,154 nodes; instructions, cycles):

| The lean parser, whole | Instructions | Cycles |
|---|---|---|
| Writing the old node table (not the tape) | 362.4M | 69.3M |
| The tape, codes renumbered at each edit | 1,326.0M | |
| … SHA-256 by the CPU's instructions (`sha2`'s `asm`) | 1,074.4M | 178.0M |
| … code ids at commit points | 620.8M | 116.5M |
| … every column one capacity, indexed stores | 550.6M | 106.9M |
| … the hot path inlined, one check a push and a close | 465.2M | about 97M |
| … the widths a type (`Narrow`), the commit over local slices | 425.6M | 96.2M |

Over the same files, the lean parser with a writer that keeps only keys and starts
(what the parser reads back) is 354.2M (69.3M cycles); the digest alone is 14.2M (11.6M).

**Why the column stores were slow** (each part skipped in turn, one build):
the code ids cost 32 instructions a node, a close's size and tail 27, a push's offset 22.
The stores were not the cost. Around each one:

- **Every column was reached through the struct.** Each store reloaded the column's pointer
  and length, and the commit loop reloaded six fields a node: a 2-byte store, as far as
  the compiler knew, could change the writer's own fields.
- **The widths were read as the writer ran.** A store was a load of the width, a compare,
  a branch and the address arithmetic, for a value fixed for the whole file.
- **Two or three checks where one does.** "Ends before its last descendant" is a tail
  that wraps; "some node is marked" is a threshold of 0.
- **Calls.** `push` and `close` were calls with their registers saved.

Now the writer's widths are a type parameter: `Narrow` (2, 2, 2, nearly every file) is
fixed at compile time, `Widths` is any. The lean parser is compiled for both.
A push's offset is 8 instructions, a close's size and tail 13, a code id 11.
About 57M above the keys-and-starts writer remain, 49 a node:
those 32, a block's base every 16 nodes, a commit a statement, the wraps' shifts,
the new columns' zeroing, and the columns moved into place at the end.

## Tests

- `rr_syntax`'s `lean_tape_is_the_engines`: every `.rr` file and seven prefixes of each,
  the lean tape the reference parser's nodes through the writer, byte for byte.
- The engine's `lean_rr_tape_is_the_engines_image`: every `.rr` file and half of each,
  and three files whose columns need 4 bytes: the lean tape is the engine's own
  `encode_rr(…).to_bytes()`, byte for byte.
- `wide_tapes_are_parsed_again_at_their_widths`: a 70,000-byte string, a 70,000-byte
  comment, 20,000 facts.

## Tier 2

Tier 2 writes the same bytes (docs/parser-generators/37): a working row a node
(key, start, end), the columns at their place, the widths constants of its code.
Over the corpus, the interpreter's, the JIT's and the ahead-of-time tapes
are the lean parser's, byte for byte.

Next: wide files parsed again at their widths, not refused; Metal's own kernel.

In the hub (2026-11-13), tier 2 writes format 15's header as the lean parser does: at
`rerail_tape_format::header`'s offsets, with no digest (the Cranelift code, the ahead-of-time
Rust, the harness). Its codes carry no supertype classes yet. The `.rr` grammar has none, but
the Bitlex JSON parser (tree-sitter-json's `_value`) does: tier 2 records them beside the image
(`Parser::image_supers`), and the engine reads the image back with its own readers
(`tape::resident`) and makes the tape with them (`json_parser::tape::encode_written`). Writing
the classes into the codes would make that one pass the image alone.
