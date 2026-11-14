# 0181: The lean parsers, profiled: which simdjson tricks apply, which were applied, which do not pay

Status: built (user, 2026-11-13: "please profile the lean parsers (*js*, python, rerail) and
identify any opportunities for improvement: either more SIMD instructions, or whatever else. I
want all the relevant SIMDJSON-type tricks that could be applied to be applied"). Numbered past the
highest number in any worktree (0180).

## Intro

The three lean parsers each write a tape directly from a file:

- `rerail-ecma-lean` reads JavaScript, TypeScript and TSX (0178);
- `rerail-python-lean` reads Python (0163, 0169);
- `rerail-rr-syntax`'s lean parser reads `.rr` rules.

They all write through one tape writer, `rerail_tape_format::Writer`, and all borrow from
simdjson's design. This decision records:

- a profile of each parser;
- each simdjson trick, and whether it was already there, applied now, or measured and dropped;
- the changes made, each measured and checked against the oracles.

## How it was measured

- **Cost:** each crate's `costs` example (`examples/costs.rs`) under `/usr/bin/time -l`,
  profiling build. The numbers are instructions retired, the least of 3 runs.
  - The list is read once, then each pass tapes every file: 15 passes (20 for `.rr`).
  - Cycles are given too, but the machine's load was 25 to 55 while measuring, so they are noisy.
- **Profile:** Xcode's Time Profiler (`xctrace`) over the same runs, symbolized by function, inlined
  function and line.
- **Sameness:** each `costs` run prints a digest of every column of every tape. A change is kept
  only where the digests are the same before and after. The parsers' oracles against tree-sitter
  were run too, and they are unchanged:
  - ECMAScript: JS 1,235 same / 1 refused; TS 790 / 9; TSX 600 / 0. On the cut-and-edited sets,
    every text with an error is refused.
  - Python: e-4k 3,916 same / 1 refused; e-edit 2,138 same / 5 refused. None differ.

Sets: 1,236 JavaScript files, 800 TypeScript, 600 TSX, 1,000 Python, and the repository's `.rr`
files.

## What the profiles showed

At the start, with `Lexer::token` and the token buffer counted together as lexing:

| parser | lexing | the tape writer | allocation, copies | parsing |
|---|---:|---:|---:|---|
| JavaScript | 33% (token 23%, lex_one 5%, stage 1 4%) | 16% (`Writer::block` 12%, insert 2%) | 11% (memmove 6% from `insert`) | the rest |
| Python | 36% (token 16%, lex_joined 15%, stage 1 4%) | 9% (`block` 7.5%) | — | the rest |
| `.rr` | 33% (`lex`) | 6% | 9% (for each small file) | the rest |

The writer's `block` was the one shape fault: each push could rewrite a block's offsets, and an
insert rewrote every block after it.

## simdjson's tricks, one by one

| trick | ECMAScript | Python | `.rr` |
|---|---|---|---|
| Two stages: classify every byte with SIMD first, then scan the masks | there (`block`, 7 masks) | there (now 5 masks) | there (`micro::Window`) |
| Nibble-table classes (`vpshufb` / `vqtbl1q`) | there | there | there |
| Runs and stops by trailing zeros over a mask | there | there; **now also a line's indent** | there |
| Structural index with bit flattening | **does not apply**, see below | **does not apply** | there (`lex`'s starts) |
| Batches, not one token a call | **applied: 16 tokens a refill** | there (the whole file, then the parse) | there (the whole file) |
| Perfect hash over a masked 8-byte load (simdjson's `true`/`false`/`null` check) | there | there | there |
| Lookup tables in place of searches | **applied: the dropped-token check** | — | there (`TOKEN`) |
| Derived columns in one final pass | **applied: the writer's bases and offsets** (all three parsers) | | |
| No copies: buffers moved, not cloned | **applied: `finish_columns` moves `size` and `tail`** | | |
| Buffers reused between documents | **measured, dropped**, see below | | |
| Over-allocated output written behind, with no check a write | — | — | there |
| SIMD UTF-8 validation | not needed: a byte from 0x80 on is decoded only where a token meets one | | |
| In-string mask by carry-less multiply (prefix XOR of quotes) | **does not apply** | **does not apply** | — |

**Why no whole-file index for ECMAScript or Python.** simdjson can find every structural character
of a JSON file in stage 1, because JSON has one lexical mode. In ECMAScript, what `/` begins
depends on the parse:

- a regular expression, where an operand may start;
- a division elsewhere.

The same holds for a `}`, which may close a block or resume a template's text, and for `<`, which in
TSX and JSX may open an element. For the same reason, a quote's meaning (and so an in-string mask)
cannot be known ahead: a quote may stand in a comment, a regular expression or a template.

Python's strings, f-strings and comments interleave in the same way. So the mode-free parts are
indexed: stage 1's masks. The mode-dependent parts are read where the parser stands. The `.rr`
dialect has one mode, and its lexer already writes simdjson's index.

## Changes made

### The tape writer: bases and offsets in one final pass

Each `push` checked its start against its block's base. At a block's first node, or where an offset
would wrap or need 4 bytes, the push rewrote the whole block (`Writer::block`). An `insert` rewrote
every block from its place to the end.

The writer keeps every node's start anyway. Now `finish` computes, in one straight-line pass:

- each block's base (the least start among its 16 nodes);
- each node's offset from it;
- whether an offset needs 4 bytes.

The `OFF_WIDE` marks are gone. Push, insert and truncate no longer touch offsets or bases.
`finish_columns` also moves the `size` and `tail` columns out instead of copying them.

A unit test (`offsets_and_bases_are_computed_at_finish`) covers offsets past 0xffff, which force a
redo at 4 bytes, and decodes the image back.

### ECMAScript: a restore keeps the tokens lexed since its mark

A speculation that is backed out (an arrow's parameters, a generic call) truncated the token buffer
to its mark, and every later token was lexed again. Over the JavaScript set that was 281 K of the
2.64 M tokens lexed (10.7%).

Lexing has no state, so those tokens are the same ones, unless `relex_at` cut the buffer for
another mode after the mark. The mark now holds a count of such cuts, and a restore truncates only
where the count changed.

### ECMAScript: tokens lexed 16 at a time

A refill now lexes up to 16 tokens in one loop, where it was one cold call a token. A batch stops
after any token where the parser may switch modes (`/`, `/=`, `` ` ``, `}`, `<`, `>` and the `>`
runs). A token lexed ahead and then cut is only work lost.

Comments lexed ahead are harmless: every use of the comment list is bounded by a position. A batch
of 64 measured no better, and worse for TypeScript.

### ECMAScript: the dropped-token check is a hash probe

`stores(parent, token, field)` asks whether a tape leaves out a token. It ran a binary search of
`kinds::DROPPED`, 212 keys a dialect, for every token written. The keys now sit in a 1,024-slot
open-addressed table a dialect, built at compile time: 1.2 probes on average, against about 8
dependent loads.

A perfect multiply-shift hash would need about n² slots for 212 keys. A test checks the tables
against `DROPPED` over every key below 2²⁴.

### Python: a line's indent is a run of spaces, one step

The layout scanner counted indentation a byte at a time. Stage 1 now keeps a fifth mask, spaces,
and the scanner measures a run of them with one trailing-zeros step (`Blocks::space_end`).

## Results

Instructions retired, base (before this work) and now; the digests are the same:

| parser | passes | base | now | change |
|---|---:|---:|---:|---:|
| JavaScript | 15 | 25.52 G | 19.80 G | −22.4% |
| TypeScript | 15 | 7.82 G | 6.47 G | −17.2% |
| TSX | 15 | 4.40 G | 3.75 G | −14.9% |
| Python | 15 | 20.40 G | 18.17 G | −10.9% |
| `.rr` | 20 | 15.27 G | 14.89 G | −2.5% |

Cycles (noisy, under load): JavaScript 6.32 → 5.36 G, Python 4.24 → 3.92 G, `.rr` 3.33 → 2.77 G.

Each step on its own, in instructions:

| step | JS | TS | TSX | Python | `.rr` |
|---|---:|---:|---:|---:|---:|
| writer: one final pass for bases and offsets | −15.7% | −10.6% | — | −9.8% | −2.6% |
| a restore keeps its tokens | −3.7% | −4.3% | −6.5% | | |
| the dropped-token hash | −1.3% | −2.8% | −1.2% | | |
| batches of 16 tokens | −3.0% | −2.8% | −1.3% | | |
| the space mask | | | | −1.6% | |

## Measured and dropped

- **Buffers kept between files** (as simdjson's parser keeps its buffers between documents): the
  writer's working columns and `.rr`'s token buffer, kept for each thread. Instructions and cycles
  did not change. Minor page faults rose, from 2.3 K to 8.3 K, over the `.rr` set. malloc already
  reuses freed blocks, and a reused column still has to be zeroed to its new length.
- **Checking only short names for a string prefix** (Python): no change. `string_prefix` already
  exits at the first byte for nearly every name, and the check scans the run twice.
- **No bounds check after `fill`** (ECMAScript `tok`): no change.

## Open

The rest of the profile is spread out. Each item below is about 1 to 2%:

- **ECMAScript's first-byte dispatch** (`token_at`'s match, about 5%). A 256-entry table for the
  single-byte tokens would replace the jump table's indirect branch with a two-way one.
- **ECMAScript's whitespace loop** (`skip`). One mask of blanks and line ends would take a run of
  them in one step, with the line-end flag read from the EOL mask over that range.
- **Numbers** are spanned a byte at a time in ECMAScript and Python. The `WORD` mask covers nearly
  all of a number's bytes; only `.`, an exponent's sign and `_` need a look.
- **`.rr`'s look ahead** (`kind_at`, about 5%) checks its bounds though the buffer has `EOF`s
  after the tokens. The parser can step by 2 past the last token, so the padding does not yet prove
  an unchecked read safe.
