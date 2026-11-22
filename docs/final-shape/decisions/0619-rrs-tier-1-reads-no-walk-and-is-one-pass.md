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

What stays: the records hold each token's code, because tier 2 reads it (0618), and, until step 4, its last byte. Stage 1 keeps the
masks the token loop reads for the file's blocks, as the lean lexer's `Blocks::of` does.

## Step 3: tier 1 hands tier 2 no mask (0586's part)

0586 made JSON's tier 2 read its tokens from the bytes, so its tier 1 keeps no mask for it. `.rerail`'s tier 2 reads
tier 1's records instead (0618), because its tokens' codes (kind, word, marks) are tier 1's. So the part of 0586 that
applies is the masks. Tier 2 read a mask only to split a region read in parts, and `.rerail` has none. Still, tier 1
built an empty `Masks` into its `Lexed`, and tier 2 held it (`P::m`), called a `split` that did nothing at each
record, and carried helpers nothing called (`first_bit`, `bit`, the pieces' `piece`, `settle`, `drop_pending`, …).

Now, where no region is read in parts:

- tier 1's `Lexed` has no `masks`, and there is no `Masks` struct;
- tier 2 holds no mask and makes no `split` call;
- tier 2 writes only the runtime's helpers its code calls (`drop_uncalled`, as 0586's byte form does).

Kotlin reads regions in parts, so it keeps its masks, and its file is unchanged.

## Step 4: tier 2 reads each token's end from the bytes (0586's other half)

0586 made JSON's tier 2 read from the bytes what tier 1 had found for it. In `.rerail`, after step 3, that was each
token's last byte (`lasts`). Tier 2 now reads it from the bytes, and tier 1's records hold only each token's first
byte and its code. The codes stay, as the goal's first item asks (0618).

- **Tier 2's `token_last(src, n, s)`** gives the last byte of the token tier 1 starts at `s`
  (`bitlex_lex_rust/byte_ends.rs`).
  - Its rule is the first whose start holds at `s`, as tier 1's dispatch takes it, with the grammar's assumptions at a
    start read in.
  - Its end is the rule's, as tier 1's token loop finds it: a run's last byte, a pair's, or a region's closer after the
    opener.
- **Each mask the end reads is made of the bytes, 64 at a time** (`rerail_simd8`'s backends), in place of stage 1's:
  - a class, by its byte ranges;
  - `prev` and `next`, as the window a byte back or on;
  - `escaped`, word by word with its carry, as tier 1's (a string's closer, `quote & ~escaped`).
  So `/*`'s closer `slash & prev(star)`, the backticks' `tt` and `ttt`, and a number's `num_run` are read as the
  grammar states them.
- **The source tier 2 reads is the padded one.** `parse(buf, n, lx)` takes it as tier 1's `lex` does.
- **Only where every rule has its byte form** (`ends_from_bytes`): no region read in parts, no second closer (`or`),
  no region `after` another. Elsewhere tier 1 keeps its last bytes. So Kotlin's files are unchanged.

Tier 1 still finds each end in its own pass, since the next token starts after it, and its checks read the token's
bytes. So a region's bytes are read twice: by tier 1's masks, and by tier 2's search for the closer. That is 0586's
shape: there too, tier 1 still makes `q` and `esc` for its own tests, and tier 2 finds the string's end from the bytes.

The record tests (`tier1s_records_are_the_interpreters`, the samples and the corpus) compare each token's last byte as
`token_last` reads it with the interpreter's. A last byte moved by one in a string's end fails them at the first
string.

The farm run also took in four probes and benchmarks of the JIT, which are run by hand (`#[ignore]`). One passes run as
its note says (`rr_words_text`, with `RR_WORDS`). Three fail: `rr_tier1_split`, `rr_sparse_marks_bound` and
`rr_tier2_clusters`. They fail the same way, at the same line, with the grammar from before step 1. They read the JIT
and the native code, not the generated Rust, so these steps did not cause them.

## Two faults the one pass had, found by measuring it

A benchmark (`rr-aot-speed`, and `rr_aot_against_jit` in `rr_parser`'s tests) read the corpus both as its files and
as one file. As one file, tier 1 took about 180 times as long a byte: it was quadratic. Two faults:

- **A search with no end read the blocks not made.** Their words are zeros until stage 1 makes them, so a search for
  a bit past the made blocks reads every word to the file's end. Each row's closer search is cut at the blocks made
  (`first_bit(&m_q[..made], …)`, `made_searches`). Two helpers had no end at all: `any_in` (a gap's newline and
  form-feed marks) found the first bit after the gap and then tested it against the gap's end, and `run_once_last` (a
  number's one `.`) did the same for the dot. Both now read only the words up to their end (`first_in`). This is in
  the helpers that Kotlin's tier 1 shares too, so its file changes in those two functions alone.
- **A region with an `else` form paused nowhere.** `t2`'s read has `else e_t2 bytes 2` in its `none` arm. If the
  closer was in a block not made yet, the arm took the empty literal instead of waiting. Now that arm pauses too, as
  the others do, while blocks remain. Samples hold a two-backtick literal, a string and a block comment across 1, 3
  and 11 blocks; the fault fails them.

On a Linux arm64 farm host, from `rr-aot-speed` (product build, the least of 7 rounds, the 1,831 files, 12.39 MB):

| | Before | After |
|---|---|---|
| Tier 1, the files | about 300 ms | 33 ms |
| Tier 1, the corpus as one file | about 56 s | 44 ms |
| Tier 2 | 63 ms | 63 ms |

Most of the cost was inside the files as well, because a file's last line and its numbers sent the searches to its end.

## Checks

On a Linux arm64 farm host:

- `rr_parser`: all 51 library tests, the corpus's two (the tapes and tier 1's records, 1,831 files since step 4;
  1,784 before), and `rust_is_current`.
- `rerail-rr-aot` builds without `test-hooks`.
- JSON's, Kotlin's, the compiler's and bitlex's library tests pass, and their files are unchanged.
- Each generator file I changed is clean under clippy. That includes the warnings that were there before:
  `decide` takes the exits as `exits_ending` finds them, which also drops an argument.

On a Linux x86_64 farm host, after step 4: `rr_parser`'s tape and record tests, the corpus's included.

Not run: Metal, the slow tier.
