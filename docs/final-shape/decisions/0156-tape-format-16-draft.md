# 0156: Tape format 16, a byte tape read in place, in fixed blocks at rest (draft)

Status: the run's default format since 2026-11-17 (0340, the user: "Switch to format 16"); the tradeoffs below
are still open. Draft (2026-11-13), a design pass for the user's review. Nothing is built. Revised the same
day after the user's answers: open choices are written as tradeoffs to measure, not as bars to meet
("Tradeoffs", below).
Revised 2026-11-15: sections 1–4 rewritten as the form the lab measured
([t39](../notes/t39-tape-primitives.md) steps 7–16). The event stream, the literal-pool codec and
the content-defined blocks of the first version are in Departures.
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
content "plus some kind of index structure" for random access; less data in memory. The first
version's event stream and resident form, and the decode-speed experiments, drifted from it. They
are in Departures; sections 1–4 are now the form read in place.

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

**The proposal in one paragraph.** A tape is five arrays over a file's named nodes in preorder,
read where they lie (1):
- a byte a node for its code, numbered by frequency per language, so that below a cutoff the byte is
  the code;
- a slot: an inner node's size, or, for a leaf, the size of the next node's previous sibling;
- each start as a gap from the previous start, and each end as a length;
- an absolute start every 8 nodes.

A short side list per 256-node block holds the few values too wide for their array. Every width and
the numbering are fixed per language, so a writer streams. A repeated subtree is the same bytes in
its codes and slots, and in its gaps and lengths too where its text repeats. At rest each array is cut into fixed blocks of 1,024 nodes, each
raw or one LZ4 block over the language's dictionary, so one read decodes one block (2). Cold storage
may use zstd over whole arrays. Loading inflates to exactly the in-memory bytes. Text goes in a
parallel stream of piece ids that rebuilds the source exactly where it is asked for (6). Hashes live
in the store per algorithm (5).

## 1. The in-memory tape

Rewritten on 2026-11-15 from what the lab measured
([t39](../notes/t39-tape-primitives.md) steps 7–19). The event stream this section first proposed
is in Departures.

### 1.0 The version mark

Byte 0 of every tape, in memory and at rest, is its **version mark**: the language's tape revision.

- **One number covers every choice** a reader must share with the writer: the layout, the code table
  and its numbering, the widths, the checkpoint interval and block size, and the dictionaries. A
  change to any of them is a new revision of that language.
- **1–254 are revisions.** 0 is never written, so zeroed bytes are never taken for a tape. 255 says
  the revision goes on in the next two bytes (little-endian), for when 254 run out.
- **A reader reads only the revisions it knows.** Any other tape is rewritten before it is read:
  re-parsed from its source, or converted from its revision. The check is one byte a tape, made
  before its blocks are read or queued.
- **The mark is the tape's only header.** The language is the row's that holds the tape, and the
  node count is in the arrays' lengths.

Format 15's header (an 8-byte magic, the tag, a u32 version, the widths and counts) is replaced by
this byte. The revisions a build can read are listed with each language's code table. The lab marks
every tape with revision 1 and checks it before every load and every device dispatch
(`f16::revision`, E27).

### 1.1 Arrays, one entry a node

A tape is one file's named nodes in preorder, as five arrays read where they lie. Node i is entry i
of every array except the checkpoints, which have one entry for every 8 nodes:

```text
array        entry                                      width
codes        the code's number, or a prefix             1 byte
slots        size << 1, or a leaf's sibling slot        a language's width, in whole bytes (8 bits on
                                                        every language measured)
gaps         start − the previous node's start          a language's width (8 bits measured)
lengths      end − start                                a language's width (8 bits measured)
checkpoints  the absolute start of nodes 0, 8, 16, …     a file's width from its source length, in
                                                        whole bytes (24 bits for files under 16 MB)
```

Every array but the checkpoints has one width for every file of the language, fixed with the
language's code numbering (1.2) under the language's revision (1.0). A value too wide for
its array is stored as the all-ones value, an escape that names nothing (1.5).

So a writer knows every width before a file starts. A repeated subtree is the same bytes in its
codes and slots wherever it sits and in whatever file, and in its gaps and lengths too where its
text repeats (requirement 3; t39 steps 7, 8, 13). Only the checkpoints differ. The same bytes are
also what a compressor matches (2.1).

### 1.2 Codes: one byte, numbered by frequency

A **code** is a node's (kind, field, flags, supertype memberships). Kind and field are the
grammar's own ids. The memberships are a mask over the supertypes the grammar lists for the kind
(below). Each language has one **code table** for its grammar build and format version. There is
no per-tape table.

