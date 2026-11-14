# Parser generators: the problem

Status: problem statement, 2026-11-06. The design is in `01-branch-free-design.md`.

## Intro

Rerail turns every source file into a *tape*: the file's syntax tree in preorder, stored as the
v9np columns of decision 0025 (`TAPE_FORMAT_VERSION` 11). Today a tape is built in two steps. A
tree-sitter parser builds a tree, then `TapeImage::encode` walks that tree with a cursor and
writes the columns.

Parsing is now the largest single cost of a cold audit. simdjson parses JSON about seventy times
faster than tree-sitter does. It gets there by not branching: the input becomes bitmasks, the
bitmasks are combined with logical operations and prefix sums, and the output positions come from
prefix counts.

The aim here is the same for tree-sitter grammars: source bytes in, Rerail's own tape image out,
with no data-dependent branches, on CPU SIMD and on Metal. The first languages are JSON, which has
a known solution, and Python, which is fairly regular. All tree-sitter languages come later. A file
that tree-sitter parses with a syntax error keeps going to tree-sitter.

## 1. What parsing costs today

Measured 2026-11-06 on the data-layer repository, without `.venv`, `node_modules` or `.git`.
The parsers are tree-sitter 0.25.10 with the tree-sitter-python 0.25.0 and tree-sitter-json 0.24.8
crates. simdjson is 5.0.5 (Homebrew, arm64 kernel), using its DOM parser, which builds simdjson's
own tape. Times are user+sys CPU on one thread, with machine load about 8.

| | Python | JSON |
|---|---:|---:|
| files | 13,088 | 14,669 |
| source | 171.0 MB | 567.7 MB |
| tree-sitter parse | 8.04 s, 21.3 MB/s | 14.94 s, 38.0 MB/s |
| cursor walk over the trees | 2.19 s | 8.43 s |
| simdjson DOM parse | | 0.21–0.23 s, 2.5–2.7 GB/s |
| nodes | 34.2 M (0.20 per byte) | 169.2 M (0.30 per byte) |
| named nodes | 65.1% | 45.0% |
| leaves (tokens) | 66.3% | 72.8% |
| files with an error or missing node | 46 (0.35%) | 1 |
| deepest node | 83 | 42 |

- **simdjson against tree-sitter-json:** 68 times faster on parsing alone, and about 100 times
  faster once the cursor walk is added. The cursor walk only counts nodes. It is a lower bound on
  what `TapeImage::encode` costs, which also writes the columns.
- **Load matters.** At load 47 the same tree-sitter runs took 12.2 s (Python) and 23.3 s (JSON).
- **An earlier run agrees.** On 5,725 corpus files (95.7 MB), recorded in
  `docs/transducer-design.md`, tree-sitter parsing took 8.65 s, and tape encoding and restoring
  took another 4.57 s.
- **Against the cold-audit target of 1–2 s:** parsing the repository's Python alone takes 8 s of
  CPU, about 1 s on eight cores, before encoding.

The two parsers do not write the same output. Section 3 compares their sizes.

## 2. What the tape holds

A v9np image (`native/src/code_rule_dsl/tape.rs`, `TapeImage`) has five columns, one row per node
in preorder, each column aligned to 16 bytes, followed by two tables:

| Column | Width | Holds |
|---|---|---|
| `code` | 2 bytes | index into the tape's code table |
| `size` | 2 or 4 bytes | nodes in the subtree, the node included |
| `base` | 4 bytes per 16 nodes | the least start of the block's nodes |
| `off` | 2 or 4 bytes | `start − base[i / 16]` |
| `tail` | 2 or 4 bytes | `end − start(i + size − 1)`: the end, measured from the last descendant's start |

- **The code table** holds each distinct folded code once, in order of first use. A folded code is
  the kind, the incoming field, the flags (extra, error, missing), any contextual alias, and up to
  8 supertype memberships. Memberships are recorded as the tree-sitter cursor reports them, so
  they depend on where the node sits, not only on its kind.
- **Widths** are 2 bytes unless a column's largest value needs 4. They are chosen per document.
- **The tape includes every node tree-sitter emits:** anonymous tokens such as `(` and `,`, extras
  such as comments, and `ERROR` nodes. Two thirds of all nodes are leaves.

Decision 0026 makes this image the only per-node structure Rerail stores. Decision 0032 keeps tapes
resident in 64 MiB slabs that the device reads in place. A parser on the device could write a
tape's columns straight into a slab.

## 3. Why Rerail's tape is larger than simdjson's

Measured over the same 14,669 JSON files (567.7 MB). Rerail's image sizes follow
`TapeImage::finish`'s rules applied to the tree-sitter trees: column widths, 16-byte padding, the
header and the code table. simdjson's sizes are its tape's words plus its string buffer, which
holds 4 bytes of length, the unescaped bytes and a NUL per string.

| | records | bytes | per source byte |
|---|---:|---:|---:|
| simdjson tape | 46.2 M words | 369 MB | 0.65 |
| simdjson string buffer | 23.6 M strings | 560 MB | 0.99 |
| **simdjson total** | | 930 MB | **1.64** |
| **Rerail v9np image** | 169.2 M nodes | 1,594 MB | **2.81** |
| Rerail, Python, for comparison | 33.5 M nodes | 292 MB | 1.75 |

Counting only structure, Rerail's image is 4.3 times simdjson's tape. Counting everything each
parser writes, it is 1.7 times. The gap comes from three places.

