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
and gives the same tape, node for node:

- A token is a kind and a span. No token holds a string.
  A word such as `code` is compared in the text.
- No statements are built. Each node is written in preorder as it opens.
  The open nodes and their children are a stack.
- An operator found after its left operand (`a - b`, `x <= y`)
  puts its node before the operand: those are the tape's last nodes.
- Comments are placed as the parse passes them,
  by the same rules as the engine's flattening step.
- Backtracking cuts the tape, the stack and the comments back to a mark.

Its test, `lean_tape_is_the_engines`, compares it with the engine's tape
on every `.rr` file of the repository.
Each file is also cut at seven points, so the recovery paths run:
10,896 texts in all.

It is a first draft, with no tuning.
Words are compared as strings, and each operator node shifts the nodes after it.

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
| tier 1 | 532M | 136M | 280 / 71 | 3.9 |
| tier 2 | 642M | 174M | 338 / 92 | 3.7 |
| **compiled, both tiers** | **1,174M** | **310M** | **618 / 163** | |
| lean: lexer alone | 409M | 79M | 216 / 42 | 5.2 |
| lean: lexer and parse | 804M | 151M | 423 / 79 | 5.3 |
| engine's Rust parser | 3,372M | 842M | 1,776 / 443 | 4.0 |

"A node" is instructions / cycles per tape node.
The load average was about 20 during the run.

What it shows:

- **The compiled grammar takes twice the lean parser's cycles**
  (310M against 151M), and 1.5 times its instructions.
- **Tier 2 against the lean parse alone** (the whole less the lexer:
  395M instructions, 72M cycles):
  1.6 times the instructions, 2.4 times the cycles.
  Its IPC is lower too, 3.7 against 5.3:
  its branches are harder to predict and its loads wait longer.
- **Tier 1 against a lexer that reads a byte at a time:**
  1.3 times the instructions and 1.7 times the cycles.
  The lean lexer does no batching and has no SIMD,
  so tier 1 should beat it, not trail it.
- The engine's parser costs 5.6 times the lean one.

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

The lean parser can itself go further (words by kind, not strings;
no shifting for operators): the target is lower still.
