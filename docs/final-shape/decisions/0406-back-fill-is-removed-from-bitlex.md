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
| a gap mark that the token after the gap decides | `mark K = E, unless at A …` | `Lexer::asi`'s test of the next token |
| the token's own later bytes | a test at the token: `escape "…" at KIND when …`, a mark's `at KIND when …` | the token's reader |
| what the text is inside: a header, a declaration's run | the tree's rules | the parser's state |

A grammar not yet moved says `deprecated back_fill` on a line of its own. That is the
deprecation period: Kotlin's step 35 and the `.rr` ladder's steps 15 to 21 carry the line, and
so do the tests of the IR's backward pass. When Kotlin's grammar is moved, the line goes from
the language as well.

**What stays.** The block program's `BackFill` operation stays an operation of the IR: Metal's
lowering and the interpreter's keep it. The language writes one itself for a region kind with
`else` (`Regions::body`). There it means "this region's closer comes later in the file", and on
the CPU that meaning is the region's reader, which looks for its closer from its opener: `.rr`
step 22's tier 1 makes no backward pass.

## Tests

- `syntax::tier2_tests::back_fill_is_refused`: a mask's and a number's `back_fill` are refused
  with the message; with `deprecated back_fill`, parsed.
- The tests of the backward pass (`remade_tests`, `gating_tests`, `compile_tests`' aligned
  ones, bitlex's number tests) say `deprecated back_fill`: they test the IR, which keeps it.