- **The table comes from the grammar.** It holds every code the grammar allows, not the codes a
  corpus showed, so no file can hold a code outside it (below, E25).
- **A corpus only orders it.** The codes a training corpus showed are numbered by frequency, the
  most frequent first. The rest follow in a fixed order of the grammar's.
- **Below the cutoff, the byte is the code.** A language with more than 256 codes has a cutoff of
  255: bytes 0–254 are the 255 most frequent codes, and byte 255 is the one prefix.
- **Past the cutoff**, a node's code is 255 + a low part kept in its block's side list (1.5): one
  byte to 511 codes, two beyond. Every tree-sitter grammar measured but JSON's and Vue's passes 511
  (1,843–3,696 codes), so its low part is two bytes. The codes the corpus never showed are all past
  the cutoff, and the nodes past it stay as few as before (they are the rare codes either way).
- **A language with 256 codes or fewer** (JSON 70, Vue 76) has no cutoff.
- Measured on files the numbering never saw: 0.36–1.12% of nodes past the cutoff on Python, Rust,
  TypeScript and JavaScript (t39 step 13).

A rule's kind or field test compiles to numbers, once per language. A test for one code below the
cutoff is one byte compare. A set of codes is a 256-bit set, read 64 nodes at a time, and it decodes
the low byte only where the set holds a code past the cutoff. Reading the code at random costs a
compare and a branch, 0.55–0.63 ns on the M3 (t39 steps 13 and 15). In order, a block's codes are
widened in bulk and its side list patched in, at 0.09 ns a node.

The field stays in the code. Without it, kind, flags and memberships fit a byte with no prefix.
But the field then needs the parent's code and the child's index, a 5–20 KiB table, and 3–6× the
read, for 7–12% less disk (t39 step 14).

What is in the code table, from the grammar:
- **Kind ids**, per language, including contextual aliases in the grammar's alias range. Tree-sitter
  gives an aliased node its alias symbol's id, and the spike's "kind aliases assigned codes once per
  grammar at registration" is the same idea (`tape-v8-measurements.md` Part 1). There is no per-tape
  alias table (t35 52).
- **Field ids**, u8: SQL has 61 fields today, three short of format 15's limit of 64, and Haskell has
  84.
- **Memberships**, a mask over the supertypes the grammar lists for the kind, in ascending order. A
  node's actual memberships depend on where it sits (decision 0025's finding still holds), but they
  are always a subset of that list, which comes from `node-types.json`. On the 31 grammars in the
  cargo registry at most 11 supertypes can contain a kind (Ruby's `constant`; Python 4, Rust 4,
  JavaScript 3), so 16 bits hold every mask.
- **Flags**: extra, error, missing. The varying `extra` of one Python kind is its flag bit (t35 13).

On Rerail's vendored grammars (`native/grammars/*/grammar.json`, supertypes expanded through hidden
rules), plus tree-sitter-php 0.24.2 and tree-sitter-rust 0.24.2:

```text
grammar       fields   supertypes   most candidates for one kind   codes seen (rerail, data-layer)
python          32        6           4  (identifier)                391, 440
javascript      36        5           3                              321, 308
typescript      43        7           5  (member_expression)         469
php             31        5           3                              326
rust            31        6           4                              494, 362
sql             61        0           0                              148, 54
```

Kind ids stay per language. Global ids (language × 2^16 + kind) stay in indexes only, as the
spike's Option B and decision 0055 say.

**Every code the grammar allows (E25).** The table is derived from the grammar's `node-types.json`,
generated from `grammar.json` by the generator the build uses (tree-sitter-generate 0.27.1), or as
the registry crate ships it. A code comes from one of these places:

- **a field** of a type, or **its unfielded children**: a kind listed there, or every kind under a
  supertype listed there. Its memberships are the supertypes on its path, or any inner part of the
  path, because `node-types.json` names only the outermost supertype a place allows;
- **the root**: the grammar's first rule;
- **an extra**: under any node, no field, flagged extra;
- **an ERROR**: under any field or none, flagged error, or error and extra when the parser skipped
  it;
- **under an ERROR**: any kind, no field, with any membership path it has anywhere;
- **a missing leaf**: wherever a leaf of its kind can be, flagged missing.

`tape16 gcodes` lists them and checks every node of the rerail and data-layer captures against the
list. Every captured code is on it, in all 11 languages:

