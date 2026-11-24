# 0595: A template's span is tested in the rows it may hold in

Status: decided (the user, 2026-11-21: "please address these": the third). Follows 0594 and 0562 (the loop keeps a
template's code as its own state).

## Intro

**The problem.** A span (`t_code`, a `${…}`'s code; `rt_code`, a raw string's) was tested at the loop's top, at
every token: `if s >= span_0.1 { … } else if s >= span_0.0 { … }`, and again after the loop. That covered:
- its end, the token at it;
- a token that covered its end, found at the token after;
- the starts in it.

The lean lexer reads a template's code inside the string (`interpolation_end`) and tests nothing at other
tokens.

## The decision

- **Each test is in the rows it may hold in.** A row's `open` values come from the dispatch (`row_opens`):
  those of its groups no row before it is picked at alone. Each test's `open == K` is folded against them
  (`fold_open`, `fold_bool`).
  - **The starts in it** (`s >= span.0 && s < span.1`): in each row where a test is not `false`. Kotlin: the
    raw string's row, and the `rt_code` test in the string's, the character's and the backtick name's rows.
  - **Its end** (`s == span.1`): in each row whose first bytes may be the span's end byte. Kotlin: the
    punctuation's.
  - **A token that covers its end** (`s < span.1 && span.1 <= l`): in each row whose bytes after its first may
    be the end byte. A region token's may be any byte.
- **No reset, no test after the loop.** Each test is bounded by the span, so a span already passed is never
  read again. A token that covers the end is found in its own row, so nothing is left for the end of the file.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The tests are where
the tokens they are about are read.

## What changed

- `bitlex_lex_rust.rs`:
  - `RowArm` carries its first and tail bytes;
  - `dispatch` returns each row's `open` values;
  - the span tests are made per row;
  - `fold_open` and `fold_bool` are new.
- `tier1.rs` regenerated.
- The test `the_template_spans_are_the_interpreters`. It has a text where what follows still reads as tokens
  (`"${"}"}""`, `"${'}'}""`, `"${"""x"""}"""`). Turning off the string's or the character's "covers its end"
  test, the raw string's test, or the `rt_code` test in the generated file fails it.
- The end test at the punctuation's row never holds in Kotlin. The `}` there is punctuation only where the
  region opened last is no string, character or backtick name, so a comment. `t49` refuses a comment in a
  template's code.

Kotlin: 56/56 with the corpus.
