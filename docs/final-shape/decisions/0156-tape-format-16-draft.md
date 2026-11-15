# 0156: Tape format 16, events, blocks and a codec the GPU decodes (draft)

Status: draft (2026-11-13), a design pass for the user's review. Nothing is built. Revised the same
day after the user's answers: open choices are written as tradeoffs to measure, not as bars to meet
("Tradeoffs", below).
Replaces, if accepted: 0025 (v9np), 0140's writer, and parts of 0026 and 0032.
Survey: `notes/t35-tape-format-issues.md` (cited below as "t35 n").

## Fixed: the tape is read in place, never decoded (the user, 2026-11-14)

This is not a tradeoff. It holds for every candidate, and a design that breaks it is wrong by
construction.

- **No decoding, no transformations.** Every question a reader asks is answered on the in-memory
  tape's bytes, where they lie, as every simdjson operation runs on simdjson's tape. Nothing turns
  the tape into another form: no columns, trees, records, rows or copies of facts.
- **The questions** are kind, field, flags, named, supertype memberships, span (start and end),
  children, next sibling, subtree end, parent, line, text and `span_within`. Each needs a direct
  read, or a bounded walk, on the tape.
- **Compression in memory must be readable in place.** Examples: block-local bit widths read with
  a shift and a mask; a repeated subtree stored once and referenced; LZ whose references name
  *compressed* content, with an index (requirements 2 and 3). Entropy coding is not readable in
  place, so it is only for storage.
- **On disk, a general codec (zstd) is allowed.** It is not an in-memory format: loading inflates
  it to exactly the in-memory tape bytes. Balancing it is part of the design, but not most of it.
- **The measure** of a candidate is its in-memory bytes per node, and the in-place cost of each
  question, weighted by what rules actually ask (E7). At-rest bytes and load cost are a separate,
  smaller axis (E10).

The user's requirements always said this: simdjson's tape; compression the GPU reads; the same
structure with the same bits "without affecting the decodability"; LZ references into compressed
content "plus some kind of index structure" for random access; less data in memory. Sections 1.1
and 4 below, and the decode-speed experiments, drifted from it. Section 4 is withdrawn; 1.1's
deltas stay only where a question can still be answered in place (T9).

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
2^32 nodes. Files have no limit but the 2^48-byte placement (3.3). The user accepted the 2 GiB
limit (2026-11-13). If it ever binds, it can be lifted without changing the record: a longer gap is
written as a chain of gaps, and a longer leaf as a leaf with continuations (zero-width extra events
carrying the next 2 GiB).

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
shows format 15's limits would fail a grammar we may add: Haskell has 84 fields, past its 64.

On Rerail's vendored grammars (`native/grammars/*/grammar.json`, supertypes expanded through hidden
rules), plus tree-sitter-php 0.24.2 and tree-sitter-rust 0.24.2:

```text
grammar       fields   supertypes   most candidates for one kind
python          32        6           4  (identifier)
javascript      36        5           3
typescript      43        7           5  (member_expression)
php             31        5           3
rust            31        6           4
sql             61        0           0     61 fields: three short of format 15's limit of 64
vue              0        0           0
```

Every mask fits in 5 bits today, so 16 bits leave room. SQL's 61 fields mean the u8 field is needed
now, not only for grammars we might add. This counts from `grammar.json`; E2 checks it against the
parser's cursor on a corpus.

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
literal source, so it becomes several copies. And there is no entropy stage, so a literal costs a
whole byte; zstd's entropy stage is what this gives up to stay parallel and random-access. How much
that costs in bytes, against what it buys in decode speed and access, is the first tradeoff to
measure (T1 in "Tradeoffs").

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
  (t35 25). Node ordinals after it still shift; see T7.
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

## 4. Decoding and the resident form (withdrawn, 2026-11-14)

This section proposed decoding each block into a resident per-node form of about 26 B a node,
which every reader would use. That breaks the fixed requirement above: readers read the tape
itself. What it was for still holds, in the tape: parent, next sibling, span and line must be
answerable in place (T9), and validation and hashing happen once at admission.

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

**The text stream may replace the source** (the user, 2026-11-13). The source does not go away:
the repository has it, and git's hashes stay. But a loaded document need not hold it, so memory
shrinks. Named-only tapes are already 1.3–1.8× the zstd'd source (t28's table), so the comparison
(E5) is structure + pieces against tape + source, both at rest and resident. See T6 for what each
level costs to load.

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
- **Preorder ordinals as identity** (t35 25): they still shift after an edit (T7).
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

