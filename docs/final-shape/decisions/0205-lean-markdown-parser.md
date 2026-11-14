# 0205: Markdown is read by a lean parser that writes tree-sitter-md's block tape, or refuses

Status: built (user, 2026-11-14: "next we need PHP (including Blade), Markdown, HTML and CSS").
Markdown became a product language in 0199 (`languages/markdown.json`, `lang-markdown`, tape tag
19), read by tree-sitter-md's block grammar until now. Its inline content stays open (0199).

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node.

The lean parsers of 0163 through 0200 write that tape directly. Each one writes exactly the tape
tree-sitter's tree encodes to, and refuses any file it cannot read exactly. This decision builds the
same parser for Markdown's block grammar, at rank 0, before tree-sitter-md.

Markdown differs from the other languages: its block structure is decided by tree-sitter-md's
external scanner, not by its grammar. So the parser keeps that scanner, ported line by line, and
asks it first at each token.

```
a .md file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the block grammar, top down, writing the tape (no backtracking)
       at each token: tree-sitter-md's scanner (ported), given the external tokens the state takes
       where it finds none: the grammar's own tokens (words, white space, punctuation)
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-md
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery.

## Prior work consulted

- **ast-index-spike indexes:** the spike's sample corpus (`ast-index-sample-corpus`) holds 4,326
  Markdown files under 400 KB; the oracle's sets are drawn from them.
- **0200 (lean CSS), 0199 (lean HTML), 0198 (lean PHP), 0196, 0195, 0193, 0163, 0181.** **Agrees and
  follows:** the same crate shape, writer and oracle. "No error recovery: one behavior" is kept as
  is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`;
  a setext heading, found at its underline, is put before its paragraph with `Writer::insert`.
- **The engine's implied-token automata (`tape::tokens`).** **Followed:** the parser stores exactly
  the tokens the automata keep (`implied.tsv`, `kinds::DROPPED`, and where the gap decides, by the
  gap). It never writes a token to filter it out afterwards.
- **tree-sitter-md's block grammar and external scanner**, as the engine vendors them
  (`native/grammars/tree-sitter-markdown/markdown/src/{grammar.json,scanner.c}`): the scanner is
  ported as it is; the readings below follow the grammar, checked by the oracle.

## Decision

### The crate: `rerail-markdown-lean`

`native/crates/markdown_lean`, in the shape of `rerail-css-lean`:

- `block`: stage 1. A mask a class for each 64-byte block: white space, a word's stops (white
  space, a line's end, ASCII punctuation), line ends, `` ` `` or a line end, tabs, and UTF-8
  continuation bytes. Words and white space runs are the lowest set bit of a block's mask; the
  scanner's skips over a line's rest are too, its column (a tab to the next stop of 4, a character
  one on) counted from the tab and continuation masks.
- `scan`: tree-sitter-md's external scanner, line by line: the stack of open blocks, matched at
  each line's start; the block starts (thematic breaks, list markers, ATX and setext markers, fences,
  HTML blocks' seven starts and ends, block quotes, pipe tables, metadata); the soft line break's
  look at the next line. A scan that finds nothing leaves the scanner as it was.
- `parse`: the document and its sections (by heading level), paragraphs, setext and ATX headings,
  lists and items (task markers), block quotes, fenced code (info strings: a language, the brace
  form), indented code, HTML blocks, link reference definitions on one line, pipe tables, metadata.
- `gen_kinds.py`: writes `kinds.rs` from the block grammar and `implied.tsv`.

### Where tree-sitter's choices are kept

These were found by the oracle, and the parser keeps each as tree-sitter reads it:

- **A node's extent** runs from its first token to its last, hidden tokens included (the grammar
  has no extras): a paragraph holds its line ending, a section runs to the next heading of its level
  or above.
- **A link reference definition wins over a paragraph** where it holds (its dynamic precedence in
  tree-sitter's GLR parse): a paragraph starting with `[` is read ahead as a definition first.
- **Tokens stored by the gap, not by the kinds.** The automata drop a gap's implied tokens (a
  label's `[` and `]`, a definition's `:`) only where every token of the gap is implied: `[a]` stores
  no bracket, `[a-b]` stores all three tokens. A title's opening quote is always stored, and so is
  the rest of the gap it starts; a closing quote alone after an escape is not.
- **A table's pipes:** the first and last of a row are stored, those between cells are not. A cell
  runs to the pipe after it, white space included; at a line's end, to its last token; at the
  text's end, to the end.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error. tree-sitter-md has one in these readings, which the parser refuses:
  - an HTML block that a blank line ends inside a list item or a block quote;
  - a label that starts with an escape (`[\<b>]`) and is not a definition;
  - a list marker that ends the text.
- a link reference definition that goes on past its line, or holds a `&` (an entity, which the
  grammar reads in its own way);
- an info string that starts with `,`;
- bytes that are not UTF-8, a NUL.

### In the engine

`tape::markdown_lean::Lean` is Markdown's `TreeParser`, registered at rank 0 under `lang-markdown`.
A file it refuses goes to tree-sitter-md. Its two fields (`heading_content`, `info_string`) map to
tree-sitter-md's ids by name.

## Results

**The oracle** (`rerail-markdown-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets:

- **Corpus:** 800 Markdown files of the sample corpus, whole.
- **Cut and edited:** 200 of them, each whole, cut at 3 places, and with 3 lines cut short in the
  middle: 1,230 texts.
- **Second sample:** 400 other Markdown files of the corpus, whole.

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| corpus (800) | 787 / 0 / 11 | 0 / 0 / 2 |
| cut and edited (1,230) | 1,194 / 0 / 16 | 0 / 0 / 20 |
| second sample (400) | 389 / 0 / 11 | 0 / 0 / 0 |

No tape differs. The refused clean texts are multi-line link reference definitions (the Rails
guides' `` [`name`]:`` then the URL on the next line) and definitions holding `&`.

**Engine tests:**

- `markdown_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape. The three error readings above are refused.
- `the_product_reads_markdown_with_the_lean_parser`: the engine's parse route gives the lean tape,
  and a refused file gets tree-sitter's.

**Cost** (`markdown_parse_costs`, profiling build, over the 787 corpus files the lean parser reads:
4.99 MB, 784,047 nodes):

| | instructions a node | cycles a node | ns a byte |
|---|---|---|---|
| lean: the parser's columns | 731 | 156 | 7.3 |
| lean: the tape | 747 | 162 | 8.2 |
| tree-sitter-md: parse | 13,961 | 2,817 | 128.5 |
| tree-sitter-md: the tape | 16,767 | 3,556 | 151.3 |

The lean tape costs about a twenty-second of tree-sitter's, in cycles.

## Open

- **The refusals listed above.** Each is a file tree-sitter reads instead; none is a wrong tape.
  Multi-line link reference definitions are the most common.
- **Markdown's inline content** (tree-sitter-md's inline grammar, as guest documents of the
  `inline` nodes): open, as in 0199.
- **The scanner's remaining byte loops** (a line's start, the table row count) run over a few bytes a
  line; the per-line rest is stage 1's.
