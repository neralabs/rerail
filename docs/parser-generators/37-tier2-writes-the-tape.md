# Tier 2 writes the engine's tape

Status: building (2026-11-13), under decision 0140.

## Intro

The engine's tape is a byte string: a header, five columns and a code table
(decision 0140). The lean parser writes it as it parses
(`rerail_tape_format::Writer`).

Tier 2 still writes a node table of its own, a 24-byte row a node:
kind, field, start, end, size, parent, extra flag. That table is not the tape.
Making a tape from it would be a conversion, which the tape's design rules out.

This page is how tier 2's generated code writes the tape's bytes itself,
in every backend: the interpreter, Cranelift, Rust ahead of time, and Metal.

## What tier 2 keeps, and what it writes

Working state, a 4-byte cell a node, never part of the tape:

- `key`: the node's kind and field (the JIT's 32-bit kind-and-field word);
- `start`, `end`: its bytes.

The tape's columns, written at their place as the node is made or closed:

- `size` and `tail` at the node's close;
- `off` at its push, from its block's `base` (each 16 nodes' least start);
- `code` at a commit point: the code ids, numbered by first use.

The parent goes. The tape has no parent column, so nothing keeps one:
the wraps, drops and pending extras that remapped parents no longer do.

## The widths are constants of the compiled code

A file's widths (2 or 4 bytes for `size`, `off`, `tail`) are fixed
while it is parsed. They are compile-time constants of tier 2's code,
so each store is one instruction, as the lean parser's `Narrow` (page 35, decision 0140).

- Tier 2 is compiled at the narrow widths (2, 2, 2), which hold for nearly every file.
- A value that does not fit sets a flag; the file is parsed again at the widths it needs,
  by code compiled at those widths (made the first time a file needs them).
- Exactness (4 where 2 would hold) is checked only where a flag is set:
  a rare, outlined check over the columns.

## Each word's writes

| Word | Writes |
|---|---|
| `LEAF` | key, start, end; offset; size 1; tail |
| `OPEN` | key, start; offset |
| `CLOSE` | end; size; tail from its last descendant; a commit where it closes a top-level node |
| fold, `MERGE` | the leaf closed again; the nodes after it taken back, their block's base again |
| `WRAP` | the columns from the wrapped node up one cell; the blocks from there again |
| `RETAG`, `NODE_FIELD` | the key; a committed node's codes taken back from it |
| drop | the columns compacted; sizes and tails again; the blocks from there again |
| `ACCEPT` | the root closed; the codes committed; the image's end |

Each rare one (a wrap of more than a leaf, a drop, a block again, a commit,
the image's end) is an outlined generated function, a step a node.

## The image's end

At `ACCEPT` the columns are moved into place after the code column,
and the code table and header are written (the user's choice: the size column's place
depends on the node count). On the CPU this is a generated function.

Open: the header's digest (SHA-256 of the source). The CPU takes it
from the CPU's SHA instructions, in the harness. On Metal it must be a kernel's.

## Order, each step tested

1. The interpreter's tape: its nodes through the writer (`stream::image`).
   Done: the lean parser's bytes over the corpus.
2. Cranelift writes the tape's bytes; its node table goes.
   Done: the lean parser's bytes over the corpus (1,362 files, 1,360 taken).
3. Rust ahead of time (`rust_stream`), the same.
   Done: the JIT's bytes over the corpus and the in-place cases (1,366 inputs).
4. Metal (`tier2_on`). Tier 2 there is the JIT's code over the device's tier 1:
   its bytes are the JIT's. A kernel of its own, and the digest, are open.
5. `NodeRow`, `Columns` and `engine_columns` go; `rr_parser::tape` gives bytes.
   Done. The language sets its tape tag on the lowered program (`Structure::tag`);
   the compiler has no default.

Every step: the corpus's tapes are the lean parser's, byte for byte.

## Cost

Over the corpus (1,360 files, 8.80 MB, 1,166,154 nodes), tier 2 alone,
on tier 1's output made before:

| Tier 2 | Instructions | Cycles |
|---|---|---|
| Cranelift, writing the old node table | 248.6M | |
| Cranelift, writing the tape | 328.2M | 88.1M |
| Rust ahead of time, the old node table | 224.9M | |
| Rust ahead of time, the tape | 274.1M | 73.5M |

The lean parser, lexer and all, is 454.6M (133.3M cycles) in the same run.

## Still to do

- Wide files: code compiled at the widths a file needs, the file parsed again,
  in place of refusing it to the fallback.
