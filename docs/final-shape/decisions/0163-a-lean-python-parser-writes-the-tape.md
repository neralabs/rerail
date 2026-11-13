# 0163: A lean Python parser writes tree-sitter-python's tape directly

Status: built (user, 2026-11-13: "I want to work on a lean parser for Python / we will eventually turn
it in to a bitlex parser, but for now I want a lean version just like the rerail one. Same deal: it
writes the tapes directly, no intermediate stuff, and uses the SIMD tricks")

## Intro

A Python file's tape came from tree-sitter-python: it parses into a tree, and the engine then walks the
tree and packs the tape (0140). Both steps cost several thousand instructions a node.

The `.rr` parser showed the other way: a hand-written parser that writes the tape as it parses, with no
tree in between. This decision does the same for Python. The parser is an interim one. Later it is to
become a Bitlex parser, generated from the grammar.

The tape it writes is tree-sitter-python's, node for node: the same kinds, fields, supertypes, extras
and stored tokens. So rules, matchers and findings see no difference. A file it cannot read (a syntax
error, or a construct it does not handle) goes to tree-sitter-python, which recovers from errors.

```
a Python file -> the lean lexer (64-byte blocks) -> tokens + layout + extras
              -> recursive descent, writing the tape (rerail_tape_format::Writer) -> the tape
                   refused
              -> tree-sitter-python -> its tree -> the tape (0140)
```

## Prior work consulted

- **ast-index-spike indexes:** the spike parsed with tree-sitter only; it has no lean parser to compare.
- **0140** (the tape's writer) and **0161** (the tape format, 11 to 15). **Agrees:** the lean parser uses
  the same writer, and stores only the tokens format 15 stores.
- **0148** (parsers routed by what the host runs). **Extended:** the lean parser is Python's rank-0
  implementation; tree-sitter-python stays the fallback.
- **0158** (JSONC read by simdjson). **Same shape:** a fast reader for the files it accepts, the general
  parser for the rest, tapes checked byte for byte against tree-sitter's.
- **String matching is chunked; no per-byte loops** (standing rules). **Agrees:** names, blanks,
  comments and string text are found 64 bytes a step, by NEON compares packed into a mask as simdjson
  packs them (a portable 8-bytes-a-word version elsewhere, checked against it).
- **Lessons ledger #10** (measure the code that runs). **Agrees:** costs are of the engine's own encode
  path, against tree-sitter-python's parse plus the engine's encode.

## Decision

The crate is `rerail-python-lean` (`native/crates/python_lean`). It depends only on the tape writer
and `unicode-ident`.

- **The lexer** is tree-sitter-python's lexer and external scanner in one pass. It gives the tokens,
  the layout tokens (NEWLINE, INDENT, DEDENT, zero width, where the scanner gives them), the pieces of
  strings and f-strings, and the extras (comments, line continuations) in a list of their own.
- **The parser** is recursive descent over the tokens. A node is written as it opens, with its kind,
  field and supertypes. A node found after its first child (an operator after its left operand) is
  put before it with `Writer::insert`.
- **The context gives the supertypes.** A slot is a field plus a context (an expression, a primary
  expression, a pattern, a parameter, or none), and the context decides which supertype sets a node
  gets. An assignment target, parsed as an expression, is turned into a pattern afterwards.
- **Extras** go where tree-sitter puts them: into the node open when the parse next opens a node or
  reads a token.
- **The engine side** (`tape::python_lean`) maps the parser's names to tree-sitter-python's ids once,
  and hands the columns to `TapeImage::written`. It is registered as Python's parser at rank 0.

Tree-sitter's choices the parser copies (each found by the oracle, below):

- A splat in a list or set is a `list_splat` around an expression. But a lone `[*name…]` or `{*name…}`
  with any postfix takes the alias form, with the postfixes on top.
- `type <expr> … =` on one line is a type alias.
- In a raw string, backslashes alone before the quote become part of `string_end`.
- `as` binds the else operand of a conditional expression.
- In an interpolation, a `:` at depth 0 always starts the format spec.
- `with (X as y):` with one item is a `parenthesized_expression` holding an `as_pattern`.
- A trailing `,` is stored in brackets; in an expression or pattern list only with two or more
  elements; and always at statement level.

Refused, and so read by tree-sitter-python: `print(a, *b)`, where tree-sitter's tree comes from a GLR
fork the parser does not follow.

## Results

The oracle (`oracle`, in the crate) compares the lean parser's tapes with tree-sitter-python's, as the
engine writes them (`python_tape_expect`), line by line:

| set | same | differ | refused |
|---|---:|---:|---:|
| 1,000 files | 1,000 | 0 | 0 |
| 3,916 files | 3,916 | 0 | 0 |
| 3,917 files (another sample) | 3,916 | 0 | 1 (`print('TRACE', *args)`) |
| 56 hand-written cases | 56 | 0 | 0 |
| 4,000 files cut at a random byte | 1,806 | 0 | 2,194 |

All 2,194 refused cut texts are ones where tree-sitter's tree has an error; none was read wrongly. The
engine's tests check the same through the product's path
(`python_tapes_from_the_lean_parser_are_tree_sitters`, `the_product_reads_python_with_the_lean_parser`).

Costs on arm64 (M5), profiling build, over the 1,000 files (16.9 MB, 2.26 M nodes), the best of three
passes (`python_parse_costs`):

| | instructions a node | cycles a node | ns a byte |
|---|---:|---:|---:|
| lean: the tokens alone | 228 | 58 | 1.8 |
| lean: the parser's columns | 584 | 129 | 4.1 |
| lean: the tape (the engine's path) | 604 | 141 | 4.4 |
| tree-sitter-python: the parse alone | 5,624 | 1,230 | 38.9 |
| tree-sitter-python: the tape | 8,912 | 1,966 | 62.5 |

The lean tape costs about 15 times fewer instructions than tree-sitter-python's. The lexer is under 40%
of it. The lexer reads a block in place rather than copying it, and checks for a line end before
running the layout scanner: 643 instructions a node before those two changes.

## Open

- **Bitlex.** The parser is to become a Bitlex parser, generated from tree-sitter-python's grammar; this
  one is its oracle and its cost to beat.
- **GLR forks.** `print(a, *b)` and any other construct where tree-sitter's tree comes from a fork are
  refused. They are rare (1 file in 3,917).
- **Error recovery.** Built in 0169: an error where a line is being typed is recovered as
  tree-sitter-python recovers it; other errors still go to tree-sitter-python.
