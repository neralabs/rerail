# 0593: The token loop reads a block's first bytes

Status: decided (the user, 2026-11-21: "please address these", of four differences from the lean lexer found by
comparing the Rust; this is the first). Follows 0591.

## Intro

**The problem.** Tier 1's token loop found each token's first byte with a search from the last token's end:
`while let Some(s) = first_clear(&m_ws, at, n)`. The lean lexer's `lex_blocks`, simdjson's stage 2, takes a
block's token starts as one word (`!space & …`), and walks it with `trailing_zeros`. It clears the bits before
each token's end, and goes to the block a token ends in.

## The decision

- **The loop is two loops.** The blocks of 64 bytes from the last token's end, and in each block the bits of
  `firsts = !skip[b] & (!0 << (at - base))`:
  - the token's first byte is `base + firsts.trailing_zeros()`;
  - after the token, `firsts &= !0 << (at - base)`;
  - a token that ends past its block goes on at the block it ends in (`b = at / 64`).
- **A refusal leaves both loops** (`break 'tokens`).

`firsts` does not drop the bytes after a word's first, as the lean lexer's starts do
(`!(word & (word << 1))`). The bits before a token's end are cleared, so they are never read.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. A search from each
token's end reads the skip mask's word again for each token of a block. The block's word read once is
`lex_blocks`' form.

## What changed

- `bitlex_lex_rust.rs`: the loop's template; the body's `break`s are `break 'tokens`.
- `tier1.rs` regenerated.

Kotlin: 56/56 with the corpus; the compiler's bitlex 9/9; bitlex 11/11; `rerail-kotlin-aot` builds without
`test-hooks`.
