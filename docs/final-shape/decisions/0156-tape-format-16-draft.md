# 0156: Tape format 16, events, blocks and a codec the GPU decodes (draft)

Status: draft (2026-11-13), a design pass for the user's review. Nothing is built.
Replaces, if accepted: 0025 (v9np), 0140's writer, and parts of 0026 and 0032.
Survey: `notes/t35-tape-format-issues.md` (cited below as "t35 n").

## Intro

**What a tape is for.** A tape is the engine's copy of one file's parse tree: each named node's
kind, field, flags, supertypes and span. Rules, facts, SQL and the GPU all read it.

**Why change it now.** Format 15 is not deployed anywhere. The t35 survey found that its shape
forces most of what writers and readers then work around:
- Nothing is final until the file ends: column places, widths, the tail rule and the code numbers.
  So writers buffer whole files, re-parse and copy, and nothing can stream (t35 1–5, 14, 19).
- Codes are numbered per tape, so the same node has different bits in different files (t35 6–9).
- Positions are absolute, so an edit changes everything after it (t35 25).
- Stored tapes are whole zstd frames. The random access the format was chosen for is unreachable at
  rest, and the GPU cannot decode them (t35 29–30).

The user added five requirements (t35 U1–U5, as amended on 2026-11-13):
1. **GPU decode.** Compression the GPU decodes, and that compresses reasonably. Dictionaries are
   fine: one can be shared by every GPU invocation.
2. **Random access while compressed.** Possibly an LZ4 variant whose references point into
   *compressed* content, plus an index.
3. **Same structure, same bits**, without hurting decoding.
4. **A second hash** when SHA-256 is deprecated, recorded without wasting bits.
5. **Loading without the full source**, through a compact side form that rebuilds the text that
   is not implicit. And **files up to 256 TB** (2^48 bytes).

**The proposal in one paragraph.** A tape becomes a stream of events: each node opens and closes,
and every number is relative to the event before it, so a repeated subtree is repeated bytes. The
stream is cut into blocks at content-defined boundaries; a block knows nothing of where it sits, so
an edit changes only the blocks around it and identical blocks are stored once. Each block is
compressed with a literal-pool LZ, in which every reference points into the compressed block's own
literals or into a shared dictionary, never into decoded output. So any byte decodes in one hop, a
GPU decodes with no synchronisation between copies, and a small index gives random access inside a
compressed block. Absolute positions, depths, parents and sizes are recomputed at decode by prefix
scans. Text goes in a parallel stream of piece ids that rebuilds the source exactly where it is
asked for. Hashes live in the store per algorithm, and over blocks as a Merkle tree.

## 1. The node stream

### 1.1 Events, not columns of absolutes

Every node has an *open* and a *close* event. In nesting order (preorder opens, postorder closes,
interleaved as the tree nests), each event's position is stored as its distance from the event
before it. Two streams, each with exactly one entry per node:

```text
open stream, in preorder, one record per node          close stream, in postorder, one per node
  kind   u16   grammar's kind id                          trail  u32   end − position of the event
  field  u8    grammar's field id (0: none)                            before this close
  flags  u8    extra, error, missing                                   (bit 31: measured backwards)
  memb   u16   supertype memberships, a mask (1.3)
  lead   u32   start − position of the event before
  pops   u32   closes between the previous open and this one
```

