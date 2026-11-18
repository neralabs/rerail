# 0332: Kotlin, Swift and Scala in Bitlex: ladders against the lean parsers, and the compiler's additions

Status: decided (user, 2026-11-16: "build bitlex versions of each of these"). Kotlin's step 1 is
built. Swift and Scala are not started.

## Intro

A Bitlex version of a language is its grammar written in Bitlex and compiled by the Bitlex
compiler. Tier 1 (masks over 64-byte blocks, then the token loop) and tier 2 (the tree grammar's
automaton) are lowered through the transducer's backends, as `.rr`'s and JSON's are
(docs/parser-generators/12, 24). Its output is the engine's tape, written as it parses.

Each language is built as `.rr` was: a ladder of grammar steps (`<lang>_parser/bitlex_ladder/`).
Each step adds constructs, and the compiler grows only where a step needs it. A step refuses
what it does not read yet, and a refused file goes to the next parser down: the lean parser,
then tree-sitter. So a step never writes a wrong tape. It only covers fewer files.

```
a .kt file -> compiled Bitlex grammar -> tape        (where the ladder reaches)
              refused -> lean Kotlin parser -> tape  (0305; tree-sitter where it refuses)
```

## The reference

The reference is the lean parser's tape. 0305 and 0326 test that tape against tree-sitter's,
node for node. The grammar names the lean parser's kinds and fields (`rerail_kotlin_lean::KINDS`,
`FIELDS`), so a compiled key is the lean parser's key. The grammar's own names (marks, flags) take
ids from 0x4000 and never reach a tape, as rr's do. The crate's tests compare the two tapes as
text (`image_text` against `tape_text`). A corpus test (ignored, `RERAIL_KOTLIN_CORPUS`) counts
the files that read the same, are refused, or differ. "Differ" must stay 0.

## Kotlin, step 1 (`01-headers.bitlex`)

Step 1 reads the package header, the imports and the comments:

- `package`, `import` and `as` are soft keywords and implied.
- An import's dots are implied (a rule retagged as `identifier`).
- `.*` is a `wildcard_import` over the `*`, found by a token mark at the `.`.
- A header ends at a line break in code, or at the file's end (the `line` mark).

A file with a quote, apostrophe or backtick in code, a byte above 0x7f, or a nested block comment
is handed over. A file's first string opener is always in code, so the comment regions are exact
for every file that is not handed over.

A comment on a header's own line belongs to the header in tree-sitter's tree, because the
scanner's semicolon is the line break after it. The `line` mark `settles` (below).

## Kotlin, step 2 (`02-declarations.bitlex`)

Step 2 reads declarations, types, statements and the core of expressions:

- **Declarations:** functions, properties, classes, interfaces and objects, with modifiers,
  annotations, type parameters, primary constructors and delegation specifiers.
- **Types:** user types, nullable types and function types.
- **Statements:** `for`, `while`, `do … while`, assignments (to a name, or to a chain of
  navigations and indexings, through a reading), and expressions.
- **Expressions:** tree-sitter-kotlin's precedence levels, each a chain of `wrap`s; calls,
  navigations, indexings, `if`, jumps and literals.

The scanner's automatic semicolon is a mask (`semi_at`). It is a line break in code, except before
a token the scanner lets go on the line: one of `, . : * % > < = { [ ( ? | & /`, `!=`, `as`,
`where`, `catch`, `finally`, `else` (but not before `->`), `by` or `constructor`. Before a `}` it
is a second mark, `semi_close`. A block's statements may end with it; a class body's members must.

A word in the lean parser's soft-keyword table is a soft keyword here too. Where it is a name,
tree-sitter keeps it as a leaf under `simple_identifier` (or `type_identifier`): the `name` and
`type_name` choices take it through a retagged node. `by` and `constructor` are hard keywords
here, so a file that names something with them is handed over.

What step 2 does not read is handed over:

- by an escape: strings, `;`, numbers with letters, reals, three-byte operators, and a line
  that starts with `get` or `set` (an accessor);
- or by the grammar's refusal: lambdas, `when`, `try`, enums, companions, `as`, labels and
  object literals.

Corpus (491 files): 9 the same, 482 refused, 0 differ (step 1: 5 the same).

## Kotlin, step 3 (`03-strings.bitlex`)

Step 3 reads strings with `$name` templates, as a region read in parts (0323):

```text
token '"' = start o_str read str parts {
    interpolation_identifier_start = id_dollar bytes 1
    interpolated_identifier = prev(id_dollar) last id_last
    string_content = lone bytes 1
  } else string_content
```

- The quotes are implied.
- The content keeps its escapes (`\n`, `\$`), as tree-sitter's does.
- A `$` before a name is `interpolation_identifier_start`, and the name runs to its last word
  byte (a part to a `last`, below).
- A `$` before anything else is a one-byte content token. The content after it is another,
  because tree-sitter-kotlin's scanner splits there.
- The string's opener counts as a token byte for the scanner's semicolon, so a line that starts
  with a string ends the line before.

Escapes hand over raw strings, `${…}` templates, characters and backtick names.

Corpus: 11 the same, 480 refused, 0 differ. The refusals, by the reference evaluator's first
reason: characters and backtick names 112, the grammar 93, `${…}` 73, `::` 71, numbers with a
letter or a dot 65, `;` 29, accessors 15, raw strings 13.

## Kotlin, step 4 (`04-characters.bitlex`)

Step 4 reads characters and backtick names.

- A character is a second region read in parts (`chr = sq .. sq_close`). tree-sitter's
  `character_literal` spans the quotes and the content, and keeps no leaf for either: both are
  implied, the content by its kind (`implied character_literal "'" char_content`).
- An escape is a `character_escape_seq`. `\uXXXX` keeps its `\u` as a token under the escape's
  node and its digits as none: a part `"\\u"` of two bytes, then an implied `uni_digits` of four,
  retagged together as the escape.
- A backtick name is a region read whole, a `simple_identifier` (or a `type_identifier` where a
  type is named).
- None of the three runs over a line: one that does is refused.

Corpus: 12 the same, 479 refused, 0 differ. The refusals: `::` 110, the grammar 108, `${…}` 90,
numbers 80, raw strings 31, `;` 30, accessors 18, three-byte operators 5, others 7.

## Kotlin, step 5 (`05-references.bitlex`)

Step 5 reads callable references and trailing commas.

- **Two readings of `::`, as the lean parser's.**
  - At a primary's start, a name before `::`, or a `::` alone, is a `callable_reference`. The
    name is a `type_identifier`; the `::` and a `class` after it are implied.
  - After any other operand (`this`, a navigation, a call), `::` is a navigation suffix, which
    keeps its leaves.
  - The name before `::` is told by a mask mark (`cref`): from the word's last byte, the next
    solid byte starts a `::`, carried back to its first. `this` is left out, as the lean
    parser reads `this::f` as a navigation. A token mark could not leave it out: `this` is a
    keyword of the name's token, and a mark's first token takes no `~`.
