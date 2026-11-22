# 0556: Tier 1's masks are one pass over the blocks, its classes nibble tables the compiler makes

Numbered 0551 where it was written (Swift-Kotlin-Scala-support); renumbered at the merge into Parser-generators-final-shape, whose 0551 is JSON's Rust from the grammar.

Status: decided (the user, 2026-11-21: "do 1 and 2 first", the first of the steps that bring the Rust the
Bitlex compiler writes for tier 1, `bitlex_lex_rust`, to the lean lexer's shape). Follows 0415 (Kotlin's
generated parser in the lean parser's shape) and 0550 (Rust from the grammar, not from an intermediate form).

## What

The lean lexer's stage 1 (`rerail_kotlin_lean::block`, `Blocks::of`) reads the file once, a 64-byte block a
step, and keeps a few masks a block: its classes from simdjson's two nibble tables, the rest from compares,
over `rerail_simd8`, compiled for each backend. The generated tier 1 made each class and each mask in a pass
of its own over the whole file, kept every one as the file's words, and found its classes with a range
compare a class in hand-written NEON (a byte at a time off arm64).

Now tier 1 has a stage 1 of the same shape, written from the grammar:

- **The classes are nibble tables the compiler makes.** A class of one byte is a compare (`eq_mask`).
  Another is its bytes grouped by high nibble: the high nibbles with the same low ones are one bit, the
  product of the two sets, set in both tables (`LO_P`, `HI_P`). Eight bits a pair of tables; a class takes the
  first pair with room for all of its bits. Kotlin's 47 classes: 30 compares, 25 bits in 4 pairs.
- **Every mask not made of the regions is in the same pass.** Its word at a block is its expression over its
  operands' words there. `prev` reads the block before; `next` the block after, so a mask that reads a
  `next` is made one block behind its operand (its `lag`); `fill` and `escaped` carry from block to block.
  Each mask keeps its last words in a ring (as many as the largest lag needs, a power of two).
- **Only what a later stage reads is kept as the file's words**: the masks the regions' walk, the masks made
  of the regions, the token loop and tier 2 read. An escape is tested at its block (`refused`), not over a
  kept mask.
- **On `rerail_simd8`'s backends**, by its `dispatch!` (NEON, AVX2, SSE, the fallback), as the lean lexer's.

## Rules

- A mask is in stage 1 unless its expression reads a region's mask (`regions H { … }`'s hidden, `o_K`, `f_K`,
  its `else` names) or a mask made of one, wherever the grammar declares it.
- Past the file's end every mask is 0 there (`lv`, the block's bits in the file, masks a `~`, a `prev` and the
  carried operations), so a `next` at the last block reads nothing past it.
- What is still apart from the lean lexer is the next steps' (the masks the regions' walk and the masks made
  of the regions read, kept as the file's words, go when the token loop reads the regions, step 2), not a
  choice.