A worked example, `f(a, b)\n` in Python, named nodes only (format 14's rule is unchanged):

```text
event                    at   delta   record
open  module              0    0      #0 module            lead 0  pops 0
open  expression_stmt     0    0      #1 expression_stmt   lead 0  pops 0
open  call                0    0      #2 call              lead 0  pops 0
open  identifier f        0    0      #3 identifier/func   lead 0  pops 0
close identifier f        1    1                                          trail 1   "f"
open  argument_list       1    0      #4 argument_list     lead 0  pops 1
open  identifier a        2    1      #5 identifier        lead 1  pops 0         "("
close identifier a        3    1                                          trail 1   "a"
open  identifier b        5    2      #6 identifier        lead 2  pops 1         ", "
close identifier b        6    1                                          trail 1   "b"
close argument_list       7    1                                          trail 1   ")"
close call                7    0                                          trail 0
close expression_stmt     7    0                                          trail 0
close module              8    1                                          trail 1   "\n"
```

`g(x, y)` anywhere else, in any file, writes the same 7 open records and 7 close records, bit for
bit (requirement 3). Every delta is the length of one gap or one leaf, never a distance across the
file, so values stay small (the spike: v7's delta varints compress well, absolute positions badly;
`tape-v8-measurements.md` Recommendation 1).

**Fixed widths, always.** Every field has one width in every tape: no per-document widths, no
`Redo`, no width branches in readers (t35 14–16). The bytes that are nearly always zero cost almost
nothing once compressed (2). Limits: one gap or leaf under 2 GiB (bit 31 of `trail`); a block under
2^32 nodes. Files have no limit but the 2^48-byte placement (3.3).

**What is not stored, and why that is now cheap.** Size, parent, depth, absolute start and end are
all derived at decode by scans (4). The writer never patches a cell it already wrote: a node's size
was the reason it had to (t35 3).

**Wraps, late fields, take-backs.** A writer keeps an uncommitted window, as the lean writer's
commit points do now. Records are fixed-width and relative, so:
- A wrap (an operator found after its left operand) inserts one record. The operand's `lead` and
  `pops` move to the wrapper, and the operand's become 0. Nothing after the window changes: no
  renumbering, no rebased blocks, no marks (t35 4, 8, 20, 21).
- A late field or kind rewrites the one record in place.

### 1.2 Early ends

Where tree-sitter gives a node an end before its last descendant's start, its close is measured
backwards: bit 31 of `trail`. That node alone carries it. There is no document-wide
`TAIL_FROM_START` and no re-parse (t35 19, 27).

### 1.3 Codes are the grammar's

There is no per-tape code table. `kind` and `field` are the grammar's own ids, the same in every
tape, and `memb` is a bitmask over the supertypes the grammar lists for that kind, in ascending
order. A node's actual memberships depend on where it sits (decision 0025's finding still holds),
but they are always a subset of that list, which comes from `node-types.json` and is fixed for the
grammar build. So the mask is exact, and needs no learned table, catalog or per-tape entry.

