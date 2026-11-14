# 0178: JavaScript, TypeScript and TSX are read by a lean parser that writes tree-sitter's tape, or refuses

Status: built (user, 2026-11-13: "Now that's done, I want a lean parser for JS / Typescript / JSX,
using exactly the same approach"). Drafted as 0170; renumbered past the highest number in any
worktree (0170 is another worktree's).

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, a code a node (kind, field,
supertypes). For JavaScript, TypeScript and TSX, the tape came from tree-sitter's parse and then an
encoding pass over the tree.

The lean Python parser (0163, 0169) showed a cheaper way: a parser that writes the tape directly,
node for node the tape tree-sitter's tree encodes to, and refuses any file it cannot read exactly.
This decision builds the same parser for the three ECMAScript dialects:

```
a .js/.jsx/.ts/.tsx file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the lexer: runs and stops by trailing zeros; every word one hash probe
  -> recursive descent over the dialect's tree-sitter grammar, writing the tape
       automatic semicolons: tree-sitter's external scanner, ported
       a syntax error, or a construct not read exactly -> refused -> tree-sitter
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery (0169's finding: exact recovery needs
tree-sitter's parse table and its cost competition).

## Prior work consulted

- **ast-index-spike indexes:** the spike parsed ECMAScript with tree-sitter only. There is no lexer
  or parser of its own to compare against.
- **0163, 0169 (the lean Python parser).** **Agrees and follows:** the same crate shape, stage 1,
  keyword hash, writer and oracle. 0169's "no error recovery: one behavior" is kept as is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`,
  and the engine takes the columns as written (`TapeImage::written`).
- **0162 (Typing-support: SIMD lexing techniques).** **Agrees:** nibble-table byte classes and a
  multiply-shift perfect hash over a masked 8-byte load.
- **The engine's implied-token automata (`tape::tokens::implied_tokens`).** **Followed:** a tape
  leaves out the tokens its kind implies. The parser stores exactly the tokens the automata keep.
  It does not write tokens and filter them afterwards ("tapes never generate implied tokens").

## Decision

### The crate: `rerail-ecma-lean`

`native/crates/ecma_lean` reads all three dialects with one parser (`Dialect::{JavaScript,
TypeScript, Tsx}`). TypeScript's part of the grammar is `parse/ts.rs`: types, annotations,
parameters, signatures, and the declarations JavaScript lacks. The differences between the dialects
are flags in the parser, not separate parsers:

- `.ts` reads `<T>x` as a type assertion;
- TSX reads `<` as an element's start, and a generic arrow only as `<T,>` or `<T extends …>`.

**Stage 1** (`block`) gives seven masks a 64-byte block:

- name bytes, blanks, line ends;
- the bytes that may end a run inside a string, a template, a block comment, a regular expression
  or a JSX text.

The masks are written once over `rerail-simd8`'s operations, so stage 1 runs on NEON, AVX2, SSE4.2
or the scalar fallback.

**The lexer** reads a token where the parser asks, in the mode the parse state reads: a `/` is a
regular expression only where an operand may start. Template text, JSX text and JSX tags are read in
their own modes, and the token buffer is cut where a mode changes (`relex_at`).

**Words** are all names. A word's code comes from one probe of a 1,024-slot perfect hash of its
first 8 bytes and its length. The multiplier is found at compile time, and a table where two of the
82 words share a slot does not build. The parser decides where a word is a keyword. Most of
TypeScript's words (`type`, `declare`, `readonly`, …) are names in other places.

### The tape, as tree-sitter's tree encodes it

A node's key is the parser's own: a kind, a field, a supertype set, and the extra flag. The engine
maps each to the dialect's grammar ids by name (`tape::ecma_lean`). One list of kinds serves all
three grammars. A kind a grammar lacks has no id there, and the parser never writes it in that
dialect.

**Stored tokens** come from a table, not from hand-written rules. The engine's automata decide which
tokens a tape keeps. The ignored test `ecma_implied_tokens` writes the tokens they leave out for
each dialect, as (parent, token, field) rows, to `implied.tsv`. `gen_kinds.py` turns those rows into
`kinds::DROPPED`, and the parser asks `stores(parent, token, field)` before it writes a token.
Delimiters are the exception. A delimiter is stored in some positions and not in others, and the
parser tells them apart. Examples:

- a trailing comma stores itself and the closing bracket;
- a string stores its closing quote only when it is empty;
- an array hole stores the delimiter before it and its own `,`.

**Supertypes** depend on where a node stands: its slot's context.

- In an expression, a primary expression is in both supertypes, `{expression,primary_expression}`.
- In a type position:
  - a primary type is `{type,primary_type}`;
  - in a primary-only position (an array type's element, a lookup type's object, `keyof`'s
    operand), it is `{primary_type}`;
  - function, constructor, readonly and infer types are `{type}`.

### Where tree-sitter's choices are kept

The tape must be tree-sitter's even where its grammar reads a text in a surprising way. These were
found by the oracle and are kept as tree-sitter reads them:

- **Automatic semicolons** come from tree-sitter's external scanner, ported. JavaScript's
  `scanner.c` and TypeScript's `common/scanner.h` differ: in TypeScript, only a block comment that
  holds a line end counts as a line break. The parser asks the scanner with the valid symbols of its
  own parse state.
- **`as` and `satisfies`** attach after a left-associative chain is read (`(a + b) as T`). Inside a
  right-associative operand they attach to the operand (`a ** (b as T)`).
- **A dotted type after `as`**, at a statement's level: `x as A.B.C` is `(x as A.B).C`, a member
  expression over the `as` expression. With one dot it stays a type.
- **`readonly`** forms a `readonly_type` over a whole type (`readonly A[] | B` is
  `readonly (A[] | B)`), and its token is not stored.
- **`new () => A & B`** is `(new () => A) & B`: both rules are `prec.left(0)`, and tree-sitter takes
  the reduction. The parser reads the constructor type's return as one operand and refuses an `&` or
  `|` after it.
- **In TypeScript**:
  - a member expression stores its `.`, also in a JSX element's name;
  - `?.` is an `optional_chain` node;
  - an optional call holds its `?.` directly;
  - `default` in an import or export specifier is an `identifier`;
  - a declared `undefined` is an `identifier`.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error;
- decorators;
- `unique symbol`;
- escapes in template literal types;
- `using` read as a name (tree-sitter's lexer takes it as the declaration keyword, and a name
  `using` can fail to parse);
- `await (…)` followed by a postfix (tree-sitter reads a call of a name `await`);
- import types followed by type arguments.

### In the engine

`tape::ecma_lean::Lean` is the `TreeParser` for each dialect. It is registered at rank 0 for
JavaScript (`lang-javascript`), and for TypeScript and TSX (`lang-typescript`). A file it refuses
goes to the dialect's tree-sitter grammar. So does a file with embedded languages, or one whose text
the language rewrites before parsing.

## Results

**The oracle** (`rerail-ecma-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets are corpus files from the benchmark repositories, and cut and
edited copies of them, each cut 3 times and edited 3 times:

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| JavaScript, 1,236 files | 1,235 / 0 / 1 | — |
| JavaScript, cut and edited (310 files) | 622 / 0 / 0 | 0 / 0 / 1,306 |
| TypeScript, 800 files | 790 / 0 / 9 | 0 / 0 / 1 |
| TypeScript, cut and edited (200 files) | 405 / 0 / 2 | 0 / 0 / 876 |
| TSX, 600 files | 600 / 0 / 0 | — |
| TSX, cut and edited (150 files) | 296 / 0 / 0 | 0 / 0 / 680 |

No tape differs: each is tree-sitter's or a refusal. The 9 TypeScript refusals include `unique
symbol`, `using` declarations, import types with type arguments, `await (…).x`, and the constructor
type above.

**Engine tests:**

- `ecma_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases in each dialect, byte for
  byte against tree-sitter's encoded tape. Syntax errors are refused.
- `the_product_reads_ecma_with_the_lean_parser`: the engine's parse route gives the lean tape.

**Cost** (`ecma_parse_costs`, the files the lean parser reads, best of 3 passes, profiling build,
instructions retired a node):

| dialect | files | nodes | lean: the tape | tree-sitter: parse | tree-sitter: parse and tape |
|---|---:|---:|---:|---:|---:|
| JavaScript | 1,235 | 2,261,121 | 757 | 4,812 | 8,407 |
| TypeScript | 790 | 845,099 | 613 | 4,718 | 8,020 |
| TSX | 600 | 492,775 | 602 | 5,995 | 9,446 |

Cycles a node: 158, 141 and 139 for the lean tape, against 1,808, 1,940 and 2,071 for tree-sitter's.
That is 11 to 16 times fewer instructions, and 11 to 15 times fewer cycles.

## Open

- **Decorators.** Refused for now (TypeScript and JavaScript). They are common in Angular code.
- **`using` declarations** and **`unique symbol`**: refused.
- **The JavaScript cost per node** (757) is above TypeScript's (613), though JavaScript has fewer
  bytes a node (5.1 against 6.2). Not yet profiled.
