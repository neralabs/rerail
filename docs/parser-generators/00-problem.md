# Parser generators: the problem

Status: problem statement, 2026-11-06. No decisions yet; open questions are at the end.

## Intro

Rerail turns every source file into a *tape*: the file's syntax tree in preorder, stored as
columns. Today a tape is built in two steps. A tree-sitter parser builds a tree, then the encoder
walks that tree with a cursor and writes the tape.

Parsing is now the largest single cost of a cold audit. This page measures it, describes how
simdjson parses JSON a hundred times faster, and sets out what it would take to do the same for
tree-sitter grammars. The aim is a tape written straight from the source bytes by data-parallel
primitives, on CPU SIMD and on Metal.

The first languages are JSON, which has a known solution, and Python, which is fairly regular.
All tree-sitter languages come later.

## 1. What parsing costs today

Measured 2026-11-06. The corpus is the data-layer repository, without `.venv`, `node_modules`
or `.git`. The parsers are tree-sitter 0.25.10 with the tree-sitter-python 0.25.0 and
tree-sitter-json 0.24.8 crates. Times are user+sys CPU on one thread. Machine load was about 47,
so absolute times are pessimistic.

| | Python | JSON |
|---|---:|---:|
| files | 13,088 | 14,669 |
| source | 171.0 MB | 567.7 MB |
| parse CPU | 12.24 s | 23.30 s |
| throughput | 14.0 MB/s (72 ns/byte) | 24.4 MB/s (41 ns/byte) |
| cursor walk over the trees | 3.32 s | 13.02 s |
| nodes | 34.2 M (0.20 per byte) | 169.2 M (0.30 per byte) |
| named nodes | 65.1% | 45.0% |
| leaves (tokens) | 66.3% | 72.8% |
| files with an error or missing node | 46 (0.35%) | 1 |
| deepest node | 83 | 42 |

The cursor walk only counts nodes. It is a lower bound on the encoder's cost, which also writes
the tape.

An earlier run on 5,725 corpus files (95.7 MB), recorded in `docs/transducer-design.md` on the
final-shape branch, agrees. Tree-sitter parsing took 8.65 s; tape encoding and restoring took
another 4.57 s.

The cold-audit target is 1–2 s for the whole repository. Parsing the Python alone takes 12 s of
CPU, about 1.5 s on eight cores, before encoding. For comparison, simdjson parses JSON at
2–3 GB/s on one core, about 100 times tree-sitter-json's rate.

## 2. What a tape must hold

The fidelity target is the tape that the current pipeline writes. Each node stores:

- **Code.** A per-tape code standing for the node's kind, its incoming field, its flags (named,
  extra, missing, error), any contextual alias, and its supertype memberships. Memberships are
  recorded as the tree-sitter cursor reports them, and they depend on how the node was derived,
  not only on its kind.
- **Size.** The number of nodes in its subtree. This gives the end of the subtree in preorder.
- **Span.** The node's start and end bytes.

The tape includes every node tree-sitter emits: anonymous tokens such as `(` and `,`, extras
such as comments, and `ERROR` nodes. Two thirds of all nodes are leaves, so most of a tape is the
token stream. The other third is the structure that needs parsing.

## 3. How simdjson does it

simdjson (Langdale and Lemire, "Parsing Gigabytes of JSON per Second", 2019) parses in two
stages.

**Stage 1 finds the structure with bitmasks.** It reads 64 bytes at a time. For each block it
computes one 64-bit mask per fact:

```text
input       {"a":"x\"y", "b":[1,2]}
backslash   .......1...............   byte is '\'
escaped     ........1..............   odd run of backslashes before it
quote       .1.1.1...1..1.1........   '"' and not escaped
in_string   .11.11111..11..........   prefix XOR of quote (a carry-less multiply)
structural  1...1.....1....1.1.1.11   {}[]:, outside strings
```

The quote mask becomes an "inside a string" mask with a prefix XOR. Structural characters inside
strings are masked off with one AND. The set bits are then turned into a list of byte offsets
(the *structural index*) by a tight loop of count-trailing-zeros. Character classes come from two
table lookups on the byte's high and low nibbles, a single shuffle instruction.

