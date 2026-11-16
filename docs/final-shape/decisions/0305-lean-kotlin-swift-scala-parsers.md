# 0305: Lean parsers for Kotlin, Swift and Scala, tested against tree-sitter

Status: decided (user, 2026-11-16: "please create and test Rerail lean parsers for Swift, Kotlin
and Scala … get to Treesitter-equivalence for direct-to-tape parsers on valid input"). Kotlin
built; Swift and Scala follow in this record.

## Intro

A lean parser reads one language's text and writes the engine's tape for it directly: the same
nodes, spans and kept tokens as tree-sitter's tree encoded by the engine, without building a tree.
Where it cannot be sure it reads a text exactly as tree-sitter does, it refuses, and the caller
uses tree-sitter. There is one behavior: tree-sitter's tape, or a refusal.

The engine had none for Kotlin, Swift or Scala; it has no product grammar for them yet either. So
the three grammars come in as **test dialects**: their `grammar.json` (tree-sitter-kotlin-sg
0.4.1, tree-sitter-swift 0.7.4, tree-sitter-scala 0.26.2) sit under
`native/grammars/token-grammars/`, the engine builds their token automata (which tokens a node
keeps), and its test hooks write tree-sitter's tapes for any list of files (`python_tape_expect`
with `RERAIL_PY_DIALECT=kotlin|swift|scala`). Nothing in the product reads them
(`token_automata::TEST_ONLY`).

```
a .kt file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: words, blanks, line ends, string stops)
  -> lexer: a token at a time, into a buffer 16 at a time; keywords by the perfect-hash switch
  -> recursive descent over tree-sitter-kotlin's grammar, writing the tape in preorder
       (each token kept or left out by the engine's automata, as stored_children decides)
  -> the tape, or a refusal
```

## The shape (the same as the other lean parsers)

- **Stage 1** is `rerail-simd8`: the block's byte classes as 64-bit masks (`block.rs`), so a name,
  a run of blanks, a comment's end and a string's next stop are found a block at a time.
- **The lexer** is stateless: a token from a byte offset. Keywords are one probe of a perfect hash
  (`keyword_switch!`). A word is a keyword only where the parser's state takes it, as tree-sitter's
  keyword extraction does: the parser decides, not the lexer.
- **The writer** (`tw.rs`) is the SQL parser's (0226): a node opened when its kind is known, or put
  over what was read since a mark (an operator over its left operand); tokens decided by the
  engine's automata, at once where the kind decides, else held until the parent closes. `tw.rs` is
  shared by the three crates (`gen_kinds.py` writes each crate's tables in one shape).

## Kotlin

- **Automatic semicolons.** tree-sitter-kotlin's external scanner decides where a line end ends a
  statement. It is ported (`Lexer::asi`), with two effects the tree shows:
  - a semicolon after a comment on the statement's line sits after the comment: the comment is
    inside the semicolon's parent, which ends there;
  - a block comment the scanner reads through on the next line is lost to the tree (the scanner's
    `skip` moves the token's start past it): a zero-width comment where the scanner stopped, or a
    zero-width semicolon there (`/** doc */` before `abstract class` is in no node).
- **Strings** are read piece by piece as the scanner reads them (a `$` alone is its own piece, a
  piece ends before a closing `"""`, `\$"` ends the string with the piece before it). An
  interpolation's expression is parsed from the token buffer cut at its `${`.
- **Where tree-sitter's GLR parse keeps two readings**, the parser follows the one tree-sitter keeps,
  probed: a generic call's `<…>(…)` goes over a whole operation binding tighter than a comparison
  (`a ?: f<T>()` is `call(elvis(a, f), <T>())`; not over `<` or `==`); a statement's annotation
  before an expression is a `prefix_expression`'s, before a loop or assignment the statement's.
- **Refused**: anything not read exactly, and one known GLR shape: an `as`/`is` type closing its
  type arguments at a line end with a generic call (`name<`) on the next line (tree-sitter takes
  the comparison reading for `as java.util.List<Int>` there, not for `as List<Int>`).

Oracle (`cargo test -p rerail-kotlin-lean -- oracle --ignored`, `RERAIL_KOTLIN_OUT` = the engine's
expected tapes): 495 files from 25 repositories (`~/orca/rerail-shared/lean-corpus/kt`), 491
without errors: **486 the same, 3 refused, 2 differ**; all 4 error texts refused. The 2 differ where
tree-sitter reads `public open class Table { … }` as an infix expression whose right operand is a
lambda; it reads the same text as a class when the file is cut just after it, so the choice
depends on the GLR versions alive from the text after it. Left open.

About 90 MB/s on one core of an M-series Mac under load (`kotlin_costs`, the corpus 20 times).

## Testing loop

The engine writes expected tapes once per corpus (`python_tape_expect`, under Relax, a few
minutes); the lean crate's oracle compares all files in about a second under Relax
(`relax test rerail-kotlin-lean --lib -- oracle --ignored`), with the caches shared
(`RELAX_SHARE=rerail`, `RELAX_SHARED_OPEN=1`). Probing tree-sitter's choice on a short text is
the same engine run over a list of probe files. Golden unit tests hold tree-sitter's tapes of short
texts (`parse::tests`).
