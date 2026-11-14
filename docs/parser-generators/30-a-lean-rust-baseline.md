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
- The lexer is a block lexer in Bitlex's shape (page 31):
  a block's token starts found at once, written without a branch a token,
  comments, strings and backtick literals read by micro-lexers.
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
Each figure is the least of seven rounds.
The Rust parsers are built as release code:
`opt-level=3`, overflow checks and debug assertions off.

```
cargo test -p rerail-rr-parser --no-run \
  --config 'profile.dev.package.rerail-rr-syntax.opt-level=3' \
  --config 'profile.dev.package.rerail-rr-syntax.overflow-checks=false' \
  --config 'profile.dev.package.rerail-rr-syntax.debug-assertions=false'
RR_ROUNDS=7 <test binary> rr_against_rust --ignored --nocapture
```

| | instructions | cycles | a node | IPC |
|---|---|---|---|---|
| tier 1 | 532M | 106M | 280 / 56 | 5.0 |
| tier 2 | 642M | 151M | 338 / 79 | 4.3 |
| **compiled, both tiers** | **1,174M** | **257M** | **618 / 135** | |
| lean: lexer alone | 156M | 33M | 82 / 18 | 4.7 |
| lean: lexer alone, page 31's last change | 133M | 33M | 70 / 17 | 4.1 |
| lean: lexer and parse | 466M | 84M | 245 / 44 | 5.6 |
| engine's Rust parser | 3,235M | 670M | 1,704 / 353 | 4.8 |

"A node" is instructions / cycles per tape node.
The load average was 35 to 60 during the run.

Earlier runs on this page built the Rust with the debug profile's
overflow checks and debug assertions still on.
They cost the engine's parser 4% of its instructions.

What it shows:

- **The compiled grammar takes 3.1 times the lean parser's cycles**
  (257M against 84M), and 2.5 times its instructions.
- **Tier 2 against the lean parse alone** (the whole less the lexer:
  310M instructions, 51M cycles):
  2.1 times the instructions, 3.0 times the cycles.
  Its IPC is lower too, 4.3 against 6.1:
  its branches are harder to predict and its loads wait longer.
- **Tier 1 against the lean lexer:**
  3.4 times the instructions and 3.2 times the cycles,
  though both have the same block shape (page 31).
- The engine's parser costs 8 times the lean one.

A first draft of the lean parser, which compared words as strings
and scanned with byte loops, took 804M instructions and 151M cycles.
Most of the difference was its lexer (409M instructions, 79M cycles).

### Where the lean parser could still go

It is not the floor:

- every token and node access is bounds-checked;
- each term goes down four calls (`term`, `product`, `primary`
  and the list it is in), as recursive descent does.

The lexer's 33M cycles is the bar for tier 1 (page 31 says why
its block shape is at parity with a byte lexer on this corpus).
The parse's 200 instructions a token is a fair bar for tier 2.

## Counters and hot spots

The tapes are now named-only (page 32): 1.15 million nodes, not 1.9.
`rr_profile_loop` runs one form over the corpus for a while,
for `xctrace` (Time Profiler, CPU Counters).
CPU Counters splits the core's issue slots four ways:
useful, waiting on the back end (processing), waiting on fetch (delivery),
and thrown away after a mispredicted branch (discarded).

| | useful | processing | delivery | discarded |
|---|---|---|---|---|
| lean, whole | 51% | 17% | 11% | 21% |
| lean, lexer alone | 38% | 31% | 5% | 25% |
| tier 1 | 48% | 41% | 5% | 6% |
| tier 2 | 39% | 39% | 8% | 14% |

The lexer waits on the back end and on mispredicted branches.
The parse alone (the whole less the lexer) is mostly useful slots;
what it loses, it loses to branches: recursive descent on token kinds.

Three changes, measured with `rr_against_rust`, least of 7 rounds:

| | instructions | cycles |
|---|---|---|
| before | 390M | 77.1M |
| no index of lines | 381M | 71.7M |
| `push` inline, comments out of line | 368M | 70.6M |
| `leaf_over`, `open`, `close`, `anon` inline | **357M** | **68.7M** |

- **No index of lines.** A setting's or a declaration's tail ends at its line's end.
  The parser made an index of every newline the first time it asked (3.5% of the time).
  Now it reads the bytes between the tail's last token and the next for a newline,
  each byte once.
- **`push` inline.** It saved and restored seven register pairs a node,
  because placing a comment made it too big to inline.
  Now one compare says no comment is near, and placing one is a cold call.

One change made it worse, and was undone: writing a run's tokens
four at a time without a branch, as simdjson writes its index.
The lexer went from 32.5M to 39.0M cycles:
a token here is more work than an index entry,
and the tokens written past a run cost more than the branch saved.

What is left, by the profile:

- The lexer is 43% of the time.
  Its losses are back-end waits (the token's table loads and its store)
  and mispredicts, which the counters do not place.
  Placing them needs Instruments' discarded-sampling mode, which `xctrace` cannot choose.
- The parse costs about 195 instructions a node:
  every token read and node write is bounds-checked,
  the parser's state goes through memory between calls,
  and each term goes down `term`, `product`, `primary`.

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
it takes three times the lean lexer's cycles, in the same shape (page 31).
