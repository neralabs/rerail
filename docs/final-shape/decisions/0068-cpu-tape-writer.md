# 0068: The CPU tape writer: preorder from the masks, one packer

Status: accepted (user, 2026-11-07: "complete all of this", the list item "The tape writer
(steps 3–4)"; step 3 of `docs/parser-generators/13-tape-on-the-gpu.md` §7)

## Intro

A block program compiled from a Bitlex file has `node` and `code` rows: which masks say where
each node kind starts and ends, and which facts hold at a node's start. The tape writer turns a
file's masks into its tape. This record fixes how the CPU executors do it, and how its bytes are
made equal to tree-sitter's.

```
masks (one bit per byte) --> starts and ends, a word at a time --> nodes in preorder
                         --> TapeImage::from_written (the tape's one packer) --> v9np bytes
```

## Prior work consulted

- **ast-index-spike indexes:** the mldb tape row (O(1) navigation from precomputed sizes over the
  source buffer) and the spike's `TAPE_AST_GREP_RECONSTRUCTION`. **Agree:** the size column is
  the structure, and no parent or child table is written.
- **Lessons ledger #1** (one row per node is fatal; the tape is the truth). **Agrees:** the
  writer's only per-node output is the tape's own record. Nothing else per node is kept.
- **Lessons ledger #32** (store anonymous nodes). **Agrees:** every token is a leaf node, as in
  tree-sitter's tree, so `{`, `,` and `"` are records.
- **Lessons ledger #10** (measure the code that runs). The corpus check runs the writer over
  Cranelift's batch output, the path a wave would take.
- **Doc 13 §3.3:** "on the CPU the blocks run in order, so a stack of open starts per level
  replaces the search". **Agrees.**
- **Doc 13 §3.2:** generated code writes the records in the same loop as the masks.
  **Departs for now:** the writer runs after the masks, over their output words, for both CPU
  executors. See "Not decided here".

## Decision

1. **`transducer/packed/tape_writer::write`** reads the outputs a word at a time. In a word
   the only loop is over its set bits: starts, ends and level drops.
   - **A start** of row `r` (rows in rank order) takes the next slot `ord`. Its field (the first
     field fact that holds) and supertypes are read at its byte.
   - **An end bit** of row `r` closes its open nodes. Without a level that is all of them; with
     a level, the stack for the level after the bit. The size is the count started so far minus
     `ord`; the end is the bit's byte plus one.
   - **Errors** send the file to tree-sitter (§1.4):
     - a leveled node still open when the level drops below its start's;
     - a level below zero, or not zero at the end;
     - a node never closed;
     - a node ending after its parent;
     - more than one root.
2. **Records come out in preorder,** so first-use code numbering is exact on the CPU, and no
   guess is needed (§3.4 is the GPU's).
3. **One packer.** `TapeImage::finish` is split into parent sizing and `pack`.
   `TapeImage::from_written` packs written nodes with the same code: codes folded and numbered
   by first use, `base` per 16 nodes, and the narrowest widths. Equal nodes give equal bytes by
   construction.
4. **`tape_writer::tape_of(program, outputs, fallback, grammar, source)`** is the entry point
   for any executor's outputs. `None` means the file goes to tree-sitter.
5. **The Bitlex file is corrected where tree-sitter differs.** tree-sitter-json 0.24.8's number
   accepts only `-` as an exponent's sign. `json.bitlex` accepted `+`, and the byte comparison
   found it.

## Results

- Lexer inputs and JSON cases: every tape not sent to tree-sitter is equal byte for byte, on the
  interpreter and on Cranelift.
- The 774 tracked JSON files of a data-layer checkout (39.4 MB): 771 tapes equal, 3 to
  tree-sitter, 0 differ. 15.4 s of CPU in a debug build, tree-sitter's parse included.

## Not decided here

- **Writing records inside Cranelift's loop (§3.2).** It needs the open stacks carried across
  blocks, as loop state or a memory region. The masks pass would then write the tape in one
  pass.
- **JSON as a stored tape dialect.** It has a test-only tape tag (251) for the comparison.
