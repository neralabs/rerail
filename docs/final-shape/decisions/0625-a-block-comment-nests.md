# 0625: A block comment nests

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0621.

## Intro

**The problem.** A Kotlin block comment nests: `/* a /* b */ c */` is one comment. The lean lexer's
`block_comment_end` counts its `/*` and `*/` from the opener on. The regions closed a comment at its first
`*/`, so the grammar refused every block comment with a `/*` in it, by an escape.

The count is state carried along a region, as decision 0621's nest of braces is. That nest, though, is
between regions: a template's code. This count is within one kind: its own openers inside it.

## The decision

- **`K = O .. C nests D else E`** (`Region::nests`, `D` from 1 to 15). In the region, an opener that is not
  at a closer opens a level, and a closer closes the innermost one. The region closes at a closer with none
  open. `E` holds at the byte after an opener past `D` levels. A kind that nests has no `or`, `outside`,
  `else` or `after`.
- **The rounds** (`Regions::body`). A level `j` is a carry `K_lv{j}`, and `K_in{j}` is "at level `j`, not
  deeper". The fill closes at `closers_past & K_in0`. A level opens at the byte after an opener in the fill:
  `prev(O & ~C) & prev(f_K, 2)`. Read at the opener itself, the `prev` of the fill would fold into the
  opener's read ahead (`next(star)`), and a carry would reach its next with no `prev`.
  `next K_lv{j} = fill(push_j, pop_j)`, as the nest's levels are.
- **The lowering.** Regions with a kind that nests lower through the generic rounds, as a nest does
  (`nested_regions`). The token's reader arm reads to `closers_past & K_in0`, the closer the rounds found.
  So the interpreter and the JIT read the token's end where the masks put it, with no count of their own.
- **The Rust writer**, phase 1 and 2: the closer search is the lean lexer's loop. From the byte after the
  opener, the next closer, unless an opener comes first, which opens a level. A closer with a level open
  closes it (`closer_from`). Past `D` levels, the gate walk sets `E` and opens none, and the token loop
  refuses.
- **Kotlin**: `block = block_open .. block_end nests 3 else c_deep`. The escape for a nested comment goes.
  Two escapes are left:
  - `c_deep`: a comment nested past 3 levels in it;
  - `*/*/` in a comment. There `block_end` (a `/` after a `*`, not two bytes after a `/*`) refuses the
    second `/`, where the lean lexer has already closed at the first `/`.

## Why

The criterion is the lean lexer's `block_comment_end`. It counts, and only carried state says that. The
depth is 3, as each level adds carries to every proof of the group. The corpus has no comment nested in a
comment.

## What changed

- `tools/bitlex/src/syntax.rs`: `Region::nests`, its syntax and checks, the levels in `Regions::body`.
- `bitlex_masks.rs`: a kind that nests lowers through `nested_regions`, and its reader reads to the outer
  closer.
- `bitlex_lex_rust.rs`: `closer_from`, in the gate walk and the token loop.
- `kotlin.bitlex`: as above. `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- Tests: `nested_block_comments_are_the_lean_parsers`: three levels, `/*/` in a comment, after a token, a
  line break in one, a doc comment, `**`. The refused list has four levels, `*/*/` and an unclosed nest.
