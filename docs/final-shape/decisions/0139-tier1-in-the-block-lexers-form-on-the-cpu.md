# 0139: Tier 1 on the CPU in the block lexer's form

Status: decided (2026-11-13); step 1 built (2026-11-13), steps 2 to 4 to build.
The design: docs/parser-generators/36.

## Intro

**The problem.** Tier 1, the compiled parser's lexer, makes every check at every byte:
about 2,500 instructions a 64-byte block, 464.6M over the corpus ahead of time.
The lean lexer, in the same block shape, makes four masks a block and then works a token
at a time: 132.4M.

**The decision.** On the CPU, tier 1 takes the lean lexer's form; Metal keeps the
bit-parallel form. CPU and GPU diverge as little as they can: each new piece is a
construct of the block program with two lowerings, not a second module.

1. **Regions** (strings, comments, backtick literals) are a construct of their own,
   `regions { … }`: an opener, a closer, what the file's end gives.
   On the CPU, a walk over a block's openers, each to the first closer after it
   (a trailing-zero count); on Metal, today's `rounds until settled`.
   `tt_later`, its `back_fill` and the backward pass go: tier 1 on the CPU is one pass.
2. **Checks a token makes** move to the token: keywords as token marks
   (a change to the `.rr` grammar), a number's length and first bytes,
   a byte no token takes as a token kind.
3. **The records from the starts alone**: one walk over a block's starts,
   each token's end from its run; no `lasts` extract.
4. **Primitives**: two-byte punctuation as one nibble product.

**Loops.** Page 12 kept the mask passes free of loops. On the CPU, tier 1 may loop over a
block's regions and its tokens, never over its bytes. Accepted where it is needed.

Target: tier 1 on the CPU within 1.5 times the lean lexer, about 200M instructions.

## Step 1, built

- **The construct.** `regions H { K = O .. C [bytes N] [outside] [else E bytes M] }`
  in Bitlex is a rounds group that is also a walk (`block::Walk`).
  The interpreter's meaning is the walk; a test checks it is the rounds' fixed point
  on every value of the `.rr` grammar, over a corpus sample and over closers that cross
  a block's end (`the_walks_are_the_rounds`).
- **On the CPU** (`native_cpu_walk`), each walk is a generated function, called once a
  block. It loops over the block's regions, each found by a trailing-zero count.
  A region open at the block's end, and closer bytes that run past it, are carried in
  one state field.
  An `else` asks whether the closer comes later in the file. Past the block, the walk
  scans the bytes for it, eight a word, by the closer's byte tests. A scan that finds
  none sets a flag, so later openers of that kind take the `else` with no scan.
  So each byte is scanned at most once.
- **On Metal**, the same program runs the rounds, as before.
- **`end_on`** was a keyword seen past blanks and comments (`back_fill`): it is now the
  token mark `tokens "." "end"` (a step 2 change, made here because it held the last
  backward pass).

Tier 1 on the CPU is one forward pass, down from five, with no masks stored between passes.
Over the corpus ahead of time it went from 700.4M instructions to 630.0M (395 to 355 a node).
The cut is smaller than the passes' share suggested:
most of pass 0's masks were needed by the later pass, and are still made there.
Steps 2 to 4 take operations out of that one pass.