```text
language     codes the grammar allows   codes seen (rerail, data-layer)
python          2,045                    420, 471
typescript      3,647                    499, 177
tsx             3,696                     49
javascript      2,419                    351, 339
php             2,761                    365
rust            1,843                    512, 389
sql             1,937                    155, 57
json               70                     26, 26
vue                76                     28
.rr            14,896 (bound)            107
blade           2,035 (bound)             37
```

The list over-approximates: "any inner part of the path" and "anything under an ERROR" allow more
than a parser makes, and that costs only numbers past the cutoff no node uses. `.rr` and Blade are
parsed by Rerail's own parsers, which declare no node types, so their bound is every kind under
every field, plain, extra or missing. Each own parser should declare its node types as a
`node-types.json` does, which would bring `.rr` from 14,896 to about its fields' real pairs. Until
then the bound fits the two-byte low part.

### 1.3 Structure: the slot

An inner node's slot is its size (`size << 1`). A leaf's slot (`… | 1`) describes the node after it,
which always has a previous sibling with the same parent: it holds that sibling's size
(`<< 2 | 1`), or, for every 8th sibling, the distance back to the parent (`<< 2 | 3`).

- **Subtree end** and **next sibling**: `i + size`, from the node's own slot, or `i + 1` for a leaf.
- **First child**: `i + 1`, when the slot's tag says inner.
- **Parent**: the node before is the parent if it is inner. Otherwise its slot names the previous
  sibling (step there and repeat) or the parent. A read takes at most 7 sibling steps, 0.9–1.8 on
  average.

The sibling form keeps a flat array's slots small, which the back distance to the parent did not
(JSON escaped 14–19% of its slots that way), so one width a language costs under 0.05 B a node
(t39 step 8). Its price is the point parent: 2.5–3.7× a stored back distance at random. Rules read a
parent at a point 0.58 times a node at the median and 13.7 at the heaviest, which comes to 0.4–1.7%
of a rule's CPU (t39 step 9).

A leaf could name the parent wherever the distance is short, and the sibling only past that (E31,
t39 step 23). Memory stays the same, since nothing more escapes, and repeats keep their bits. Up to
15, it takes 73–75% of the sibling steps away. But only a point parent gains, 10–20%, which is
0.06–0.34% of a rule's CPU, for up to 1.6% more disk. The bulk primitives read no parent, or few. So
every 8th sibling alone holds the parent distance.

### 1.4 Positions: gaps, lengths, checkpoints

In preorder, starts never decrease. So a start is stored as its gap to the previous node's start,
and an end as the length `end − start`. The absolute start of every 8th node sits in the
checkpoints.

- **A start at random** is its checkpoint plus the sum of at most 7 gaps. The sum is read in place,
  a word at a time: each word's fields are folded pairwise into their sum, and an exact all-ones
  test finds escapes.
- **In order**, a start is a running sum.
- **Early ends** need nothing special: a length is `end − start` however the node's end relates to
  its descendants', so format 15's `TAIL_FROM_START` and its re-parse go (t35 19, 27).
- Gaps and lengths are the same wherever the text is the same. Of the structurally repeated nodes,
  24–31% keep their gaps and lengths within a file on code, and 3–12% across files: the rest differ
  in names, literals or spacing (t39 step 8).

### 1.5 Escapes and side lists

Every array but the checkpoints is cut, for its exceptions, into blocks of 256 nodes. A block's
escapes, and its codes past the cutoff, are listed as (offset in the block, value), sorted.

The list is kept in parts, one for each fixed block of 1,024 nodes (2.1), so a fixed block and its
part are stored, loaded and decoded together. A part holds, in order:
- a 2-byte end for each of its four 256-node blocks;
- the offsets;
- the values: a code's low part, one or two bytes, or an escaped value at whole bytes.

A part with no entries is empty. Where each part starts is in the block index (2.1), so a 256-node
block's entries are found from that start and two of the ends (E26).

- **The escape marks itself.** A value equal to all-ones, or a code byte at or past the cutoff, is
  the only kind of read that looks at the list. Every other read never touches it.
- **The search** takes at most 8 probes. A rank count (the prefix bytes before the node, a word
  at a time) reads as fast and drops the offsets, at 1 B less per listed node (t39 step 15).
- **Measured shares** (E24, data-layer Python): 0.36% of codes past the cutoff, slots 1.8%, gaps
  0.05%, lengths 3.1%.

The ends and part starts take 0.012 B a node per array, against 0.016 for the 4-byte start index
first measured. The rest of a list is each entry's offset byte and value. The readers read the parts
where they lie (E27), so the bytes a block decodes to are the bytes every read reads.

### 1.6 Lines and text