## Tradeoffs

None of these is decided. Each is a choice whose cost on one side and gain on the other have to be
measured before it is made. Each lists the options, what moves between them, and the experiment that
measures it.

### T1. The codec: how much compression to give for parallel decode and random access

```text
option                          bytes at rest   GPU decode            random access      sync between copies
literal-pool LZ (2)             most            fully parallel        one hop, K seqs    none
literal-pool LZ + rANS/plane    fewer           parallel by lanes     per rANS block     none
LZ4, references into output     fewer           threadgroup per blk   from block start   where they overlap
zstd + dictionary (today)       fewest          host only, today      whole frame        n/a
```

- **What moves:** bytes (store size, I/O, memory for frames that are kept compressed) against
  decode throughput on the device, the latency of one random read, and the encoder's cost.
- **What moves it:** the planes' zero runs and the dictionary's hit rate decide how close the
  literal-pool codec gets to zstd. An entropy stage recovers bytes but decodes in lanes, not one
  byte at a time.
- **Measured by:** E1 (bytes per node at rest, per language and option) and E3 (decode GB/s and the
  latency of one read, per option). Wanted: the curve, so the choice is made from it.

### T2. What the codec sees: byte planes, field planes or records

- **Byte planes** make zero high bytes one run each. But a repeated subtree becomes one copy per
  plane (18 copies).
- **Whole records (16–18 B)** make a repeated subtree one long copy. But their zero bytes are
  literals unless the codec has runs inside records.
- **Field planes** (one per field, at its width) sit between the two.
- **What moves:** bytes, and the sequence count, which is decode work.
- **Measured by:** E1, all three layouts, on the same blocks.

### T3. Block size and checkpoint interval

- **Larger blocks:** fewer placement entries, fewer parents found across blocks, and better
  compression within a block.
- **Smaller blocks:** finer edits and sharing (more identical blocks), more threadgroups to occupy
  the device, and a smaller resident unit.
- **The checkpoint interval K** trades index bytes (about 12 B per K sequences) against the work of
  one random read (up to K sequences).
- **Measured by:** E1 and E4, at block targets of 1K, 4K and 16K nodes, and K of 16, 64 and 256.

### T4. The dictionary: size and scope

- **Per grammar build** (shared across repositories) or **per repository** (trained on its own
  code).
- **Larger:** more copies hit it, but more resident memory. A dictionary resident on the device
  counts against the 1 GB, and every GPU invocation shares it.
- **Training cost and versioning:** a new dictionary is a new id, old blocks keep theirs, and the
  store keeps every dictionary any live block names.
- **Measured by:** E1, at 16, 64 and 256 KB, per grammar and per repository.

### T5. What is stored and what is derived