- **A trailing comma keeps its `,` and the list's closer.** The lean parser leaves a list's
  tokens out only where they are the ones the grammar implies, and after a trailing comma they
  are not. A mark `trail` (`,` before `)`, `>` or `]`) takes the comma and the closer through
  a choice whose tokens are not implied. This holds in all seven lists.
- **A generic call in a tight operand is refused.** The lean parser puts the type arguments
  over the whole operation in the right operand of a comparison or of any operator that binds
  tighter (`a to f<T>()` is `call(infix(a, f), <T>())`). Those operands read a primary with no
  generic call, so the `generic` mark is untaken and the file is refused.

Corpus: 21 the same, 470 refused, 0 differ. The refusals: the grammar 209, `${…}` 90, numbers
80, raw strings 31, `;` 30, accessors 18, others 12.

## Kotlin, step 6 (`06-lambdas.bitlex`)

Step 6 reads lambdas and trailing lambdas.

- **The tape.** A `lambda_literal` has its braces implied and keeps its parameters' `->`.
  Without parameters, `{ -> 1 }` keeps its `{` too: that isn't the implied word. Its
  `lambda_parameters` are `variable_declaration`s.
- **Trailing lambdas.** After a call's arguments, or alone after an operand, a lambda is an
  `annotated_lambda` in the call's `call_suffix`.
- **Three marks at the `{` tell the readings apart.**
  - `blk` is a token mark: a `{` after `)`, `else`, `do`, `try` or `finally`.
    - Where a control structure's body goes, it is a block, as the lean parser reads it.
    - After a call's arguments, it is that call's lambda, not a new call's. Without the mark,
      `f(1) { }` had two LL(1) readings.
  - `lam_params` and `lam_bare` are mask marks for a lambda with parameters or with a bare `->`.
    From the `{`, they fill the bytes that parameters are made of (the lean parser's
    `lambda_params_end`), and the byte after the fill must start `->`.
- **Refused for now:** destructured parameters and a trailing comma after the parameters.

Corpus: 32 the same, 459 refused, 0 differ. The corpus test now names, for a grammar refusal,
the text at the furthest token the reference evaluator reached (`bitlex::tree` reports it).

## Kotlin, step 7 (`07-annotations.bitlex`)

Step 7 reads annotations on lines of their own, use-site targets and file annotations.

- **A line break after an annotation** is the scanner's semicolon, but the lean parser reads on
  to what the annotation applies to. The annotation takes the `semi` mark, which leaves no
  leaf.
- **A use-site target** (`@get:`) is a `use_site_target` in the annotation, its word and `:`
  leaves. A token mark after `@` tells it from a type.
- **`@file:` before the package header** is a `file_annotation`, its `@`, `file` and `:`
  leaves. `file` is a keyword for this alone: as a name, it is the lean parser's plain
  `simple_identifier`.

Corpus: 60 the same, 431 refused, 0 differ. Most of the gain is the compiler fix below
(item 15): `@Inject lateinit var` and its like had been refused.

The corpus test now names, for a grammar refusal, the text at the token the interpreter
refused (`stream::Refused::at`). The reference evaluator's furthest token came from paths it
backtracked out of, and misled.

## Kotlin, step 8 (`08-templates.bitlex`)

Step 8 reads `${…}` templates:

```text
val s = "x${a.b}z"
  string_literal
    string_content "x"
    interpolation_expression_start "${"
    interpolated_expression          a.b, read as any code
    interpolation_expression_end "}"
    string_content "z"
```

A string's region now ends at a `"` or at a template's `$`. A second region, the tail, opens at
the template's `}` and ends the same way. The template's code between them is code, read as
anywhere else.

The regions find a template from its bytes alone: `${`, then code up to the first `}` on its
line. A `${` is a template only where its `$` ends a string's region, so one in a comment is
not. These are refused for now:

- a template whose code holds a quote, a brace, a comment or a line break;
- an empty `${}`;
- a `$` in code.

