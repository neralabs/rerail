# 0195: Java is read by a lean parser that writes tree-sitter-java's tape, or refuses

Status: built (user, 2026-11-13: "the next three parsers I want are Rust, C-sharp and Java"; the
order Rust, Java, C#). Java is a product language from this step on: `languages/java.json`, the
`lang-java` feature, tape tag 15.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node (kind,
field, supertypes). Until now, Java was not a product language.

The lean Python, ECMAScript and Rust parsers (0163, 0169, 0178, 0193) write the tape directly. Each
writes exactly the tape tree-sitter's tree encodes to, and refuses any file it cannot read exactly.
This decision does two things:

- it adds Java as a product language, read by tree-sitter-java;
- it builds the same lean parser for Java, at rank 0 before tree-sitter-java.

```
a .java file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the lexer: runs and stops by trailing zeros; every word one hash probe
       a string or text block is one token over the whole literal
  -> recursive descent over tree-sitter-java's grammar, writing the tape (no backtracking)
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-java
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery.

## Prior work consulted

- **ast-index-spike indexes:** the spike's sample corpus (`ast-index-sample-corpus`) holds
  `elasticsearch` and `spring-framework`. 700 of their files are one of the oracle's sets below.
- **0193 (lean Rust), 0178 (lean ECMAScript), 0163 and 0169 (lean Python), 0181 (their profile).**
  **Agrees and follows:** the same crate shape, stage 1, keyword hash, writer, batched token
  buffer (16 tokens a refill) and oracle. "No error recovery: one behavior" is kept as is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`,
  and the engine takes the columns as written (`TapeImage::written`).
- **The engine's implied-token automata (`tape::tokens::implied_tokens`).** **Followed:** the
  parser stores exactly the tokens the automata keep (`implied.tsv`, `kinds::DROPPED`, and a gap's
  delimiters by its shape). It never writes a token to filter it out afterwards.
- **The language pack registry (`docs/language-packs.md`).** **Followed:** Java's pack is
  registered like Rust's: a language file, a `lang-java` feature in every crate, a token grammar
  (`native/grammars/token-grammars/java`, tree-sitter-java 0.23.5), and an adapter. C# (tag 16) is
  registered in the same step; its lean parser is the next decision.

## Decision

### Java and C# as product languages

- `languages/java.json` and `languages/csharp.json`: suffixes `.java` and `.cs`, tape tags 15 and
  16, the `tree-sitter` parser. Every capability is `not-supported-yet` for now: a rule cannot yet
  ask for a Java-specific question.
- `lang-java` and `lang-csharp`, in every crate's features and in `all-languages`.
- The adapters (`language/adapter/java.rs`, `csharp.rs`) name the families' comment, parenthesis,
  attribute and `this` kinds.

### The crate: `rerail-java-lean`

`native/crates/java_lean`, in the shape of `rerail-rust-lean`:

- `block`: stage 1, five masks a 64-byte block (name bytes, blanks, line ends, a string's stops, a
  block comment's stops).
- `lex`: tokens where the parser asks, with no state. A word's code comes from one probe of a
  1,024-slot perfect hash; the parser decides where a word is a keyword (tree-sitter's keyword
  extraction). A string or text block is one token over the whole literal, so a look ahead never
  sees a string's text as tokens; the parser reads its pieces from the bytes.
- `parse`: declarations, statements, expressions (precedence climbing over tree-sitter-java's
  `PREC`), types, and patterns.
- `gen_kinds.py`: writes `kinds.rs` (264 kinds, 40 fields, 184 dropped tokens) from the token
  grammar and `implied.tsv`.

The parser never backtracks: every choice is made by looking ahead over tokens (`scan_type`,
`declaration_ahead`, `cast_ahead`, `record_pattern_ahead`, …).

### Where tree-sitter's choices are kept

The tape must be tree-sitter's even where its grammar reads a text in a surprising way. These were
found by the oracle, and the parser keeps each as tree-sitter reads it:

- **A statement is a declaration** where modifiers, or a type and a name, start it, and the name is
  followed by what follows a declarator's name (`=`, `;`, `,`, `[`, `:`).
- **Casts.**
  - `(a) - b` and `(a) + b` are a sum or a difference. Any other operand after `(a)` makes a cast:
    `(a) (b)`, `(a) ++b` and `(A) !b` are casts.
  - `(a)++` is an update of a parenthesized expression.
  - A dotted name is a type: `(a.b) - c` is a cast.
  - The operand of a cast is in `primary_expression` alone, even with one type, because
    tree-sitter takes the intersection cast's form there.
- **An assignment as an operand.** An assignable operand (a name, a field access, an array access)
  before an assignment operator is the assignment's left at any precedence: `a & b += 4` is
  `a & (b += 4)`, and `-a = 1` is `-(a = 1)`.
- **`switch` groups.** Each `case …:` starts a group of its own: `case 1: case 2: f();` is two
  groups, the first without statements.
- **A case label ends at its `->`.** `case A -> {}` is a rule, never the lambda `A -> {}`.
- **Keywords as names.** tree-sitter lexes a keyword as a name wherever the keyword is not valid.
  Two such places are read:
  - `case null, default ->` (`default` an identifier);
  - `@A T this` in parameters (a parameter named `this`, while `T this` and `T Outer.this` are
    receivers).
- **Stored tokens that follow from a gap's shape:**
  - `""` stores both quotes. A string whose first piece is an escape or an interpolation stores its
    opening `"`.
  - A text block stores its `"""`s. A text block's `\` before a character no escape names is a
    token the tape leaves out.
  - `[] []` (more than one pair in a gap) stores the brackets.
  - A trailing `,` in an array or element-value initializer stores `, }`. An enum's trailing `,`
    is stored, with the `}` after it.
  - An empty statement or member (`;` in a block, a class body, a switch group) is stored, and so
    is the gap it stands in (`{ ; }` stores its braces).
  - `if (c) ; else ;` stores both `;`s and the `else`.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error;
- a keyword where tree-sitter's state takes none, which tree-sitter reads as a name, beyond the
  two places above (`a.return x;` in a cut text);
- a trailing `,` in an argument list (a tree-sitter error).

### In the engine

`tape::java_lean::Lean` is Java's `TreeParser`, registered at rank 0 under `lang-java`. A file it
refuses goes to tree-sitter-java.

## Results

**The oracle** (`rerail-java-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets:

- **Corpus:** 1,507 files.
  - 700 come from the ast-index sample corpus (`elasticsearch`, `spring-framework`).
  - 807 come from 85 of the stored repositories (`~/nera/sampo-repos`, by `tmp/corpus/extract.sh`,
    12 a repository).
  - 7 of the files have tree-sitter errors.
- **Cut and edited:** 513 of those files, each cut at 3 places and with 3 lines cut short in the
  middle.
- **Probes:** 133 hand-written cases, one for each reading above.

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| Java, 1,507 files | 1,500 / 0 / 0 | 0 / 0 / 7 |
| Java, cut and edited (513 files) | 788 / 0 / 1 | 0 / 0 / 2,372 |
| probes, 133 hand-written cases | 129 / 0 / 0 | 0 / 0 / 4 |

No tape differs: each is tree-sitter's or a refusal. No clean file is refused. The one clean cut text
refused reads a keyword as a name (`content.` cut before `return true;`, which tree-sitter reads as
a declaration of `true` with type `content.return`).

**Engine tests:**

- `java_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape. Syntax errors are refused.
- `the_product_reads_java_with_the_lean_parser`: the engine's parse route gives the lean tape, and
  a refused file gets tree-sitter's.

**Cost** (`java_parse_costs`, the 1,500 clean corpus files, 14.7 MB, 1,754,163 nodes; best of 3
passes; profiling build; load 20 to 25):

| step | instructions a node | cycles a node | ns a byte |
|---|---:|---:|---:|
| lean: the parser's columns | 547 | 125 | 3.4 |
| lean: the tape | 576 | 139 | 3.8 |
| tree-sitter: parse | 5,448 | 1,110 | 30.4 |
| tree-sitter: parse and tape | 9,148 | 1,914 | 52.6 |

The lean tape takes 16 times fewer instructions and 14 times fewer cycles than tree-sitter's parse
and tape.

## Open

- **C#'s lean parser** is the next step (the order Rust, Java, C#).
- **Java's capabilities** are `not-supported-yet`: rules cannot yet ask Java-specific questions.
- **Not yet profiled** against 0181's list of simdjson tricks.
