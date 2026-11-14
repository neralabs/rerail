# 0193: Rust is read by a lean parser that writes tree-sitter-rust's tape, or refuses

Status: built (user, 2026-11-13: "the next three parsers I want are Rust, C-sharp and Java"; the
order Rust, Java, C#). Drafted as 0182; renumbered past the highest number in any worktree (0192).

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node (kind,
field, supertypes). For Rust, the tape came from tree-sitter-rust's parse, then an encoding pass
over the tree.

The lean Python parser (0163, 0169) and the lean ECMAScript parser (0178) write the tape directly.
Each writes exactly the tape tree-sitter's tree encodes to, and refuses any file it cannot read
exactly. This decision builds the same parser for Rust:

```
a .rs file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the lexer: runs and stops by trailing zeros; every word one hash probe
       tree-sitter-rust's external scanner ported: floats, raw strings, string content,
       nested block comments, doc comments
  -> recursive descent over tree-sitter-rust's grammar, writing the tape (no backtracking)
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-rust
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery.

## Prior work consulted

- **ast-index-spike indexes:** the spike parsed Rust with tree-sitter only. Its sample corpus
  (`ast-index-sample-corpus`, the `rust-lang/rust` checkout) is one of the oracle's sets below.
- **0163, 0169 (lean Python), 0178 (lean ECMAScript), 0181 (their profile).** **Agrees and
  follows:** the same crate shape, stage 1, keyword hash, writer, batched token buffer (16 tokens a
  refill) and oracle. "No error recovery: one behavior" is kept as is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`,
  and the engine takes the columns as written (`TapeImage::written`).
- **The engine's implied-token automata (`tape::tokens::implied_tokens`).** **Followed:** the
  parser stores exactly the tokens the automata keep (`implied.tsv`, `kinds::DROPPED`, and a gap's
  delimiters by its shape). It never writes a token to filter it out afterwards.

## Decision

### The crate: `rerail-rust-lean`

`native/crates/rust_lean`, in the shape of `rerail-ecma-lean`:

- `block`: stage 1, five masks a 64-byte block (name bytes, blanks, line ends, a string's stops, a
  block comment's stops).
- `lex`: tokens where the parser asks, with no state. A word's code comes from one probe of a
  1,024-slot perfect hash; the parser decides where a word is a keyword (tree-sitter's keyword
  extraction). The parser itself lexes the few places its state reads otherwise: an integer field
  after `.`, a `>>` or `&&` split in two, a macro repetition's separator.
- `parse`: items, statements, expressions (precedence climbing over tree-sitter-rust's `PREC`),
  patterns, types, and token trees (a macro invocation's, a macro rule's pattern and body).
- `gen_kinds.py`: writes `kinds.rs` (301 kinds, 31 fields, 232 dropped tokens) from the engine's
  token grammar (`native/grammars/token-grammars/rust`) and `implied.tsv`.

The parser never backtracks: every choice is made by looking ahead over tokens (`angle_end`,
`colon_ahead`, `item_ahead`, …).

### Where tree-sitter's choices are kept

The tape must be tree-sitter's even where its grammar reads a text in a surprising way. These were
found by the oracle, and the parser keeps each as tree-sitter reads it:

- **Statement ends.** A block-like expression at a statement's level ends the statement, unless the
  next token cannot start a statement (`.`, `?`, `as`, `=`, a binary operator but `-`, `*`, `&`,
  `|`, `..`, `<`).
- **Macro invocations at a statement's level:** followed by `;`, an expression statement; followed
  by `}`, the block's value (`{_expression}`); otherwise an item (no supertype).
- **Match arms:** a block-like value without a `,` that is not the last arm has no supertype.
- **Every binary operator is left-associative**, assignment and ranges included.
- **`break` and `..` with an operator after them.** Where the next token also continues an
  expression, the operand is left out and the token is an operator on the operand-less node:
  - `break (a, b)` is a call of `break`;
  - `break -1` and `break *x` are binary expressions over `break`;
  - `a..*b` is `(a..) * b`, and `..=x` is `(..) = x` (no range starts with `..=`).

  `return (a, b)` and `a..(b)` keep their operands.
- **`x as T < y`** is an error: after a type's name, `<` always opens type arguments.
- **A bounded type's `+`** applies only in a full type slot. Inside a reference, a pointer, `dyn`,
  `impl`, a cast, a function type's return or a bound list, it does not.
- **`impl 'a + Tr`:** a bound list that starts with a lifetime (or `use<…>`) is the `impl`'s trait,
  a `bounded_type` inside the `abstract_type`. `impl A + B` stays `(impl A) + B`.
- **`impl<T> ::a::B`** reads `<T>` as type parameters. It is a qualified type only with an `as`
  inside (`impl <T as A>::B`).
- **`fn(_, _)`**: a bare `_` in a function type's parameters is the token, with no supertype.
- **Raw strings:** the content starts after any white space that follows the opener, as the
  scanner skips it.
- **Token trees:**
  - the keywords tree-sitter's `_non_special_token` lists are stored tokens; other words are
    identifiers;
  - `'a` is `'` and a word;
  - `$x` in a macro invocation's tree is `$` and an identifier;
  - the opener is always stored; the closer is left out only right after a named child.
- **Stored tokens that follow from a gap's shape:**
  - a trailing comma stores `, closer`;
  - a `_` in a list makes its whole gap stored;
  - `pub(crate)` stores its `(`, while `pub(self)`, `pub(super)` and `pub(in …)` leave it out;
  - a variadic parameter with a pattern stores its `:`.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error. These include constructs tree-sitter-rust's grammar lacks, which it reads with
  an error: `pub impl`, `pub type X;` (an associated or foreign type with a visibility), a raw or
  byte string as an `extern` ABI, `fn(&_)` (tree-sitter reads a reference pattern and then finds no
  `:`), macros 2.0 (`pub macro`), `const impl`, `[const]` bounds, and `${concat(…)}`;
- a macro repetition whose separator holds a `/`.

### In the engine

`tape::rust_lean::Lean` is Rust's `TreeParser`, registered at rank 0 under `lang-rust`. A file it
refuses goes to tree-sitter-rust.

The engine's census tests take the `rust`, `java` and `csharp` dialects:

- `python_tape_expect` (`RERAIL_PY_DIALECT`), the oracle's expected tapes;
- `ecma_implied_tokens` (`RERAIL_IMPLIED_DIALECTS`), the implied-token table.

## Results

**The oracle** (`rerail-rust-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets:

- **Corpus:** 2,022 files.
  - 693 come from the ast-index sample corpus (`rust-lang/rust`).
  - 1,329 come from 103 of the stored repositories (`~/nera/sampo-repos`, by `tmp/corpus/extract.sh`).
  - 51 of the files have tree-sitter errors.
- **Cut and edited:** 288 of those files, each cut at 3 places and with 3 lines cut short in the
  middle.

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| Rust, 2,022 files | 1,971 / 0 / 0 | 0 / 0 / 51 |
| Rust, cut and edited (288 files) | 548 / 0 / 0 | 0 / 0 / 1,246 |
| probes, 120 hand-written cases | 114 / 0 / 0 | 0 / 0 / 6 |

No tape differs: each is tree-sitter's or a refusal. No clean file is refused.

**Engine tests:**

- `rust_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape. Syntax errors are refused.
- `the_product_reads_rust_with_the_lean_parser`: the engine's parse route gives the lean tape, and
  a refused file gets tree-sitter's.

**Cost** (`rust_parse_costs`, the 1,971 clean corpus files, 27.5 MB, 4,214,902 nodes; best of 3
passes; profiling build; load 38 to 70):

| step | instructions a node | cycles a node | ns a byte |
|---|---:|---:|---:|
| lean: the parser's columns | 466 | 119 | 4.4 |
| lean: the tape | 487 | 130 | 4.8 |
| tree-sitter: parse | 5,231 | 1,204 | 45.1 |
| tree-sitter: parse and tape | 9,134 | 2,080 | 78.5 |

The lean tape takes 19 times fewer instructions and 16 times fewer cycles than tree-sitter's parse
and tape.

## Open

- **Macros 2.0** (`pub macro`), `const impl` and `[const]` bounds are tree-sitter errors today, so
  they are refused (and tree-sitter's recovery reads them).
- **Not yet profiled** against 0181's list of simdjson tricks.
