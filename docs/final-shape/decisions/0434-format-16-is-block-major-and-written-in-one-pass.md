# 0434. Format 16 is block-major and written in one pass, with a presence structure beside it

Status: built (audited 2026-11-23). Accepted (2026-11-18). **0631 (2026-11-22):** format 15 is removed; the writer
described here is the only writer, and no edit makes a tape format 15's.

Numbered 0399 where it was written (the hub's 0399 is "a `\u` escape's checks run at its token"). Numbered 0426 on
django-bugs (until roll-up 7) and on CUDA-support (until roll-up 8).

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
  - `set_start(node, start)` (an open node);
  - `commit()`;
  - `finish() → (tape, presence)`.
- **The state:** the output's address, the open stack (each open node: its index, its children so far, its last
  child, its start), the window's edit state, and each pending block's side-list entries.
- **When each cell is written:**
  - a node's code, gap, its checkpoint (every K-th node) and its length (a leaf's) when it is opened or closed;
  - an inner node's slot (`size << 1`) at its close;
  - a leaf's slot when the node after it is opened, or at the end.
- **The commit frontier and the window** (amended at step 2, below).
  - `commit()` says no edit reaches before the nodes so far but for the open ones (a statement parsed: what
    `Writer::commit` means today).
  - The window is the nodes since the last commit. The caller holds it in its own working state (the lean `Writer`'s
    columns), where its edits are made: a wrap (`insert`, a chain's wraps), `insert_leaf(s)`, `truncate` and `rewind`.
    The commit passes the window's nodes to the ops in preorder: `open`, `leaf`, and `close` where a subtree ends.
  - Only two edits reach a node the ops already have: `retag` (a kind found at a node's close) and `set_start` of an
    open node. Both write the node's cells again in its block, which is not final while the node is open. An open
    node passed on is closed in the ops once its subtree is.
  - An edit before the last commit (anything but those two) makes the tape format 15's, said once a language
    (`keeps_15`), as a code the table lacks does.
  - The window is all the writer's columns hold: a commit gives the cells of the nodes it passes back, and the next
    nodes take them (`WINDOW` = 4,096 cells at first, more where a statement is longer). The parse may still read
    its open ancestors (their key, start, and once closed their size and end), which the writer keeps beside the
    ops' open stack. A read of any other node passed on is lost, as an edit before the commit is.
  - So a format 15 tape cannot be finished from the columns. Where format 16 fails, the sink changes its target to
    format 15 and asks for the parse again at the same widths (`Sink::again`); the parsers' loops already parse
    again on a `Redo`.
  - Every lean parser with a table commits at its top-level items: Python, JavaScript and TypeScript, Rr, C#, Java
    and PHP at their statements and declarations, Rust at its items, CSS at its rules, HTML and Vue after each tag, text or entity, Markdown before each document-level block (not between a
    paragraph and the setext underline that may follow it), and Markdown's inline parser every 256 nodes, which
    it writes in order and never edits. SQL does not commit yet: its program holds its tokens (each `;`) until it
    closes, and `claim` decides then, from all its children, which the tape stores; its window is the file until
    `claim` decides a token as it comes.
- **When a block is final:** every node in it is closed, the node after its last is open, and the frontier is past
  it. A block that holds an open ancestor stays pending; the root's block finishes last.
- **16z's piece order is the order its blocks were finished**, which follows the writer's commits: a parser that
  commits at its statements and the conversion, which commits at each node, give the same blocks in another order.
  A reader goes through the pieces' index, so the order is not part of what a tape says; the bytes are the same for
  the same parser and source, not across writers.
- **One compile, one entry point.** The parse calls the same ops whatever the format. The format is a field of
  the writer's state, read only by the block sink (§3), once per 1,024 nodes.
- **Written once, three backends.** The ops are written in CLIF, as the reads are (0310), and translated to Rust,
  Cranelift and MSL. Until then, the Rust reference in `tape_format::f16` is what the CLIF ops are tested against.
- **The callers:**
  - lean parsers call the ops through their `Writer`. Its target is given where it is made: each parser takes a
    `Sink` (its `target()`, and its `finish`) where it took a finish closure, and makes its writer
    `Writer::new(…).to(sink.target(), src)`. A closure is a sink for format 15. Each key's number is the language's
    number of its code (`Sixteen::number`), resolved once a code id;
  - tier 2's tape words (LEAF, OPEN, CLOSE, MERGE, WRAP, RETAG, drop, ACCEPT) become calls of them, once the ops are
    in CLIF (step 4): tier 2's code is Cranelift's, and calls them there;
  - simdjson's kernels (JSON) switch at run time, by their sink: with an event buffer set, the walk's `open`, `leaf`
    and `close` append events (three words each) instead of writing format 15's columns, and the buffer is passed to
    the ops when full (4,096 nodes) and at the end (`rerail_simdjson::events_with`). One compile, one entry point;
    the walk never edits a node, so each window is committed;
  - tree-sitter's cursor walk calls `open`, `leaf` and `close` directly, through an output trait (`TreeOut`) whose
    other implementation is format 15's pending nodes. Every walk that writes a tree uses it, through
    `TapeImage::walked`: the cursor walk, an embedded program's tape with its placeholders left out
    (`omitting`), Blade's Forte tree and the format strings' trees. A walk never edits a node, so it commits after
    each one, and 16z's blocks are compressed as soon as their nodes close.
- **Format 15 keeps its writer until it is dropped.**

### 3. 16 and 16z: a block sink

- **When a block is final**, the sink either leaves it in place (16) or compresses its four pieces and appends
  them (16z). The pieces are codes, slots, gaps and lengths, each with its side-list part.
- **16z's stored form is unchanged:**
  - the header, the side lists' part starts, the checkpoints (raw) and the newline index;
  - the piece index (written at the end, since pieces are appended in the order blocks finish);
  - the payload.
- **Placement** decodes each block into its place in the block-major tape. A block decodes alone, as before.
  - **Since the merge of Native-and-GPU-strata (2026-11-18):** a 16z tape is never laid out whole (decision 0353
    step 4b). It is placed as stored, and a reader decodes the block a read falls in into a cache entry
    (`rerail_clif::reads`' `z_decode`, written once in CLIF for the CPU, the JIT and the kernels). The entry holds
    the block as a format 16 tape of one block: column `q` at `nb × C[q]`, `C[q]` the least that clears the piece
    before it (its column, then its side part) and 16 bytes of room. The view's `columns` hold `C`, as a
    block-major tape's hold the bytes a row has before each column, so format 16's reads read the entry unchanged.
    The checkpoints are copied from where they are stored raw. The newline index is read in place (`lines` in the
    16z view). `decode_into` and its relatives remain only as a test's oracle.
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
- **Measured, not taken (2026-11-18).** The lab's `zrange` (`lab/tape16/src/zlevel.rs`) parses each 16z piece
  into LZ4 sequences whose matches are node ranges: at each row, an earlier subtree of the block with the same
  bytes in the column (a), else one whose descendants have them, the root's edge cell a literal (b); each match
  extended as LZ4 extends one; the side-list part literals; each piece stored by 16z's rule. Against HC 9 on the
  same pieces: dl-10 +32% (JSON, 7.0 M nodes), +16% (Python, 2.5 M), +17% (Rust), +18% (JavaScript); rr +21%
  (2.0 M nodes). LZ4's fast compressor is +2–4% there (+12–13% on JSON and rr). Node ranges are a subset of the
  matches HC 9 finds (a block's column is at most 8 KB, inside LZ4's 64 KB window, and HC 9 searches every
  position), so they cannot be smaller; they could only be faster, and the fast compressor is both faster and
  smaller than them. 16z keeps HC 9 (decision 0434 §3).

### 6. The ops in CLIF (step 4, 2026-11-18)

The reference writer (`f16::Writer`) holds its state in `Vec`s that grow and a map of the pieces seen. A device
holds none of those. The ops in CLIF work on one region of memory whose layout is fixed when it is made:

- **The writer's state** (`rerail_clif::writer::State`, `repr(C)`, at the start of its region): the document's
  constants (the language's widths, cut and per, the escapes, the source's length, the block's stride and column
  offsets, 16 or 16z, whether a presence a block is kept), the variables (`n`, the last start, the frontier, the
  first failure), and each part of the region as an offset and a capacity:
  - the open nodes, a stack of `{node, children, last, start}`;
  - the blocks: each one's room and whether it is final. In 16 a block's room is its place in the tape, which is
    the output; in 16z a room is one of a few, given back when its block is final and compressed;
  - the side lists of the blocks not yet final: four sorted lists a room, each up to a block's rows (`Z`). They are
    the reference's `side`, kept as a node offset and a value an entry. An entry inserted shifts the ones after it,
    at most a block's;
  - the laid parts: each final block's four side-list parts as stored (`part_into`), in the order blocks become
    final, with each one's start and length by block. `finish` puts them in block order in the tail (16), or a
    block's are compressed with its columns (16z);
  - the presence: the document's bitmap, and one a block where asked;
  - 16z's output: the payload, the piece index (start, length, kind), each list's part lengths, the checkpoints,
    and the pieces seen (a hash table of a piece's 64-bit hash to its number). A repeat is taken only after the
    earlier piece's stored bytes compare equal (a raw piece's with this one's, an LZ4 piece's with this one's
    compressed form; below), so a hash collision costs a compare, never a wrong tape.
- **The ops** are CLIF functions whose one pointer is the state, as a relation's reads take their descriptor
  (the backends' `Family::Relation` convention, so no new family): `w_open`, `w_close`, `w_leaf`, `w_commit`,
  `w_retag`, `w_set_start` and `w_finish`, on top of `w_put`, `w_put_code`, `w_settle`, `w_finalize`, `w_lay`
  and the 16z block sink (an LZ4 compressor in CLIF, its hash table in the state). A part is named by its offset
  from the state's address, so a region moves whole. A cell is written as a word (its bytes in a 64-bit load and
  store, the bytes past it kept), so each part keeps 8 bytes of room past its capacity; no op has a loop a byte.
  Each
  returns 0 or its failure, the first of which is kept: the reference's (a bound, a node outside the root, an edit
  of a final block), or *full*, naming the part of the region with no room. A full op writes nothing past its
  part's capacity.
- **Who sizes the region.** The host makes it from the document's size, and before each op reserves what the op
  can need, as a `Vec` reserves before a push (a part's buffer grown and the state's offset moved: the writer's
  own memory, not a tape), so a host op never finds a part full. The device makes it from the batch's
  budget; a document whose writer fills is parsed again on the host, counted (as a full arena is, 0334).
- **One implementation.** Once the CLIF ops give the reference's bytes, the host's `Writer` is the generated
  Rust ops behind the same API, and the hand-written writer stays only as the tests' oracle.

**Done (2026-11-18): format 16's ops.** `rerail_clif::writer` holds the state and the ops; `rust_source` makes
`tape_format/src/generated_writer.rs` (checked by `the_tape_crates_writer_is_this`), and every op passes
Cranelift's verifier (`the_ops_verify`). `f16::ops::OpsWriter` is the host's region behind `f16::Writer`'s API.
R13 holds for format 16: the ops write the reference's bytes and presence for escaped codes and values, late
closes, a commit every node, every 97 or none, `retag` and `set_start`, a presence a block, one node. R14 holds:
the reference's broken cases fail with the same first failure, and a part held at a capacity (the tape, the open
nodes, the block table, the laid lists and their places, the side areas given back) fails as full, naming it,
with nothing written past its room.

**16z's compressor is LZ4 HC level 4, ported to CLIF (2026-11-18).** 0156 §2.1 stores an LZ4 piece at HC level 9.
A device writer cannot call liblz4, and HC 9 searches 256 chain entries a position (and adds a pattern analysis);
level 4 searches 8 with the same parser. The lab's `tape16 zlevel` cuts each file's format 16 tape (the shipped
tables) into 16z's pieces as the writer does and stores each as 16z does (LZ4 where it saves a tenth), by each
compressor (M5 Max, lz4 1.10.0):

| capture, language | nodes | LZ4 fast | HC 2 | HC 3 | HC 4 | HC 9, B/node |
|---|---|---|---|---|---|---|
| dl-10, python | 2.47 M | +3.8% | +1.7% | +0.1% | +0.0% | 2.797 |
| dl-10, json | 6.99 M | +13.2% | +6.7% | +1.2% | +0.5% | 1.299 |
| rerail-5b, rust | 1.41 M | +4.3% | +1.9% | +0.2% | +0.1% | 2.993 |
| rerail-5b, typescript | 0.32 M | +4.3% | +1.9% | +0.2% | +0.0% | 2.954 |
| rerail-5b, rr | 0.58 M | +11.4% | +4.5% | +0.6% | +0.2% | 1.848 |
| rerail-5b, json | 2.81 M | +10.7% | +5.1% | +1.5% | +0.7% | 1.339 |
| sqlite, python | 0.47 M | +3.9% | +1.7% | +0.1% | +0.0% | 2.806 |

So HC 4 costs at most 0.7% of the pieces' bytes, and a compressor without a chain (LZ4's fast one) 4–13%. The
ops port liblz4's `LZ4HC_compress_hashChain` at level 4 exactly (no dictionary, no pattern analysis, no chain
swap), and the reference writer calls liblz4 at level 4, so R13 compares 16z's bytes too. The port's tables (a
hash of 2^15 u32s, a chain of 2^16 u16s) are kept across pieces as a reused liblz4 context keeps them: each
piece's indices start 64 KB past the last piece's end, so an earlier piece's entries lie below its low limit and
are never read, and the tables are cleared only past 1 GB, as liblz4 does; the output is the fresh context's.
A repeat compares a candidate's stored bytes: a raw piece's with this piece's, an LZ4 piece's with this piece's
compressed form (the compressor is a function of the piece's bytes), so no piece is decoded to find a repeat.

**Done (2026-11-18): 16z's block sink.** `w_zblock` stores a final block's four pieces at the payload's end in
turn (`w_zpiece`: a repeat, else LZ4 where it saves a tenth, else raw), keeps its checkpoints and gives its room
back; `w_zfinish` lays 16z's tail and header. `w_lz4` is liblz4's level 4 (`the_clif_lz4_is_liblz4_hc4`: its
bytes on empty, short, random, run-heavy and tape inputs and past the chain's 64 KB reach, through one reused
state, and unlike HC 9's). `finish` is two ops, `w_finish_blocks` then `w_finish_tail`, so the host reserves the
tail's room once the laid parts are known (a device calls `w_finish`, both). R13 now covers 16z byte for byte,
with raw, LZ4 and repeat pieces, and random trees (every tail alignment, escaped gaps and lengths); a test build
fills the tape's fresh bytes with 0xa5, so a byte the ops leave unwritten fails it. At scale (the lab's `tape16
zops`): every file of dl-10, dl-50, rerail-5b, rr and sqlite, 6,731 files and 19.6 M nodes in 11 languages,
written by the ops' Rust and by the reference, gives the same bytes in 16 and in 16z. That check found the one
difference the tests had not: the tail's padding before the part starts was left as the last block's moved
columns, not zeros. Cost (M5 Max, the lab's opt-level 3, dl-10, instructions retired a node, the format 15
tape's reads included): python 16 by the ops 836 against the reference's 728, 16z 1,189 against 1,081; json
796 against 699, 1,078 against 985. So the ops' Rust cost 10–15% more than the hand-written writer, once the
host reserves only where an op can need room (a close only when its node's block is behind the last begun).
The gap was the translator's: it checked each load of a part's cell against a tape's span (`ld`), a span the
writer does not have (its parts are buffers of their own; the ops passed `u64::MAX`), so every cell load paid a
compare and a branch that bounded nothing. `rust::translate_ops` translates the ops with plain loads: their
bounds are the host's reservations. And an op is inlined only where it is `hot`, run at every node (`w_put`,
`w_value`, `w_put_code`, `w_settle`) or at every position (`w_lz4_find`); a block's or a piece's work stays a
call. Then, over dl-10's 9.5 M nodes in all its languages: 16 by the ops 678 instructions a node against the
reference's 708, 16z 865 against 1,010. The compressor inserts each position into its hash chain, as
liblz4's does: the codec's work, not a scan of the text.

**Done (2026-11-18): the ops on the JIT and on the device (R15).** `rerail_clif::writer::msl_source` writes the
ops as MSL, `rerail-tape-format`'s `writer.metal` (`WRITER_MSL`, checked by `the_tape_crates_writer_kernels_are_these`),
through the MSL translator the relation reads use: each op takes the region's buffer and the state's offset in it,
so a part's place, an offset from the state, is an offset in the buffer. The ops' function numbers start past the
reads' (`ID_BASE`), since the translators look a function's family up by its number. `rerail_clif::jit::Writer`
compiles every op into one JIT module, each calling the others. R13's cases run three ways against the reference
writer, byte for byte in 16 and 16z with the presence: the generated Rust (`the_clif_ops_write_the_references_tape`),
the JIT (`the_jit_ops_write_the_references_tape`, `OpsWriter` calling the JIT's ops in place of the Rust's), and
Metal (`the_device_writes_the_references_tape`: one kernel thread replays the recorded calls over the state and
its parts in one shared buffer). Each fails when the tail's padding write is taken out of the CLIF. On the device
the region does not grow: the test gives each part the room the host's run of the same calls reserved; sizing the
region from bounds before a parse is tier 2's (R17), with a part found full the device's failure (fail::FULL).

**Since roll-up 8 (2026-11-19): the device's ops are UKL (decision 0440).** `rerail_clif::writer::ukl_source` writes
them through the UKL translator the reads use, into the library's `<ukl/writer.h>` (checked by
`the_tape_crates_writer_kernels_are_these`); `writer.metal` and the MSL translator are gone. R15's test runs the
replay kernel on the process's device (`rerail_gpu`), whatever its backend.

**Since 2026-11-20: the device's ops are written by hand (decision 0524).** `<ukl/writer.h>` is the same ops as
functions written by hand, not the translator's output; `writer::ukl_source` and its check are gone. R15's test holds
them to the reference, and `the_writer_headers_constants_are_the_ops` holds their layout constants to `writer.rs`'s.

**Done (2026-11-18): the host's writer is the ops (R16).** `f16::Writer` is the ops in CLIF through their Rust
(`f16/ops.rs`); the hand-written writer is `f16::reference::Writer`, compiled only for the tests and for the lab,
which turns on the tape crate's `reference` feature (no build of the engine does), and so is liblz4's compressor:
the reader keeps liblz4's decompressor. `the_writer_is_the_clif_ops` locks it: a broken node is the ops' failure,
by its code and node, and `write_from` writes the reference's tape in 16 and 16z. Tests of the writer's behaviour
(R4's "the tape is the writer's room", R6's late edits, broken nodes) now run the ops; tests of the reference's
own state (`blocks_finish_when_closed`) and `Tape16::blocks`, a 16-to-16z conversion only tests use, stay with
the reference. For R4 the host reserves at `finish` exactly what `w_finish_tail` checks in 16 (the last block's
columns moved down, then the laid parts, their starts, the newline index), and the 16 tape's part starts with
the reference's hint for the tail, so a tape within the hint is the writer's room. The lab's `zops` now compares
`f16::write_from` with `f16::reference::write_from`: the same bytes on all 6,731 captured files.

**R17, tier 2: what it is now, and the plan (2026-11-18).** Tier 2 (the compiler's `native_cpu_stream`, a
program's words as one Cranelift function) writes format 15 in place as it parses (decision 0140): a working row
a node (key, start, end; 12 bytes, never part of the tape) and format 15's size, offset, tail and code columns.
Its words edit nodes already made: `WRAP` puts a node around the last one made at a level (moving the table after
it up a cell), `MERGE` joins leaves, `REREAD` drops a node and its subtree, `RETAG` and `FIELD` reach the last
node made or an open one, and an extra's leaf can be deferred past later nodes. It commits (numbers the codes) at
each close of a child of the root, up to that child, which stays open to edits; an edit before the commit takes
the codes back (`uncode`). The engine runs tier 2 in the Bitlex JSON parser (simdjson is the engine's JSON
parser) and over tier 1's buffers on Metal (`rerail_metal::tier2_on`); tier 2 itself runs only on the CPU:
nothing generates it on Metal yet (decision 0348).

The plan, as the lean parsers' (R7a): the working rows are tier 2's window, and a format 16 run passes nodes to
the ops at its commits, never making format 15's image. The run's context gains the writer's state and each key's
number in the language's table. At a commit, an outlined function (a step a node, as the code commit is) opens
each node from the last passed up to the commit point, closing those whose subtree ends before it, then commits
the ops; the root and the kept child stay open. A commit point is final for the ops only where no word can reach
before it: the uncoded cases (an edit before the commit) move the commit point back, not the codes. The ops are
compiled into tier 2's module, called as its own outlined functions are, and its region is sized from the
program's bounds, as its rows are (`Bounds`, `OVERFLOW`): a full part runs the file again with the bound's room.
`tier2_writes_format_16_by_the_ops` parses JSON by tier 2 into 16 and 16z and compares each with `write_from` of
its format 15 tape. Tier 2 on the device (R17's second half) waits on tier 2 on Metal.

**Done (2026-11-18): tier 2 writes format 16 by the ops on the CPU (R17's first half).** A tier 2 compiled with
`compile_with(.., sixteen)` (`Parser::sixteen`, on first use) holds the writer's ops in its module
(`rerail_clif::writer::define_ops`, which the ops' JIT now uses too) and two functions of its own that call them:
`native_cpu_tape::pass` (nodes `from..to` in preorder: the ops' open nodes whose subtrees end there closed, each
node opened by its number and committed) and `native_cpu_tape::retag`. Its run numbers no codes and makes no image:
at a top-level close it passes the nodes up to the commit point before this one, and at `ACCEPT` the rest, root
closed. `Parser::image16(bytes, lang, numbers, z)` makes the writer, reserves its room for the bound's nodes and a
guessed depth (`f16::Writer::reserve_run`), runs, and finishes; a part found full, or tier 2's own room overflowed,
runs it again with the bound's room. `CompiledStream::numbers(lang)` is the language's numbers by tier 2's keys
(kind, field and supertype bits), `u32::MAX` for none, which the pass makes the ops' failure (`fail::NUMBER`).

What the build found. (1) 16z lays its blocks in the order they become final, so its bytes depend on when the
ops commit; the pass commits after each open, as `write_with` does, and the ops' calls are then `write_with`'s,
whatever the batches. (2) A commit point is not final: a fold of the root's two closed leaves retags the first,
which is the commit point before the second's. The ops therefore take nodes one commit point behind; an edit before
the commit point moves it back (`Table::uncode`), and an edit of a node already passed is the ops' failure
(`fail::PASSED`). (3) The root is passed and still editable (`RETAG` of the innermost open node): a retag of a node
the ops hold is a `w_retag` too. (4) Format 15 had both faults as stale codes: a fold, and a retag of the root,
after their nodes were committed left the old kind's code. A retag before the commit point now takes the codes back
(`Table::retag`), so tier 2's image is the interpreter's.

`tier2_writes_format_16_by_the_ops` checks 16 and 16z, byte for byte, against `write_from` of tier 2's own format
15 tape on every ladder step's documents, long ones (3,000-element lists, 300 and 400 deep, 1,500 records, 800 JSON
objects with comments) and ones with 70,000 blanks. `edits_of_nodes_passed_reach_the_ops` runs two hand-written
programs, a fold before the commit point and a retag of the passed root, and checks the format 15 image against the
interpreter's nodes and both formats against the writer's. Each fails when the lag, the root's `w_retag`, or the
fold's taking back is removed. Not yet measured: the run's cost a node against format 15's image plus `write_from`.

R17's second half, tier 2 writing its tape on the device, waits on tier 2 on Metal, its own track: the user put
steps 5 and 6 first (2026-11-18).

**Step 6: what there is, and the plan (2026-11-18).** The writer makes the presence (§4, R10), and every engine
producer drops it where it takes the tape (`f.tape`): the lean parsers' finish, `.rr`'s, the walk's (`TreeOps`),
simdjson's, and the conversion from format 15 (`TapeImage::to`). What leaves files out today is the store's
positional search index (`search_index.rs`): each node's kind as an FTS token, in preorder, built by a maintenance
backlog that decodes stored tapes again. The warm path (`workspace/persisted.rs`) skips a clause in a file when
the index has none of its anchor's root kinds. On the cold path a clause's roots are `nodes_of_kinds` or the anchor
index's kind postings, each built over every node of the tape.

The plan, in three parts:
1. **The image holds its presence.** `TapeImage` keeps the writer's `Presence` beside its format 16 tape, with each
   block's (the writer asked for blocks: a few words per 1,024 nodes), from every producer. A format 15 image, or
   one read from the store, has none until part 3.
2. **The planner leaves out what an anchor cannot touch.** A clause none of whose root kinds the document has
   matches nothing there: no roots are looked for, and no anchor index or posting is built for it. A walk for a
   clause's roots (`nodes_of_kinds`, the postings) passes over the blocks that hold none of them. On the device,
   the label kernels' scans start only in the blocks that hold an anchor's kinds.
3. **The store keeps it**, as a derived fact in a block, written once where it is computed (0353): on 0456's kept
   facts (packed columns in a chain of mapped files, SQLite holding pointers only), which are on
   `final-rerail-shape-rr-align` and not yet here. Then the warm path reads it in place of the FTS index's kinds
   for format 16 tapes, and fields too.

**Done (2026-11-18): part 1, and part 2's clauses.** Every producer's writer is asked for each block's presence, and
`TapeImage::presence` keeps it (the lean parsers, `.rr`, the walk, simdjson, and the conversion from format 15 by
`f16::write_from_with_presence`). `evaluate_document` rules out a clause none of whose root kinds the document's
presence holds (`TapeDoc::may_have_kind`; an error root is always possible) before its anchor index is built:
`a_clause_whose_root_kinds_the_presence_lacks_does_not_run`.

A document where no clause runs builds no anchor index (`FileAnchorIndex::build` walks every node whatever its
plans), so a document the presence rules out entirely is not read past its parse.

**Done (2026-11-18): part 2 on the device, by file.** A label pass's roots over codes run in a file only where its
presence may hold one of the domain's kinds and each `KIND` entry test's kind (`matcher_metal::may_have_roots`, into
the `active` bits `Roots::runs_in` reads), and so do a wave's clause roots:
`metal_passes_skip_the_files_their_presence_rules_out`; `lowered_clauses_find_the_same_roots_on_metal` still holds.

**Blocks: what is left, and why.** On the CPU a clause's roots come from the file's kind index (`memo::kind_index`,
the anchor index's postings), one walk that serves every kind of every clause; leaving out the blocks one clause's
kinds miss saves nothing there unless no clause needs the block. A block skip pays where a walk is for given kinds:
the device's sweeps, which would read each block's presence words before its 1,024 nodes. The kernels read only
what lies in the residency slabs, so the block presence must be laid there beside the tape, not uploaded from the
image's `Vec` (0353); that is part 3's layout, so the device's block skip comes with it.

**Done (2026-11-18): part 3, the store keeps it.** 0456's branch is merged here (75078a7e7). The presence is an entry
of a kept-facts space of its own (`content_store::kept_facts::presence`), keyed by the tape's write-back key
(`core::tape_key`: its source's digest, language, grammar and format versions), its rows the words in their order
(the document's, then each block's), one column kept as given. A wave's presences are gathered and written as one
file of the chain, behind the run, with the tapes; a read of a queued tape's presence meanwhile gets the writer's own.
A corpus run maps the space once, before the first wave, beside the tapes it reads then, and a tape read from the
store is given `TapePresence::Kept`, which reads its words by position (`Packed::get`) in the mapped file: nothing is
decoded and no `Presence` is made of it (`a_stored_tape_is_given_the_presence_its_writer_made`,
`a_runs_tapes_are_kept_with_their_presences`). `placed_parse` had dropped the parse's presence when it made the image
again from its room: carried.

The workspace keeps a published file's presence under the same key and selects a stored file by it before the search
index (`persisted::presence_selects`): every candidate clause none of whose root kinds the tape has is ruled out, with
no index built and no tape decoded. A tape with an error node rules out none, as the index's syntax errors do not: an
error entry's kind is `ERROR_CODE` (0, tree-sitter's end symbol, never a node's), so its bit says the tape has one.
Where the presence cannot tell (none kept, a clause over a derived document, project evidence, no root kinds) the
index is asked as before (`kept_presences_skip_only_error_free_tapes_that_cannot_match`).

**Withdrawn (2026-11-19): the host's skip bits for the device.** 82a7c73a5 had the host test each block's presence
against each rule's kinds and write a bit a group into the root table, and part 2 had it do the same per file (the
`active` bits, `may_have_roots`). Both are host work for the kernels, and a structure only the kernels read. The
user: "no host work for Metal kernels and now separate data structures for Metal kernels. Host and metal must read
exactly the same data and start from the same point." Both are gone; the tests that lock the device's skips
(`metal_passes_skip_the_files_their_presence_rules_out`, `..._the_blocks_...`) are ignored until the kernels read
the presence themselves.

**The shape (2026-11-19).** The presence has one home that both the host and the device read in place, through one
read written once in CLIF, from the same view (`TapeView`) as the tape: the host's planners (`may_have_kind`, the
workspace's selector, the CPU's root walks, which skip the blocks too) and the kernels (`root_mask_in`, the rule's
kinds a mask over the presence's words) read the same words. No host-made answer is handed to the device.

**Built (2026-11-19).** The home, as stored: a format 16 or 16z tape's stored bytes are its home (`emit` gives the
extent), so the presence is in the same frame as the tape, placed, admitted and loaded with it; nothing else holds
it (the kept-facts entry of part 3 went). Format 16 and 16z are both stored as they are (the store had wrapped format
16 in zstd at rest), so the presence is read the same way in both, in the stored frame. Since 2026-11-19 no tape of
any format is compressed at rest: each is in a segment as it is (0295 step 5, floor 7). `F16_LAYOUT` 5. The read: `tape_has_kind(block, kind)` (CLIF; block
`NONE`, or one past those held, reads the document's; a tape without a presence answers 1). Its readers: the host's
`Tape::has_kind` (`may_have_kind`, `block_may_have_kind`, the selector's `StoredPresence` over an unloaded frame,
`Roots::may_in` for the CPU's root walk, which then reads no code of a block left out) and the kernels'
`rule_may_in`, which `root_mask_in` asks once a group of 64 nodes before reading any code. An ERROR node's kind is
`ERROR_CODE` in the presence (`presence_kind`, on both). The tests: `ops_write_the_tape` (every block and kind, 16
and 16z, against the writer's own), `a_stored_tape_holds_the_presence_its_writer_made`,
`metal_passes_skip_the_blocks_their_presence_rules_out` (a presence made to lie about one block: the device and
the host both leave that block's roots out).

## Requirements, each with its test

| | Requirement | Test |
|---|---|---|
| R1 | Block-major tapes read as the tape they were written from, node by node: every read, every format | `f16::tests` (the existing round trips, over the new layout), `tape_reads_*` |
| R2 | A one-block tape is the region layout: no part starts ahead, no padding | `one_block_is_the_regions` |
| R3 | A tape of n nodes is `header + (B−1)·S + last block + 8 + tail` bytes: no padding in the last block | `the_last_block_is_compact` |
| R4 | The writer writes each cell at its final place: at `finish` it moves the last block's columns down and appends the tail, and nothing else is copied | `finish_moves_only_the_last_block` |
| R5 | The ops (`open`, `close`, `leaf`) give the tape `write` gives from the same nodes, in 16 and 16z | `ops_write_the_tape` |
| R6 | `retag` and `set_start` of a node the ops have, escaped values and block edges among them, give the tape of the nodes written right | `retag_and_set_start` |
| R7 | A lean writer's edits (a rewind, a wrap, a retag after a commit, a close of a node passed on open) give the tape format 15 gives | `a_parsers_writer_finishes_in_format_16_as_in_format_15`, `the_lean_parsers_write_each_format_directly` |
| R7a | A format 16 writer's columns hold the window, not the file; a read of a node given back or an edit before the window makes the parse run again for format 15; each lean parser with a table writes a file of a few hundred items in format 16 with no such fallback | `a_format_16_writers_columns_are_the_window`, `the_lean_parsers_write_each_format_directly` |
| R8 | 16z written by the ops is the 16 tape's blocks, each decoded in place alone | `sixteen` (the existing check), `a_block_decodes_alone` |
| R9 | A 16z block is compressed only when final: the root's block last; the pending blocks are those holding open ancestors | `blocks_finish_when_closed` |
| R10 | The presence is the kinds and fields the tape's nodes have, per document and (optionally) per block | `presence_is_the_tapes_kinds` |
| R11 | The device reads block-major tapes as the CPU does | `the_label_kernels_compile_for_every_format`, `on_metal_anchored_walks_find_the_unanchored_roots` |
| R12 | No format 15 column, image or tape is made for a format 16 run (lean, tier 2, tree-sitter) | `no_intermediate_tape` (step 3) |
| R13 | The CLIF ops write the reference's tape, in 16 and 16z, for any sequence of ops the reference takes (escapes, late closes, `retag`, `set_start`, repeats), on the Rust backend and the JIT | `the_clif_ops_write_the_references_tape` (step 4) |
| R14 | An op fails as the reference does; a full part fails as full, and nothing is written past its capacity | `the_clif_ops_fail_as_the_reference`, `a_full_writer_writes_nothing_past_its_parts` (step 4) |
| R15 | The ops on the device (MSL, UKL since roll-up 8) write the same tape | `the_device_writes_the_references_tape` (step 4) |
| R16 | The host's `Writer` is the generated ops; the hand-written one is the tests' oracle only | `the_writer_is_the_clif_ops` (step 4) |
| R17 | Tier 2's tape words call the ops, and its GPU path writes its tape on the device | `tier2_writes_format_16_by_the_ops`, `edits_of_nodes_passed_reach_the_ops` (CPU, done); the device waits on tier 2 on Metal |

## Steps

1. The layout, the reads in CLIF, the Rust reference writer's ops (`open`, `close`, `leaf`, `commit`, `finish`),
   the 16z sink and the presence (R1–R5, R8–R10). `write` becomes a caller of the ops.
2. The edits that reach the ops (`retag`, `set_start`) (R6); the lean `Writer` passes its committed nodes to the
   ops through its target (R7). Then its columns shrink to the window, and the parsers that do not commit yet
   commit at their statements (R7a).
3. Tree-sitter's cursor walk and derived documents call the ops (R12); then tier 2's words.
4. The ops in CLIF, three backends; tier 2's GPU path writes its tape on the device.
5. Node-range LZ4 for 16z (§5), after the lab measures it. Measured and not taken: HC 9 stays (§5).
6. The store keeps the presence; the planner leaves out the documents and blocks a program's anchor cannot touch.

## Measured

- **Step 1** (b5fbb9ed5). The engine's format tests and the device's read the block-major layout as before
  (R1, R11). The GPU's time on the label kernels, M3 Max, under load (28–60): 265k–282k µs, against 388k µs for the
  same run with the reads off and 480k–830k µs recorded earlier that day. That spread is the machine's sharing, so
  the new addressing costs nothing measurable.
- **Step 3, JSON.** simdjson's kernels give their events to the ops (`encode_16`); a file the strict read refuses is
  walked again as JSONC, as before. `no_intermediate_tape` holds for JSON, and its bytes are those format 15's
  image converts to (`the_lean_parsers_write_each_format_directly`). Tier 2's JSON parser is not the engine's
  (simdjson is); its words call the ops in step 4. The index writer (`tape_from_index`, for a CPU no kernel runs on)
  keeps format 15.
- **Step 2, second part.** The lean writer's columns are the window (R7a). A format 16 run's writer for a large
  file had columns for every node (two `u32`s and the size and tail cells, 12–16 bytes a node, from the guess
  `Writer::new` is given); it now has them for the nodes since the last commit, 4,096 cells at most for a usual
  file.
- **Step 3, first part.** The cursor walk, `omitting`, Blade and the format strings write through `walked`.
  `no_intermediate_tape` parses one file in each language with a format 16 table, through the engine's entry
  point, and finds no document made in format 15 first. The exceptions:
  - JSON's parsers: simdjson's kernels (`rerail_simdjson`, which write format 15's columns), and tier 2, until they
    call the ops;
  - the lean parsers whose languages have no table yet (C and C++, Go, Kotlin, Scala, Swift). They take a `Sink`
    when their tables come.
- **Step 2.** The edits are made in the caller's window, not in the tape (amended §2). The decision first had the
  ops move a block's cells for a wrap, re-encoding the siblings' slots and the checkpoints after it. But the lean
  parsers already make every edit in their own columns, and their commits mark where edits stop, so only a retag
  and an open node's start can reach a node the ops have.
