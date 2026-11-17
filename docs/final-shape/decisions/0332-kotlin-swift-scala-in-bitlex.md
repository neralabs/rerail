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

Single-extra grammars compile as before: rr's tests and the compiler's targeted tests pass.

## Found on the way

- Q31: the ladder test `07-textes` was already red at the merged Parser-generators commit. It is
  theirs to fix.
- Q30: done in this decision (the settling mark).
- Q32: a token with no end rule ends at different bytes in the CPU tier 1 and in the interpreter.
  The grammar gives its punctuation a `pair` rule, as rr's does.

## Next

Kotlin's ladder goes on. Next: `init`, `companion object`, `super`,
`when`, `try`, extension receivers (a region whose
code nests: a count of braces), `::`, numbers, then lambdas, `when` and `try`, then enums,
companions, accessors and the remaining operators. Swift and Scala start the same way, in their own crates: their step 1 is
imports and comments, with Scala's comment kinds `comment` and `block_comment`.