**Stage 2 writes the tape.** It walks the structural index with a small state machine and writes
one 64-bit tape word per value. An opening bracket's word holds the tape position of its matching
close. That is the same information as a Rerail tape's subtree size.

**On a GPU** the same tape falls out of scans. Nesting depth is a prefix sum of +1 for each open
and −1 for each close. Matching brackets are pairs at the same depth. A node's tape ordinal is a
prefix count of node starts before it.

## 4. Why it works for JSON, and what that means for Python

simdjson works because JSON's tokens alone decide the tree. An opening bracket always opens a
node and a closing one always closes it, whatever came before. Grammars with this property are
called *input-driven* or *visibly pushdown*. Their parsing reduces to bitmasks, scans and
pairing.

Python, after lexing, is mostly like that:

- **Brackets** `()`, `[]` and `{}` nest visibly.
- **Strings** open and close visibly once their prefix and quotes are known. f-strings nest
  expressions inside `{}`.
- **Blocks** nest visibly once the lexer has produced INDENT and DEDENT tokens. Newlines inside
  brackets are not tokens.

What is left is the flat token sequence inside one bracket or one logical line, which needs a
grammar. Most of it is expressions, which operator-precedence parsing handles. Operator-precedence
grammars can be parsed locally, so they parallelise (Barenghi, Crespi Reghizzi et al., PAPAGENO,
2015). The rest is a short statement-level grammar per logical line. Each of these segments is
independent once the nesting is known, so they parse in parallel.

That suggests five layers. Each one is a data-parallel primitive or a set of independent small
tasks.

| Layer | Work | Primitive |
|---|---|---|
| 0. classes | byte → character class | table lookup per byte (shuffle on CPU) |
| 1. lexing | tokens, strings, comments, numbers | DFA transition maps composed by a prefix scan; prefix XOR for the simple quote case |
| 2. nesting | bracket depth, logical lines, INDENT/DEDENT | prefix sum; masking; all-nearest-smaller-values for the indentation stack |
| 3. segments | grammar inside each bracket or line | independent small automata, generated from the grammar |
| 4. tape | ordinals, sizes, spans, codes | prefix count, then scatter |

Pareas (Voetter, Huijben and Rietveld, 2022) is a GPU compiler that lexes and parses a C-like
language this way: lexing by composed state machines, parsing by bracket matching and prefix
sums. It shows the approach is practical on a GPU.

## 5. Where tree-sitter fidelity is hard

Each of these must reproduce tree-sitter's output exactly, or fall back to tree-sitter.

1. **Context-aware lexing.** Tree-sitter lexes with a mode chosen by the parser state. Python's
   soft keywords (`match`, `case`, `type`, `print`, `exec`) are keywords in some places and
   identifiers (`keyword_identifier`) in others. A lexer that runs first has to leave them
   undecided for layer 3.
2. **The external scanner reads the parser's state.** `scanner.c` (15.7 KB) produces NEWLINE,
   INDENT, DEDENT, string tokens and comments. It asks which tokens the parser would accept
   (`valid_symbols`). "Inside brackets" is read that way, not by counting brackets. So is the
   rule for whether a comment before a dedent belongs to the inner or the outer block. Each such
   rule needs a lexical equivalent that agrees on every file.
3. **Where extras attach.** Tree-sitter places comments in whichever node is being reduced when
   they are read. This depends on the LR automaton, not on the grammar text.
4. **Conflicts and precedence.** The Python grammar declares 9 conflicts, so tree-sitter parses
   parts of it with GLR, and uses dynamic precedence to choose. Examples are a tuple against a
   parenthesised expression, and `print` as a statement against a call.
5. **Tree shape.** Hidden rules (names starting with `_`) and inlined rules leave no node.
   Aliases rename nodes. Fields label edges. Supertype memberships come from hidden supertype
   nodes on the derivation path.
6. **Error recovery is out of scope.** Recovery is heuristic and cannot be matched in general.
   Only 0.35% of Python files have an error or missing node, so those files can go to tree-sitter.
   The fast parser must then never accept a file tree-sitter rejects, and the reverse.

