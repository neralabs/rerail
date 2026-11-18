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
- **`x as a.b.C<T>` at a line's end** (69 probes, `golden.rs`): where the next line starts with
  an operand (a name, a literal, a prefix operator) and the comparisons read on to the line's end,
  tree-sitter keeps `(x as a) . b . C < T > next`: the type is the first name, the dots navigate,
  and `<`/`>` are comparisons with the next line's operand. Where the next line starts with `(` or
  `{`, the type is the first name again and `.C<T>(…)` a generic call on the navigation. An
  unqualified type, or a declaration's, keeps the whole type. Read with a trial (`Tw::save`,
  `Tw::rewind`): the comparison reading is tried, and kept only where it ends at the line's end.
  The next line's hard keywords (`for`, `while`, `as`, …) are names there, where only a name can
  be, as tree-sitter's lexer reads them.
- **Refused**: anything not read exactly.

Oracle (`cargo test -p rerail-kotlin-lean -- oracle --ignored`, `RERAIL_KOTLIN_OUT` = the engine's
expected tapes): 495 files from 25 repositories (`~/orca/rerail-shared/lean-corpus/kt`), 491
without errors: **489 the same, none refused, 2 differ**; all 4 error texts refused. The 2 differ
where tree-sitter reads `public open class Table { … }` (and `private sealed interface I { … }`) as
an infix expression (`public open class`, the name, a lambda). Probed: two modifiers before
`class`/`interface`, and a function after the closing `}`, flip it (`fun y() = 2` after the class
does; the class alone or with a property after does not); it is the GLR's choice between versions
alive past the class, not read here. Left open.

205 MB/s on one core (see "Speed").

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
- **GLR choices probed** (about 110 probe texts against tree-sitter, 15 in `golden.rs`):
  - an optional chain right after a call in `try`'s or `await`'s operand: the operand goes on
    through the chain's next call or subscript where the expression goes on after it on its line
    (an operator, `.`, `[`, `?`, `!`, `as`; `.` on the next line), and then takes a ternary too;
    it ends at the first call where the statement ends, a closure follows, or the chain has no call;
  - `-.x` in a ternary's middle is a navigation of the operator, elsewhere prefixes;
  - `.repeat(…)`, `.each(…)`: a prefix `.` over a value pack expansion, a value parameter pack;
  - `@A (…) async`/`throws`/`rethrows` with a space: the attribute's arguments where the
    parentheses read as arguments (a trial), and the function type's parameters are then a user
    type named by the keyword; else (`inout T`, `A<B>`, `any P`) the function type's parameters;
  - `A.B<C, D>(…)`, `A.B<C, D> { … }`: a constructor where the comparisons cannot parse;
  - `#if _runtime(_ObjC) && …`: a function name not of the grammar ends the directive, and the
    rest of the line is an expression statement whose first call has the directive as function;
  - a binary chain over calls nests right for seven calls read into right operands, then left
    inside the last (tree-sitter keeps six versions), counted per expression.
