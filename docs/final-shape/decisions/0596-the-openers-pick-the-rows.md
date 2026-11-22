# 0596: The openers pick the rows

Status: decided (the user, 2026-11-21: "please address these": the fourth, "strings and comments are opened as
numbered regions with `plain` state; lean reads a string to its end"). Follows 0595 and 0588 (the dispatch).

## Intro

**The problem.** The loop picked a token's region and then its row:

```
let open = match group { 5 => if bit(&m_dq3, s) { 2 } else if bit(&t5, s) { 3 } else { usize::MAX }, … };
if matches!(open, 0 | 1 | 2 | 3 | 4 | 5) { plain = open; }
let row = match group { 5 => if open == 3 { 7 } else if open == 2 { 8 } else { usize::MAX }, … };
```

`open` was read after the row too: by the gap masks' `here_j` and by two start escapes (`open == usize::MAX`).
The lean lexer's token reader takes `"` and reads the string to its end (`string_end`): the first byte picks
the reading, with no region number between.

## The decision

- **Fused.** In a group whose region rows are all before its rows of code, and where each region it may open
  has a row, each opener's test picks its region's row, and the rows of code follow. Then nothing makes
  `open`.
- **Each row knows its region.** By the dispatch (0595's `row_opens`):
  - the row of a region of `plain`'s kinds sets `plain = k`;
  - each gap mask's `here_j` is set in each row, folded (`here_1` is `xs_2` in a string's row);
  - the start escapes are folded, so the `;` escape is `bit(&m_semicolon, s) && fs_3`.
- A row that still reads `open` is refused, as a generator error.

`plain` stays, as the lean lexer's state for a template's `}`: which string's tail a `}` reopens (`string_end`
returning to its loop).

Strings and comments still end where their read's stop mask finds their closer (`first_bit(&t6, s + 1)`), as
`string_end`'s `string_stop`. A template's code is still read as the code's tokens: tier 2 reads them as records,
where the lean parser reads a string's pieces again from the string.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The region number was
a step the lean lexer does not take.

## What changed

- `bitlex_lex_rust.rs`:
  - `Picks.fused` and the fused chains in `dispatch`;
  - `RowArm.starts`, folded after the dispatch;
  - `plain` and `here_j` per row;
  - a def every row reads stays at the loop's top.
- `tier1.rs` regenerated.

Kotlin: 56/56 with the corpus; the compiler's bitlex 9/9; bitlex 11/11; `rerail-kotlin-aot` builds without
`test-hooks`.