The tail is a reader of its own. It starts at `}`, and the compiler moves its kind to a byte
of its own (item 7's kind bytes).

A first cut made the string's tokens mask tokens, as JSON's are. Its content could then start
at any byte, so every state had to test start masks, and tier 2 went past 64K words. Reading
the string in parts keeps those rows inside the region.

Corpus: 62 the same, 429 refused, 0 differ. 25 files are refused for a template's quote, brace,
comment or line break. Most of the 90 template files now fail on the features further down
the ladder.

## Kotlin, step 9 (`09-numbers.bitlex`)

Step 9 reads every number the lean lexer reads:

- decimal, hex (`0x`) and binary (`0b`) integers;
- reals, with a fraction, an exponent, an `f`, or several (`.5`, `1.5e-3f`, `2f`).

```text
val a = 0xFFL + 1u
  long_literal [8..13]          the `L` is no leaf
    hex_literal [8..12]
  unsigned_literal [16..18]
    integer_literal [16..17]
```

Each part of a number is a mask: the integer digits, the fraction's dot and digits, the
exponent's letter, sign and digits, and the `f`. Each kind is a token from its start to the
number's last byte (`last`).

A suffix (`L`, `u`, `U`, `uL`, `UL`) is a hard keyword of `simple_identifier`. The grammar
wraps the integer in `long_literal` or `unsigned_literal` where one follows it. A mark could say
the suffix touches the integer, but the marks and extras already fill a record's 16 mark bits.
So these are refused instead:

- a name with a suffix's text;
- a word touching a number that is no suffix.

After that, any suffix after an integer touches it.

`/` was never in the operators' class: no file with a division had come this far.

Corpus: 85 the same, 406 refused, 0 differ.

## Kotlin, step 10 (`10-members.bitlex`)

A class body's members:

- companion objects;
- initializers (`init { … }`);
- a property's getter and setter.

`super` is an expression.

```text
class A {
    var z = 0
        private set           a `setter` beside the property, a member of the body
    val w get() = 2           a `getter` inside the property's declaration
}
```

An accessor on a line of its own is the next member of the body, not a part of the property,
as the lean parser reads it. A mark (`acc`) says where a line starts with `get` or `set`, or
with a visibility modifier and then `get` or `set`. Elsewhere, such as a call to `get` that
starts a line in a function, the grammar refuses. The mark replaces `param_mod`.
`vararg`, `noinline` and `crossinline` become hard keywords of `simple_identifier`, which also
frees a mark bit.

The mark is restricted to code. Without that, a string's bytes might hold it, and the strings'
regions were no longer quiet: Cranelift refused a region read in parts.

Two of the lean parser's behaviours surfaced with these files. Each is refused for now:

- **A soft keyword after `$`.** In `"$field"`, the lean parser gives the soft keyword a token
  of its own inside `interpolated_identifier`.
- **A swallowed block comment.** Take a block comment that starts a line and is followed by a
  word starting with `a`, `e` or `w` (`abstract`, `external`), other than `as`, `else` or
  `where`. The scanner swallows it into the semicolon it inserts, and the tree loses it
  (`Asi::Swallow`).
  - Open: right after a delegation specifier, or a property with no initializer, a word starting
    with `b` does the same. That is not refused yet.

Corpus: 124 the same, 367 refused, 0 differ.

## Kotlin, step 11 (`11-when.bitlex`)

`when` is an expression. It has an optional subject (an expression, or a `val` bound to one)
and then its entries, one per line. An entry has conditions or `else`, then `->`, then a body.
A condition is an expression, a range test (`in`) or a type test (`is`). The line breaks
between entries are the scanner's semicolons, as a class body's are.

A `{` after `->` is a block (`blk`), as after `)` or `else`, so `1 -> { … }` is a body and
not a lambda. No grammar change was needed beyond the nodes.

Corpus: 137 the same, 354 refused, 0 differ.

## Kotlin, step 12 (`12-semicolons.bitlex`)

A `;` is a semicolon of its own. It is a token the tape keeps no leaf of, but the span of the
node it ends counts it: `package a;` is `package_header [0..10]`. That is why it is an implied
token and not a mark: a mark has no width.

- A line break after a `;` is not a second semicolon. Blanks and comments may come between
  (`after_semi`).
- A `}` after a `;` closes as after a line break (`semi_close`). A class body's members must
  end with one: the lean parser refuses `class A { val x = 1 }` on one line, and reads
  `class A { val x = 1; }`.
- A list of statements or members takes `';' [item]` beside `semi item`, so `a(); }` and
  `a(); b()` both read.
- A `;` that starts a line, or ends the file, comes with the line break's semicolon as well.
  Both are refused for now.

Corpus: 144 the same, 347 refused, 0 differ.

## Kotlin, step 13 (`13-try-as.bitlex`)

- **`try`** is an expression: its block's statements, its catches, and its `finally`.
- **`as`** binds tighter than `*` and looser than a prefix operator: each operator level's
  operand chain takes `(as_op)*` after its unary.

```text
val a = -b as C * d as E + f
  additive_expression
    multiplicative_expression
      as_expression (prefix_expression -b) as C
      *
      as_expression d as E
    +
    f
```

**A `when` entry holds its own semicolon.** tree-sitter's `when_entry` takes the `_semi` that
ends it. So a comment before an entry's line break is the entry's, not the `when`'s:
`else -> 0 // two`. The entry ends with the mark (`semi | semi_close`), and the `when` lists
its entries with nothing between.

**Block comments the scanner reads.** Take a block comment that starts a line after a token's
line, where the lean lexer asks the scanner for a semicolon (`asked_blk`). Before another
comment, the tree keeps a comment of no width where the next one starts. That is refused, as is
the step 10 case (a word with `a`, `e` or `w`), now on the same narrower mask.

The first mask took any line break before the comment, and refused 38 files whose header
comments come before any token. A comment before the file's first token is never read by the
scanner.

Corpus: 171 the same, 320 refused, 0 differ.

## Kotlin, step 14 (`14-chains.bitlex`)

A lambda with no parameters whose first line opens a lambda with parameters
(`Row { xs.forEach { x -> … } }`) was read as having the inner lambda's parameters. The run of
parameter bytes from a `{` (`lam_fill`) went on through the inner `{`, because a fill's start
byte inside it continues the fill. Now the byte before any `{` in the run ends it (`lam_last`),
so the search back from a `->` stops at the inner `{`.

Corpus: 174 the same, 317 refused, 0 differ.

## Kotlin, step 15 (`15-receivers.bitlex`)

A function or a property may name a receiver type before its name: `fun String.faint()`,
`val A.B.c: Int`. Two decisions need more than the next token:

- **Where the receiver starts.** It starts with a name, as the declared name does.
- **Which `.` ends it.** A user type's loop of `.`s would go on through it.

A mask finds both:

1. It takes the run from after `fun`, `val` or `var` to its `(`, `:`, `=`, `{` or line end.
2. The run's last `.` ends a receiver.
3. The run's first token past the type parameters starts it.

One mark, `dotted`, holds at both, and also at an import's `.` before `*`, which it replaces
(`wild`). An import holds no receiver and a declaration no `.*`, so the merge loses nothing. The
marks plus extras fill all 16 bits of a record's code, so a new mark has to share a bit like
this.

A type parameter with a bound stops the run at its `:`, so that receiver is refused at its `.`.
Refused for now:

- **A nullable receiver** (`String?.f`): the lexer reads `?.` as one operator.
- **A generic function** (`fun <T>`): the token mark `generic` (`simple_identifier "<" …`) holds
  at `fun`, as a keyword's record has a name's kind. The statement then takes the expression's
  path. A mark's own token can't yet say "a name, but not these words" (step 16).

Corpus: 185 the same, 306 refused, 0 differ.

## Kotlin, step 16 (`16-generic-functions.bitlex`)

A function's type parameters come after `fun`: `fun <T> List<T>.g()`.

Step 15 refused these because the token mark `generic` (`simple_identifier "<" …`) held at
`fun`. That was a compiler bug, not a missing feature: the compiled program and the reference
evaluator disagreed.

- **The reference evaluator** gives a hard keyword's token the keyword's own kind (`fun`), so
  `simple_identifier` does not match it there.
- **The compiled program** tested the record's kind, which is the row's (`simple_identifier`),
  with the keyword only in its word.

A compiled element of kinds now also lists the kinds' hard keywords (`TokenElem::except`, `x`
in the RIR's `tokenmark` rows). A record with one of those words is not of the kinds. Soft
keywords stay the row's kind, as in the evaluator. All three places that evaluate token marks
test the list: the interpreter (`TokenElem::takes`), Cranelift's tier 2 (`token_pattern`), and
the generated Rust (`rust_stream::pattern`).

A row made only of keywords (`required`, kind 0) is the exception: its kind in a token mark names
the row, and every token of the row is a keyword. rr's `io_bare` (`"." "input" hash_word`) means
`#include` by `hash_word`. Bitlex's evaluator gives that token the kind `#include`, so there the
evaluator and rr's tests still disagree. That is open: the evaluator should match a keywords-only
row's kind.

The 16 generic functions are no longer refused at `fun`. Each now meets another refusal further
in (mostly `by` and type constraints), so the counts hold.

Corpus: 185 the same, 306 refused, 0 differ.

## Kotlin, step 17 (`17-delegates.bitlex`)

- **A property's delegate:** `val x by lazy { 1 }` is a `property_delegate`.
- **A delegated supertype:** `I by b` wraps the user type in `explicit_delegation`.
- `by` keeps no leaf.

`===`, `!==` and `..<` are tokens of three bytes, a row of their own (`op3`): a mask and a
`last` two bytes on. The two-byte operators' `pairs` read two bytes only. `as?` stays refused:
the lean lexer reads it as one token, but `as` is a word of `simple_identifier`. A row of its own
at `a` would take first bytes from a kind the token marks name.

Corpus: 204 the same, 287 refused, 0 differ.

## Kotlin, step 18 (`18-objects.bitlex`)

**Object literals.** `object : X() { … }` keeps `object` and `:` as leaves. A statement's
`object` declares one, so the literal is told apart by a mask: an `object` before `:` or `{`,
not after `companion`. The mask joins the shared mark `dotted`, since no expression holds a
receiver.

**Secondary constructors.** `constructor(…) : this(…) { … }` is a class member. The scanner lets
a line that starts with `constructor` go on the line before only in a class's header (a primary
constructor). So `goes` takes `constructor` only within `class_head`, from `class` to its `(`,
`{` or `=`. In a body, the line break before it is a semicolon.

