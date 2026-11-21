# 0199: HTML is read by a lean parser that writes tree-sitter-html's tape, or refuses

Status: built (user, 2026-11-14: "next we need PHP (including Blade), Markdown, HTML and CSS").
HTML, CSS and Markdown become product languages here, as Java and C# did in 0195; HTML is the
first of the three to get a lean parser.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node.

The lean parsers of 0163 through 0198 write that tape directly. Each one writes exactly the tape
tree-sitter's tree encodes to, and refuses any file it cannot read exactly. This decision does two
things:

- it registers HTML, CSS and Markdown as languages the product reads;
- it builds the lean parser for HTML, at rank 0, before tree-sitter-html.

```
an .html file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> stage 2: recursive descent over tree-sitter-html's grammar and its scanner's rules,
     writing the tape (no backtracking)
       implicit end tags, void elements, raw text: as the scanner decides them
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-html
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery.

## Prior work consulted

- **SQL index prototype indexes:** the prototype's sample corpus holds 1,044
  HTML files across its repositories; they are the oracle's two sets.
- **0195 (Java and C# become product languages).** **Followed:** the same registration steps, done
  for three languages at once (below).
- **0198 (lean PHP), 0196, 0195, 0193, 0178, 0163, 0169, 0181.** **Agrees and follows:** the same
  crate shape, stage 1, writer and oracle. "No error recovery: one behavior" is kept as is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`.
- **The engine's implied-token automata (`tape::tokens`).** **Followed:** the parser stores exactly
  the tokens the automata keep (`implied.tsv`, `kinds::DROPPED`). It never writes a token to filter
  it out afterwards.
- **tree-sitter-html 0.23's grammar and external scanner** (`grammar.js`, `scanner.c`, `tag.h`):
  the readings below follow them, checked by the oracle.

## Decision

### Three product languages

As fee488066 did for Java and C# (0195):

| language | file | extensions | tape tag | grammar |
|---|---|---|---:|---|
| HTML | `languages/html.json` | `.html`, `.htm` | 20 | ast-grep's tree-sitter-html |
| CSS | `languages/css.json` | `.css` | 21 | ast-grep's tree-sitter-css |
| Markdown | `languages/markdown.json` | `.md`, `.markdown` | 22 | tree-sitter-md 0.5.4's block grammar, vendored |

The tags were 17, 18 and 19 when drafted; the hub had given those to ESLint directives, Java and C# (merged
2026-11-14), so they are 20, 21 and 22 (Markdown's inline grammar 23).

- Each has a feature (`lang-html`, `lang-css`, `lang-markdown`) in every crate, a grammar entry in
  `capabilities.json`, a token grammar for the implied-token tables, and an adapter
  (`language/adapter/{html,css,markdown}.rs`). Every capability starts as not supported yet.
- The test-only `Html` dialect that Vue and Svelte embeddings used is renamed `TestHtml`, so the
  product's `Html` is one definition.
- A Markdown file's tape is its block grammar's. Its inline content (tree-sitter-md's second
  grammar) is not read yet; it is an open item.

### The crate: `rerail-html-lean`

`native/crates/html_lean`, in the shape of `rerail-php-lean`:

- `block`: stage 1. A mask a class for each 64-byte block: a text's stops, ASCII white space, a tag
  name's bytes, an attribute name's and value's stops, `<`, `>`, each quote, bytes past ASCII.
- `tags`: tag names as the scanner reads them (`tag.h`). Each of its 126 names is a type of its own
  (one binary search over packed names); any other name is a custom tag. It also holds the void
  elements and which element may contain which (`tag_can_contain`).
- `parse`: the document, elements with their implicit ends, start and end tags, attributes,
  comments, the doctype, raw text (`script`, `style`), entities and text.
- `gen_kinds.py`: writes `kinds.rs` from the token grammar and `implied.tsv`.

### Where tree-sitter's choices are kept

These were found by the oracle, and the parser keeps each as tree-sitter reads it:

- **Implicit end tags** follow the scanner's stack: an element ends where a start tag it may not
  contain opens (`<p>` before `<div>`), where an end tag of an element further out closes, and at
  the text's end for `html`, `head`, `body` and the elements that may not contain a custom tag.
- **An empty `script` or `style`** has a zero-width `raw_text`.
- **A leading byte order mark** is skipped: the document starts after it.
- **White space is ASCII only.** A no-break space (U+00A0) is text, both for the grammar's `\s`
  and for the scanner.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error. An end tag with no open element (`</div>` at the top level) is one: tree-sitter
  reads it as an `ERROR`.
- an element left open at the text's end that the scanner would not close there (`<p>` alone);
- `<` followed by white space.

### In the engine

`tape::html_lean::Lean` is HTML's `TreeParser`, registered at rank 0 under `lang-html`. A file it
refuses goes to tree-sitter-html.

## Results

**The oracle** (`rerail-html-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets:

- **Corpus:** 645 HTML files of the sample corpus, each whole, cut at 3 places, and with 3 lines
  cut short in the middle: 3,831 texts.
- **Second sample:** 399 other HTML files of the corpus, whole: 395 texts read.

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| corpus, cut and edited (3,831) | 1,526 / 0 / 0 | 0 / 0 / 2,305 |
| second sample (395) | 377 / 0 / 0 | 0 / 0 / 18 |

No tape differs. Every clean text is read; every text with a syntax error is refused.

**Engine tests:**

- `html_css_and_markdown_files_are_read_by_their_grammars`: each new language's file is read by
  its grammar, through the product's registry.
- `html_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape. Syntax errors are refused.
- `the_product_reads_html_with_the_lean_parser`: the engine's parse route gives the lean tape, and a
  refused file gets tree-sitter's.

**Cost** (`html_parse_costs`, 609 of the corpus files the lean parser reads, 151,648 nodes; best of
3 passes; profiling build; load near 100):

| step | instructions a node | cycles a node | ns a byte |
|---|---:|---:|---:|
| lean: the parser's columns | 250 | 58 | 2.1 |
| lean: the tape | 281 | 89 | 2.5 |
| tree-sitter: parse | 4,550 | — | — |
| tree-sitter: parse and tape | 7,386 | 1,707 | 47.1 |

The lean tape takes 26 times fewer instructions and 19 times fewer cycles than tree-sitter's parse
and tape.

## Open

- **`script` and `style` contents:** closed by 0238, which reads them as JavaScript, JSON and CSS
  guest documents.
- **Markdown's inline content:** closed by 0238 (guest documents of tree-sitter-md's inline grammar).
- **The refusals listed above.** Each is a file tree-sitter reads instead; none is a wrong tape.
- **CSS and Markdown lean parsers:** 0200 and 0201.