Measured here on the 31 grammars in the cargo registry: at most 11 supertypes can contain a kind
(Ruby's `constant`; Python 4, Rust 4, JavaScript 3), so 16 bits hold every mask. The same count
shows format 15's limits would fail a grammar we may add: Haskell has 84 fields, past its 64. Rerail's vendored,
patched grammars still need the same count (experiment E2).

The removals:
- **Code ids:** no first-use numbering, no 1 MiB id map, no renumbering, no `DeviceCode` rebuild per
  admission (t35 6–9, 19 in section 8).
- **Fixed limits:** kind is u16 and field u8, so the 1,024-kind and 64-field limits go (t35 12).
- **Alias table:** contextual aliases become kind ids in the grammar's alias range. Tree-sitter
  gives an aliased node its alias symbol's id, and the spike's "kind aliases assigned codes once per
  grammar at registration" is the same idea (`tape-v8-measurements.md` Part 1). The per-tape alias
  string table goes (t35 52).
- **Extra:** the varying `extra` of one Python kind is just its flag bit (t35 13).

Kind ids stay per language. Global ids (language × 2^16 + kind) stay in indexes only, as the
spike's Option B and decision 0055 say.

## 2. The codec: literal-pool LZ

A block's records are split into byte planes. Byte k of every record's field f forms one plane, so
the high bytes of `lead`, `pops` and `trail` are long runs of zeros. The planes, concatenated, are
compressed as one stream:

```text
block   header   format, dictionary id, planes, node count, decoded length, checkpoint count
        seqs     sequences: (literal run length, then a copy or a run)
        lits     the block's literal bytes, contiguous
        index    a checkpoint every K sequences: output offset, literal offset, sequence offset
```

A sequence is a literal run (bytes taken in order from `lits`), then either:
- a **copy**: n bytes from a *pool offset*, where the pool is the shared dictionary followed by this
  block's `lits`; or
- a **run**: one byte repeated n times. A zero plane is one run.

The rule that makes it decodable on the GPU and at random: **a copy never reads decoded output.**
It reads literal bytes, which sit in the compressed block or in the dictionary. So:
- **Any output byte is one hop away.** Find its sequence through `index` (a binary search), decode at
  most K sequences, copy from the pool. That is random access inside a compressed block
  (requirement 2, as the user sketched it).
- **There are no dependencies between copies.** The user's `v8-decompress-on-gpu.cc` needs a
  `__syncthreads()` wherever a back-reference overlaps its own output (offset < length). Here no copy
  depends on another. Each thread takes K sequences from a checkpoint; output and literal offsets
  come from a prefix sum over the threadgroup; then every copy runs in parallel.
- **The dictionary is shared.** One dictionary per (grammar build, format), stored once, content
  addressed, resident on the device for every invocation (requirement 1, user's note). A dictionary
  is never replaced in place: a new one has a new id, and old blocks keep naming theirs.
- **Repeats become references.** Requirement 3 makes a repeated subtree identical bytes in each
  plane, and the encoder references the first occurrence's literals or the dictionary's.

The cost of the rule: a repeat whose first occurrence was itself partly a copy has no contiguous
literal source, so it becomes several copies. There is no entropy stage, so literals cost a byte
each; zstd's entropy stage is what this gives up to be parallel and random-access. Whether the
result meets "no size increase" is experiment E1, the first gate. If it does not, the fallback is an
entropy stage that GPUs decode in parallel (interleaved rANS per plane), which gives up some random
access.

A block may also be stored raw (no codec). Then every field is fixed-width at a known offset, so
pure SQL can read it with `substr` arithmetic, as the spike's fallback did. That needed uncompressed
blobs and was 43× slower, so it stays a fallback (`ast-index-spike-history-index.md`, binary tape +
TVF row).

## 3. Blocks and placement

### 3.1 Content-defined block boundaries

The writer cuts a block after a committed node when a rolling hash over the last w open records
meets a condition, with a minimum and a maximum count (first guess: 1,024 to 16,384 nodes, aiming at
4,096, to be measured). The cuts depend only on nearby content:
- **Edits change only nearby blocks.** After an edit, the cuts realign within a block or two, and
  every other block is byte-identical. A one-line edit re-encodes one or two blocks, not the file
  (t35 25). Node ordinals after it still shift; see question Q3.
- **The store keeps each block once**, by its body hash: across revisions, vendored copies and
  generated files.

A block body holds no absolute value: deltas are local, and codes are the grammar's. A close whose
node opened in an earlier block lives in the block where its event falls. So the body's bytes depend
only on that span of the tree and its text.

### 3.2 Placement: the index structure

The tape object is its placement table: one fixed-size entry per block.

```text
entry   body id          the block's content id (5)
        first node       u64 ordinal of its first open
        base             u64 position of its first event (48 bits used)
        entry depth      u32 depth before its first open
        min depth        u32 least depth inside it
```

- **Absolute position** = the block's base + a scan inside the block.
- **Depth** = entry depth + a scan of (1 − pops).
- **A parent in an earlier block** is found from the depths: the nearest earlier block whose min
  depth is below the node's depth, found by a range-minimum search over the entries. This is the
  range min-max tree of succinct balanced-parentheses trees (Navarro and Sadakane): the open/close
  streams are balanced parentheses with labels.

A file of one block, which is most files (152k of 179k tapes are under 512 B, t28), is stored as the
block alone: no placement row, no side table. The spike rejected chunking the tape into a side table
(+0.41 B/node, D S067). That holds for small files, which stay whole. Large files must be chunked
anyway (3.3), and repeats and edits gain from it. This is a departure from the spike, and it is
justified only above one block.

### 3.3 Large files

Positions and ordinals at block level are u64 (2^48 bytes = 256 TB is the stated bound). Inside a
block everything is u32, block-relative, so per-node decoded records stay 32-bit. A cross-block
reference is (block, local) or a u64 ordinal. A stored block is far below SQLite's 2^31−1-byte blob
limit (1 GB by default), which no single-frame format could meet for such files (t35 U5).

## 4. Decoding and the resident form

A decode turns a block into the one resident per-node form that every reader uses:
- the codec (2);
- opens' event indexes from a prefix sum of `pops`, with closes filling the gaps in postorder;
- absolute positions from an inclusive scan of deltas in event order, plus the block's base;
- depth from a scan of (1 − pops);
- parent and size by an all-nearest-smaller-values pass over depth (parallel on the device, a stack
  on the CPU);
- line numbers from the text stream's newline counts (6), with no source scan.

```text
resident record (columns, 16-byte aligned, block-relative u32)
  kind u16, field u8, flags u8, memb u16, start u32, end u32, parent u32, size u32, line u32
```

That is about 26 B a node, in one place. Today a read builds `Rec` (40 B), `Facts` (about 39 B),
`MetalNode` (44 B) and the anchor index's entries (about 100 B), each per phase (t35 35–36). Format
16 cannot by itself stop the engine making copies. What it does is make the decoded block complete
enough that none is needed: parent, next sibling (i + size), span and line are there. Decoding happens
once at admission, with validation and hashing then too (t35 38–40). That is the spec's "once per
admission" rule (`rule-engine-direction/spec.md:474`), made the only path.

On the CPU the parent pass is the stack that already exists (4.1–5.1 ns a node). Storing a parent
delta, as the spike's rec6 did (1 B in 96.6% of nodes), is left to E3: format 16 derives it, and
adds a stored plane only if the derivation measures as a cost.

## 5. Hashes and identity

The tape holds no digest (as since format 12). Identity lives in the store:
- **A block's id** is a hash of its body. **A tape's id** is a hash of its grammar build, its format
  and its placement table, and so a Merkle root over its blocks. A reader of one block verifies that
  block alone, which a 256 TB file needs.
- **Hashes are kept per algorithm.** The store records an object's digests as (algorithm, bytes).
  Today a hash is a fixed four 64-bit lanes (`content_store/schema.rs`), and that becomes a
  variable-length digest, so SHA-384, SHA-512 or a successor fits.
- **A second algorithm is added without re-encoding.** It is computed as objects are read (they are
  decoded anyway; BLAKE3 hashes in parallel on the device) and recorded beside the first. When every
  live object has it, the first can be retired. The tape's bits never change (requirement 4: no
  wasted bits, because nothing in the format holds a hash).
- **In the engine,** `[u8; 32]` and `tape_identity`'s SHA-256 become an algorithm-tagged content id
  (83 files hold `[u8; 32]` today, t35 U3).
- **Git's own hashes** (SHA-1, SHA-256) stay as git defines them.

## 6. Text: a stream of pieces

Each event's delta covers one *piece* of the source: a leaf's text (`trail` of a leaf), or a gap
(`lead`, or `trail` of an inner node). A gap is whitespace plus the tokens the grammar implies:
`(`, `, `, `)\n    `. The text stream holds one piece id per event, in event order:

```text
piece table   per id: length, newline count, the bytes (in the block's pool or the dictionary's)
text stream   per event: a piece id                         compressed as in 2, in its own planes
```

There are three levels, each loading without the source:
- **Structure:** the node stream alone. Spans, shapes, kinds.
- **Names:** plus the leaf pieces. Identifier and literal text: what most text tests read. The leaf
  pieces are the deduplicated leaf vocabulary the spike's value index requires
  (`ast-index-spike-history-index.md` §11), so a leaf piece id can be that index's value id.
- **Source:** plus the gap pieces. The exact bytes: CRLF, BOM and invalid UTF-8 alike, since pieces
  are bytes. Hashing them gives the source's hash.

This does not conflict with formats 14 and 15. No token is rebuilt from the grammar, and the tape
still has no node for an implied token. A gap piece is the literal text of the gap, and rules still
cannot read the token as a node (t31). Gaps repeat heavily, and the dictionary holds the common ones.

Whether the store then keeps sources at all is question Q2. Named-only tapes are already 1.3–1.8×
the zstd'd source (t28's table), so the comparison (E5) is structure + pieces against tape + source.