## 6. The grammar format

The format needs a textual form. It should hold:

- token definitions as regular expressions, with character classes;
- nesting pairs: brackets, string delimiters, and a layout rule for indentation;
- productions with fields, aliases, hidden rules, supertypes and precedence, named as in
  tree-sitter so that the tape's codes follow directly;
- the scanner's behaviour as declarations: an indentation stack, a delimiter stack for strings,
  and newline suppression inside brackets.

Tree-sitter grammars are JavaScript, but each one ships its compiled `src/grammar.json`. An
importer can read that JSON without Node. It gets everything except the external scanner, which
is C. For Python the scanner would be written by hand in the format's scanner declarations. The
importer should classify each rule into a layer and report the rules that fit no layer.

## 7. Transducers that write tapes

Rerail's transducers today read tapes. They walk a tape and emit matches, and they run on the CPU,
an arm64 JIT and Metal (`docs/transducer-design.md` on the final-shape branch). They have no scan,
compaction or pairing primitive. The device side of chunked text matching (open item O66: a state
map per chunk, composed by a prefix scan) is planned but not built.

A parser needs *stream* transducers that read bytes and write tapes. Their primitives:

| Primitive | CPU lowering | Metal lowering |
|---|---|---|
| map: per-byte class or flag | NEON table lookup (`vqtbl1q_u8`) | one lane per byte |
| scan with an associative operator: sum, XOR, max, composition of small transition maps | carry-less multiply (`vmull_p64`) for XOR; word-level prefix ops | `simd_prefix_exclusive_sum`, threadgroup scan, decoupled look-back across groups |
| segmented scan | the same, with segment flags | the same |
| compaction: mask → offset list | count-trailing-zeros loop | prefix count, then scatter |
| pairing: match open with close | depth stack | sort or bucket by depth |
| segment automaton | table-driven loop per segment | one thread or SIMD group per segment |
| tape scatter | sequential write | prefix count gives each node's slot |

Combining transducers here means fusing adjacent maps and scans into one pass over a block, as
simdjson's stage 1 does. Building the device scan once also covers O66.

## 8. Testing fidelity

The test compares, for every corpus file, the tape from the new parser with the tape from
tree-sitter and the current encoder. They must hold the same nodes: code fields, sizes and spans.
A mismatch reports the first node that differs and its source position. Files that tree-sitter
parses with errors are checked for agreement on "has an error" only.

The corpora are the data-layer repository's Python and JSON (counts above), CPython's standard
library, and each grammar's own tree-sitter test corpus.

## 9. A possible first sequence

1. **Harness.** A tape differ and baseline CPU timings for tree-sitter parse plus encode, per
   language, on the final-shape tape format.
2. **JSON.** Write the grammar by hand in the new format. Build a scalar reference interpreter of
   the stream primitives, then NEON, then Metal. Exit when every JSON file's tape matches and CPU
   throughput is at least 1 GB/s.
3. **Python.** Build the `grammar.json` importer and write the scanner declarations. Lower layer
   by layer, and track the share of files with identical tapes until it reaches every file
   tree-sitter parses without error.

## 10. Open questions

1. **Which branch.** This worktree is on `main`, whose tapes are format 7. The final-shape branch
   (`jeremybarnes/final-rerail-shape`, 585 commits ahead) has format 11 (v9np). The parser should
   target v9np, so this work should probably start from that branch.
2. **What "identical" means.** Byte-identical tape images, or the same node sequence with the
   per-tape code table numbered canonically? The code table is numbered per tape, in order of
   first use.
3. **Error files.** Is falling back to tree-sitter acceptable for files with errors (0.35% of
   Python)?
4. **Textual syntax.** A new syntax of its own, or one close to tree-sitter's DSL so that
   imported grammars read as their sources do? Node is not on the path in either case.
5. **Layer 3's form.** Generated precedence parsers per segment are fastest. Tree-sitter's own LR
   tables, run per segment from a known start state, would make fidelity easier to reach. A
   possible order is LR tables first, then precedence parsers where they match.
