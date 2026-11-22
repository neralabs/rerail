# 0619: `.rerail`'s tier 1 reads no walk, and is one pass

Status: done by the implementer, in steps. The item is the user's (2026-11-22, goal: "Then move .rr's tier 1 to the
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
