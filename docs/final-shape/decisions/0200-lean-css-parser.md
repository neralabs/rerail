# 0200: CSS is read by a lean parser that writes tree-sitter-css's tape, or refuses

Status: built (user, 2026-11-14: "next we need PHP (including Blade), Markdown, HTML and CSS").
CSS became a product language in 0199 (`languages/css.json`, `lang-css`, tape tag 18), read by
tree-sitter-css until now.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node.

The lean parsers of 0163 through 0199 write that tape directly. Each one writes exactly the tape
tree-sitter's tree encodes to, and refuses any file it cannot read exactly. This decision builds the
same parser for CSS, at rank 0, before tree-sitter-css.

```
a .css file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> recursive descent over tree-sitter-css's grammar, writing the tape (no backtracking)
       the scanner's two questions (is white space a descendant combinator? is `:` a pseudo-class?)
       answered by a look ahead over bytes, as the scanner looks
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-css
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery.

## Prior work consulted

- **ast-index-spike indexes:** the spike's sample corpus (`ast-index-sample-corpus`) holds 2,009
  CSS files under 400 KB; the oracle's two sets are drawn from them.
- **0199 (lean HTML), 0198 (lean PHP), 0196, 0195, 0193, 0178, 0163, 0169, 0181.** **Agrees and
  follows:** the same crate shape, writer and oracle. "No error recovery: one behavior" is kept as
  is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`;
  a node found to wrap what came before it (a selector's combinator, a binary value) is put before
  its first child with `Writer::insert`, not by reading the text again.
- **The engine's implied-token automata (`tape::tokens`).** **Followed:** the parser stores exactly
  the tokens the automata keep (`implied.tsv`, `kinds::DROPPED`, and where the gap's state decides,
  by that state). It never writes a token to filter it out afterwards.
- **tree-sitter-css 0.25's grammar and external scanner** (`grammar.js`, `scanner.c`): the readings
  below follow them, checked by the oracle.

## Decision

### The crate: `rerail-css-lean`

`native/crates/css_lean`, in the shape of `rerail-html-lean`:

- `block`: stage 1. A mask a class for each 64-byte block: white space, an identifier's bytes, `*`
  (a comment's end), `\n`, and each quote's string stops. The parser's runs (white space,
  identifiers, comments, string pieces) are the lowest set bit of a block's mask.
- `parse`: the stylesheet, rule sets, at-rules (`@import`, `@media`, `@charset`, `@namespace`,
  `@supports`, `@scope`, `@keyframes`, any other), media and feature queries, selectors, blocks,
  declarations and values. Comments are read ahead with the white space before a token and written
  where the next node opens, as tree-sitter places extras.
- `gen_kinds.py`: writes `kinds.rs` from the token grammar and `implied.tsv`.

The grammar has no conflicts. Its scanner decides two things, and the parser asks the same two
questions over the bytes ahead, as the scanner reads them:

- **White space after a selector** is a descendant combinator where a selector's first byte follows
  it, or a `:` whose pseudo-class reading holds.
- **A `:`** is a pseudo-class's colon where a `{` comes before the next `;` or `}` (outside a
  comment), or the text ends first.

A selector is read left to right. `.class` binds to the operand before it; `:`, `::`, `#` and `[`
take the whole selector so far. Each such node is put before the selector already written.

### Where tree-sitter's choices are kept

These were found by the oracle, and the parser keeps each as tree-sitter reads it:

- **The lexer takes the longer token.** In a value, a number's unit that starts a longer
  `plain_value` is not the number's: `72px/1` is `72` and the `plain_value` `px/1`.
- **A string goes on past a line break.** A line break cannot start a `string_content`, so the lexer
  skips it as white space and the string's next piece starts after it.
- **A leading byte order mark** is skipped: the stylesheet starts after it.
- **Identifiers may hold U+00A0 to U+00FF** (the grammar's `[\xA0-\xFF]`); any other character past
  ASCII is refused.
- **Tokens stored by the gap's state, not by the kinds:**
  - a string's opening quote is stored, and its closing one only in an empty string;
  - a function's argument `,` is stored (a pseudo-class's arguments, written as `arguments` too,
    store none);
  - `*` is stored in `universal_selector`;
  - a combinator with no selector before it (`> div` in a nested block) is stored;
  - a keyframe's `to` holds its token; `from` holds none.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error. A keyframe selector that is a float (`66.48%`) is one: the grammar takes an
  integer there.
- a character past ASCII outside a string or a comment, except an identifier's U+00A0 to U+00FF;
  bytes that are not UTF-8;
- a namespace prefix in an attribute selector (`a[*|href]`).

### In the engine

`tape::css_lean::Lean` is CSS's `TreeParser`, registered at rank 0 under `lang-css`. A file it
refuses goes to tree-sitter-css.

## Results

**The oracle** (`rerail-css-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets:

- **Corpus:** 571 CSS files of the sample corpus, each whole, cut at 3 places, and with 3 lines cut
  short in the middle: 2,620 texts.
- **Second sample:** 400 other CSS files of the corpus, whole: 369 texts read.

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| corpus, cut and edited (2,620) | 684 / 0 / 2 | 0 / 0 / 1,934 |
| second sample (369) | 342 / 0 / 0 | 0 / 0 / 27 |

No tape differs. The two refused clean texts are the namespace prefix above.

**Engine tests:**

- `css_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape. Syntax errors are refused.
- `the_product_reads_css_with_the_lean_parser`: the engine's parse route gives the lean tape, and a
  refused file gets tree-sitter's.

**Cost** (`css_parse_costs`, profiling build, over the 469 files of the first set the lean parser
reads: 1.56 MB, 241,340 nodes):

| | instructions a node | cycles a node | ns a byte |
|---|---|---|---|
| lean: the parser's columns | 484 | 100 | 5.3 |
| lean: the tape | 502 | 113 | 5.7 |
| tree-sitter-css: parse | 4,493 | 1,059 | 54.1 |
| tree-sitter-css: the tape | 7,447 | 1,826 | 92.3 |

The lean tape costs a sixteenth of tree-sitter's, in cycles.

## Open

- **The refusals listed above.** Each is a file tree-sitter reads instead; none is a wrong tape.
- **CSS in HTML:** `style` elements and attributes read as CSS guest documents (0199's open item).
