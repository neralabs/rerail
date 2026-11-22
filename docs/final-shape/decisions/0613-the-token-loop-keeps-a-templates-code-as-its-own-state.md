# 0613: The token loop keeps a template's code as its own state

Numbered 0562 where it was written (Swift-Kotlin-Scala-support); 0613 here, the hub's 0562 being another.

Status: decided (the user, 2026-11-21: "Next: step C item 3, the escapes tested over code … Then the template
masks, which need state kept in the loop, and the semicolon masks. After those, nothing reads the regions' walk
and it can go (step D)", the instruction "do this"). Follows 0556 (tier 1's masks one pass over the blocks; its
step 2, the regions read in the token loop) and 0607 (a grammar states what the token loop knows).

## Intro

**The problem.** Kotlin's grammar says a template's code as masks over the file. Its `{` is `t_brace`, the byte
after a `$` that closes a string's region. Its code is `t_code = fill(prev(t_brace), rbrace)`, the bytes from
there to the next `}`. Five escapes read them: a brace, a line break, a comment or a template in the code; a
`}` that is in a string of the code; a raw string in the code; a string, a character or a backtick name in a
raw string's template; an empty `${}`. Each was a mask made after the regions' walk, and its bits tested over
the whole file.

The lean lexer reads a template where it reads the string: at the `${`, it reads the code to its `}`. The
generated tier 1 should do the same, from the grammar, so that nothing it reads needs the regions' walk.

## The decision

- **`assume "WHY" at ends NAME`**: mask `NAME` holds only at the last byte of a token that reads a region.
  Kotlin says it of `t_brace`: the `{` is the last byte of the string token that ends at `${`. It changes no mask.
  The prover asks `NAME & ~lasts`, where `lasts` is the last bytes of the rows that read a region (cached as
  0414's). Where it does not prove it, a test build checks that no byte of a token but a region token's last
  holds it. A mask that is a `&` of such a mask is one too (`rt_brace`, the raw string's).
- **What the loop knows at a region token's end** (`at_end`). Where the read ended, at closer `c` and last byte
  `l`, are known:
  - each part's closer, its test over `close`, and where it leaves `l`;
  - the bytes the walk hides from `c`;
  - the opener's bytes after its first that no closer can be. A `//` comment's last byte is past its opener.
  From these the regions' masks are known from the byte before `c` to the byte after `l`. A mask of stage 1 is
  its bit there, or none where it needs a byte the closer's are not (`bytes_at`, through `prev` and `next`).
  The closer itself holds at `c`. So `t_brace` at a string's end is `bit(&m_t_dollar, close)`: the string
  ended at `${`. It is none at the byte after every region token, and at a comment's end.
- **A span** is a fill the loop keeps: `F = fill(prev(T), E)`, with `T` assumed at ends and `E` a mask of
  stage 1. It opens at a region token where `T` holds at the last byte: `span = (l + 1, c)`, where `c` is the
  next `E` from `l + 2` (`first_bit`). An escape that reads the span is told at five places (`in_span`): the
  token's last byte, the span's first byte, the rest of the span, `c`, and elsewhere. It must be none at the
  token's last byte and elsewhere. Where it holds:
  - **as a mask of stage 1 in the span**: tested where the span opens (`any_in(&V, l + 1, c - 1)`,
    `bit(&V, l + 1)`). An example is the brace, line break, comment or template in the code. In the span after
    its first byte, `E` is none.
  - **as a region's mask that holds only at a region token's start**: a test at the starts in the span (`open
    == K`). Examples are the raw string, and the string, character or backtick name.
  - **at `c`**: where a token starts there, its value at the start (the `}` must open a tail). Where a token
    covers `c`, its value inside a token, here `}`, so it always refuses.
- **Outside a region token**, an at-ends mask is none (`in_region`). In a `&`, a side that is none makes the
  `&` none, even where the other side is not known.

## Kotlin

All five template escapes are now tests in the token loop. `t_brace` at ends is checked by the tests: the prover
does not reach it yet. Its fills `t_code` and `rt_code` are spans 0 and 1.

## Why not

- **Hand-writing Kotlin's template state in the compiler**: it would not come from the grammar, and a grammar
  with another template would get none.
- **The compiler finding that `t_brace` holds only at token ends alone**: it depends on how every row reads, a
  fact about the token loop, as 0607's are. The writer states it, and the prover or the tests check it.
- **A region kind for a template's code**: its bytes are code, read as tokens. A region hides them.