- **Line**: an Elias–Fano index of the newline offsets, 0.19 B a node, part of the tape. A node's
  line is the rank of its start among them. Every node's line in order is a merge (t39 step 1).
- **Text**: the names and source streams of section 6. A leaf's text is a span into the source, or
  its id in the names stream.

### 1.7 Each question, in place

Every question reads the arrays where they lie, with nothing built beside them. Costs are ns a
call on the M1 Max at load 3–4 (E24, `tape16 inplace` and `tape16 prim`, the data-layer, rerail and
`impl/sqlite` captures, one thread).
Every answer was checked against the tree.

| question | how | F16 | XF | format 15 |
|---|---|---|---|---|
| kind, field, flags, supertypes | the code byte; a code past the cutoff reads its block's list | 1.0 | 1.2 | 1.0 |
| a set of codes, 64 nodes at a time | the bytes through a 256-entry set | 0.4–0.5 a node | 0.3 | |
| subtree end, next sibling | the slot | 3.7–3.9 | 3.0–3.3 | 1.3–1.4 |
| children of a node, all of them | first child `i + 1`, then siblings by slots | 3.7–4.6 a node | 7.0–8.1 | 3.5–3.9 |
| parent, at random | at most 7 sibling steps | 5.9–11.7 | 3.9–4.5 | 45–4,100 (a search) |
| start, at random | a checkpoint and a fold of at most 7 gaps | 4.9–5.0 | 2.0–2.2 | 1.2–1.3 |
| end, at random | the start and a length | 5.6–6.0 | 4.7–5.4 | 3.1–3.8 |
| every span, in order | a running sum | 2.5–2.6 a node | 4.4–4.6 | |
| line, every node / one node | a merge with the newline index / its rank | 2.9–3.3 / 20–40 | 2.7–3.1 / 16–39 | |
| text | the source span, or the names stream (6) | | | |

Format 15 reads absolute columns at byte widths, so its point reads are the fastest, at twice the
memory, and it has no parent: each is a search. XF is the explicit tape with per-block widths, about
half format 15's size (t39 steps 1–6). The point parent and the point start are what the relative
forms cost. Reading in order through cursors, which E17 assumed of the engine, is as fast as XF or
faster.

On Metal (E29, t39 step 21), the same reads as kernels, one thread a question over the whole corpus,
take 0.07–0.37 ns a question on the M1 Max, within 1.5× XF's. The parent is the exception at
0.22–0.30 ns, 1.6–2.3× XF's, as on the CPU. Every answer was checked against the CPU reader's.
The bulk primitives (E30, t39 step 22), one thread a group of 64 nodes, take 0.01–0.09 ns a node to
match a set on the M3, 0.06–0.09 for has from counts and 0.07 for spans, level with XF's or
faster. Climbing from each group's first node, nearest and outermost took 1.5–2.5× XF's. So
outermost is a prefix max over the S-nodes' ends, with no climb, and nearest climbs only from a group
that has an S-ancestor, up to the first that encloses the group (E31, t39 step 23). Both are then
level with XF's or faster: 0.10–0.16 and 0.14–0.36 ns a node on the M1 Max.

### 1.8 What it costs, together (E24)

Measured files only: the numbering, widths and dictionaries were learned on the other half
(E24, `tape16 f16`). The in-memory totals are E25's, with each code table the grammar's (1.2): the
two-byte low parts add 0.006–0.011 B a node. Bytes a node:

| | data-layer Python | data-layer JSON | rerail Rust | rerail TypeScript | rerail `.rr` | rerail JSON |
|---|---|---|---|---|---|---|
| codes | 1.03 | 1.02 | 1.04 | 1.04 | 1.02 | 1.02 |
| slots | 1.08 | 1.10 | 1.09 | 1.10 | 1.07 | 1.25 |
| gaps | 1.04 | 1.02 | 1.03 | 1.04 | 1.05 | 1.02 |
| lengths | 1.13 | 1.07 | 1.10 | 1.11 | 1.06 | 1.15 |
| checkpoints | 0.30 | 0.35 | 0.29 | 0.27 | 0.28 | 0.36 |
| **in memory** | **4.58** | **4.55** | **4.55** | **4.57** | **4.47** | **4.80** |
| format 15, in memory | 8.63 | 9.21 | 8.57 | 8.80 | 8.50 | 9.91 |
| zstd 3 + dictionary, whole arrays | 1.90 | 0.95 | 2.07 | 2.08 | 1.44 | 0.92 |
| format 15, zstd 3 + dictionary | 2.34 | 1.14 | 2.52 | 2.55 | 1.82 | 0.93 |
| fixed blocks of 1,024, LZ4 HC + dictionary, with the index | 2.27 | 1.23 | 2.42 | 2.29 | 1.57 | 1.43 |
| escaped: codes past the cutoff / slots / gaps / lengths, % | 0.36 / 1.8 / 0.05 / 3.1 | 0 / 2.4 / 0 / 1.4 | 1.1 / 2.2 / 0.01 / 2.5 | 1.1 / 2.6 / 0.06 / 2.6 | 0 / 1.0 / 0.02 / 0.4 | 0 / 6.3 / 0.1 / 3.8 |