**Destructuring.** `val (a, b) = p` is a `multi_variable_declaration`. Its `(` is a leaf of the
lean tape; its `)` is not.

Refused: `class A : I by d {`. The lean parser ends the delegate's expression before the body,
while here `d {` is a call with a trailing lambda.

Corpus: 221 the same, 270 refused, 0 differ.

## Kotlin, step 19 (`19-type-arguments.bitlex`)

`<` opens type arguments where the lean parser's lookahead does (`type_args_end`):

- they close at a `>`;
- that `>` is followed by `(`, `{`, `.`, `?.` or `::`.

The `generic` mark was a token mark that guessed from the first tokens after a name. It took
`Build.VERSION.SDK_INT < Build.VERSION_CODES.O` for type arguments, and had no way in for
`a.b<T>()`. It is now a mask mark at the `<` itself. It holds where the bytes from the `<` are a
type's (words, blanks, `, . ? * @ : < > [ ] & -`) up to such a `>`, and where the `<` touches the
name before it.

A level's postfix loop takes `generic` and then a call's type arguments (`postfix_level`), after
a name or a navigation alike. A tighter operator's operand does not take it (`postfix_op`): there
tree-sitter's GLR parse puts the call over the whole operation, which is refused. In a type, type
arguments and type parameters take the mark where it holds.

Not the lean parser's, and refused: a spaced `f <T>()`, where the mask needs the `<` to touch the
name; and `<(A) -> B>`, where a `(` is no type byte.

Also refused: a comment on a line that starts with `@`. The annotation takes the line's
semicolon, so the comment would be the annotation's, while the lean parser keeps it the
declaration's.

Corpus: 267 the same, 224 refused, 0 differ.

## Kotlin, step 20 (`20-enums-labels.bitlex`)

What step 20 reads:

- **Enum classes.** An `enum_class_body` holds entries with arguments and bodies, `,` between
  them, then `;` and members. A trailing `,` and the `}` after it are leaves, as in the lean tape.
  The `trail` mark takes `}` for this.
- **`private constructor`.** `decl_mod` holds before `constructor` and `enum`.
- **Reified type parameters**, and `reified` is no longer a soft name.
- **Spread arguments.**
- **`return@x`, `break@x`, `continue@x`, `this@X`.**
- **`as?`.**

The lean lexer reads a word with the `@` it touches as one token, and `as?` as one token. Here
`simple_identifier`'s token ends at that `@` or `?`. `return@` and the others are hard keywords,
so the token takes them without a row of its own. `enum` before `class` gets a row of its own
(`enum_kw`). A shared kind would clash with the soft name `enum`.

`assign_id` was a token mark that named `simple_identifier`. It is now a mask: a word, then
blanks, then an assignment operator. With that, no token mark names `simple_identifier`.

The lean parser's type after `as` is a single segment: `x as java.util.List<Int>` is navigation
over `x as java`. `as_type` takes a single segment, so a `.` after it is refused.

Still refused: a label's definition (`loop@ for …`, `lit@{ … }`). Its own row at every letter
doubled tier 2's tests at letters, which put the program past the 65,533 words that a `u16`
operand reaches. It is open: either a label ends `simple_identifier`'s token at its `@`, as
`return@` does, with the grammar telling a label from a name, or tier 2's operands widen.

Corpus: 311 the same, 180 refused, 0 differ.

## Kotlin, step 21 (`21-raw-strings.bitlex`)

A raw string is a region of `hidden`. It runs from `"""` to the last three quotes of the next
run of three or more (`rstr = dq3 .. rclose bytes 3`). One reader at `"` reads it and a string
(`read str | rstr parts`, compiler item 26).

The lean lexer's scanner gives a raw string's content as follows:

- a token ends after each run of quotes (it reads the run looking for `"""`), and before a
  template;
- the opener's two quotes after its first and the closer are in no leaf;
- the quotes of a closing run past three are in no leaf either.

Here the two opening quotes are a part (`'""'`), and a run of content that reaches quotes is a
part to the run's last quote (`q_start`). The grammar takes both, and the closer, as implied
tokens.

Refused:

- `${…}` in a raw string;
- `\$` (a backslash escapes nothing in a raw string);
- a closing run of four quotes or more;
- an opening run of four or more, except the empty raw string's six. The closer would be
  found inside the run.

Corpus: 327 the same (step 20: 311), 164 refused, 0 differ. The top refusals are still a
template whose code holds a quote, a brace, a comment or a line break (24); a block comment
before a word with `a`, `e` or `w` (18); and the grammar at `.`, `(` and `constructor`.

## Kotlin, step 22 (`22-destructured-parameters.bitlex`)

```kotlin
m.forEach { (k, v) -> g(k) }
for ((k, v) in m) g(k)
val b = x as java.nio.Buffer
fun f(a: (url: String) -> Unit) {}
fun <T : Any> B.get(): T = x
```

- A lambda's parameter and a `for`'s variable may be a `multi_variable_declaration`.
- The type after `as` is a dotted name, or one name with type arguments. With both, the lean
  parser reads a type at a block's end but, where an expression follows on the next line, a
  comparison over navigation (`x as a` `.b.C < D > g()`). A `<` after a dotted name after
  `as` is escaped.
- A function type's parameter is a `parameter` where a name and `:` start it. The mark shares
  `assign_id`'s, now `named` (a name before `=` or `:`): a code holds 16 marks, and the 15
  marks and 2 extras were one too many. No assignment or argument holds a name and `:`.
- A receiver's run (`decl_run`) goes on past a `:` inside the type parameters, so
  `fun <T : Any> B.f()` reads. Nested type arguments there (`<T : C<T>>`) are still refused.

Still refused: a function type's receiver (`HTML.() -> Unit`), and a property's type
parameters (`val <T> List<T>.second`).

Corpus: 355 the same (step 21: 327), 136 refused, 0 differ.

## Kotlin, step 23 (`23-types-and-aliases.bitlex`)

```kotlin
fun f(s: (() -> Int)?) = 1
val l = mutableListOf<(Int) -> Unit>()
typealias O = (Int) -> Unit
@Entity(primaryKeys = [A, B])
class A
@Inject
constructor(private val b: B)
```

