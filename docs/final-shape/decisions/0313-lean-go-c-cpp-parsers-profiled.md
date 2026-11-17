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
| the lexer (`token_at`, `lex_batch`, `name_end`, `number`, stage 1) | about 45% | 185 instructions a token alone (C) |
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

## simdjson, the gold standard

simdjson 5.0.5 is vendored (`native/vendor/simdjson-5.0.5`). The engine reads JSON with it (0134):
its stage 1 costs 28 instructions a node there, and its DOM stage 2 34.

**What its lexer does.**
- **Stage 1, per 64-byte block** (two blocks a step, for the core's parallelism):
  - `json_character_block::classify`: operators by one table lookup and a compare, whitespace by
    another; each mask packed by NEON pairwise adds.
  - `json_string_scanner`: escaped bytes by its carried odd-run formula; then the in-string mask is
    `prefix_xor(quote)`, one carry-less multiply, carried to the next block by a sign shift.
  - `json_scanner`: a scalar's first byte is a scalar byte not after one (`follows`, a shift and a
    carried bit).
  - `structural_start = (op | scalar_start) & ~in_string`.
- **The index.** `bit_indexer::write` flattens the starts into positions: a popcount, then four
  positions a step, unrolled to 24, written behind into a buffer with room to spare, the tail
  bumped by the count. The last block's starts are written while the next block is classified.
- **Stage 2** walks the positions with no bounds check: the index ends in sentinels (the length,
  twice, then 0), and the text has 64 bytes of padding. It is a state machine of `goto`s on each
  token's first byte, and the tape writer appends with no capacity check.

**What this parser has of it.** Stage 1's primitives are simdjson's: the same nibble tables and the
same NEON packing (`rerail_simd8`). What it lacks is the rest of the shape. It writes no index.
Each token is found by itself: a mask scan for a blank run or a name's end, a `match` on its first
byte, a `Result` and a push into the buffer.

**The floor, measured** (`c_lean/examples/stage1_floor.rs`, C's 829 files, 150 passes, the fixed
costs taken off). It is simdjson's stage 1 in C's classes: word and gap masks, a start at every
byte that is neither a gap nor a word byte after a word byte, flattened by simdjson's indexer. It
hides no strings or comments and joins no two-byte punctuators. Its 1.415 M starts a pass stand
against the lexer's 1.354 M tokens and 21 K comments.

| | instructions a token | cycles a token | cycles a byte |
|---|---:|---:|---:|
| simdjson's shape, C's classes: stage 1 and the index | 15 | 4.0 | 0.63 |
| the lexer now | 146 | 48 | 7.6 |
| the whole parse | 476 | 113 | 17.8 |

The lexer is 43% of the parse's cycles. Its shape, not its primitives, is twelve times the floor.

**What C needs beyond the floor, and what it costs.**
- **Regions: strings, character literals, comments, raw strings.** JSON's `prefix_xor` does not
  apply, as a quote may sit in a comment or a character literal and `//` in a string. Page 36's
  region walk does: in a block with an opener, the openers left to right, each closed by the first
  closer after it (a trailing-zero count of the closer's mask), carried across blocks. A block with
  no opener (most) pays one test.
- **Punctuators of two and three bytes**: a pair mask by nibble product (page 31), and its second
  bytes taken out of the starts; `<<=`, `>>=`, `...`, `->*`, `<=>` by a third-byte test in stage 2.
- **A token's end**: a name's from the word mask's run, a punctuator's from its length; stage 2
  reads them as it takes the token.
- **Directives.** A directive's argument (`preproc_arg`) and `#include <…>` are read from the bytes.
  The relex (`relex_over`) becomes a seek in the index to the first start past the piece: no
  tokens lexed again.
- **Kinds and words**: the kind by a table on the first byte; a word code only for a name, as now.

Page 31 measured this shape for `.rr` at 85 instructions and 21 cycles a token. For C that would
take the lexer from 48 cycles a token to about 20: about a quarter of the parse's cycles.

## Open

- **The lexer is still the largest part**: about 39% of C++'s time, 146 instructions and 48 cycles a
  token for C alone, against about 85 instructions for `.rr`'s block lexer (page 31) and 15 for
  simdjson's shape (below). The block lexer's shape fits C's
  tokens better than ECMAScript's:
  - C has one lexical mode outside directives (0181 found ECMAScript's modes rule it out there).
  - Directives' arguments, `#include <…>`, raw strings and character literals would be the rare
    starts that break a run.

  On `.rr`, page 31 measured that shape at parity with a byte lexer in cycles, but that byte lexer
  was already at 21 cycles a token; this one is at 48. The design is in "simdjson, the gold
  standard" above. Build it in `rerail-c-lean` first, behind the same digest and oracles, then Go.
  **Built: 0318** (the lexers 145 → 119 and 133 → 117 instructions a token).
- **Stage 1's lazy masks**: the stop masks of strings, block comments and directive arguments
  (`STOP`, `STAR`, `ARG`) made where one starts (page 31's micro-lexers), not for every block.
  Stage 1 is about 3% of C++'s time.
- **`angle_scan`'s repeats** (above), and `stores`' dialect select a call: under 1% each.
