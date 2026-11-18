# 0399. Format 16 is block-major and written in one pass, with a presence structure beside it

Status: accepted (2026-11-18)

## Intro

Every parser should write formats 16 and 16z as it parses, on the CPU and on the GPU, with no intermediate node
format. Today every format 16 tape goes through format 15:

- the lean parsers' `Writer` keeps 15-shaped working columns for every node, then `finish_16` runs `f16::write`;
- tier 2 (the Bitlex compiler's backends) writes format 15 bytes;
- tree-sitter walks its cursor into a format 15 image, then `f16::write_from` converts it.

Format 16 was meant to be written in one pass (0156 §7), but its layout prevents it:

- **Ten regions end to end.** Every region's start, and the side lists' part starts ahead of them, depend on the
  final node count and the side lists' sizes.
- **No cell has its final place before the parse ends.** `write` fills five growing arrays and the side lists,
  and `Tape16::laid` joins them at the end.
- **The writer's peak is about 23 bytes a node** (26 for 16z), for a tape of about 4.5 bytes a node.

The user (2026-11-18):

- "We can adjust the 16 and 16z formats so that this is easier. They are not written in stone yet."
- "We can accumulate things, just not do a full transform from an intermediate format."
- "Incremental is the parser. One pass + whatever needs to be written at the end from accumulated information."
- "I really want for 16 to be designed with writing in one pass in mind and for this to be fast."
- "I want this to be a switch based on common abstractions, not via copying and specialization of the kernels."

Then: "go ahead with block major; we output both the tape and a separate kinds/fields structure; for each block,
optionally, a kinds/fields structure."

## Decision

### 1. The layout: a header, fixed-stride blocks, a tail

```text
header      fixed size: n, the stride, block 0's start and the tail's starts (written at the end)
block 0     codes[Z] | slots[Z·w0] | gaps[Z·w1] | lengths[Z·w2] | checkpoints[Z/K·cw]
block 1     … at block 0's start + S, S = Z·(1 + w0 + w1 + w2) + Z/K·cw
last block  the same columns at its own row count R = n − b·Z (no padding), then 8 bytes of room
tail        the four side lists (each block's part, as today), their part starts, the newline index
```

- **Constants:** Z = 1,024 nodes a block (the side lists' part and 16z's block, as before), K = 8.
- **Addressing.** Node i's cell in column r is at `blocks + (i >> 10)·S + R(b)·before[r] + (i & 1023)·w[r]`, where:
  - `R(b) = min(Z, n − b·Z)`;
  - `before[r]` is the bytes a row holds before column r: 0, 1, 1 + w0, 1 + w0 + w1, 1 + w0 + w1 + w2.
- **Checkpoints** are the block's last column. Checkpoint c (node c·K) is in block c >> 7, at row c & 127.
- **A full block's place depends only on its index.** So the writer writes every cell at its final address. At
  the end it does three things:
  - moves the last block's columns down to its row count, at most S bytes;
  - appends the tail;
  - fills in the header.
- **A one-block tape** (n ≤ 1,024: most files) is today's region layout, with no side-list part starts ahead of it.
- **Side lists' value widths are known before the first node.**
  - Slots: 4 bytes (n < 2³⁰, which every read's u32 index needs anyway).
  - Gaps and lengths: the bytes the source's length takes.
  - Codes' low parts: the language's, as before.
- **Escapes arrive out of order** (an ancestor's size escapes at its close). So each block keeps its entries until
  the tape ends, and its part is laid into the tail then.
- **The view (`TapeView`) holds:**
  - `columns[0]`: block 0's start;
  - `columns[1..5]`: `before[1..5]`;
  - `stride`: S (a new field).

  The side lists, part starts and newline index keep their fields.
- **The reads change once, in CLIF** (0310): `cell(r, i)` and `checkpoint(c)` compute the address above, and the
  Rust and MSL backends are generated from them.
- **Marks.**
  - The magics become `TMPTAPB6` (16) and `TMPTAPBZ` (16z).
  - `formats::F16_LAYOUT` goes to 4, so a store built before reads cold, not wrong.

### 2. One writer, its ops the abstraction

- **The ops** take a writer state:
  - `open(number, start) → node`;
  - `close(end)` (the innermost open node);
  - `leaf(number, start, end)`;
  - `wrap(at, number, start)`: an open node before the nodes from `at` on;
  - `retag(node, number)`;
  - `set_start(node, start)`;
  - `truncate(len)`, `checkpoint` and `rewind`;
  - `commit()`;
  - `finish() → (tape, presence)`.
- **The state:** the output's address, the open stack (each open node: its index, its children so far, its last
  child, its start), the window's edit state, and each pending block's side-list entries.
- **When each cell is written:**
  - a node's code, gap, its checkpoint (every K-th node) and its length (a leaf's) when it is opened or closed;
  - an inner node's slot (`size << 1`) at its close;
  - a leaf's slot when the node after it is opened, or at the end.
- **The commit frontier.**
  - `commit()` says no edit reaches before the nodes so far (a statement parsed: what `Writer::commit` means today).
  - Edits (`wrap`, `retag`, `set_start` and `truncate`) touch only the window: the nodes from the frontier on, and
    the open nodes.
  - A wrap moves the window's cells up one row, across a block edge where it falls. It re-encodes the slots that
    describe its new node's children (their sibling counts and their parent change) and the checkpoints after it.
- **When a block is final:** every node in it is closed, the node after its last is open, and the frontier is past
  it. A block that holds an open ancestor stays pending; the root's block finishes last.
- **One compile, one entry point.** The parse calls the same ops whatever the format. The format is a field of
  the writer's state, read only by the block sink (§3), once per 1,024 nodes.
- **Written once, three backends.** The ops are written in CLIF, as the reads are (0310), and translated to Rust,
  Cranelift and MSL. Until then, the Rust reference in `tape_format::f16` is what the CLIF ops are tested against.
- **The callers:**
  - lean parsers call the ops through their `Writer`, which keeps only the window's working state;
  - tier 2's tape words (LEAF, OPEN, CLOSE, MERGE, WRAP, RETAG, drop, ACCEPT) become calls of them;
  - tree-sitter's cursor walk calls `open`, `leaf` and `close` directly.
- **Format 15 keeps its writer until it is dropped.**

### 3. 16 and 16z: a block sink

- **When a block is final**, the sink either leaves it in place (16) or compresses its four pieces and appends
  them (16z). The pieces are codes, slots, gaps and lengths, each with its side-list part.
- **16z's stored form is unchanged:**
  - the header, the side lists' part starts, the checkpoints (raw) and the newline index;
  - the piece index (written at the end, since pieces are appended in the order blocks finish);
  - the payload.
- **Placement** decodes each block into its place in the block-major tape. A block decodes alone, as before.
- **The 16z writer's peak** is the compressed output, plus the pending blocks (those holding open ancestors),
  plus one block's match state.

### 4. A presence structure beside the tape

- **The writer also outputs a document's presence:** the kinds and the fields its nodes have, as bitmaps over the
  language's kinds and fields.
- **Optionally, one presence per block** (`Presence::blocks`).
- **It is not in the tape.** Its use is deciding not to load, decompress or dispatch a document (or a block), so
  it belongs where that decision is made. The store keeps it beside the document's row as a derived fact, written
  once where it is computed (0353).
- **The writer sets two bits a node,** from the language's tables of each number's kind and field.
- **Census** (a5eab5c73, 100 corpus0 files): kinds and fields alone rule out 20% of the (clause program, file) pairs
  and 12% of the (label program, file) pairs, against 39% and 21% that any file-level test could.

### 5. 16z's compression by node ranges (later step)

- A subtree is contiguous in every column, so an earlier subtree of the same shape is an LZ4 match, apart from its
  edge cells. At close, the writer hashes each subtree bottom-up and registers it.
- When a block is final, its pieces are written as ordinary LZ4 sequences: matches from the registered subtrees
  within the block, literals for the rest. Decoders do not change.
- Measured against LZ4 HC 9 in the lab before it replaces it.

## Requirements, each with its test

| | Requirement | Test |
|---|---|---|
| R1 | Block-major tapes read as the tape they were written from, node by node: every read, every format | `f16::tests` (the existing round trips, over the new layout), `tape_reads_*` |
| R2 | A one-block tape is the region layout: no part starts ahead, no padding | `one_block_is_the_regions` |
| R3 | A tape of n nodes is `header + (B−1)·S + last block + 8 + tail` bytes: no padding in the last block | `the_last_block_is_compact` |
| R4 | The writer writes each cell at its final place: at `finish` it moves the last block's columns down and appends the tail, and nothing else is copied | `finish_moves_only_the_last_block` |
| R5 | The ops (`open`, `close`, `leaf`) give the tape `write` gives from the same nodes, in 16 and 16z | `ops_write_the_tape` |
| R6 | A wrap, across a block edge too, gives the tape of the same nodes written in order | `a_wrap_across_a_block_edge` |
| R7 | `truncate` and `rewind` give the tape of the nodes kept | `truncate_and_rewind` |
| R8 | 16z written by the ops is the 16 tape's blocks, each decoded in place alone | `sixteen` (the existing check), `a_block_decodes_alone` |
| R9 | A 16z block is compressed only when final: the root's block last; the pending blocks are those holding open ancestors | `blocks_finish_when_closed` |
| R10 | The presence is the kinds and fields the tape's nodes have, per document and (optionally) per block | `presence_is_the_tapes_kinds` |
| R11 | The device reads block-major tapes as the CPU does | `the_label_kernels_compile_for_every_format`, `on_metal_anchored_walks_find_the_unanchored_roots` |
| R12 | No format 15 column, image or tape is made for a format 16 run (lean, tier 2, tree-sitter) | `no_intermediate_tape` (step 3) |

## Steps

1. The layout, the reads in CLIF, the Rust reference writer's ops (`open`, `close`, `leaf`, `commit`, `finish`),
   the 16z sink and the presence (R1–R5, R8–R10). `write` becomes a caller of the ops.
2. The edits (`wrap`, `retag`, `set_start`, `truncate`, `rewind`) (R6, R7). The lean `Writer` writes format 16
   through the ops, keeping only the window's working state.
3. Tree-sitter's cursor walk and derived documents call the ops (R12); then tier 2's words.
4. The ops in CLIF, three backends; tier 2's GPU path writes its tape on the device.
5. Node-range LZ4 for 16z (§5), after the lab measures it.
6. The store keeps the presence; the planner leaves out the documents and blocks a program's anchor cannot touch.

## Measured

(Filled in as the steps land.)