- **About half of format 15's memory, on every language** (4.5–4.8 B a node against 8.5–9.9). Every
  array is close to its one byte. The rest is the escapes' lists (2–25% of an array) and the
  checkpoints, which are absolute starts and the largest part after the four arrays.
- **Against the other in-place forms:** XF takes 4.15–4.59 B a node on the same captures, MF
  (bit-packed, predicted positions) 3.37–3.97, and BE (balanced parentheses) 2.43–2.86. MF and BE
  are smaller in memory. But MF's questions cost about XF's (its ends 2×), BE's 3–7× more, and both
  are larger on disk (zstd of the in-memory bytes, data-layer: MF 2.04, BE 1.36, F16 1.31 B a
  node).
- **On disk, the smallest form measured**: 17–21% under format 15 with zstd on code and data-layer
  JSON, level on rerail's JSON. In fixed blocks with random access, 6–12% over LZ4 on whole arrays on
  code (19% over zstd on Python), and 24–38% on JSON. The index costs 0.02–0.03 B a node.
- **Loading** costs 4.6–5.6 ns a node from fixed blocks (LZ4) and 9–14 from whole arrays (zstd) on
  code. Both include the lab's allocation for each array and, for zstd, a dictionary context per
  call, so they are upper bounds. One block of 1,024 codes decodes in 0.25–0.9 µs.
- **The E7 rule mixes** cost 2.5 ns a node at the median rule (XF 3.0, format 15 2.2), 17.4–18.5
  for all 121 rules (XF 18.2–20.2), and 181–192 for the heaviest (XF 186–208). Only the tree walk,
  which climbs 5 parents a node, costs more than XF (83–113 against 73–80).

## 2. At rest

Loading inflates the stored bytes to exactly the in-memory arrays. Nothing is rebuilt and nothing
is transformed.

### 2.1 Fixed blocks, for random access while compressed

Each array of a file is cut into fixed blocks of N nodes (N = 1,024 proposed). So node i's block is
`i / N`, at a fixed offset in the decoded array. A 4-byte index entry per block says how the block
is stored:

- **LZ4**: one block in LZ4's format (HC level 9), whose matches reach only the block itself and the
  language's dictionary for that array;
- **raw**: the bytes as they are, read in place. The checkpoints are always raw, since absolute
  starts do not compress, and so is any block LZ4 does not shrink by a tenth;
- **repeat**: a pointer to an identical block stored earlier, in the file (4 bytes) or in a pool the
  store shares (8 bytes). This pays only on JSON (2–15% of blocks); code repeats are found by
  matches into the dictionary instead.

Each block carries its nodes' part of the array's side list (1.5) after its fixed-width bytes, in the
same LZ4 block, so one decode gives both. The index entry for a block also names where its side
part goes, so it takes 8 bytes. Against E24's form, which kept each side list as one LZ4 block per
file and array, this takes −2% to +5% at rest (E26). A read decodes one block: 0.2–0.7 µs for 1,024
nodes on the M3. Against the same arrays compressed whole, random access costs 4–11% at N = 1,024
and 1–5% at 4,096 on code (t39 step 11). A reference to one earlier block of the file takes back a
quarter of that, for a second decode.

LZ4 is the block format GPU decoders read, and a block's matches never leave it or the dictionary,
so a threadgroup can decode one block with no other block's output (requirement 1).

**On the device (E26, `tape16 f16-gpu`).** One SIMD group decodes one block, with every block of a
capture in one dispatch. Its 32 lanes read each token together, copy literals and matches 32 bytes
at a time into threadgroup memory (a match closer than 32 bytes in rounds of its distance), and write
the block out to its array and its side part to the side list. One thread a block, as the reference
library decodes, is 1.6–2.7× slower. Every byte was checked against the arrays. ns a node, warm,
every language in one dispatch:

