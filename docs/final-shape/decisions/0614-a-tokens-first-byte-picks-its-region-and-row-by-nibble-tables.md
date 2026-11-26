# 0614: A token's first byte picks its region and its row by nibble tables

Numbered 0588 where it was written (Swift-Kotlin-Scala-support); 0614 here, the C preprocessor's 0588 being another.

Status: decided (the user, 2026-11-21: "no, use simdjson's approach for this where possible. We already have
done some of the proving infrastructure", then "I mean 1": the first byte's class from nibble tables picks the
row, as stage 1's classes are found, in place of a `match` on the byte). Follows 0587.

## Intro

**The problem.** The token loop picked a token's region and its row by two chains, each arm one mask's bit at
the token's first byte, in the grammar's order:
- `if bit(&m_line_open, s) { 0 } else if bit(&m_block_open, s) { 1 } else if …` for the region opened;
- `if (bit(&m_slash, s) && open == 0) { … } else if bit(&m_alpha, s) { … } else if bit(&t96, s) { … } …` for
  the row.

So a token of punctuation read every arm's mask before its own. The lean lexer's block lexer, simdjson's two
stages, reads a token's first byte once, by tables (`NAME_START`, `ONE`).

## The decision

- **The first byte's group, by nibble tables** (`dispatch`):
  - The start bytes are split in groups: two bytes are alike where each opener and each row may start at both
    or neither (`bytes_at`), and starts at every byte of both or not (`exact_bytes`).
  - Each group is a few products of high and low nibbles, one bit each in two tables, `DISPATCH_LO` and
    `DISPATCH_HI`. This is simdjson's `classify` and stage 1's classes' form.
  - A token's group is the lowest bit of `LO[c & 15] & HI[c >> 4]`.
- **In a group, only the arms that may start there**, in their order:
  - each is read as `at_start` reads it, of the group's bytes (`start_in`), so one that starts at every one of
    them is no test;
  - a row's test of the region opened is folded against the regions the group may open, so `'` opens a
    character and is its row, with no test.
- **Then the row's code, once** (`match row { … }`). A test build's checks where the loop first reads an
  assumption stay in that row's test.

Kotlin's 13 rows and 8 regions are 16 products in `u16` tables:
- a letter or `_` is the name's row, with no test;
- `'` and `` ` `` open their regions and are their rows;
- `/` tests the comments' openers only;
- `"` tests the strings' openers only;
- a digit tests the numbers' rows only;
- punctuation tests its own row only.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The lean lexer
decides a token's kind from its first byte with a table, and simdjson finds a byte's class with two nibble
lookups. The grammar's rows are proved disjoint enough by their byte sets that one lookup leaves at most a few
tests, and those only where a row's start reads its context (a number not inside a name, a `/` that opens a
comment).

## What changed

- `bitlex_lex_rust.rs`:
  - `dispatch`, `RowArm`, `Picks`, `if_chain`;
  - `at_start` takes a universe of start bytes (`start_in`);
  - the rows and the openers are arms, not chains.
- `tier1.rs` regenerated: `DISPATCH_LO`, `DISPATCH_HI`, `match group` for `open` and the row, `match row` for
  the rows' code.

Kotlin: 55/55 with the corpus; bitlex 11/11; the compiler's bitlex 9/9; `rerail-kotlin-aot` builds without
`test-hooks`.
