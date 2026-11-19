# 0430: A line break before a token is the token loop's, as the lean lexer's `nl`

Status: decided (2026-11-18), the first step of 0429's "marks at every token" and "automatic
semicolons" (items 3 and 6). Follows 0429, 0177 and 0406.

## Intro

Kotlin ends a statement at a line break, unless the next line goes on with it. The lean lexer
notes, for each token, whether its gap held a line break. That is `Tok::nl`, set while
`Lexer::skip` passes the blanks and comments. The parser asks the full question only where its
state takes a semicolon: `t.nl && self.ends()`. Only `asi()` reads the bytes after the token.

The generated parser does it the other way round. Tier 1 computes the semicolon as a mask at
every byte of every block (`lf_end`, and `after_semi` under it). A pass after the tokens then
lifts each bit to the next token (`late_marks`), and tier 2 tests the mark at every token.

## The decision

1. **The grammar says what the lean lexer notes.** `semi`'s mask is a line break in code, the
   gap's (`lf & code`). The `;` before it is the grammar token before (`unless after ";"`), not a
   fill over the bytes after the `;`.
2. **The token loop notes it.** On the CPU, a gap mark whose mask is a class of blank bytes
   in code is set while the token loop skips the gap. That is where the lean lexer sets `nl`;
   there is no mask and no late pass for it. The compiler finds such a mark by its mask's shape
   (a byte class, and `code`), never by its name. Metal and Bitlex's reference keep the mask
   lifted to the next token, as now.
3. **Tier 2 asks where its rule takes one.** This comes after steps 1 and 2: the mark is tested
   where a rule can take a semicolon, as `ends()` is, not at every token.

Each step keeps the tapes the same: the kotlin-parser tests, the corpus against the lean
parser's tapes, and `aot_tapes_are_the_jits`.
