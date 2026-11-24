# 0594: A kept fill's value is made in the rows that read it

Status: decided (the user, 2026-11-21: "please address these": the second). Follows 0593, 0609 (the type after
`as` a kept fill) and 0564 (the loop keeps the semicolon's fills).

## Intro

**The problem.** Each kept fill's value before the token (`fs_i`, `xs_i`, `fgap_i`) was made at the loop's top,
for every token. The lean lexer keeps a token's state for semicolon insertion (`nl`, and `carry` over comments),
and reads it where it reads a `;`.

Most rows read the `as` type's fills (`fs_4`, `fs_6`) only through their updates. A region token's update was
`fill_4 = xs_4 && l == s`, the fill dying after the token's first byte. That is false: no region token is one
byte.

## The decision

- **A region token is never one byte** where its read looks for the closer from the byte after its first and
  ends at the closer or after it (`closer_last`), or where it ends before its closer and its opener's second
  byte is no closer's (`read_never_one_byte`). There a dying fill's update is `false`.
- **A def at the loop's top only where code outside the rows reads it, or every row does.** Otherwise it is
  first in each row that reads it, after the row's comment.

Kotlin:
- `fs_2`, `xs_2` and `fgap_2` (after a `;`) stay at the top; the gap marks read them.
- `fs_3` and `fs_5` (a line end, a `;`) are made in the comments' rows, which carry them, and in the
  punctuation's, which reads the `;` escape.
- `fs_4`, `xs_4` and `fs_6` (the type after `as`) are made in the rows of code: the name, the numbers, the
  punctuation and the backtick name.
- The region rows make none: their updates are `false`.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The lean lexer reads
a token's state where a token's reading needs it.

## What changed

- `bitlex_lex_rust.rs`:
  - `read_never_one_byte`, in the kept fills' region updates and `within_fill`'s;
  - the top's defs are kept at the top or put in the rows (`//~defs`).
- `tier1.rs` regenerated.

Kotlin: 56/56 with the corpus.