- **Stored per node:** the open and close records only.
- **Derived at decode:** position, depth, parent and size.
- **A stored parent delta** (rec6's, 1 B in 96.6% of nodes) would turn the parent pass into a
  subtraction: more bytes for less decode work. A stored size does the same for subtree skips.
- **The balance differs by reader:** the device derives with scans in parallel; the CPU with a
  stack; SQL climbs parents, and in the spike a climb was 1.6–3× faster than interval containment.
- **Measured by:** E3, derive against store, on CPU and Metal. E7 counts how often each reader asks
  for a parent.

### T6. Text: which level a reader loads

- **Structure only** needs no text.
- **Names** add the leaf pieces.
- **Source** adds the gaps too.
- **What moves:** memory and I/O per loaded document against the work of rebuilding text for a text
  test. A regex over source text needs the source level, or the text tests move to leaf pieces,
  where the value index already is.
- **Piece granularity:** a gap as one piece, or split into its implied part and its whitespace. A
  split makes more pieces repeat, but costs more ids.
- **Lengths:** `lead` and `trail` duplicate the pieces' lengths when text is present. Keeping both
  lets structure load alone. Dropping the deltas when text is stored saves bytes, but ties positions
  to the piece table.
- **Measured by:** E5, bytes and load time per level, against tape + source as today.

### T7. Node identity

Today a node is its preorder ordinal in its file. Four ways to name a node:

| | Key | Survives an edit elsewhere | Navigation | Key width in rows | Shared across files |
|---|---|---|---|---|---|
| A | preorder ordinal | no: every later node shifts | arithmetic (`i + 1`, `i + size`) | u32, u64 past 2^32 nodes | no |
| B | (block, local index) | yes, outside the edited blocks | arithmetic within a block; through placement across blocks | block id interned to u32 + u16 | the block's own nodes, yes |
| C | content path (granules, definition ids, 0121) | yes, while the path holds | needs a map to positions | a hash or interned id | no |
| D | ordinal plus each edit's shift map | renumbered by the map | as A | as A | no |

- **A** is today's. It is dense and fastest to navigate, but a 9-line commit changed 929k input rows
  (t35 25), and 0121 built granules and definition ids on top to recover from that.
- **B** comes nearly free with content-defined blocks. Its costs:
  - a node next to an edit changes key even if it is unchanged, because its block was re-cut;
  - rows carry a wider key;
  - a fact that depends on context outside its block (a parent, a scope) is still per file.
  It also opens a choice: facts computed per block body could be shared by every file holding that
  block, where they depend only on the block.
- **C** is the most stable, but it is not a position, so every read through it goes through a map.
- **D** keeps A's keys and makes downstream maintenance apply each edit's shift, as DRed sessions
  apply deltas (0077). The shift is a range: nodes from i on move by k.
- **Constraint:** rows are budgeted at 4.5–8 B (the approved index target), and a key's width counts
  against it.
- **Measured by:** E4's history replay, rows changed per commit under A, B and D, and E8, the key
  width's effect on the index and fact stores.

### T8. Base relations: kept as rows, or read from the tape

Inside a run the engine already reads base relations off its decoded tape (`Facts`; `span_within` per
looked-up node, decision 0050). At its boundaries it copies them: a memo entry stores each document's
base rows, and the kept cross part stores its inputs' rows as frames, `span_within` made for every
node. The choice is whether those boundaries keep the tape and rebuild what a reader asks.

- **Rows:** ready in the engine's form, so a hit decodes nothing more. But they are many times the
  tape. `span_within` alone is 9–13 rows a node, and a probe is a search over a sorted column (or a
  second sort, for `span_within` by its outer side).
- **The tape:** one copy, and a reader's probes are array reads (children in CSR, a parent climb, a
  subtree interval). But every reader must decode it first. The engine has no rebuild at the
  boundaries yet: in spike A (memo entries without base rows), hits lacking their base rows became
  misses and were evaluated again, and the edit route got slower (0.54 → 1.43 s on `impl/sqlite`).
- **What it asks of format 16:** decoding a whole document must be cheap on the CPU, not just on the
  GPU. Children, subtree ends and lines must come out of that decode with no extra pass that costs
  much (T5). `line` must be answerable without the source, so the line table belongs in the tape
  (T6). And `span_within` is never stored, because the tree already holds it.
- **Measured by:** E9.

### T9. Each question, answered in place

Every question must be answered on the tape where it lies. The tradeoff is how each field is
stored: what it costs in bytes, what it costs to read in place, and whether a repeated subtree
keeps the same bits.