## 7. Writers

One writer, in the format crate, with the same calls everywhere:

```text
open(kind, field, flags, memb, start)   append an open record: lead and pops from the last event
close(end)                              append a close record: trail from the last event
commit()                                the window from here back is final; a block may be cut
```

There is no `finish` that moves columns, no header written last (the placement table is appended),
no code table, no width guess and no re-parse. The work a node is what decision 0140 measured as
the floor, keys and starts only (354.2M instructions on the `.rr` corpus against 425.6M with
`Narrow`, doc 35), plus splitting planes at a block cut. E6 measures it.

- **The tree-sitter path** writes events from its cursor walk. `Pending`, `pack`, the size fold and
  the code fold go (t35 43). The per-node cursor status call stays, for the membership mask.
- **On the GPU,** a writer needs no serial step. Each thread writes fixed-width records at an
  index from a prefix sum; `pops` is a difference of depths; and block cuts are candidates from a
  parallel rolling hash, chosen with a scan under the min/max spacing. The format no longer blocks a
  GPU tape writer (t35 50).
- **One definition.** simdjson, the lean parser and tier 2's Cranelift, AOT and Metal backends call
  or generate from the crate's constants. There are no copies (t35 46–47).

## 8. SQL

Tapes are read through native table functions that take a `sqlite3_blob` handle, never a value (the
spike, D S062–S064), and decode only the blocks a query touches: no whole-frame decode and no
8-entry LRU of whole tapes (t35 30). A raw block (2) can be read by pure SQL.