- A parenthesized type (`parenthesized_type`, nullable over it) where a `:` is before its
  `(` and another `(` after it. Its mark shares `generic`'s: no type starts with `<`. A
  function type whose first parameter is a function type is refused there.
- `(` and `)` are type bytes for `generic`, so type arguments hold function types.
- `typealias`, with modifiers and type parameters (`type_alias`).
- A collection literal is a primary expression (tree-sitter-kotlin reads it anywhere).
- A line of a class's header that starts with an annotation (no arguments) before its
  `constructor` goes on the line before: the primary constructor's modifiers.

Still refused: `fun interface` (its `fun` opens a function too), `suspend` function types,
an annotated expression (`@Suppress("x") if …`), `constructor` and `by` as names.

Corpus: 375 the same (step 22: 355), 116 refused, 0 differ.

## Kotlin, step 24 (`24-strings-in-templates.bitlex`)

```kotlin
val v = "${System.getProperty("user.name")}"
val p = "${name.replace('.', '/')}/a"
```

A template's code ends at the first `}` on its line, found from the bytes before the regions.
A string or a character in the code is a region, as in any code. Where that `}` is inside one
(`"${f("}")}"`), it opens no tail (`o_tail`), and the file is refused. A brace, a template, a
comment or a line break in the code is still refused.

Corpus: 390 the same (step 23: 375), 101 refused, 0 differ.

## Kotlin, step 25 (`25-class-delegates.bitlex`)

```kotlin
class A(d: C) : C by d {
}
```

The lean parser ends a delegate's expression before the body's `{`, where a name and a `{` are
a call with a trailing lambda. That `{`, after a `by` in a class's or an object's header, has
the mark a class body takes (`class_body = [dotted] '{' …`). The mark ends the expression's
loop, as `trail` ends a list's. It shares `dotted`: no expression's loop takes a `{` that carries it.

The first corpus run read an object literal's delegate (`object : I by itr {`) as a call: the
file differed. The escape had kept it out for the class delegate it held too. The mark now
holds in an object's header as well.

Corpus: 391 the same (step 24: 390), 100 refused, 0 differ. The six other files the escape
refused meet other refusals further on.

## Kotlin, step 26 (`26-variance-accessors.bitlex`)

```kotlin
fun f(a: Array<out Any>) {}
class B<in T>
fun f() {
    get(i).g(o)
    set.add(1)
}
```

- `out` is a soft keyword: a variance (`variance_modifier`) where a type projection or a type
  parameter starts, and elsewhere a name the lean parser reads as a plain `simple_identifier`.
  Types don't take it as a name, so `out` opens one choice only.
- `get` or `set` at a line's start is an accessor, unless a `.` follows it, or (for `get`) an
  argument does: then it is a call or a name, as in a function's body.
- `class` after `::` (`this::class`) is no class's keyword. It began a class header, and
  step 25's body mark then fell on a later `by lazy {`.

Still refused: a `when` entry's trailing comma. The change broke a `when` entry starting with
`in`, so it was taken out of the step. The cause was a slip in the edit, not the mark: the rule
lost its body. Step 29 reads it.

Corpus: 397 the same (step 25: 391), 94 refused, 0 differ.

## Kotlin, step 27 (`27-delegates-and-names.bitlex`)

```kotlin
fun f() {
    val state by viewModel.uiState.collectAsState()
}
fun g(
    set: AttributeSet,
) {}
```

- A declaration's receiver run (`decl_run`) ends after a `by`. The delegate's dots were taken
  for a receiver's: `state by viewModel.uiState` read as a receiver type.