```text
                    data-layer                         rerail
              GPU    CPU 1 thread  CPU every core   GPU    CPU 1 thread  CPU every core
M5 Max        0.14   2.33          0.21 (18)        0.10   2.07          0.24 (18)
M3 Max        0.16   2.41          0.39 (16)        0.18   2.37          0.48 (16)
M1 Max        0.36   2.73          0.51 (10)        0.42   2.73          0.56 (10)
```

The GPU decodes 11–45 GB/s of arrays. That is 1.3–2.7× every CPU core and 6.5–21× one, and it
leaves the CPU free. The M5 Max and the M3 were at load 17–40 from other work, and the M1 Max at
3–4. A dispatch of one block costs 5–170 µs, so the device decodes in batches. A point read on the
CPU decodes its one block.

The side lists made the case. Kept as one LZ4 block per file, they put up to 6 KB of serial decode
on one thread, which set the dispatch's time (3.2 ns a node). Cut per fixed block, every block
decoded fits the threadgroup's 3 KiB.

### 2.2 Whole files, for cold storage

A file kept only for loading may be stored as its arrays under zstd (level 3) with the language's
dictionaries. That is the smallest form measured: on data-layer Python, 1.90 B a node against format
15's 2.34 (E24). It has no random access.

### 2.3 Dictionaries

One dictionary per (language, revision, array), 64 KiB, trained on a corpus and stored once by
content id. The revision names its dictionaries (1.0), so a block does not: a new dictionary is a
new revision, and tapes of the old one keep theirs. A 64 KiB dictionary
takes most of what files share: compressing every file end to end with a 128 MiB window saves only
3–12% more (t39 step 10).

## 3. Large files

Files have no limit but 2^48 bytes:

- **Positions**: the checkpoints' width is the file's, in whole bytes, so a start past 2^32 widens
  only the checkpoints.
- **Gaps, lengths and slots** wider than their array escape. A subtree of more than 127 nodes
  escapes its 8-bit slot, as 1.8% of Python's do.
- **Node ordinals** past 2^32 are u64.
- **At rest**, the block index's offsets are u64, and a stored block is far below SQLite's
  2^31 − 1-byte blob limit.

## 4. Edits