## What this removes, and what it does not

Removed or made unnecessary, by t35 number:
- **1–5:** whole-file buffering, the moves at the end, back-patching and quadratic wraps.
- **6–9 and 11–13:** per-tape numbering and code tables, the 1,024 and 64 limits, the 8-membership
  limit.
- **14–21:** widths, re-parses, width branches, `TAIL_FROM_START`, the marks slow path, and ends
  read through the last descendant.
- **22:** line tables built by scanning the source.
- **25, partly:** edits touch one or two blocks.
- **29–31, 33:** random access at rest, small tapes' fixed overhead, and identity over a whole
  file's bytes.
- **43–50:** intermediate forms in writers, copies after writing, four implementations, and the
  obstacles to a GPU writer.
- **52:** the alias table.

Not removed by a format:
- **Sections 8 and 9 of t35** (conversions per phase, repeated hashing and validation, copies) are
  the code's. Format 16 gives them one complete resident form to converge on.
- **Preorder ordinals as identity** (t35 25): they still shift after an edit (Q3).
- **The named-only rule's effects on rule meaning** (t35 28) are untouched.

## Departures

- **From 0025:**
  - per-tape folded codes give way to grammar ids plus a mask;
  - v9np's base/off/tail columns give way to events;
  - per-document widths go.
- **From 0140:** the writer's columns-at-their-place design goes.
- **From 0026:** a single zstd frame per tape gives way to blocks under the literal-pool codec.
- **From the spike:**
  - chunking, only above one block (3.2);
  - `kf` widened past 10 + 6 bits (1.3).
- **Kept from the spike:**
  - one per-file structure, no row per node (DI Top 1);
  - preorder records with derived navigation;
  - native table functions by address;
  - kind codes local, global in indexes.
- **Kept from 0026 and 0032:**
  - nothing else stored per node;
  - at-rest and resident forms designed together, with 16-byte aligned resident columns.

## Experiments, in order

Each runs on captured corpora (data-layer, CPython, the rerail repository), scaled 1 → 10 → 50
files → full.

- **E1, size (the gate).** Format 16 + literal-pool LZ + shared dictionary, against format 15 + zstd
  with its dictionary, at rest, per language. Also format 16 under plain zstd and plain LZ4, to
  separate layout from codec. Bar: the stated "no size increase", or the measured ratio put to the
  user (Q1).
- **E2, codes.** On Rerail's vendored, patched grammars: supertypes per kind (16 bits?), fields
  (u8?), kinds (u16), and the alias ids.
- **E3, decode.** On Metal: GB/s and ns a node for codec + scans + the parent pass, cold and warm,
  against today's host zstd + restore (0.7 ms a document). On the CPU: the same, against
  `restore_tree`.
- **E4, edits.** Blocks changed per commit over the data-layer repository's history, against whole
  files today, and how many blocks the store shares.
- **E5, text.** Structure + pieces + dictionary, against tape + zstd'd source; and names-only load
  size.
- **E6, writer.** The lean parser through the event writer, against 425.6M instructions (doc 35).

## Questions for the user

- **Q1.** "Compresses reasonably": does the "no size increase" bar still stand for a codec chosen
  for GPU decode and random access, or is a measured ratio acceptable?
- **Q2.** If the source level of the text stream is exact, may it replace stored sources, keeping
  only git's hashes for the source? Or should both be kept?
- **Q3.** Should node identity stay the preorder ordinal, or become (block id, local index), so that
  a node's key survives edits elsewhere in the file? That changes keys downstream (granules, facts,
  memos).
- **Q4.** Is "no single gap or leaf of 2 GiB or more" acceptable as the one per-node limit?