**1. 4.7 times as many records: Rerail keeps the concrete syntax tree.** The corpus has 36.1 M
JSON values. simdjson writes one word for each string, true, false and null, and two for each
number and container. Rerail writes a node for every node tree-sitter emits:

| Rerail nodes with no simdjson record | nodes | share |
|---|---:|---:|
| `"` tokens, two per string | 47.2 M | 27.9% |
| `,` | 16.4 M | 9.7% |
| `:` | 15.3 M | 9.0% |
| `{ } [ ]` | 14.1 M | 8.3% |
| `string_content`, inside each non-empty string | 24.1 M | 14.2% |
| `pair`, wrapping each key and value | 15.3 M | 9.0% |
| **total** | **132.4 M** | **78.3%** |

A non-empty string is four nodes (`string`, `"`, `string_content`, `"`) against simdjson's one
word. A key–value pair is seven nodes (`pair`, the key's four, `:` and the value) against two
words. The anonymous tokens are kept because rules and tree-sitter queries match them, and because
findings report their spans.

**2. Every node stores its position; simdjson stores none.** Per node, the image holds:

| column | bytes per node |
|---|---:|
| `code` | 2.00 |
| `size` | 3.15 |
| `off` | 2.00 |
| `tail` | 2.00 |
| `base` | 0.25 |
| code table and header | 0.02 |
| **total** | **9.42** |

- **`off` and `tail` are 4 of those bytes.** They locate the node in the source. simdjson's tape
  has no source positions at all.
- **`size` averages 3.15 bytes, not 2.** A document with more than 65,535 nodes stores 4-byte
  sizes, and 57.5% of the JSON nodes are in such documents.
- **Much of this is implied by the kind.** 72.8% of nodes are leaves, whose `size` is always 1.
  Every anonymous token has a fixed width.
- **The format is fixed.** Decision 0025 locks it, and the parser must write it as it is.

**3. simdjson stores decoded values; Rerail reads them from the source.** simdjson's string buffer
is 60% of its output: every string copied, unescaped, with its length. Numbers are converted to
binary. Rerail stores no values, since a node's text is `source[start..end]`. If simdjson did not
copy its strings, the gap would be the 4.3 times above.

**What this means for the parser.** At 0.30 nodes per JSON byte and 0.20 per Python byte, the cost
of producing one node decides the speed, not the cost of reading one byte. At simdjson's input
rate (2.6 GB/s), the JSON image would be written at about 7 GB/s. That is within one core's store
bandwidth, but only if each node takes a handful of instructions and no branches.

## 4. How simdjson does it

simdjson (Langdale and Lemire, "Parsing Gigabytes of JSON per Second", 2019) reads 64 bytes at a
time and computes one 64-bit mask per fact:

```text
input       {"a":"x\"y", "b":[1,2]}
backslash   .......1...............   byte is '\'
escaped     ........1..............   follows an odd run of backslashes
quote       .1.1.1...1..1.1........   '"' and not escaped
in_string   .11.11111..11..........   prefix XOR of quote (one carry-less multiply)
structural  1...1.....1....1.1.1.11   {}[]:, and not in_string
```

- **Character classes** come from two 16-entry table lookups, on each byte's high and low nibble.
- **The in-string mask** is a prefix XOR of the quote mask.
- **Masking:** one AND removes structural characters inside strings.
- **Flattening:** a loop of count-trailing-zeros turns the set bits into byte offsets. It always
  writes 8 offsets and advances by the popcount, so there is no branch per bit.

simdjson's second stage, which writes its tape, still branches per token. It is a state machine
over the offsets. Its open-bracket words hold the position of the matching close, which carries
the same information as Rerail's `size`. Section 2 of `01-branch-free-design.md` shows that Rerail's tape can be
written without that state machine.

## 5. Where tree-sitter fidelity is hard

A tape written without tree-sitter must equal the one written from tree-sitter's tree, byte for
byte. These are the places where that is hard:

1. **Context-aware lexing.** Tree-sitter lexes with a mode chosen by the parser state. Python's
   soft keywords (`match`, `case`, `type`, `print`, `exec`) are keywords in some places and
   identifiers (`keyword_identifier`) in others.
2. **The external scanner reads the parser's state.** Python's `scanner.c` produces NEWLINE,
   INDENT, DEDENT, the string tokens and comments. It asks which tokens the parser would accept
   (`valid_symbols`). Being inside brackets is detected that way, and so is whether a comment
   before a dedent belongs to the inner or the outer block.
3. **Where extras attach.** Tree-sitter places a comment in whichever node is being reduced when
   the comment is read. This depends on the LR automaton, not on the grammar text.
4. **Conflicts and precedence.** The Python grammar declares 9 conflicts, resolved by GLR and
   dynamic precedence. Examples are a tuple against a parenthesised expression, and `print` as a
   statement against a call.
5. **Tree shape.** Hidden rules (names starting with `_`) and inlined rules leave no node. Aliases
   rename nodes. Fields label edges. Supertype memberships come from hidden supertype nodes on the
   derivation path.
6. **Error recovery is out of scope.** Files with syntax errors go to tree-sitter. The new parser
   must detect them exactly: never accept a file tree-sitter rejects, nor the reverse.

## 6. Testing fidelity

The test compares, for every corpus file, the new parser's image bytes with
`TapeImage::encode(tree-sitter's tree).to_bytes()`. On a mismatch, it decodes both images and
reports the first node that differs, with its source position. Files that tree-sitter parses with
errors are checked only for agreement on "has an error".

The corpora are:

- the data-layer repository's Python and JSON (counts above);
- CPython's standard library;
- each grammar's own tree-sitter test corpus.
