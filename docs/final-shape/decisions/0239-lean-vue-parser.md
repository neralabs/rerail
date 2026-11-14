# 0239: Vue files are read by a lean parser, and their embedded programs are built from the tape

Status: decided (user, 2026-11-14: "please do both of these", on lean parsers for Markdown's inline
grammar and for Vue).

## Intro

A Vue single-file component (`.vue`) is two documents in the engine:

- the **host**: the file's own tree, in tree-sitter-vue's grammar (blocks, elements, attributes,
  directives, interpolations);
- the **embedded program**: the component's `<script>` blocks and its template's expressions,
  written out as one JavaScript or TypeScript program (`vue::view`, `embed::script_block`).

Until now, both came from tree-sitter: tree-sitter-vue parsed the host, and the view read
tree-sitter's tree to build the program. This decision does two things:

- **the view reads the host's tape**, not tree-sitter's tree (`embed::host`). Every embedder now
  works from the tape, so whichever parser read the host, the same program is built;
- **a lean parser reads the host** (`rerail-vue-lean`), registered at rank 0 for
  `GrammarDialect::Vue`. A file it refuses goes to tree-sitter-vue.

```
a .vue file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8)
  -> stage 2: one pass over tree-sitter-vue's grammar and its scanner's rules, writing the tape
  -> the host's tape
       -> FileImage::embedding: the view, read from the tape in place -> the program
```

There is one behavior, as for every lean parser: tree-sitter's tape, or a refusal.

## Prior work consulted

- **0199 (lean HTML).** **Followed:** the crate's shape, stage 1, the tag stack and its implicit
  ends. tree-sitter-vue's scanner reads tags with the same `tag.h` as tree-sitter-html's (it
  differs in layout only), so the tag rules are one definition: `rerail_html_lean::tags`, now
  public, with `TEMPLATE` added.
- **0234 (lean Markdown inline), 0226 (lean SQL).** **Followed:** the engine's token automata
  (`automata.tsv`, `kinds`' tables) decide which tokens the tape keeps. **Departs from 0234:** this
  parser has one reading, so it writes as it goes. Each node with an automaton holds the tokens of
  its open gap until its next named child or its end comes, then decides the gap as the engine's
  `stored_children` does. No token is written and then removed.
- **"Tape read in place."** **Followed:** the embedders read the host's tape where it lies
  (`TapeImage`'s columns: codes, sizes, starts, tails). Nothing is decoded or rebuilt. A parent is
  found by stepping down from the root, and a node's children by skipping each one's subtree.
- **"Language-generic interfaces."** **Followed:** `Embedder::build` takes a host node of the tape,
  in `rerail-lang`; Vue's embedder (in `rerail-lang-ecma`) and the tests' (`TestScript`) read it.
- **tree-sitter-vue's `scanner.c` (ce8011a), and tree-sitter's lexer, read for this with a
  logging probe over the generated parser.** **Followed:** every rule below.

## Decision

- **Embedders read the tape.** `Embedder::build(root: HostNode, source)`. `HostNode` answers
  what an embedder asks: a node's kind, named or not, its bytes, its named children, its parent
  and named siblings. The tape stores every named node; the tokens it leaves out are anonymous,
  and no embedder reads one. `FileImage::embedding` runs the embedders on a host tape. The
  tree-sitter path (`TapeImage::encode`) and the lean one both call it.
- **Texts are the scanner's.** A text runs to a `<` followed by a letter, `!`, `?` or `/`, or to
  `{{`. At a line break, it ends before the break when white space and then `<` follow; `>` there
  is an error. White space a text starts with is the text's own, unless a line break comes before
  anything else. A carriage return not followed by a line feed takes the byte after it with it.
  These are the scanner's cases one for one (`text`), and the stops are stage 1's mask.
- **Interpolations.** `{{`, then the scanner's text to `}}`, which may be none, then `}}`.
- **Elements.** As in HTML, with two differences from tree-sitter-vue's grammar:
  - a `<template>` element never ends implicitly: an implicit end tag is a token only where an
    `element`'s children may end;
  - an element still open at the file's end is an error, because the scanner finds no token there.
- **Directives.** A directive is one of:
  - `v-name`, with an optional `:argument`;
  - shorthands (`:a`, `@a`, `.a`, `#a`) with their arguments.
  
  Either is followed by optional `.modifiers` and a value. A `.` right after an argument starts
  modifiers. A shorthand after white space continues the attribute while it has no value
  (`:a :b="c"` is one attribute, as the grammar's `repeat1` and tree-sitter's lexer read it).
  An argument may be `[dynamic]`.
- **Stored tokens:** a quoted value's opening quote, and a shorthand's `:`, `@`, `.` or `#`.
  Everything else is implied: closing quotes, `<`, `>`, `</`, `/>`, `=`, `{{`, `}}`, a
  modifier's `.`, the `:` after a directive's name, and a dynamic argument's brackets. These come
  from the automata, not from a list.
- **Raw text** (a script's, a style's) runs to `</script` or `</style`, in any case, with the
  scanner's quirk kept: a byte that breaks a partial match is passed over. Each `<` is compared
  with the delimiter's 8 bytes at once (`upper`, a word's letters made upper case).
- **Refused, rather than guessed:**
  - every syntax error;
  - a NUL;
  - white space past ASCII where it would end a token (the grammar's `\s`), for example a
    no-break space in an attribute;
  - a character past ASCII where the scanner asks the locale (`iswalnum` after a tag name,
    `iswalpha` after `<`, `towupper` in raw text);
  - `<` followed by white space.

## Results

Oracle (`oracle`, against tree-sitter's tapes of the sample corpus's `.vue` files):

| Set | Files | Same | Differ | Refused | With errors (all refused) |
|---|---|---|---|---|---|
| The corpus's `.vue` files | 2,630 | 2,629 | 0 | 0 | 1 |
| Each also cut at 4 places (`RERAIL_PY_CUTS=4`) | 13,146 | 2,651 | 0 | 0 | 10,495 |

The engine's Vue tests (the view's corpus of 958 eslint-plugin-vue fixtures, its oracle, and the
embedding tests) pass with the view reading the tape.

Cost (`vue_parse_costs`, the 2,629 files both read, 10.8 MB, 524,053 host nodes; best of three
passes, load average about 75):

| Step | Instructions a host node | Cycles a host node |
|---|---|---|
| lean: the tape | 635 | 185 |
| tree-sitter: parse alone | 6,046 | 1,405 |
| lean: the documents (the tape and the program built from it) | 26,288 | 7,375 |
| tree-sitter: the documents | 34,675 | 9,560 |

The host's tape costs 9.5 times fewer instructions than tree-sitter's parse alone. A file's
documents cost 24% fewer: most of their cost is the embedded program, which the view parses with
tree-sitter's JavaScript or TypeScript grammar.

## Open

- **The embedded program is parsed by tree-sitter.** The view writes the program's text, then
  parses it with tree-sitter-javascript or -typescript (`View::build`), about 25,000 instructions
  a host node. `embed::Parse::Tape` already carries a lean parser's tape of a program, so the
  lean ECMAScript parser (0163) could read it.

- **White space past ASCII in tags** is refused. Reading it would mean deciding, for each such
  character, whether the grammar's `\s` and the scanner's `iswspace` agree.
- **Comments inside a tag, `<?`, and `<` followed by white space** are refused; none is in the
  corpus.
