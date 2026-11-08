# 0080: Tier 2 writes the tape

Status: accepted (user, 2026-11-08: "please complete the tape writing").

## Intro

simdjson's stage 2 writes its tape as it walks the tokens. Tier 2 does the
same: the compiled words write the file's tape directly, in v9np's node
columns, byte for byte equal to the tape of tree-sitter's tree. No Rust step
runs over the nodes afterwards.

## Prior work consulted

- **0068 (the CPU tape writer):** one packer, codes numbered by first use,
  narrowest widths. **Kept:** the same bytes. **Departs:** no packer runs after
  the walk; the walk writes the columns.
- **Doc 17 §5e (widths speculated):** widths guessed before the file, ORs as it
  goes, at most one second pass. **Agrees.**
- **0079 (the words):** a node's field and supertypes are words after its
  `OPEN` or `LEAF`. **Kept:** they are read when the function is compiled.

## Decision

1. **The columns.** Each node's code (2 bytes), size, offset from its block's
   base, and tail, each at its width, plus a base per 16 nodes. The base is the
   first node's start, since starts do not fall in preorder.
   - A `LEAF` writes all four.
   - An `OPEN` writes its code and offset.
   - Its `CLOSE`, or `ACCEPT` for the root, writes its size and tail.
2. **Codes.** A node's code (kind, field, supertypes) is known at compile time
   from its `OPEN` or `LEAF` and the `NODE_FIELD` and `NODE_SUPER` words right
   after it. No jump may land on those words: the compiler refuses the program
   otherwise. Each distinct code has a slot. At run time a slot gets its number
   at first use, from a table, as the packer numbered them.
3. **Widths.** The function is compiled for a shape: its widths (2 or 4) and
   whether tails are from a node's last descendant's start. The shape is
   guessed from the stream:
   - the size width from at most two nodes a token, plus the root;
   - the offset and tail widths as two bytes.

   The run ORs each column's values, and notes a node that ends before its
   last descendant's start. If the guess was wrong, the function for the right
   shape runs once more. Shapes are compiled once each and cached.
4. **The image.** `TapeImage::from_columns` takes the columns and the codes in
   first-use order. It does nothing per node.

## Results

- The lexer inputs, the JSON cases and the tier-2 files: every tape that is
  not sent to tree-sitter is equal byte for byte. A file tree-sitter finds an
  error in is never given a tape.
- A 70 KB string takes the second pass: its tail needs four bytes.
- The data-layer checkout's 436 JSON files (15.7 MB): 436 equal, none sent to
  tree-sitter, 7 second passes.
- Cost: see doc 19.
