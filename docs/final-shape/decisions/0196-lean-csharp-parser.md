# 0196: C# is read by a lean parser that writes tree-sitter-c-sharp's tape, or refuses

Status: built (user, 2026-11-13: "the next three parsers I want are Rust, C-sharp and Java"; the
order Rust, Java, C#). C# has been a product language since 0195 (`languages/csharp.json`, the
`lang-csharp` feature, tape tag 16), read by tree-sitter-c-sharp until now.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node (kind,
field, supertypes).

The lean Python, ECMAScript, Rust and Java parsers (0163, 0169, 0178, 0193, 0195) write the tape
directly. Each writes exactly the tape tree-sitter's tree encodes to, and refuses any file it cannot
read exactly. This decision builds the same parser for C#, at rank 0 before tree-sitter-c-sharp.

```
a .cs file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the lexer: runs and stops by trailing zeros; every word one hash probe
       a string, verbatim, raw or interpolated literal is one token over the whole literal
       a directive (#if, #region, …) is one token to its line's end
  -> recursive descent over tree-sitter-c-sharp's grammar, writing the tape (no backtracking)
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-c-sharp
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery. There are no modes.

## Prior work consulted

- **ast-index-spike indexes:** the spike's sample corpus (`ast-index-sample-corpus`) holds
  `runtime` and `PowerShell`. 820 of their files are one of the oracle's sets below.
- **0195 (lean Java), 0193 (lean Rust), 0178 (lean ECMAScript), 0163 and 0169 (lean Python), 0181
  (their profile).** **Agrees and follows:** the same crate shape, stage 1, keyword hash, writer,
  token buffer and oracle. "No error recovery: one behavior" is kept as is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`,
  and the engine takes the columns as written (`TapeImage::written`).
- **The engine's implied-token automata (`tape::tokens::implied_tokens`).** **Followed:** the
  parser stores exactly the tokens the automata keep (`implied.tsv`, `kinds::DROPPED`, and a gap's
  tokens by its shape). It never writes a token to filter it out afterwards.
- **tree-sitter-c-sharp 0.23.5's grammar and external scanner** (`grammar.js`, `src/scanner.c`):
  the readings below follow them, checked by the oracle.

## Decision

### The crate: `rerail-csharp-lean`

`native/crates/csharp_lean`, in the shape of `rerail-java-lean`:

