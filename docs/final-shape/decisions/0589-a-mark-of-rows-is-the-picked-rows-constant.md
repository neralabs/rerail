# 0589: A mark of rows is the picked row's constant

Status: decided (the user, 2026-11-21: "now please again compare the rust code"). Follows 0614 (the first
byte's group picks the row).

## Intro

**The problem.** After the dispatch picked a token's row (0614), the loop still tested two token marks from
mask bits at the token's first byte:

```
if (open == 5 && (((bit(&m_alpha, s) || bit(&t96, s)) || … || bit(&t105, s))) { marks |= 1 << 6; }
if (bit(&m_alpha, s) && (((bit(&m_alpha, s) || bit(&t96, s)) || … || bit(&t105, s))) { marks |= 1 << 7; }
```

Each is a mark of rows (`CodeMask::Rows(a, b)`): some row of `a` and some of `b` start there. With the row
known, these are "a backtick name" and "a name". The lean lexer's token reader returns them with the token
(`Tok`'s `tick`), as part of what it read.

## The decision

- A mark of rows with no word is decided at the row picked, `p`:
  - a set's row starts at `s` where `p` is one;
  - it may start where one of the set is after `p` in a group's chain that `p` is in (the rows before `p`
    there do not start);
  - else it does not.
- Where every row decides it, the mark is in each row's constant marks, the record's `(l, kind, extra, MARKS)`.
- Otherwise the mark keeps its test after the row, as before.

Kotlin's two marks are the name's row (`0x80`) and the backtick name's (`0x40`); nothing tests them after the
row.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. A test after the row
asked again, from masks, which row the token was.

## What changed

- `bitlex_lex_rust.rs`:
  - `RowArm` carries its row and its constant marks;
  - `dispatch` returns each group's row chain;
  - the marks of rows are decided per row.
- `tier1.rs` regenerated.

Kotlin: 55/55 with the corpus. `compile_time` timed out once, in the full run at a load of 40 on 18 cores (49
tests slow), and passed alone: 62 s cold, 25 s warm. `rust_is_current`, which compiles the grammar too, took
44–45 s before and after.
