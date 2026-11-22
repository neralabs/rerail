# 0619: `.rerail`'s tier 1 reads no walk, and is one pass

Status: done by the implementer. The item is the user's (2026-11-22, goal: "Then move .rr's tier 1 to the
single pass, the way JSON's went in 0585 and 0586"). How it is done is the implementer's choice. It follows 0607 and
0608 (Kotlin's token loop reads no walk) and 0618 (`.rerail`'s tier 2 as one loop).

## What was there

`.rerail`'s tier 1 (`rerail-rr-aot`'s `tier1.rs`) worked in three steps:

1. A stage-1 pass over the blocks, which wrote 34 masks of the whole file.
2. The regions' walk, and the masks made of it, which the token loop read: each row's start (`t12` = `quote & o_str`,
   …) and each `read`'s choice of region (`bit(&m_o_line, s)`, …).
3. The token loop over the blocks.

0608 made the walk the test build's (`#[cfg(feature = "test-hooks")]`, `gate_walk`). Since the token loop still read
the walk's masks, a build without `test-hooks` did not compile: 14 names were missing.

The cause was in the generator. The loop numbers the regions it opens (`opens`, 0596) only where no region has an
`else` form, and `.rerail`'s `t2` has one (`else e_t2 bytes 2`, the empty literal). Without `opens`, `at_start` gave
up on any expression that read a region's mask, and the read's choice read the walk's `o_*`.

## Step 1: the token loop reads no walk

- **`at_start` without `opens`.** It breaks the expression down as before. A region's mask (`o_K`, `f_K`, an `else`
  form's) is whatever the grammar states of it (`assume … at starts`). If the grammar states nothing, it is unknown.
- **A read's choice tests the openers.** Where the loop numbers no regions, the region a token opens is the first whose
  opener holds at its start. The start is never hidden, because the loop is in no region. An `else` form opens at its
  region's opener too, and the read's `none` arm, which finds no closer, takes it. A test build asserts each choice
  against the walk.
- **The grammar states what the loop knows at a start** (`rr.bitlex`, 0607's form). Its start masks are named
  (`id_first`, `int_first`), and the grammar states:
  - a start is code;
  - a quote, `//`, `/*` or a backtick there opens its region;
  - a name starts at a letter, and a number at a digit;
  - an operator starts at its first byte.

  The prover proves all eight.

Now each row's start is the dispatch's group alone, or a stage-1 mask at the token (`/`: `line_open | block_open`). The
build without `test-hooks` compiles, and it has no walk.

## Step 2: one pass

- **Stage 1 and the token loop are one function a backend** (`lex_on<S>`, dispatched once, `lex_best`). Its
  `'pass` loop does two things, in turn:
  1. Stage 1 makes blocks until it has made the ones the token loop waits for (`made`, `wait`). Its code is the
     standalone pass's, step by step (`stage1_parts`).
  2. The token loop reads tokens from the blocks made so far.
- **A token is read once its blocks are made.** A token whose byte after it is in a block not made yet is read again,
  once stage 1 has made more blocks. So is a row's refusal while blocks remain, such as a string with no closer so far.
  The token has changed no state at that point.
  - Each time, stage 1 makes blocks out to twice the token's distance from its block, so a long string or comment is
    read again a few times, not once a block.
  - A token loop that comes to a block not made waits for that block.
- **The depth count is the token loop's** (`token_scans`). It reads `code`, so it was in the walk, and the build
  without `test-hooks` did not refuse a file nested past 127. Now each token of code whose bytes may hold a brace or
  a bracket counts the ones in its bytes, in order. A token of code holds no region's byte, and the count's bytes
  are none of the skipped ones.
- **The source is padded**, as simdjson's and JSON's (0585):
  - `lex(buf, n)` and `parse(buf, n)` take the file with `PADDING` spaces after it;
  - `padded(src)` makes such a buffer;
  - each block is loaded where it is, with no copy.
- **The test build runs the product's pass.** Its walk makes its own stage 1 over the file first (`stage1_best`, kept
  for the test build alone). The pass interleaves in every build, so the tests read the pauses too.

The one pass applies where the loop keeps nothing over a token's blocks but its records and counts. The conditions:
no template's string, no fill kept over a region (`assume … at ends`), no region read in parts for tier 2, and no
`prefix`. That is `.rerail`'s loop. Kotlin's keeps its two passes and its file is unchanged, and so is JSON's (the
index form, 0585).

What stays: the records hold each token's last byte and its code, because tier 2 reads them (0618). Stage 1 keeps the
masks the token loop reads for the file's blocks, as the lean lexer's `Blocks::of` does.

## Checks

On a Linux arm64 farm host:

- `rr_parser`: all 51 library tests, the corpus's two (the tapes and tier 1's records, 1,784 files), and
  `rust_is_current`.
- `rerail-rr-aot` builds without `test-hooks`.
- JSON's, Kotlin's, the compiler's and bitlex's library tests pass, and their files are unchanged.
- Each generator file I changed is clean under clippy. That includes the warnings that were there before:
  `decide` takes the exits as `exits_ending` finds them, which also drops an argument.

Not run: x86 backends, Metal, the slow tier.