- **Subtree end and next sibling.** An absolute index of the node's end (simdjson) is a direct
  read, but differs in every copy of a subtree. A self-relative distance (the subtree's size) is
  the same in every copy and is just as direct.
- **Start and end.** Absolute positions are direct reads, but no two copies match. A delta from the
  event before needs a scan, which is a decode. Between them: a block base plus block-local
  offsets; an offset from the parent's start (a walk up); or a stored start every K nodes, with
  deltas between (a walk of at most K).
- **Parent.** None (walkers come down from the parent and know it), a self-relative back distance,
  or a back pointer in the close. The pack reads the tape's parent under 0.01 times a node, and the
  heaviest rule 1.3 times (E7); through the facts arrays, rules read it far more (E17, below).
- **Line.** A line table in the tape, searched; or a line delta per node, with the same choice as
  start.
- **Widths.** Fixed-width fields (simdjson's 8 B words), or block-local bit widths read with a shift
  and a mask (E8 gives the widths each field needs).
- **Repetition.** A repeated subtree stored once and referenced, or LZ references into earlier
  records with an index from node to location. A read through a reference must stay a direct read.
  Same bits, measured (E15, [t39](../notes/t39-tape-primitives.md) step 7): XF's per-block PFOR
  widths keep only 16–89% of repeated nodes' bits within a file; one width a language, with
  escapes that name nothing, keeps all of them, within and across files, for 2–3.5% more memory on
  code. JSON's leaf slots are the exception. With the leaf slot by sibling (E16, step 8) one width
  a language costs under 0.05 B a node, JSON included; a point parent costs 1.6–3.7× more. Start
  and end as a gap to the previous start, plus a length, with a start every 8 or 16 nodes in a
  side array, take 16–24% less memory and a third less on disk on code, at 2–4.5× a point span's
  cost. Weighed by today's engine (E17, step 9): rules read a span at a point 0.47 times a
  node at the median and 17.6 at the heaviest, a parent 0.58 and 13.7, mostly through the facts
  arrays. At K = 8 both forms add 0.4–1.7% to a rule's CPU, provided the engine reads in order
  through cursors (as points, 1.5–4.3%). **Proposed: the leaf slot by sibling and positions by gap,
  both with K = 8**: about 4.19 B a node for codes, slots and positions on data-layer Python,
  against XF's 4.60. On disk (E18, step 10), same bits take 18–46% off XF's zstd size, but only where
  they are also the same bytes: at byte widths matches cover 75–86% of the bytes, bit-packed
  41–78%. Codes bit-packed with slots, gaps and lengths at byte widths cost 4.63 B a node in memory
  and 2.04 on disk with a dictionary on Python (XF 4.59 and 3.37, format 15 8.63 and 2.34); LZ4 HC
  with a dictionary is within 7% of zstd there. The absolute checkpoints do not compress (0.25 B a
  node at K = 8). Codes fit in 8 bits for every language measured (E20, E21, t39 steps 12–13):
  each language numbers its codes by frequency, and a byte below its cutoff (255) is the code
  itself, with no table; a code past it is the prefix 255 with its low byte in the block's side
  list (0.36–1.12% of nodes on files the numbering never saw). A language with 256 codes or fewer
  has no cutoff. Every repeat keeps its bytes. The numbering is fixed a language and format
  version. With them the all-byte form takes about 4.57 B a node in memory and 1.88 on disk on
  Python. (E20's overloading by the previous byte, through a 128 KiB table a language, took fewer
  escapes but sent 1–3% of reads to the table, for no faster reads.) The field stays in the code
  (E22, step 14): without it the base fits in 8 bits, but the field then needs the parent and the
  child's index, a table of 5–20 KiB, and 3–6× the read, for 7–12% less disk.
- **At rest, with random access.** Fixed blocks of N nodes, each a 4-byte index entry naming a
  repeat (in the file, or a pool), raw bytes, or one LZ4 block over the language's dictionary (E19,
  step 11): one decode a read, 1–5% over whole arrays at N = 4,096, 4–11% at 1,024, 14–23% at 256
  on code; 0.2–0.7 µs a 1,024-node block on the M3. A reference to one earlier block of the file
  takes back a quarter of the overhead for a second decode. Fixed blocks repeat only on JSON
  (2–15%).
- **Text.** A leaf's span into the source, or its id in the names stream (6). Comparison works on
  bytes, or on stream ids once a constant has been found in the pool.
- **Measured by:** E10.
- **Design pass:** [t38](../notes/t38-in-memory-tape-design.md) sets the families (explicit,
  bit-packed as in mldb's mapped int tables, balanced parentheses, DFUDS, LOUDS) against the
  questions and platforms, and names E10's candidates.

## Experiments

Each reports a curve over its options, not a pass or fail. Each runs on captured corpora
(data-layer, CPython, the rerail repository), scaled 1 → 10 → 50 files → full.

- **E1, bytes** (T1–T4). Bytes a node at rest per language, for codec × layout × block size ×
  dictionary. With format 15 + zstd + its dictionary as the reference line.
- **E2, codes.** Supertype masks, fields and kinds, checked against the parser's cursor on a corpus.
  The grammar counts are in 1.3.
- **E3, decode** (T1, T5). On Metal and on the CPU: throughput, the latency of one random read, and
  the parent pass derived against stored. Against today's host zstd + restore (0.7 ms a document).
- **E4, edits** (T3, T7). Replay the data-layer repository's history: blocks re-encoded per commit,
  blocks shared, and rows changed downstream under each identity.
- **E5, text** (T6). Bytes and load time for each level, against tape + zstd'd source; the share of
  gaps the dictionary covers.
- **E6, writer.** The lean parser through the event writer, against 425.6M instructions (doc 35), and
  the tree-sitter path against `pack`.
- **E7, readers' needs** (T5). How often each reader asks for a parent, a size, a line, a text.
- **E9, base relations** (T8). The engine's base relations (`node`, `child`, `field`, `line`,
  `span_within`) as the store encodes them, against the tape. Measures bytes, the cost to rebuild
  each side's probe form, and ns a probe.
- **E10, in place** (T9, and T1–T4 for storage). Each candidate tape is built in memory, and every
  question is answered on its bytes with nothing built beside it. An index counts only if it is
  part of the tape, and its bytes are counted. Reported per candidate and corpus:
  - in-memory bytes per node;
  - each question's cost on the CPU (ns and instructions), and cache lines touched, every answer
    checked against the tree;
  - the same cost weighted by E7's question mix, for the median, 90th-percentile and heaviest rule;
  - same bits: the share of repeated subtrees, and of CDC blocks, whose bytes are identical;
  - on disk: bytes per node under zstd (with and without a dictionary), and the load cost to inflate
    to exactly the in-memory bytes;
  - the writer's cost a node (E6).
  On Metal, the same questions as kernels, later.
- **E8, key width** (T7). Index and fact store bytes a row under each identity, against the
  4.5–8 B/row target.

E1 and E2 need only a CPU reference encoder over existing tapes, so they come first. They also
decide the layouts E3 should try on the device.

**Measured so far** (2026-11-13). The lab in `native/lab/tape16` runs E1, E3, E4, E5 and E8, and the
T3–T5 sweeps. Its README says how to rerun them and add a candidate. The first numbers are in
[t36](../notes/t36-tape16-lab-results.md):

- events under zstd take 0.45–0.57× format 15 as stored;
- literal-pool LZ decodes at 4.6–5.4 GB/s on Metal, and costs 2.4× format 15 at rest without
  replays, 1.4× with them;
- identity D (ordinal + shift map) changes 3% of rows per edit, against 71% for A and 16% for C.
- E9: the base rows the boundaries copy take 33–42 B/node, against 0.7–1.6 B/node for the tape. Rebuilt
  from the tape on one CPU thread, they cost 33–37 ns a node, against 312–498 ns to decode and index
  the rows. Probes on the tape's columns take 0.5–12 ns, against 35–77 ns on the rows.
  The engine's own lookups do no better: key prints take 47–106 ns a probe, the old hash table
  23–84 ns.
- E9, text: `text` and `token` as rows with their atoms take 7–23 B/node (every text within 256 B:
  15–19). The tape's names stream holds the text in 1.0–1.4 B/node and decodes at about 2 ns a leaf.
  Atoms win only for comparisons (0.55 ns against 2.2 for bytes). So text is read from the tape and
  interned only where a join needs it.
- E10 (2026-11-14, [t38](../notes/t38-in-memory-tape-design.md)): read in place, format 15 takes
  8.7–9.2 B a node. The bit-packed explicit tape with folded structure and predicted positions (MF)
  takes 3.4–4.0, and the balanced-parentheses tape with Elias–Fano positions (BE) 2.4–2.9. MF costs
  about X's time a question, and BE 2–3.5× that. Loading from the events (shape 2) costs 27–48 ns a
  node from 0.7–1.6 B a node of disk. Loading the tape's own bytes under zstd (shape 1) costs 1.3–4.2
  ns a node from 1.4–4× that disk.
- E9, partial decode (withdrawn as a conclusion, 2026-11-14): a single node's read through span LZ
  plus Huffman costs 5.8–15 µs, against 78–89 ns a node to decode the whole document. Both decode,
  so both break the fixed requirement. What it shows is that span LZ under entropy coding is not
  readable in place. It stays a storage candidate, not an in-memory one.

## Answered by the user (2026-11-13)

- **Size:** the choice is made from measured tradeoffs, not from a bar set before measuring (T1–T4).
- **Sources:** the text stream may replace the source in what is loaded. The source does not go
  away, but less data in memory is the aim (6).
- **Node identity:** to be explored (T7).
- **Limits:** a 2 GiB gap or leaf is fair. It can be lifted with chained gaps and leaf continuations
  if ever needed (1.1).