- `block`: stage 1, masks a 64-byte block (name bytes, blanks, line ends, a string's stops, a
  block comment's stops).
- `lex`: tokens where the parser asks, with no state. A word's code comes from one probe of a
  4,096-slot perfect hash; the parser decides where a word is a keyword (C#'s contextual keywords
  are names everywhere else). A byte order mark is white space.
- `parse`: declarations, statements, expressions (precedence climbing over tree-sitter-c-sharp's
  `PREC`), types, patterns and preprocessor directives.
- `gen_kinds.py`: writes `kinds.rs` from the token grammar and `implied.tsv`.

The parser never backtracks: every choice is made by looking ahead over tokens (`scan_type`,
`local_ahead`, `cast_ahead`, `lambda_ahead`, `is_type_ahead`, …).

### Where tree-sitter's choices are kept

The tape must be tree-sitter's even where its grammar reads a text in a surprising way. These were
found by the oracle, and the parser keeps each as tree-sitter reads it:

- **Constant patterns take the operators after them.** At the start of a chain, `x is null || y`
  is `x is (null || y)`: the constant absorbs the binary operators after it, unless another `is`
  in the range reads otherwise. A name before a binary operator, where the operator binds tighter
  than the slot allows, is an `is_expression` (a type). Generic names count as names.
- **Casts.** `(a) && b` is a cast of `&(&b)` (the `&&` split in two). `(T)[]` is a cast.
- **`foreach (A<B> x in …)`** is the comparison `A < B > x`.
- **`a.b->c`**: the left of `->` is a `qualified_name`, rebuilt from the member access.
- **Bare `this` and `base`.** An expression that is only `this` or `base` stores every token of its
  gap (`(T)this` stores its `)`).
- **Lambdas.** A bare lambda's first parameter `ref T` is a `ref_type`; `scoped Name x` is a
  `scoped_type`.
- **Declarations.** A record's base names are bare names. `unmanaged`, `struct` and `notnull`
  before `where` are constraint tokens. A type declaration's trailing `;` is dropped.
  `await f();` is an expression, never a local.
- **Preprocessor directives.** A directive ends after its line's `\n`s (a `\r` is in the text). A
  region's argument that starts with `//` is a comment. `#if` inside an attribute list is
  `preproc_if_in_attribute_list`.
- **Interpolated strings** are cut as the scanner cuts them: a content run ends before any `{` that
  opens an interpolation, a run starting at `{{` takes them, and the closing `}` token includes the
  blanks before it.
- **Stored tokens that follow from a gap's shape:**
  - a list pattern gap that holds `..` (or a trailing `,`) stores its tokens;
  - an anonymous object of bare names with a trailing `,` stores every `,` and the `}`;
  - an `#else` in an attribute list is stored.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error;
- shapes where tree-sitter's GLR reading depends on more than the parser looks at:
  - long `is` chains whose constants would absorb other `is` tests;
  - a dotted name before an operator after `is`;
  - an `is` chain a conditional access wrapped (`a || b?.c != d || e is A || …`);
  - a range ending at a conditional `?` with operators inside;
- shapes tree-sitter reads with an error, which a refusal hands to its recovery:
  - `async` as a name in an expression (`x = async;`);
  - a bare lambda whose first parameter is `in` or `out` (`(in T x) => …`, read as a tuple);
  - `..]` and `.. pattern` in a list pattern;
- constructs not yet read: `*p = …` statements, casts in case labels (`case (char)0x0A:`), `#if`
  splitting an `if`/`else`, attributes on type parameters, `new (A, int)[n]`,
  `async void (x) =>`, `.@name`, a tuple pattern arm `(false, …) =>`, `is nameof(…)`, and
  explicit interface members named by `global::`.

### In the engine

`tape::csharp_lean::Lean` is C#'s `TreeParser`, registered at rank 0 under `lang-csharp`. A file it
refuses goes to tree-sitter-c-sharp.

## Results

**The oracle** (`rerail-csharp-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets:

- **ast-index corpus:** 881 files of `runtime` and `PowerShell` (61 with tree-sitter errors).
- **Stored repositories:** 640 files of 69 repositories (`~/nera/sampo-repos`, 12 a repository,
  by `tmp/corpus/extract.sh`; 23 with tree-sitter errors).
- **Cut and edited:** 500 of those files (250 of each set), each cut at 3 places and with 3 lines
  cut short in the middle.
- **Probes:** 67 hand-written cases.

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| ast-index corpus, 881 files | 795 / 0 / 25 | 0 / 0 / 61 |
| stored repositories, 640 files | 588 / 0 / 29 | 0 / 0 / 23 |
| cut and edited (500 files) | 806 / 0 / 39 | 0 / 0 / 1,967 |
| probes, 67 cases | 65 / 0 / 0 | 0 / 0 / 2 |

No tape differs: each is tree-sitter's or a refusal. 96.2% of the clean files are read (1,383 of
1,437); the refused ones go to tree-sitter, and the list above names what they hold.

**Engine tests:**

- `csharp_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape. Syntax errors are refused.
- `the_product_reads_csharp_with_the_lean_parser`: the engine's parse route gives the lean tape, and
  a refused file gets tree-sitter's.

**Cost** (`csharp_parse_costs`, the 1,383 corpus files the lean parser reads, 19.3 MB, 2,325,697
nodes; best of 3 passes; profiling build; load 20 to 33):

| step | instructions a node | cycles a node | ns a byte |
|---|---:|---:|---:|
| lean: the parser's columns | 512 | 117 | 3.3 |
| lean: the tape | 538 | 131 | 3.7 |
| tree-sitter: parse | 9,289 | 1,876 | 52.6 |
| tree-sitter: parse and tape | 12,597 | 2,588 | 72.5 |

The lean tape takes 23 times fewer instructions and 20 times fewer cycles than tree-sitter's parse
and tape.

## Open

- **The refusals listed above.** Each is a file tree-sitter reads instead; none is a wrong tape.
- **C#'s capabilities** are `not-supported-yet`: rules cannot yet ask C#-specific questions.
- **Not yet profiled** against 0181's list of simdjson tricks.
