# 0313: The lean Go, C and C++ parsers, profiled: where the time goes, what was changed, what does not pay

Status: built (user, 2026-11-16: "please profile them, and also look at the Parser generators
worktree. I want you to identify hotspots and opportunities to use the faster techniques and
improve the speed where possible"). Numbered past the highest number in any worktree (0312).

## Intro

The lean parsers for Go (0305) and for C and C++ (0307) write tree-sitter's tape directly. They
cost a sixteenth of tree-sitter's cycles. This decision profiles them and changes what the
profile shows. It also weighs them against the techniques the Parser-generators work wrote down
(`docs/parser-generators`, pages 26, 31, 34 and 36):

- simdjson's two stages;
- a block lexer that finds every token start of a 64-byte block at once;
- a token's kind and length by one table load;
- micro-lexers for strings and comments.

Every change keeps every tape the same: a digest of every column of every tape over the corpora,
before and after, and the oracles against tree-sitter, run under Relax with debug assertions on.

## How it was measured

- **Cost:** each crate's cost example (`c_costs c|cpp LIST PASSES`, `go_costs LIST PASSES`), profiling
  build, 15 passes over the files the parser reads:
  - C++: 937 files, 10.6 MB, 1.92 M nodes a pass;
  - C: 829 files, 8.6 MB;
  - Go: 869 files, 10.3 MB.

  `RERAIL_COSTS_LEX` runs the lexer alone over the same files.
- **Profile:** Xcode's Time Profiler (`xctrace`) over 60 passes. The samples are symbolized by
  inlined function and line (`atos -i` over a `dsymutil` image). `RERAIL_COSTS_MAIN` names a file
  where the example writes `main`'s address, which gives the slide.
- **A/B:** each step was decided on this machine by instructions and cycles, both binaries
  interleaved, since the load here moves cycles by several percent. The result below is one run
  on a farm host (M1 Max, load about 1.7 after it), before and after interleaved, three rounds.

## What the profiles showed

At the start (C++; C and Go alike):

| where | share | what |
|---|---:|---|
| the lexer (`token_at`, `lex_batch`, `name_end`, `number`, stage 1) | about 45% | 247 instructions a token alone (C) |
| the token buffer's look (`fill`, `tok`, `peek_at`) | about 10% | a length check and a batch call on every look |
| `Writer::insert` and its `memmove`s | 5 to 8% | four library calls a wrap |
| the parser's look-ahead scans (C++: `scope_ahead`, `class_name_end`, `ctor_ahead`, `declaration_ahead`, `angle_scan`) | about 7% | |
| the writer's push and close, `stores`, the rest of the descent | the rest | |

## Changes made

### The writer: a short insert moves its cells in place

A node found after its first child (an operator after its left operand, a call after its
function) is inserted before it, and the nodes from there move up one. Over a C++ pass there are
780 K inserts. 58% move one node and 93% fewer than 16. Each paid four `memmove` calls, one a
column. Now an insert of 16 nodes or fewer moves them a cell at a time, from the end. This is
`rerail-tape-format`, so every lean parser gets it.

### The lexer: one table load for the token after no gap, and for an ASCII name

- **No gap.** Most tokens follow the one before at once (`f(`, `a;`). `skip` scanned the blank
  mask for every token. Now one load of a 256-byte table says whether anything is there to pass.
- **ASCII names.** A name that is no literal's prefix (`L`, `u`, `U`, `R` in C) ends where its
  word mask does. `name_end`'s checks run only where a `\u` escape or a byte past ASCII follows.

### The token buffer holds two tokens past the parser's

The parser looks at its next token many times a token. Each look checked the buffer's length. Now
the buffer holds the next token and two more from each move:

- `bump`;
- `reset`;
- a relex;
- Go's type-switch look ahead.

A look that near reads without a check. A debug assertion guards the invariant.

### A comment's start kept beside the list

Each `bump` loaded the comment list to find no comment before its token. The next comment's start
is now kept in a field (`u32::MAX`: none). It is set wherever the list or its cursor changes.

## Results

On the M1 Max, the best of three rounds of 15 passes; the digests are the same before and after:

| parser | instructions before | after | change | cycles before | after | change |
|---|---:|---:|---:|---:|---:|---:|
| C++ | 18.17 G | 16.17 G | −11.0% | 4.65 G | 4.02 G | −13.5% |
| C | 12.49 G | 10.99 G | −12.0% | 3.34 G | 2.89 G | −13.6% |
| Go | 15.68 G | 13.80 G | −12.0% | 4.06 G | 3.66 G | −9.7% |

Each step on its own, instructions, on this machine:

| step | C++ | C | Go |
|---|---:|---:|---:|
| short inserts in place | −1.8% | −2.9% | −3.5% |
| the gap and name tables | −4.5% | −6.0% | −5.1% |
| two tokens ahead | −4.3% | −2.7% | −2.8% |
| the comment's start | −0.6% | −0.8% | −1.2% |

The oracles are unchanged:

| set | same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| C, whole files | 829 / 0 / 0 | 0 / 0 / 744 |
| C, cut and edited | 1,161 / 0 / 10 | 0 / 0 / 3,277 |
| C++, whole files | 937 / 0 / 63 | 0 / 0 / 560 |
| C++, cut and edited | 1,271 / 0 / 113 | 0 / 0 / 3,614 |
| Go, whole files | 860 / 0 / 0 | 9 / 0 / 4 |
| Go, cut and edited | 1,575 / 0 / 0 | 11 / 0 / 2,266 |

## Measured and dropped

- **Wraps held as a chain** (`Writer::open_chain`, which the ECMAScript and Python parsers use): a
  chain of k wraps moves its nodes once, not k times. Opened for every expression and postfix
  chain it cost C++ 6.6% more instructions. Opened only at a node's second wrap it still cost
  3.3% more. Chains of three wraps or more are rare: two wraps move as many nodes as two inserts.
- **Punctuation by table** (page 31's form): a punctuator's kind and length from a row for its
  first byte and a column for its second, and a third byte that lengthens it (`<<=`, `->*`).
  Instructions were level, within the runs' noise of 1%. Cycles were 2 to 4% worse. The `match`'s
  jump table predicts well here.
- **A memo for the look-ahead scans.** They were counted over a C++ pass, by call and by distinct
  token position. `scope_ahead` made 457 K calls at 442 K positions, and `class_name_end` 271 K at
  267 K. `ctor_ahead` and `declaration_ahead` never ran twice at a position. Only `angle_scan`
  repeats (49 K calls at 15 K positions), and it is 0.7% of the time. Nothing is read twice.

## The Parser-generators techniques, weighed

| technique | here |
|---|---|
| Stage 1 by nibble tables, runs by trailing zeros | already there (0305, 0307) |
| Perfect hash over a masked 8-byte load for words | already there (`rerail_simd8::words`) |
| A table for a token's first byte | **applied** for gaps and names; for punctuation it did not pay (above) |
| Batches of tokens (16 a refill) | already there |
| Block lexer: every token start of a block from stage 1's masks, tokens written a run at a time (page 31) | **open**: see below |
| Micro-lexers: a stop mask built where a string or comment starts (page 31) | **open**: would drop 3 of stage 1's 7 masks |

## Open

- **The lexer is still the largest part**: about 39% of C++'s time, 217 instructions a token for C
  alone, against about 85 for `.rr`'s block lexer (page 31). The block lexer's shape fits C's
  tokens better than ECMAScript's:
  - C has one lexical mode outside directives (0181 found ECMAScript's modes rule it out there).
  - Directives' arguments, `#include <…>`, raw strings and character literals would be the rare
    starts that break a run.

  On `.rr`, page 31 measured that shape at parity with a byte lexer in cycles. The gain here would
  come from the run loop: no `match` a token, no gap scan, a word code only for words. Build it in
  `rerail-c-lean` first, behind the same digest and oracles.
- **Stage 1's lazy masks**: the stop masks of strings, block comments and directive arguments
  (`STOP`, `STAR`, `ARG`) made where one starts (page 31's micro-lexers), not for every block.
  Stage 1 is about 3% of C++'s time.
- **`angle_scan`'s repeats** (above), and `stores`' dialect select a call: under 1% each.
