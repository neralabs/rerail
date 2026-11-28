# 0624: Block comments one after another after a line break

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0451.

## Intro

**The problem.** After a line break where a semicolon may come, the lean lexer's `asi` reads a block
comment that starts the next line through, and decides by what follows it. When another comment follows,
the first loses its bytes: tree-sitter gives it no width, at the next one's start (`lost`), and `asi` is
asked again from there (decision 0451's `loses since`). The grammar said that for a line comment after it,
and refused every block comment after it: `val a = 1\n/* c */ /* d */\nexternal fun f()`, and two comments
before an `if`'s `else`.

Asked again at the second comment, `asi` is on the token's line, not past a line break: it reads the comment
as the line's and asks again past it. So:

- a line break after it decides at the token after, as after any token;
- a token on its line gets no semicolon, and the line goes on;
- the second comment is never swallowed or lost there, though a line break may be before it.

## The decision

- **`own_line`**, a hidden mark: a token first on its line, with no comment before it there
  (`solid & prev(fill(lf & code, ~ws))`).
- **`semi`'s questions**:
  - `loses since … multiline_comment with semi multiline_comment before with own_line`: the first comment
    loses its bytes, and the line break after the second decides at the token, by `semi`'s other questions.
  - `refuses since … multiline_comment with semi multiline_comment before without own_line`: a token on the
    second comment's line. There the lean lexer inserts no semicolon. A mark's questions can't make the mark
    not hold, so this stays refused.
  - `drops unless after ~(multiline_comment | line_comment) multiline_comment with semi multiline_comment
    with semi`, and the same in `loses`: where the second comment is the last before the token, the
    questions that drop or lose the comment pending last don't apply. A third comment on a line of its own
    is decided as one after a line break is, as `asi`'s recursion does.
- **Bitlex**: a gap pattern's element may be of marks alone (`with M`, `without M`: any record), as one of
  first bytes alone already was.
- **The lowering**: a `~` of every extra's kind is a grammar token, a record with none of the extras' bits
  (`extra_mark`).
- **Tier 1 as Rust**: a gap mark `A & prev(fill(M, ~W))`, where `W` holds at every skipped byte, `M` is
  a mask the loop keeps an accumulator of, and `A` holds at a grammar token's start. Its bit at a grammar
  token is the accumulator's since the record before (`rec_J`).

## Why

The criterion is the lean lexer's `asi`: each case above is a path of its code. The case left refused is
one where `asi` declines the semicolon after the grammar has given it, and would need a mark whose
holding depends on the comments between tokens.

## What changed

- `tools/bitlex/src/syntax.rs`: `gap_elem` of marks alone.
- `bitlex_masks.rs`: the `~` of every extra's kind.
- `bitlex_lex_rust.rs`: `Gap::rec`.
- `kotlin.bitlex`: `own_line`, `semi`'s questions. `tier1.rs`, `tier2.rs` and `kotlin.answers`
  regenerated.
- Tests: `block_comments_one_after_another_are_the_lean_parsers` (two and three comments on a line, on
  lines of their own, a line comment after them, before `external`, `abstract`, `.`, `+`, an `if`'s
  `else`, a call, in a class body). The token on the second comment's line is in the refused list.
