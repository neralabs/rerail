# 0406: `back_fill` is removed from Bitlex

Status: decided (the user, 2026-11-18: "remove the back fills. Start by removing that from the
bitlex language or marking it deprecated and making it crash with a message describing how to
build it properly. Then make the fixes and transformations to make the grammar generate the same
general structure and peeks as the lean parser"). Follows 0405 (Kotlin's generated parser in the
lean parser's shape).

## Intro

**The problem.** `back_fill(S, E)` is a mask whose value at a byte is made from the bytes after
it: it holds where `S` holds at the first byte from there on that is in `E` or `S`. A program
that uses one has to be made over the whole file, and then filled backward, before the value's
first word is known. In tier 1 each level of back fills is a pass of its own over the file, with
the values between passes stored a block at a time. Kotlin's step 35 has 36 of them, three deep:
7 passes and 555 stored words a block (0405).

The lean parsers never look at the bytes that way. They look ahead from where they stand, and
only where they need to know: the next token (`peek`), a run of tokens up to a `->`
(`lambda_params_end`), the bytes after a token when the parser's state can take a semicolon
(`Lexer::asi`).

## The decision

A grammar may not write `back_fill`, as a mask or as a number. The parse refuses it with
`syntax::BACK_FILL_REMOVED`, which says what to write instead:

| What the back fill asked | What the grammar writes | The lean parser's form |
|---|---|---|
| the next token, or a run of tokens from here | a token mark, `mark K = tokens [after E] A B …`, with `(A \| B)*` for a run | `peek`, `peek_at`, a forward scan |
| a gap mark that the token after the gap decides | `mark K = E, before A \| B …`, `mark K = E, unless before A \| B …` | `Lexer::asi`'s test of the next token |
| the token's own later bytes | a test at the token: `escape "…" at KIND when …`, a mark's `at KIND when …` | the token's reader |
| what the text is inside: a header, a declaration's run | the tree's rules | the parser's state |

A grammar not yet moved says `deprecated back_fill` on a line of its own. That is the
deprecation period: Kotlin's step 35 and the `.rr` ladder's steps 15 to 21 carry the line, and
so do the tests of the IR's backward pass. Kotlin's step 36 is moved: it has no `back_fill`
and no such line (below).

**What stays.** The block program's `BackFill` operation stays an operation of the IR: Metal's
lowering and the interpreter's keep it. The language writes one itself for a region kind with
`else` (`Regions::body`). There it means "this region's closer comes later in the file", and on
the CPU that meaning is the region's reader, which looks for its closer from its opener: `.rr`
step 22's tier 1 makes no backward pass.

## A gap mark's next token: `before`

`mark K = E, before A | B …` holds only where the grammar token after its gap is one of the
choices; `unless before A | B …` only where it is none of them. Kotlin's semicolon was
`ends_line & ~back_fill(code & rbrace, solid)`: a line break whose next solid byte is not a `}`.
It is now `mark semi = ends_line, unless before "}", end, settles, ends`, and the `}` after a line
break is `mark semi_close = close_at, before "}", …`.

The compiler gives the mark a pattern of one element at offset 0, the token it lands on, which
tier 2 tests as it tests a token mark's run (0129): the interpreter's `token_mark_holds`,
Cranelift's `token_pattern`, the generated Rust's `pattern_N`. A lazy gap mark (0177) asks its
gap first, then its pattern. At the stream's end there is no token: a mark the end has holds there
without its pattern, in every executor. Tier 1 makes nothing for it, and no pass.

Kotlin's step 36 (`36-no-back-fills.bitlex`) starts from step 35 with these two back fills gone:
the corpus is unchanged, 429 files the same as the lean parser's tapes, 62 refused, 0 differ.

## Runs of tokens, and several `before`s

A token mark's run, and a `before`'s, may hold one run of choices, `(A | B)*`, after its first
token and before its last: any number of tokens of those choices, then the elements after it,
tried from each of them on, as a forward scan for the first place they hold. That is the lean
parser's forward scan (`lambda_params_end`, `type_args_end`): it reads tokens from where it
stands until what it looks for holds, or a token it cannot pass. So `"<" (types)* ">" "("` holds at
`Map<A, List<B>>(`, whose first `>` has no `(` after it.

A mark may say `before …` and `unless before …` more than once. It holds where none of its
`unless before`s do and, if it has a `before`, one of them does. The program gives each its own
token mark entry of the mark's order, with `unless` on the negative ones
(`program::patterns_hold`). A `MARK` tests them together. A mark that refuses wherever it holds,
tested once a token by its bit, keeps one pattern.

In tier 2, a run is a stated loop over the records (`Step::Token`), as a skip over the extras is.
The rest after it is tried at each turn: in Cranelift's `token_pattern`, a block per turn; in the
generated Rust, `pattern_N_rest(at)`. The interpreter matches in turn (`stream::token_follows`),
and so does Bitlex's reference (`eval::seq_holds`).

A run may be of a `~`: `(~("@" | "{" | ";"))*`, any number of tokens none of those. Each
executor's element test already took a `~`.

## A token's kind from its own bytes

A decimal number's kind (integer or real) was told at its first byte by a back fill over its
digits. The lean lexer reads the number, then names its kind. The grammar now has one row for
decimal numbers, `integer_literal`, and a mark that tests the token's own bytes
(`mark real = empty, at integer_literal when count(frac_dot | e_at | f_at) at least 1`, the
token loop's test, decision 0177). The tree takes `real integer_literal as real_literal`. A number
that starts with a `.` (`.5`) is real, a row of its own.

A mark's token test may now test a kind at a first byte that another kind's rows start at too
(`0` starts a hex number too). The test runs on every token that starts there, so its mask
must hold in no token of the other kinds. The fraction, the exponent and the `f` are none of a
hex number's. An escape (`escape … at KIND when …`) is still refused there: it would refuse the
other kind's tokens.

## A comment the semicolon swallows: `drops`

The lean lexer's `asi` reads a block comment that starts the line after a line break, and
looks at what follows it. Before a word that starts with `a`, `e` or `w`, it swallows the
comment into the semicolon (`Asi::Swallow`): the parser removes the comment from its extras. The
grammar did this in tier 1, with back fills: it skipped the comment's bytes, so they made no
token. That is a choice tier 1 cannot make forward, because the word after the comment decides it.

Now every comment is a token, and the semicolon's arm drops it:
`mark semi = …, settles, drops W`. Where the mark's arm runs at a token where `W` holds, the
extra pending last is dropped from the tape (`SETTLE m=1`). `W` is forward
(`swallow_at = awe_word & after_sw`): the word right after such a comment, so the extra pending
last is that comment. The interpreter pops the node. Cranelift and the generated Rust step the
node count back, and the next node takes the row and its cells. A comment the parser's state
does not ask about stays, as in the lean parser.

## Escapes told forward

The grammar's escapes that looked past a comment (`next(back_fill(goes, solid))`, a `;` with
nothing after it) are told at the byte that decides them. Since an escape refuses the whole
file, the byte it is told at does not matter. So `block_end & next(back_fill(X, solid))` is
`X & prev(fill(block_end, solid))`: `X` at the first solid byte after the comment. An `else`
that goes on is told at the solid byte after the word, a `constructor` after modifiers at the
`constructor` (`prev(fill(start, stops))`), and a `;` that ends the file at its last byte.

## Kotlin's step 36

Step 36 has no `back_fill`. What each back fill became:

| Back fill | Now |
|---|---|
| the semicolon's next token (`goes`, `else ->`, `constructor` in a header) | `semi`'s `unless before`s; a second statement in a class's header |
| `}` after a line break | `semi_close = close_at, before "}"` |
| a lambda's parameters to `->` | `mark lam_params = tokens "{" P (P)* "->"` |
| an accessor's `get` / `set` | `acc`'s `unless before`s |
| type arguments to `>` and `(`, a parenthesized type, `suspend`, a function type's receiver, an annotated `if` / `when` | `generic`'s `before`s, one mask (each starts with its own token) |
| a declaration's receiver and its last `.` | `dotted`'s `before`s with a run |
| a real number | the token test `real` |
| a raw string's run to its quotes | a part that ends at the first of its quotes and its end |
| a swallowed comment | `drops` |
| the comment escapes, the `;` at the file's end | forward escapes |

Tier 1 is one forward pass with nothing stored between passes. The corpus is unchanged: 429 files
the same as the lean parser's tapes, 62 refused, 0 differ. The generated Rust's tapes are the
JIT's (`aot_tapes_are_the_jits`).

Each mark statement that has a pattern of its own takes a bit of the record's code, and a code
holds 16 (the extras take 2). Two statements of one kind whose patterns start with different
tokens share a mask (`dotted`, `generic`).

The forward scans differ from the lean parser's in two places, each an open item:
- A run stops at the first place its rest holds, not where a depth closes.
  `type_args_end` counts `<` and `>`.
- A run passes line breaks: they are no tokens. The annotation before `if` / `when` ran to
  a line break before.

## Tests

- `syntax::tier2_tests::a_settling_mark_drops_an_extra`: `drops` parsed with `settles`,
  refused without.
- `kotlin_parser::tests::swallowed_comments_are_the_lean_parsers`: the dropped comments, on
  Cranelift and the interpreter; the corpus and `aot_tapes_are_the_jits` for the generated Rust.
- `syntax::tier2_tests::back_fill_is_refused`: a mask's and a number's `back_fill` are refused
  with the message; with `deprecated back_fill`, parsed.
- `syntax::tier2_tests::a_mark_is_before_a_token`: `before` and `unless before` parse to their
  sequences, several on one mark, a run among them; `before ~X`, `before end`, a run first or
  last, two runs and a run of a `~` are refused.
- `ladder_tests::a_gap_mark_is_decided_by_the_token_after`: a line break ends a header unless a
  `.` and a name, or `.*`, come after it (two `unless before`s), past a comment; an import's
  alias is a token mark with a run (`simple_identifier (simple_identifier | ".")* "as"`), over
  a line break; the reference's nodes are the interpreter's and Cranelift's
  on the grammar's sentences, their mutations and fixed documents. Its grammar has no `settles`,
  which the reference does not implement (Q220).
- The tests of the backward pass (`remade_tests`, `gating_tests`, `compile_tests`' aligned
  ones, bitlex's number tests) say `deprecated back_fill`: they test the IR, which keeps it.
