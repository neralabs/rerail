# A lean Rust baseline for `.rr`

## Intro

The compiled `.rr` grammar runs in two tiers.
Tier 1 finds the tokens. Tier 2 builds the tree from them.
How fast should they be?

The engine's own Rust parser is no measure.
It builds statements with owned strings,
then a tree of vectors, then flattens the tree into the tape.
That is about 1,780 instructions a node.

So this page adds a second Rust parser, written for the tape alone,
and measures the compiled grammar against it.
It is a baseline only, not on the product's path.

## The lean parser

`native/crates/rr_syntax/src/lean.rs` reads the same grammar
and gives the same tape, node for node.
It is written as a fast hand-written parser would be:

- A token is one 16-byte record: kind, span, line,
  and the word it is when the grammar looks at that word
  (`code`, `min`, `decl`, …), found once by the lexer.
  The parser compares one-byte codes, never text.
- The lexer classes bytes by a table.
  It finds the ends of comments, strings and code literals
  8 bytes a step, as `memchr` does without SIMD.
- No statements are built. Each node is written in preorder as it opens.
  The open nodes and their children are a stack.
- An operator found after its left operand (`a - b`, `x <= y`)
  puts its node before the operand.
  When the operand is one leaf, the two nodes swap.
  Only a larger operand (`f(x) = y`) moves the nodes after it.
- Comments are placed as the parse passes them,
  by the same rules as the engine's flattening step.
  The next comment's start is kept, so a node costs one compare for them.
- An atom (a call read as a literal) renames its first child's field;
  nothing else of it is visited again.
- Backtracking cuts the tape, the stack and the comments back to a mark.

Its test, `lean_tape_is_the_engines`, compares it with the engine's tape
on every `.rr` file of the repository.
Each file is also cut at seven points, so the recovery paths run:
10,896 texts in all.

The `memchr` crate itself is not used.
The bench optimizes `rr_syntax` alone,
so `memchr` would run unoptimized, and inflate the lexer threefold.

## The numbers

The bench `rr_against_rust` runs each parser over the 1,360 files
the compiled grammar accepts (8.8 MB, 1.9 million nodes), in turns.
Each figure is the least of five rounds.
The Rust parsers are built at `opt-level=3`.

```
cargo test -p rerail-rr-parser --no-run \
  --config 'profile.dev.package.rerail-rr-syntax.opt-level=3'
RR_ROUNDS=5 <test binary> rr_against_rust --ignored --nocapture
```

| | instructions | cycles | a node | IPC |
|---|---|---|---|---|
| tier 1 | 532M | 106M | 280 / 56 | 5.0 |
| tier 2 | 642M | 152M | 338 / 80 | 4.2 |
| **compiled, both tiers** | **1,174M** | **259M** | **618 / 136** | |
| lean: lexer alone | 130M | 34M | 68 / 18 | 3.9 |
| lean: lexer and parse | 436M | 82M | 229 / 43 | 5.3 |
| engine's Rust parser | 3,370M | 700M | 1,775 / 369 | 4.8 |

"A node" is instructions / cycles per tape node.
The load average was about 24 during the run.

What it shows:

- **The compiled grammar takes 3.2 times the lean parser's cycles**
  (259M against 82M), and 2.7 times its instructions.
- **Tier 2 against the lean parse alone** (the whole less the lexer:
  306M instructions, 48M cycles):
  2.1 times the instructions, 3.2 times the cycles.
  Its IPC is lower too, 4.2 against 6.4:
  its branches are harder to predict and its loads wait longer.
- **Tier 1 against a scalar lexer:**
  4.1 times the instructions and 3.2 times the cycles.
  The lean lexer has no SIMD and reads whitespace and words
  a byte at a time, so tier 1 should beat it, not trail it.
- The engine's parser costs 8.5 times the lean one.

A first draft of the lean parser, which compared words as strings
and scanned with byte loops, took 804M instructions and 151M cycles.
Most of the difference was its lexer (409M instructions, 79M cycles).

### Where the lean parser could still go

It is not the floor:

- whitespace, words and numbers are still read a byte at a time;
- every token and node access is bounds-checked;
- each term goes down four calls (`term`, `product`, `primary`
  and the list it is in), as recursive descent does.

A SIMD lexer is tier 1's own design,
so the lexer's number is the bar tier 1 must clear by a margin.
The parse's 200 instructions a token is a fair bar for tier 2.

## Why tier 2 is heavy

Tier 2 runs 11.7 of its words a token.
Weighted by how often each runs, their instructions as lowered are
(an upper bound: not every instruction of a word runs each time):

| word | a token | instructions each |
|---|---|---|
| RETURN | 0.75 | 180 |
| SWITCH | 1.6 | 54 |
| TOKEN | 1.0 | 63 |
| MARK | 1.7 | 25 |
| LEAF | 0.9 | 36 |

Against the lean parser, three things stand out:

1. **A grammar call returns through a table in memory.**
   RETURN pops a place and finds it with a compare tree
   over the 55 places a return can go back to: about 6 branches.
   The lean parser returns with `ret`, which the CPU predicts.
2. **A switch reads the token's bytes again.**
   It loads a byte of the text and runs a compare tree
   over about 13 runs of bytes, 1.6 times a token.
   The lean parser switches on the token kind the lexer gave.
3. **Each word goes through memory for the parse state**
   (position, end, limit, whether at the end), and checks bounds again.
   The lean parser keeps its position in a register.

## What follows

The shape to build is the lean parser's, still generated
from the grammar file through the compiler:

- each grammar rule a generated function,
  so a grammar call is a native call and a return is `ret`;
- switches on a token kind found once a token, not on the text;
- the token state as values inside a rule's function.

Tier 1 needs its own look:
it trails a scalar lexer threefold.