- The accessor mark holds at no `get` or `set` before a `:` (a parameter's name).

The stop is the byte after `by` (`prev(by_w, 2)`), not its `b`. With `by_w` itself the stop
looks two bytes ahead, and the CPU's tier 1 then lost the receiver's mark wherever the run
crossed a word: 16 files were refused that the interpreter read. That is open below (Q186).

Still refused: a generic call on `?:`'s right side (`m ?: f<M>()!!`).

Corpus: 400 the same (step 26: 397), 91 refused, 0 differ.

## Kotlin, step 28 (`28-function-types.bitlex`)

```kotlin
val h: HTML.() -> Unit = {}
fun f(w: A.B.() -> Unit, v: suspend () -> T) {}
```

- A type's last `.` before a `(` ends a function type's receiver (`ft_dot`, in `dotted`): no
  expression has `.(`. Back from it over names, dots and type arguments without blanks, the
  type's first name takes `generic` (`frecv`). The type is then a `function_type` whose
  `receiver_type` holds the receiver's `user_type`. A receiver with blanks in its type arguments
  (`Map<K, V>.()`) puts the mark inside them, where no rule takes it: refused.
- The receiver's first name is no soft word. `suspend` after a `:` and before a `(` takes
  `generic` too and is the function type's `type_modifiers`. One choice opens on both.
- The alternatives after `generic` are one group, `generic (… | … | …)`: a mark is a token of
  its own, so three alternatives that each start with it were not LL(1). The compiler's message
  now says when the token it names is a mark.

Corpus: 405 the same (step 27: 400), 86 refused, 0 differ.

## Kotlin, step 29 (`29-annotated-expressions.bitlex`)

```kotlin
fun f() {
    @Suppress("DEPRECATION")
    if (a) b()
    when (x) {
        B,
        C,
        -> d
    }
}
```

- An `@` whose annotation's `)` comes just before an `if` or a `when` takes `generic`
  (`ann_expr`). Its annotation then begins a `prefix_expression` over the `if` or `when`, as
  in the lean parser's tree. An unmarked `@` begins a declaration's modifiers, so
  `@Suppress("D")` before `val` is unchanged. That annotation's arguments are not optional
  (`args_annotation`), since a `(` after its name could also begin the operand. An annotation
  without arguments before an expression is still refused.
- A `when` entry's trailing `,` before `->` takes `trail` (`"->"` added to its tokens). It is a
  leaf in its own choice (`when_trail`), because `when_entry`'s other `,`s are implied.

Corpus: 409 the same (step 28: 405), 82 refused, 0 differ.

## Kotlin, step 30 (`30-swallowed-comments.bitlex`)

```kotlin
abstract class A {
    val x = 1
    /* c */
    abstract fun f()
}
```

- The lean lexer's `Asi::Swallow` case: a block comment that starts a line after a statement's
  line, before a word starting with `a`, `e` or `w` that does not go on the line. The scanner
  folds the comment into the semicolon it inserts, so the lean tree has no comment there. Here
  the comment's bytes are skipped (`skip swallowed`; several `skip`s are ORed) and its token is
  no start (`swallow_open`).
- The swallow is read only where a semicolon may come: after a line that ends with a word, `)`,
  `]`, `}` or a quote (`ok_lf`), and that is no header's line (`hdr_gap`). A header holds its
  semicolon, so the lean parser's `package_header` ends at the next word. Before `else` it is
  escaped, since a `when` entry holds its line break too. After a `{` no semicolon may come, and
  the comment is an ordinary extra. Other line ends (`,`, an operator) are still refused.
- A pitfall found here: `fill(a, stop)` holds at its start even where the start is a stop byte.
  So "the last solid byte before the line break is X" is `fill(prev(X) & ~solid, solid)`.
  Without the `& ~solid`, a line ending `),` passed as one ending `)`. That gave one differ in
  the corpus, inside an argument list.

Corpus: 414 the same (step 29: 409), 77 refused, 0 differ. The lost comments' open item (below)
now covers the zero-width case only.

## Kotlin, step 31 (`31-soft-templates.bitlex`)

```kotlin
val s = "a $value b"
```

The lean parser gives a soft keyword after a template's `$` a leaf of its own, named after the
word, inside the `interpolated_identifier`. The token kind `interpolated_identifier` takes the
soft keywords (`keywords interpolated_identifier { } soft { … }`, the 44 words the escape
named), and the tree retags one to the node (`soft_interp`). A word with more after it
(`$values`) is a plain identifier, as before.

Corpus: 417 the same (step 30: 414), 74 refused, 0 differ.

## Kotlin, step 32 (`32-constructor-lines.bitlex`)

```kotlin
class A {
    val x = 1
    private constructor(x: Int) : this()
}
class B
internal constructor(x: Int)
```

- `constructor` after modifiers at a line's start is escaped only in a class's header now. In a
  class's body the lean parser reads a secondary constructor, as the grammar did already.
- In a header, a line of visibility modifiers (`vis_w`) or argument-less annotations before
  `constructor` goes on the line (`ctor_ann`): the primary constructor's modifiers.
- A class's header, the run back from `constructor`, and the line's modifiers (`line_mod`) stop
  at a declaration's keyword (`head_stop`). Before this, a header without a body ran on into the
  next declaration: `class A` then `internal class B private constructor(…)`.

Corpus: 418 the same (step 31: 417), 73 refused, 0 differ.

## Kotlin, step 33 (`33-names-after-dots.bitlex`)

```kotlin
fun f(limit: W) = limit.value

internal operator fun g() {}
```

`decl_mod` is now `tokens after ~("." | "?." | "::") …`. After a `.` a modifier word is a
navigation's name, even when a declaration starts on a later line.

A token mark's `after` could not take a `~` (item 28 below), so it can now. `after ~E` holds
where the token before is none of `E`'s, or where no token comes before: a modifier may be
the file's first token. The compiler gives that element an `end`, meaning no record before
holds. Five places implement it: Bitlex's syntax check and evaluator, the validator, and the
interpreter's, Cranelift's and the generated Rust's token-mark tests.

Corpus: 420 the same (step 32: 418), 71 refused, 0 differ.

## Kotlin, step 34 (`34-soft-names.bitlex`)

```kotlin
fun f(constructor: C) = g(by = "foo")
val x = when (c) {
    !in A..B -> 1
    !is C -> 2
    else -> 3
}
```

- `constructor` and `by` are soft words, as in tree-sitter-kotlin. As a name, `constructor` is
  a `simple_identifier` holding its own leaf, as the other soft words are, and `by` is a plain
  one. In the places where the keyword is read, no name can come.
- A `!` that starts a line before `in` or `is` takes `named` (`bang_test`): a `when` entry's
  negated test. Both tests start with the `!`, so they are one node (`neg_test`): it opens as
  a `range_test` and is retagged a `type_test` at an `is`.

Still refused: `fun interface`. The lean parser keeps a `fun` leaf in its `class_declaration`,
but here a function's `fun` is implied. The `interface` after it comes too late to keep it, and a
`relabel` after an implied leaf refuses (an implied leaf is no node it acts on).

Corpus: 423 the same (step 33: 420), 68 refused, 0 differ.

## Kotlin, step 35 (`35-raw-templates.bitlex`)

```kotlin
val x = """a ${b} "c" $d"""
```

A template in a raw string needs the regions to go back, after the template's `}`, to the kind
of string the template interrupted: a raw string reads its quotes as content, and closes at
`"""`, not at `"`. Two additions to `regions` (compiler item 29) make that:

```text
rstr  = dq3 .. rclose bytes 3 or t_dollar
str   = dq & ~dq3 .. dq_close | t_dollar
tail  = t_close .. dq_close | t_dollar after str chr tick
rtail = t_close .. rclose bytes 3 or t_dollar after rstr
```

- `or C2 bytes M`: a second closer with its own length. A raw string's `"""` is three bytes
  and a template's `$` one.
- `after K …`: the kind opens only where the last region opened before, of the kinds with no
  `after`, is one of `K …`. A `}` after a raw string's template opens a raw tail. After a
  string's template, it opens a tail, also when the template's code holds a string, a
  character or a backtick name (`"${f('x')}"`).

A raw tail is read as a raw string's bytes are (`rbody`), so a run that reaches a quote is a
part of its own, as in the lean tree. Its closers are the tail reader's: `'"'` of three bytes
at a `"""`, one at a tail's `"`. `string_literal` takes templates after a raw string's `'""'`.

Refused: a string, a character or a backtick name in a raw string's template's code (the last
plain region would be that one, and its `}` would open a tail), and a raw string in a
template's code.

Corpus: 429 the same (step 34: 423), 62 refused, 0 differ. The 9 files refused for a template in a raw string are read, or stop at a refusal of another kind.

## The compiler's additions

1. **Several kinds of extras in the token step.** Kotlin, Swift and Scala each have two comment
   kinds. Before this, the token step took extras only when there was one kind, and token marks
   need the token step.
   - Every extra's record still has the one extra mark bit. Tier 1, the token loop, Metal and the
     token marks read only that bit, so none of them changes.
   - Each kind after the first has a bit of its own (`Words::extras`).
   - Tier 2 gives the leaf the last such kind whose bit is set, or the first kind
     (`stream::extra_kind`, `MoreExtras` in Cranelift, the same expression in `rust_stream`).
   - This costs one mark for each kind after the first, within `records::MARKS`.
2. **Several kinds read at one first byte.** A line comment and a block comment both start at
   `/`, and the token loop has one reader for each first byte.
   - The readers of the kinds that start at one byte merge into one reader.
   - Each arm of a kind other than the reader's own carries its kind's tag (`ReadArm::retag`,
     `recarm … retag K M`). The CPU reader writes that tag for the arm it read.
   - The mask passes and Metal already retag by each row's start mask (`Records::retags`), so
     they need nothing.
   - `Reader::tags()` and `Reader::tag()` replace the reads of `Reader::retag` alone.

3. **A mark that settles (`mark … , settles`, tier 2's `SETTLE`).** A mark is placed at the
   next grammar token, so the comments before it are pending when its arm closes the
   statement, and `CLOSE` leaves pending extras out (24 §2.3).
   - The compiler gives a settling mark a mask: from each of its bits up to the next grammar
     token's start, `fill(m, starts)`.
   - The mark's leaf in its arm becomes `SETTLE mask`. It sits after the closes of the levels
     that end before the mark, and before the closes of the levels the mark ends.
   - `SETTLE` keeps in place the pending extras, from the first, that start where the mask does
     not hold: the comments before the break. The rest stay pending.
   - The interpreter, Cranelift (a loop over the pending rows, `Step::Node`) and `rust_stream`
     have it. The reference evaluator (`tools/bitlex`) parses the option and does not model it
     yet; no ladder it checks uses it.

4. **`text("…")` in masks.** The bytes of the string from each position, compiled as each byte's
   class shifted back by its offset (`next(c, k)`), and the results ANDed. The reference evaluator
   already had it.
5. **A mark that ends levels (`mark … , ends`).** tree-sitter's scanner gives its semicolon
   wherever one is valid, before the token after it. So `a` and then `b` on the next line are two
   statements, not the start of an infix call.
   - A nested level that may end where an `ends` mark holds, and has no arm for it, tests the mark
     and goes to its end, keeping the mark (`MARK_KEEP`).
   - Its caller then takes the mark.
   - Before this, a hidden mark there was skipped, the level read on, and the file was refused.

6. **A part to a `last`** (`P = E last L`, `PartLen::Last`, `ReadPart::last`). The part runs
   from a bit of `E` through the first bit of `L` at or after it.
   - In masks: its bytes are `fill(ps, prev(L))`, and its last is `L` within them.
   - `read_parts` takes the part's last from `ReadPart::last_at`. The program's text is
     `recpart K START 0 last O`.
   - A's walk searches for the last a word at a time.
   - B (tier 2's split) refuses it for now.
7. **Rows read inside a region are not dispatched by a byte.** A string's content can start at
   any byte. Three places now leave such rows (`Lowering::inside`) out:
   - the search for a free byte for a reader's kind;
   - the token marks' kind check;
   - the grammar's token starts that marks are placed on.

   Tier 2 meets these rows only in the region's mode (0337). Inside the region, a mark can only
   be skipped (if hidden) or refuse the file (if not), so it never reads a token there.
8. **A quiet region (`Reader::quiet`, `recquiet K`).** The lowering proves, with the regions'
   fixed point (`Induction`), that no mask mark and no retag holds in the region's fill past
   its opener, or at its closer. A's walk then writes plain codes (kind and word), and the CPU
   builder no longer refuses a grammar with marks, retags or gaps around such a region.
9. **An output at two delays.** A `prev` of a delayed value is the same operation at one byte
   less of delay. Outputs were deduplicated by operation alone, so `prev(x)` became `x`'s output
   at `x`'s delay: the interpolated name started at the `$`.
   - The second delay now gets an operation of its own: a `prev` by one, at one byte more
     delay. The program's delay grows to match.
   - The masks and the records still find an output by its operation.

10. **`implied` takes a token kind** (`implied R K`) beside a token's text: a part's token the
    node spans but tree-sitter keeps no leaf for.
11. **Several regions read in parts on the CPU.** A's walk is made once for each reader in parts,
    each gated on its own opener; a token that opens none of its regions passes it unchanged.
    Its refusal of marks now asks that every such reader be quiet.
12. **The prover tries each group of position bytes once.** `first_bytes` searched each of the
    256 bytes at the position, and `never` searched all of them even after one was not proved.
    A byte's answer is that of every byte its readers do not tell apart, so it now searches
    one byte per group, and `never` stops at the first byte not proved. Kotlin's step 4 had
    `row_modes` at 46 s, almost all of it here; it compiles in about 2 s again.

13. **A call's arms are one code.** A function a rule makes a call as an arm per token the
    callee may begin with, each opening the callee's node, pushing its returns and going to the
    callee's start, which switches on the token again. Those arms take no token, so their code
    does not depend on it, but it was emitted once per arm. Tier 2 now keys a call's code by
    what it does (`CallCode`): its acts, target, returns, tests, default and rule. Kotlin's
    program fell from 65,203 words to 42,623 (`PUSH` from 7,164 to 522). It had been 332 words
    under the limit.
14. **A program past a word's address is refused.** Places are `u16`. A program past them was
    an overflow panic; it is now an error that names the size, the opcodes, and the rules with
    the most words.

15. **A mark not hidden ends a nested level, as in the reference.** In the reference evaluator
    (`past_marks`), a mark that isn't hidden, met in a nested level, ends the level, and its
    caller takes it or refuses the file. Tier 2 refused it in every state that did not take
    it. It now ends a nested level that may end there, as an `ends` mark does (`AtMark::End`),
    and refuses elsewhere; a mark the level refuses still refuses. `@Inject lateinit var x`
    had been refused at `lateinit`: the called `user_type` ended there, and `modifiers`, two
    levels up, takes the mark.
16. **The interpreter names where it refuses** (`Refused::at`: the first byte of the token).
17. **A region read in parts closes in more than one way** (`closes { K = E bytes N }`,
    `Parts::closes`, `ReadArm::last`). Where `E` holds, the closer is a token of kind `K`, `N`
    bytes long. A Kotlin string's closer is its `"` (one byte), or a template's `${` (two
    bytes). The reader finds the closer's last byte as the first bit of an output (`recarm …
    last O`), as a part to a `last` does. This is supported by the interpreter's
    `read_parts`, the CPU tier 1's `read_in_parts`, and the `recarm` text form. A region that
    tier 2 splits refuses it, as it refuses a part to a last.
18. **A moved reader's kind is met by its kind.** The interpreter switched on a token's
    first byte. A reader moved to a byte no token begins with (item 2) is met at that byte's
    target, which the compiled tier 2 reaches through the record's kind. The interpreter now
    switches as the compiled code does: on the record's kind, at that kind's least byte.
    Comments were the only moved readers before, and as extras they never reached a switch.
19. **A moved byte is its kind's alone.** The rows read inside a region can start with any
    byte, the moved one included, so tier 2 saw two rows at that byte and tested the reader's
    start mask (`ROW`). The CPU tier 1 makes no such mask: its reader gives the kind. Every
    other row now drops the moved bytes from its first bytes.
20. **The size check runs before the return entries.** A saturated place (`pc`) read as a
    state with no `TOKEN`, which hid the real error.
21. **Bytes alike share their `ROW` tests.** At a byte that several rows begin, tier 2 tests
    each row's start mask (`ROW`) and jumps to its arm. Each byte had a chain of its own, so a
    number's ten digits made ten. Chains are now shared by their rule, rows, default and
    targets (`row_chains`). Without that, step 9 was past 64K words.
22. **A `last` search waits for its first word to be final.** The CPU's token loop reads a
    block's rows once the block is done. A delayed output's word for the newest block is still
    unshifted (`delayed_store`), and the next block or the row's end shifts it in. The `Last`
    rule read the token's own word unchecked, so a number at the file's end had its last
    byte read `d` bytes late. Every word it reads is now checked against the ready blocks.
23. **A token mark's kinds exclude their hard keywords** (step 16). The compiled program now
    agrees with Bitlex's evaluator, which gives a keyword's token the keyword's kind.
24. **The SAT solver picks its decisions from a heap** (step 21). Each decision scanned
    every variable for the most active one, so a solve cost the variables times the
    decisions. Step 21's compile spent over 10 minutes there; it now takes 140 s at a load of
    68. Two more fixes:
    - Activities now scale down past 1e100. The bump grew by 1.05 a conflict and reached
      infinity after about 14,500 conflicts, against a budget of 100,000.
    - `analyze` reuses one `seen` vector, where it allocated one a conflict.

    The heap orders by activity, then by number, so a decision is the variable the scan chose.
25. **Tier 2's size is checked on the program** (step 21). Places were `u16` from the first
    word made, so the limit applied to the words before `merge_words` shares those alike.
    Step 20 made 60,900 words there, against 23,000 in its program. The emitter now keeps
    `u32` places (`Place`, and `Switch` for `ByteSwitch`), and refuses a program past `u16`
    once it is laid out. `RERAIL_TIER2_SIZES` prints the sizes on stderr: the words before
    and after sharing, by opcode and by rule, and each row's and each mark's first bytes.
26. **One reader reads several regions in parts** (`read K | L parts`, step 21). Kotlin's
    strings and raw strings both start at `"`, and a byte has one reader. The reader is one
    arm: at the regions' openers taken (`o_K | o_L`), to each region's closer outside the
    others' fills (a `"` in a raw string closes no string), and each closer's last byte by a
    mask (`closer_last`). The runtimes are unchanged. The parts are
    shared; each part's mask holds in its own region.

    Each region's tokens are rows of their own (`Parts::region_rows`), the opener's too
    (`start & o_K`). A region is a mode (`row_modes`), and a row in two regions is in neither.
    Tier 2 then met the string's rows in every state: 69,000 words against 23,800.

    The same happens with a second `regions` block. Its fill is a carry of its own, which the
    hidden regions' proofs don't reach.

Single-extra grammars compile as before: rr's tests and the compiler's targeted tests pass.

27. **A reader's opener from the rows waits for its word to be final** (step 21). The two
    regions' opener (`o_str | o_rstr`) and the raw string's closer (`rclose & ~f_str`) are not
    made from the bytes alone. So the CPU's token loop runs in parts, one a block, each over
    the final words (`ready`). Its refill still reads the next block's starts, which the bytes
    make. A string whose `"` was the first start of a block was then dispatched while the
    rows' word there was not final: the opener's bit read 0, and the CPU refused the file.
    The interpreter read it. The corpus fell from 311 the same to 232 until this was fixed.

    The opener's read in `rule_last`'s reader arm and in `read_in_parts` now waits as the
    scans do (`bound`, `inside_or_past`), when the opener is not from the bytes. A single
    region's opener is from the bytes, and its code is unchanged. The test
    `raw_strings_are_the_lean_parsers` puts strings at those bytes.

28. **A token mark's `after` takes a `~`** (step 33). `after ~E`: the token before is none of
    `E`'s, or there is none. The element at offset -1 then has `end`, and every implementation
    holds it with no record before: Bitlex's evaluator, `stream::token_mark_holds` (as it
    was), the Cranelift stream's pattern, and `rust_stream`'s. The validator allows `not` with
    `end` at -1 only. An `end` stays last among the elements after the mark's token.
29. **A region's second closer, and a kind that goes back to another** (step 35). In
    `regions`, `K = O .. C bytes N or C2 bytes M`: the region closes at either, and one of
    `C2` hides `M` bytes. In the walk (`WalkKind::or`), `closer` is `C | C2` and the bytes
    past a closer at `c` are `M` where `C2` has a bit at `c`. In the rounds, they come from
    the closer's first byte, `c = prev(f) & ~f`: `prev(c & ~C2, 1..N-1) | prev(c & C2,
    1..M-1)`. `K = … after A …` (`WalkKind::after`, a mask of kinds) opens only where the last
    region opened before, of the kinds with no `after`, is an `A`. In the rounds that is
    `o_K = O & ~H & prev(fill(o_A | …, o_others))`, with the plain kinds made first so it reads
    their openers. The walks keep that kind: the interpreter's in a variable, the CPU's in the
    region loop's second parameter and in four bits of its state past the flags
    (`WalkState::width`). A kind whose condition fails takes no opener: it can change only at
    an opening, which ends the search. Bitlex's evaluator runs the rounds (`Regions::body`);
    Metal runs the rounds too. A reader in parts counts its own closers' bytes by
    `Region::closer_last`. A plain `read` of a region with `or` is refused.

## Found on the way

- Q186 (open; Q33 where it was written): the CPU's tier 1 gets a `fill` wrong where its stop is a delayed output (one that
  looks ahead) and the fill crosses a word. `decl_stop = … | by_w` in step 27 lost the
  `dotted` mark at `fun ValueMapperBuilder.forKotlin()` (bytes 2165 to 2192, across 2176) in
  pkl's `ConfigExtensions.kt`; the interpreter's mark is right. `tests::tier1_records` over
  step 27's lexer with `by_w` as the stop reproduces it. The ladder uses an undelayed stop.
  It still reproduces with every mark but `dotted` taken out and `dotted = recv_at`. It does not
  reproduce in a small lexer with the same receiver masks over plain comment and string regions,
  even with `code` or `by_w` made to look up to 30 bytes ahead. So it needs something more of
  step 27's lexer, perhaps its token loop running in parts.

- Open: the lean parser's lost block comments (26 corpus files at step 29, three escapes; step
  30 reads the swallowed ones after a statement's line). After a line
  break, before a token that goes on the line, it keeps the comment as a zero-width
  `multiline_comment` at the next token's start, inside that token's node. Before a word with
  `a`, `e` or `w` (`Asi::Swallow`), the comment is gone, and the statement ends at the word's
  start. Both need tier 2 to write what the bytes don't give: a zero-width extra at a mark's
  position, and a node that ends at a mark. A grammar step that only dropped the comment's
  token was tried and withdrawn.

- Q184 (Q31 where it was written): the ladder test `07-textes` was already red at the merged Parser-generators commit. It is
  theirs to fix.
- Q183 (Q30 where it was written): done in this decision (the settling mark).
- Q185 (Q32 where it was written): a token with no end rule ends at different bytes in the CPU tier 1 and in the interpreter.
  The grammar gives its punctuation a `pair` rule, as rr's does.

## Next

Kotlin's ladder goes on. Next: `init`, `companion object`, `super`,
`when`, `try`, extension receivers (a region whose
code nests: a count of braces), `::`, numbers, then lambdas, `when` and `try`, then enums,
companions, accessors and the remaining operators. Swift and Scala start the same way, in their own crates: their step 1 is
imports and comments, with Scala's comment kinds `comment` and `block_comment`.
