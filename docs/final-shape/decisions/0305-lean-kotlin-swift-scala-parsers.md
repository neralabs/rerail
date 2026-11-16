# 0305: Lean parsers for Kotlin, Swift and Scala, tested against tree-sitter

Status: decided (user, 2026-11-16: "please create and test Rerail lean parsers for Swift, Kotlin
and Scala … get to Treesitter-equivalence for direct-to-tape parsers on valid input"). All three
built.

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

## Swift

`rerail-swift-lean` has Kotlin's shape and shares its `block.rs` and `tw.rs` (by path). What is
Swift's own:

- **No keyword extraction.** tree-sitter-swift's grammar has no `word` rule, so its lexer takes a
  keyword wherever the state allows one. A modifier's word that starts a statement but no
  declaration (`indirect(x)`, `open(f)`) is that keyword to tree-sitter, and its parse fails: the
  parser refuses it. Contextual words (`async`, `each`, `lazy`, …) are `simple_identifier`s holding
  their token.
- **Names run through `*` and `#`.** The identifier's pattern continues through `\p{Emoji}`, which
  holds the keycap bases `*`, `#` and the digits: `i*4` and `width*height` are one name each.
- **The implicit semicolon** (the external scanner) has no width and sits where the next line's
  token starts, after any comments: a statement list ends at the `}` or `case` after it, and a
  comment on the last statement's line is inside the list. The operators that continue a line
  across a line end (`.`, `=`, `==`, `&&`, `?`, `:`, `{`, `as`, `else`, `catch`, …) are the
  scanner's. A block comment after a member's line is the scanner's token, not an extra.
- **Expressions by the parse table's precedences.** A call (`(…)`, `[…]`, a trailing closure) binds
  looser than every binary operator but assignment: `"a" + f(x)` is a call of `"a" + f`. A binary
  operator's right operand is `_expr_hack_at_ternary_binary_suffix`, which takes every operator
  after it (`a == b && c` is `a == (b && c)`) and a call with arguments, not a trailing closure.
  `try`, `await` and `unsafe` end their operand at a call (their `prec.left(0)` choices win the
  reduction over `_expression`'s `prec(-1)`), so `try a.f()!.g` is a navigation of `(try a.f())!`.
- **A property's value splits a trailing closure off a call** with arguments
  (`let x = f(a) { … }` is `call(call(f, (a)), { … })`), as do `try` and `await` inside it; a
  statement's call keeps both in one suffix.
- **GLR choices read from the corpus.** `Name<T>.member` is two comparisons
  (`Name < T > .member`) unless those cannot parse there (a `>` before `,` or `)`, a `,` outside a
  list), or an arithmetic operator follows the member, or a call on it is navigated, or it is the
  right operand of a product whose left one is such a type. `x is A.B` and `x as! A.B` before a `)`
  are a navigation of the check.
- **Refused GLR shapes** (both readings seen, or a reading tree-sitter's version pruning picks):
  an optional chain right after a call in `try`'s operand, `-.x` in a ternary's middle,
  `.repeat(…)`, `@A (…)` with a space and no `->` after it, a names-only attribute argument (a
  missing version in tree-sitter's tree), a binary chain over more than six calls (its nesting
  flips from right to left where tree-sitter prunes versions). Also refused where tree-sitter
  fails on valid Swift: a directive between switch entries, a `;` in a protocol's body, a
  condition's `{` after `try`/`await`'s operand.

The writer gained two things for Swift, used by both crates' shape: `field_since` gives a hidden
rule's field to the tokens held since a mark too (not only the nodes written), and `refield`.

Oracle (`RERAIL_SWIFT_OUT`): 376 files from 25 repositories
(`~/orca/rerail-shared/lean-corpus/swift`), 335 without errors: **325 the same, 10 refused, none
differ**; all 41 error texts refused. Golden tests: 15 probe texts (`golden.rs`), the same.

About 140 MB/s on one core under load (`swift_costs`, the corpus 20 times: 53.5 MB in 0.38 s of
user time, 12 MB resident).

## Scala

`rerail-scala-lean` has the same shape and shares `block.rs` and `tw.rs`. What is Scala's own:

- **The external scanner is ported whole** (`scan.rs`): automatic semicolons, indents and
  outdents (Scala 3's significant indentation), a `:` at a line's end, the gate before `else`,
  `catch`, `finally` and `yield`, the soft modifiers (`inline`, `opaque`, `open`, …), and the
  block comments it reads. The parser asks it with the valid set tree-sitter's state would give:
  its own tokens, those of what may follow (`fol`), and block comments. A wider set gives another
  token: after a bare `for`'s enumerators only the gate is valid, not the enclosing list's
  semicolon, or `yield` on the `for`'s column reads as a new statement. Answers are kept per
  position and valid set; `take` commits one.
- **Strings are whole lexer tokens**, interpolations included (`${…}` lexed through to its `}`), so
  a look ahead never reads a string's text as code. The parser writes the pieces afterwards.
- **A name the grammar gives at run time.** tree-sitter-scala aliases `(` to `arguments` in some
  rules. The engine's walk fails on a runtime-named child its automaton lacks, so it stores every
  child of those nodes. `gen_kinds.py` (`RUNTIME_NAMED`) turns off the automata that take no other
  token before `(`, and the parser writes `(` as a leaf. A `for (…)`'s tokens are all leaves.
- **`extension` is a soft identifier**: wherever an identifier is read, it is an `identifier`
  holding the keyword's token.
- **Shapes read from tree-sitter's tapes**, probed on short texts: a block lambda takes the
  `{expression}` class only with one statement in its body; a case clause ends at a semicolon the
  scanner gives, with the comments before it; a `new A(x) with B` base is an
  `applied_constructor_type`; `new A(x)(y)` has one argument list (the next is a call); `if (c) -1`
  keeps the parenthesized condition unless a `then` follows; a bound's `<:`/`>:` end it
  (`A >: L <: U`), but not inside brackets; colon arguments (`xs.map: x =>`, `f(a):`) take the
  operand before the `:` as the function; `10 seconds` is a postfix expression.
- **Refused** where not probed: a postfix operator inside an operation, `'{…}` quotes, given
  conditionals (`given [T](x) => …`), a type's `a.extension.B`, `macro`, and the rest of the
  corpus's unread forms (named tuples, `do … while`, `end` markers after some definitions, XML
  literals).

Oracle (`RERAIL_SCALA_OUT`): 440 files from 22 repositories
(`~/orca/rerail-shared/lean-corpus/scala`), 439 without errors: **382 the same, 56 refused, 1
differs**; the error text refused. The one that differs: tree-sitter reads
`new BoxPanel(x) { … }` as a call of an instance with a block argument there, and as an instance
with a template body when the same text is taken alone: the choice depends on the GLR versions the
file's later text keeps alive. Left open. Golden tests: 16 probe texts (`golden.rs`), the same.

About 58 MB/s on one core while the machine was loaded (`scala_costs`, the corpus 20 times: 71.6 MB
in 1.24 s of user time, 16 MB resident, `quiet-run --here` with a load average of 78 on 18 cores),
so not comparable with Kotlin's and Swift's figures.

## Testing loop

The engine writes expected tapes once per corpus (`python_tape_expect`, under Relax, a few
minutes); the lean crate's oracle compares all files in about a second under Relax
(`relax test rerail-kotlin-lean --lib -- oracle --ignored`), with the caches shared
(`RELAX_SHARE=rerail`, `RELAX_SHARED_OPEN=1`). Probing tree-sitter's choice on a short text is
the same engine run over a list of probe files. Golden unit tests hold tree-sitter's tapes of short
texts (`parse::tests` for Kotlin, `golden.rs` for Swift and Scala).
