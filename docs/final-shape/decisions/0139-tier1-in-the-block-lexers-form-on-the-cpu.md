# 0139: Tier 1 on the CPU in the block lexer's form

Status: decided (2026-11-13); building. The design: docs/parser-generators/36.

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
