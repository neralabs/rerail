# 0590: An escape at a token's start is tested in its rows

Status: decided (the user, 2026-11-21: "now please again compare the rust code"). Follows 0589 and 0587 (an
escape of the code is tested in the tokens it may hold in).

## Intro

**The problem.** Two escapes were tested at every token's first byte, before its row was picked:
- "a `;` that starts a line", as
  `(open == usize::MAX && bit(&m_semicolon, s)) && ((open == usize::MAX || (… open == 7)) && fs_3)`;
- "a dotted type after `as` before a `<`" (0609).

The lean lexer tests a `;` where it reads one.

## The decision

- **An escape at a token's start is tested in each row whose start bytes it may hold at**, first in the
  row's code. It is read by `at_start` of the row's start bytes (`start_in`). The row picked holds at its
  token's first byte, so that byte is one of its start bytes, and no escape is missed.
- **`at_start`'s `&` absorbs**: each conjunct once, and a disjunction one of the other conjuncts is a part of
  is left out (`x && (x || y)` is `x`).

Kotlin's two escapes are in the punctuation's row alone, the `;` one as
`(open == usize::MAX && bit(&m_semicolon, s)) && fs_3`.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. A test before the row
asked, of every token, whether it was a `;` or a `<`.

## What changed

- `bitlex_lex_rust.rs`:
  - `start_checks` keep their mask and its bytes, and each row tests those that may hold at its start;
  - `absorb` and `parts_of` in `at_start`'s `&`.
- `tier1.rs` regenerated.

Kotlin: 55/55 with the corpus; the compiler's bitlex 9/9.