- **Refused** where tree-sitter fails on valid Swift: a directive between switch entries, a `;` in
  a protocol's body, a condition's `{` after `try`/`await`'s operand; and a names-only attribute
  argument (a missing version in tree-sitter's tree).

The writer gained two things for Swift, used by both crates' shape: `field_since` gives a hidden
rule's field to the tokens held since a mark too (not only the nodes written), and `refield`.

Oracle (`RERAIL_SWIFT_OUT`): 376 files from 25 repositories
(`~/orca/rerail-shared/lean-corpus/swift`), 335 without errors: **335 the same, none refused, none
differ**; all 41 error texts refused. Golden tests: 30 probe texts (`golden.rs`), the same.

182 MB/s on one core, 12 MB resident (see "Speed").

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
- **XML** (`parse/xml.rs`): elements, attributes, text, comments, CDATA, processing
  instructions and embedded `{ … }` blocks and patterns, read on the bytes. Runs (text,
  attribute strings, the ends of comments) are found 64 bytes a step (`find3`, three compares and
  a bitmask over `rerail_simd8`'s backends). The token buffer is cut at the literal's start, at
  each embedded block (read as code) and at its end. Blanks before a child tag are the
  external scanner's and give no text; elsewhere `xml_text` takes them, as tree-sitter's lexer
  does.
- **Read since the first count:** quotes and splices (`'{…}`, `'[T]`, `${…}`), type lambdas,
  polymorphic function types and lambdas (`[A] => …`), dependent function types, structural
  types, `?=>`, given conditionals, `as` and other alphanumeric pattern operators, a postfix
  operator after an operation, `a op b: _*`, operators past ASCII (`≺:`, `⊕`), `a.extension.B`,
  `new A` with `with B` on the following lines, `new:`, `val x` with `: T =` on the next line.
- **Refused** still: what tree-sitter itself reads with an error (`'*` symbol literals), a `type`
  definition inside `'[…]`, and several XML nodes side by side at the top of a literal.

Oracle (`RERAIL_SCALA_OUT`): 440 files from 22 repositories
(`~/orca/rerail-shared/lean-corpus/scala`), 439 without errors: **438 the same, none refused, 1
differs**; the error text refused. The one that differs: tree-sitter reads the outermost
`new BoxPanel(x) { … }` of a 5 KB nest of `new X(…) { … }` bodies as a call of an instance with a
block argument, and every nested one of the same shape (and the same text taken alone) as an
instance with a template body. Each `new X(…) {` is ambiguous until its body closes, so the nest
holds more GLR versions than tree-sitter keeps (6), and which survives is its pruning's choice,
not the grammar's. Left open. Golden tests: 22 probe texts (`golden.rs`), the same.

161 MB/s on one core (see "Speed").

## Speed

Measured with each crate's `*_costs` example over its corpus 20 times, built with `--profile
profiling`, through `quiet-run --here`. Each figure is the least of three rounds. The third
argument `lex` runs the lexer alone (`lex_only`: every token from the start, no parse).

| | whole parse | instructions / byte | cycles / byte | IPC | lexer alone: instructions / token | cycles / token |
|---|---|---|---|---|---|---|
| Kotlin | 205 MB/s (was 180) | 106 (120) | 21.9 (25.0) | 4.9 | 146 (184) | 41 (52) |
| Swift | 182 MB/s (161) | 128 (140) | 24.7 (28.1) | 5.2 | 156 (201) | 40 (58) |
| Scala | 161 MB/s (146) | 133 (144) | 27.9 (30.4) | 4.7 | 152 (224) | 42 (52) |

The figures in brackets are from before the block lexer, chains and relex work below (commit
d7e78a59e, measured the same way).

**Hotspots** (`sample`, share of the samples):

- The lexer: `token_at` (the dispatch on a token's first byte), `lex_batch` with `skip` inlined,
  `name_end`, and Kotlin's ASI and Swift's semicolon scanner. 26-34%, at an IPC of 3.5 against
  the parse's 4.8: its branches mispredict.
- The writer (`tw.rs`, `Writer`): `claim`, `close_span`, `open`, `token`, the automata's `walk`,
  `leaf`, `finish_columns`. About 30%. `Writer::insert` and its memmove are 4-6%. Wraps cause
  most of it (Kotlin 106 of 117 samples, Scala 132 of 175): a postfix or operator chain wraps
  its growing operand again and again, so k wraps over n nodes move k times n. The rest is
  held tokens put in one insert each (Swift 83 of 153).
- Scala only: the scanner was copied with its `Vec` at each layout question, about 7% in
  malloc and free.

**Changed** (tapes' digests unchanged):

| Change | Effect |
|---|---|
| Scala's scanner holds its indentation stack inline (48 deep, deeper refused), and `read_word` a 16-byte buffer: a question allocates nothing | Scala −11% instructions |
| `Writer::insert` (shared by every lean parser): a span that fits a window (16 bytes, 8 cells) moves by one fixed-size copy, not a `memmove` call per column | −1.4% to −3% cycles |
| `skip` returns at once where the next byte touches the last token | Kotlin −2.7%, Scala −2.0% cycles |

**Then** (the opportunities below, built; same tapes, a test per step):

| Change | Effect |
|---|---|
| The block lexer (`Blocks::lex_blocks`, shared): a block's token starts at once, names from the word mask's run, one-byte punctuation and operators from two tables (kind, and the next bytes that make it longer), comments in place; the rest by `token_at` from its start, characters past ASCII by the token loop. A test compares it with the token loop on 3,000 random texts | the lexer −21% (Kotlin), −22% (Swift), −31% (Scala) instructions a token |
| No relexing after a string without an interpolation (each string threw away the tokens read after it and lexed them again); `lex_blocks` stops once its minimum is made | Kotlin lexed 943,725 tokens a pass for 487,048 with the block lexer, 497,692 now. This is where the lexer's gain reaches the parse |
| `Writer::insert_leaves`: a node's held tokens in one backward pass | neutral: a claim mostly keeps one token, over runs of about 14 nodes |
| Writer chains in `tw.rs`: from a mark's second wrap over a tail of 32 nodes or more, the wraps are held and put in one move when the mark ends; a kind whose automaton walks its children ends the chain first | corpora within noise; a 160 KB chain of 60 `.step { … }` calls −28% cycles. At every second wrap it cost 2% on the corpora, where a wrap moves 2 to 17 nodes |
| Digit runs and quoted names on `swar::class_end`/`first_of2`, Kotlin's string content on the string-stop mask | no per-byte loops left there; within noise (short runs) |

Together, against d7e78a59e: Kotlin −11.5% instructions and −15% cycles, Swift −9% and −15%,
Scala −8% and −11%.

**Tried and dropped:**

- One scan of the blank and line-end masks for a whole gap (`gap_end`): the lexer's
  instructions rose by 6%. Most gaps are one space inside a block, which the old path already
  handled cheaply.
- Each bracket's match found once for Kotlin's `assignment_ahead`, which walks a statement's
  tokens at each statement: −0.4% instructions. That gain doesn't pay for the memo's state and
  invalidation.
- Swift's `skip` fast exit: +1.7% cycles.

**Opportunities** left:

1. **The lexer's slow tokens.** Strings, numbers, two-byte operators and names that run past a
   block still go through `token_at`'s dispatch; page 30's lexer runs 70-82 instructions a token
   against our 146-156. Pairs from the masks (`::`, `->`, `?.`, `==`) and strings ended by the
   STOP mask inside `lex_blocks` would take most of them off it.
2. **The parse's own branches** (page 34, one symbol a token): the lean parsers carry the word
   code already; what remains is the parse's tests on it. The parse and writer are now about
   80% of the cycles.
3. **Kotlin's remaining relexes** (1,229 a pass: `?`/`>` splits and interpolations) cost little
   now and need no change.

## Testing loop

The engine writes expected tapes once per corpus (`python_tape_expect`, under Relax, a few
minutes); the lean crate's oracle compares all files in about a second under Relax
(`relax test rerail-kotlin-lean --lib -- oracle --ignored`), with the caches shared
(`RELAX_SHARE=rerail`, `RELAX_SHARED_OPEN=1`). Probing tree-sitter's choice on a short text is
the same engine run over a list of probe files. Golden unit tests hold tree-sitter's tapes of short
texts (`parse::tests` for Kotlin, `golden.rs` for Swift and Scala).