A tape is rewritten from the edit on: node ordinals shift, and so do the fixed blocks after the
edit, at rest. Identity D (an ordinal plus each edit's shift map) changes 3% of downstream rows per
edit, against 71% for plain ordinals (T7, E4). The content-defined blocks first proposed here would
have kept the blocks away from the edit byte-identical, but they give up the fixed offset that
random access needs (t39 step 11).

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

The node boundaries (every start and end) cut the source into *pieces*: a leaf's text, or a gap
between two boundaries. A gap is whitespace plus the tokens the grammar implies: `(`, `, `,
`)\n    `. The text stream holds one piece id per piece, in source order:

```text
piece table   per id: length, newline count, the bytes (in the file's pool or the dictionary's)
text stream   per piece: a piece id                      an array like those of 1, blocked as in 2
```

There are three levels, each loading without the source:
- **Structure:** the node stream alone. Spans, shapes, kinds.
- **Names:** plus the leaf pieces. Identifier and literal text: what most text tests read. The leaf
  pieces are the deduplicated leaf vocabulary the spike's value index requires
  (`sql-index-prototype-history-index.md` §11), so a leaf piece id can be that index's value id.
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
open(code, start)   append the node: its code byte (or the prefix, and a side-list entry), its gap
                    (start − the previous start), every 8th node a checkpoint; and the slot of the
                    leaf before it, which describes this node
close(end)          write the node's length, and for an inner node its slot (size << 1)
commit()            the window from here back is final
```

The language's numbering and widths are fixed before the file starts, so nothing moves at the end:
there is no `finish` that moves columns, no code table, no width guess and no re-parse.

- **Every cell is written once, at a fixed place.** An inner node's slot and every node's length are
  known only at its close, so they are written then, at the node's own index. That is not a
  back-patch of a value already written, but the arrays are not written strictly left to right.
- **A block's side list is final** when every node in its 256 has closed. The root's block closes
  last.
- **Wraps** (an operator found after its left operand) insert a node before the operand: within
  the uncommitted window, a move of the window's later entries by one.
- **The tree-sitter path** writes from its cursor walk. `Pending`, `pack`, the size fold and the
  code fold go (t35 43). The per-node cursor status call stays, for the membership mask.
- **On the GPU,** gaps and lengths are differences, sizes come from a scan of the depths, and every
  array is written at an index from a prefix sum. The format no longer blocks a GPU tape writer
  (t35 50).
- **One definition.** simdjson, the lean parser and tier 2's Cranelift, AOT and Metal backends call
  or generate from the crate's constants. There are no copies (t35 46–47).

**Measured (E28, `tape16 f16w`).** The lab's writer takes exactly these calls. Each node's open and
close are driven from the captured trees as a parser makes them, and its output equals the reader's
tape byte for byte on every file. It is set against format 15's writer (`preorder_image`) over the
same nodes:

```text
instructions / ns a node, M1 Max at load 3–6
                     format 15      format 16 (of which finish)
data-layer Python    227 / 12.2     190 / 12.2   (27 / 1.6)
data-layer JSON      198 /  9.0     177 /  8.8   (19 / 1.1)
rerail Rust          194 / 11.6     185 / 13.1   (25 / 1.6)
rerail TypeScript    200 / 10.9     194 / 12.0   (29 / 1.7)
rerail .rr           200 /  9.4     180 /  9.1   (17 / 0.8)
rerail SQL           186 /  8.9     193 / 11.1   (36 / 2.3)
```

- **The same cost as format 15's writer:** 3–16% fewer instructions on the large sets. The time is
  level on Python, JSON and `.rr`, and 10–25% more on Rust, TypeScript and SQL.
- **`finish`** lays the side lists in parts: 17–44 instructions a node.
- **On `.rr`**, the lean parse with format 15's write takes 454 instructions a node, and with format
  16's 437 (M5 Max).
- **It writes at the language's widths as constants**, as generated code would, and the checkpoint
  width comes from the source length, known before the first node.

## 8. SQL

Tapes are read through native table functions over the in-memory arrays. At rest they are read
through `sqlite3_blob` handles, never a value (the spike, D S062–S064), and only the fixed blocks a
query touches are decoded: no whole-frame decode, and no 8-entry LRU of whole tapes (t35 30). A raw
block can be read by pure SQL with `substr` arithmetic.

## What this removes, and what it does not

Removed or made unnecessary, by t35 number:
- **1–5:** whole-file buffering, the moves at the end, and quadratic wraps. Widths and numbering are
  fixed per language. Slots and lengths are written at a node's close into its own cells.
- **6–9 and 11–13:** per-tape numbering and code tables, the 1,024 and 64 limits, the 8-membership
  limit.
- **14–21:** per-document widths, re-parses, width branches, `TAIL_FROM_START`, the marks slow path,
  and ends read through the last descendant.
- **22:** line tables built by scanning the source at read time. The newline index is part of the
  tape.
- **29–30:** random access at rest, and whole-frame decodes.
- **43–50:** intermediate forms in writers, copies after writing, four implementations, and the
  obstacles to a GPU writer.
- **52:** the alias table.

Not removed by a format:
- **Sections 8 and 9 of t35** (conversions per phase, repeated hashing and validation, copies) are
  the code's. Format 16 gives them one form, read in place, to converge on.
- **Edits** (t35 25): an edit shifts ordinals and the fixed blocks after it (4, T7).
- **The named-only rule's effects on rule meaning** (t35 28) are untouched.

## Departures

- **From this draft's first version (2026-11-13):**
  - **The event stream** (an open and a close record per node, with `lead`, `pops` and `trail`
    deltas, and positions, depths, parents and sizes derived by scans) gives way to per-node arrays
    that answer every question in place. The events made every delta small and every repeat equal,
    but no question could be answered without a scan, which the fixed requirement rules out. The
    arrays keep the equal repeats (1.1).
  - **The literal-pool LZ over byte planes** gives way to LZ4 in fixed blocks at rest (2.1). It
    decoded at 4.6–5.4 GB/s on Metal, but took 1.4–2.4× format 15's bytes at rest, and its output
    still had to be decoded before a read.
  - **Content-defined blocks and a placement table** give way to fixed blocks. Random access needs a
    node's block at a fixed offset, and fixed blocks on code do not repeat anyway (t39 step 11).
  - **The resident decoded form** (old section 4, withdrawn on 2026-11-14).
  - **Codes overloaded by the previous byte** (E20) give way to frequency numbering (E21): no
    128 KiB table a language, and no faster reads to give up for it.
- **From 0025:**
  - per-tape folded codes give way to one table a language (grammar ids plus a mask), numbered by
    frequency;
  - v9np's base/off/tail columns give way to gaps, lengths and checkpoints;
  - per-document widths go.
- **Kept from 0140:** the writer's columns at their place, now from the first node, since the widths
  are fixed per language.
- **From 0026:** a single zstd frame per tape gives way to fixed LZ4 blocks per array, or zstd
  over whole arrays for cold storage.
- **From the spike:**
  - chunking at rest into fixed blocks (2.1);
  - codes as one byte numbered per language, in place of `kf`'s 10 + 6 bits (1.2).
- **Kept from the spike:**
  - one per-file structure, no row per node (DI Top 1);
  - preorder entries, with navigation from the slot;
  - native table functions by address;
  - kind codes local, global in indexes.
- **Kept from 0026 and 0032:**
  - nothing else stored per node;
  - at-rest and resident forms designed together, with 16-byte aligned resident columns.

## Tradeoffs

Each is a choice whose cost on one side and gain on the other is measured before it is made. T1–T3
and T9 are answered by sections 1–2 (2026-11-15); the rest are open. Each lists the options, what moves between them, and the experiment that
measures it.

### T1. The codec: how much compression to give for parallel decode and random access

**Answered (2026-11-15):** fixed blocks of LZ4 over a dictionary for random access, zstd whole
for cold storage (2; t39 steps 10–11). The literal-pool codec is in Departures.

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

**Answered (2026-11-15):** the in-memory arrays themselves, at whole-byte widths. At byte widths
a repeat's runs line up as bytes, and zstd's matches cover 75–86% of them (t39 step 10).

- **Byte planes** make zero high bytes one run each. But a repeated subtree becomes one copy per
  plane (18 copies).
- **Whole records (16–18 B)** make a repeated subtree one long copy. But their zero bytes are
  literals unless the codec has runs inside records.
- **Field planes** (one per field, at its width) sit between the two.
- **What moves:** bytes, and the sequence count, which is decode work.
- **Measured by:** E1, all three layouts, on the same blocks.

### T3. Block size and checkpoint interval

**Answered (2026-11-15):** 1,024 nodes a block at rest (4–11% over whole arrays on code), and a
checkpoint every 8 nodes (t39 steps 8, 9, 11).

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

**Answered (2026-11-15):** the choices below are made in section 1 (t39 steps 7–16). What
each option cost when it was chosen is kept here.

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
  Same bits, measured (E15–E16, t39 steps 7–8): XF's per-block PFOR widths keep only 16–89% of
  repeated nodes' bits within a file; one width a language, with escapes that name nothing, keeps
  all of them, and with the leaf slot by sibling it costs under 0.05 B a node. Gap positions take
  16–24% less memory and a third less disk on code, at 2–3× a point span's cost. Weighed by today's
  engine (E17, step 9), both add 0.4–1.7% to a rule's CPU at K = 8 if the engine reads in order
  through cursors. Codes in one byte (E20–E23, steps 12–15) and the field kept in the code (E22)
  are in 1.2. **Chosen:** 1.1–1.5.
- **At rest, with random access.** Fixed blocks of N nodes, each raw, a repeat, or one LZ4 block over
  the language's dictionary (E19, step 11). **Chosen:** 2.1, at N = 1,024.
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
  The grammar counts are in 1.2. **Answered by E25**: the table is every code the grammar allows
  (1.2), and no captured node falls outside it.
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
  On Metal, the same questions as kernels (t38 E10 §5; format 16's in E29).
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
- E24 (2026-11-15, t39 step 16): the chosen form as one tape (1.8): 4.5–4.8 B a node in memory,
  0.9–2.1 on disk, every question in place and checked.
- E25 (2026-11-15, t39 step 17): the code table from the grammar (1.2): 1,843–3,696 codes a
  tree-sitter language, and no captured node outside it.
- E26 (2026-11-15, t39 step 18): fixed LZ4 blocks decoded on Metal (2.1): 0.10–0.42 ns a node, 1.3–2.7×
  every CPU core, with each block's side-list part inside it.
- E27 (2026-11-15, t39 step 19): the side lists read in their parts, as stored (1.5), and the version
  mark (1.0) on every tape, checked before every load.
- E28 (2026-11-15, t39 step 20): the writer (7): format 15's cost a node, 3–16% fewer instructions,
  every tape equal to the reader's.
- E29 (2026-11-15, t39 step 21): every question as a Metal kernel on the tape as stored (1.7):
  0.07–0.37 ns a question, the parent 1.6–2.3× XF's, every answer checked.
- E30 (2026-11-15, t39 step 22): the bulk primitives as Metal kernels on the tape as stored (1.7):
  match, has and spans level with XF's or faster; nearest and outermost 1.5–2.5×; every answer
  checked.
- E31 (2026-11-15, t39 step 23): outermost by a prefix max and nearest by a bounded climb, level with
  XF's or faster at no bytes; a slot naming short parent distances measured and not taken (1.3).
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
