# 0623: `.rerail`'s generated Rust, read beside the lean parser

Status: the comparison is made; the fixes are in the order below. The item is the user's (2026-11-22): "please
figure out how JIT is faster than AOT, that should not ever be true".

## The question

The JIT (Cranelift, from the grammar) parses the corpus faster than the generated Rust (`rerail-rr-aot`, rustc from
the same grammar), in tier 1 and in tier 2. The build profile is not the reason: a release build (one codegen unit,
thin LTO) stays behind too. Both the generated Rust and the lean parser (`rr_syntax::lean`, hand-written) go through
rustc, and the lean parser is ahead of both. So each difference is a difference in the Rust source we generate. This
record puts the generated `tier1.rs` and `tier2.rs` beside `lean.rs` and `micro.rs` and names them.

`rr_aot_against_jit` (in `rr_parser`'s tests, run by hand) now times the lean parser's lexer and whole parse beside
both, so the three are read in one run.

## Tier 1: `tier1.rs` beside `lean::lex` and `micro::Window`

1. **The masks a block keeps.** Lean's block is a `Window`: its 80 bytes and four masks (`space`, `word`, `rare`,
   `pair`), locals of the loop, made again at the next block. Ours makes 33 masks per block, stores each into a vector
   of the whole file (`vec![0u64; nw]`, zeroed at the file's start), and reads them back from there. Seven of them are
   never read by the token loop at all (`m_quote`, `m_slash`, `m_hash`, `m_alpha`, `m_punct`, `t7`, `m_two_start`):
   they are only the inputs of other masks.
2. **A region's end.** Lean runs a region's micro-lexer from its opener, over the bytes (`micro::find`,
   `find_pair`, `string_end`): a comment, a string or a backtick literal is read once, where it is. Ours searches the
   whole-file masks for the closer (`first_bit(&m_q[..made], …)`), and when the closer is in a block stage 1 has not
   made, it pauses, makes more, and reads the token again (0619 step 2).
3. **The common token.** Lean splits a block's starts into the run of common tokens (words and operators) and the
   rare ones (`rare`: `/`, `"`, `` ` ``, digits). A common token is one table load (`TOKEN[c | pair << 8 | …]`: kind and
   length) and a word's length is its run of `word` bits. Ours sends every token through the dispatch group, then a
   `match` on the row, each row with its own reads and checks.
4. **A word's code.** Lean finds it from one 8-byte load and the length (`word_code(raw, ninth, words)`). Ours
   matches the token's slice against every word of the grammar (`word(&src[s..=l])`, a `match` on byte strings).
5. **The marks.** Lean's lexer sets none. The few statements that ask whether a token is on a line read the bytes
   from the last token to it. Ours sets each token mark by a chain of compares on the word (`if word == 40 { marks |=
   … }`, one for each mark), and each gap mark by a search of a whole-file mask (`any_in(&m_lf, from, s)`).
6. **The records.** Lean writes one 12-byte record a token (`Tok { start, end, kind, word }`) behind a raw pointer,
   into room reserved for a token a byte, so the write needs no check. Ours pushes into three vectors (`starts`,
   `lasts`, `codes`), each push with its own capacity check.

## Tier 2: `tier2.rs` beside `lean::tape_at`

1. **The widths.** Lean parses nearly every file at `Narrow`, whose widths are constants of the type, so the writer's
   `push` and `close` are written for them (`tape_at(src, &lexed, Narrow, finish)`). Ours parses at `Widths`, read
   from the writer at each `push` and `close`.
2. **A token's read.** Lean reads one record: its kind and word are bytes of it. Ours reads a token's code from
   `codes`, its start from `starts` and its end from `lasts`, and each read first asks whether the token is a piece of
   a split string (`code_at`'s `self.k < self.sub.len()`). `.rerail` splits none.
3. **What a taken token checks.** Lean's `bump` moves to the next token. Ours checks, at every token, a mark left
   open, the marks the level refuses, a lost comment, the split string's pieces, and then `flush`'s loop over the
   comments before it, with `gone.contains(&j)` for each. `.rerail` has no mark with `loses` and no split string.
4. **The tape's end.** Lean finishes into the image (`Writer::finish`). Ours finishes into `Columns`
   (`finish_columns`): the code column copied out of the image (`to_vec`), and the size and tail columns cut to
   their length (`shrink_to_fit`, which may move them). That is also a copy that 0353 forbids.
5. **Control.** Lean's rules are Rust calls. Ours is one loop over places with its own stack (0618). That is the
   goal's shape and stays: the differences above are inside the places, not in the loop.

## The order

Each step changes the generator, writes the files again, and passes the tapes and records tests (the samples and the
corpus). Tier 1 first, since tier 2 reads its records:

1. Tier 1's masks as block locals: no whole-file mask, none made that nothing reads (1).
2. A region's end found over its bytes from the opener, as lean's micro-lexers do. Then the pause has nothing to wait
   for, and one pass is lean's loop (2).
3. Common tokens as one table load and a run, and the rare ones by their rows (3).
4. A word's code from one load (4).
5. One record a token, written behind a pointer (6), and the gap marks asked where tier 2 asks them (5).
6. Tier 2 at `Narrow` where the widths are narrow (tier 2's 1).
7. Tier 2 reads one record, with no split string where the grammar splits none (tier 2's 2, 3).
8. Tier 2 finishes into the image, with no copy (tier 2's 4).

## Step 1: the one pass reads its masks from the bytes (done)

Steps 1 and 2 of the order are one change. Once no mask is kept for the file, nothing is made ahead of the loop, so a
token has no block to wait for. `bitlex_lex_rust/bytes.rs` writes each read of the loop from the bytes where it reads
them:

- **A test at a byte** (`bit(&M, i)`: the dispatch's `//`, `/*` and backtick tests, a bracket in a token) is a test of
  the bytes there, as lean's `c == b'/' && d == b'/'`.
- **A search or a count** (a closer's `first_bit`, a string's checks' `count_in`, a run's end, a number's dot, the
  depth's brackets, a gap's newline) reads its mask 64 bytes at a time from its first byte. Each window is made of the
  bytes loaded where they are: a class by its byte ranges, `prev` and `next` as the window a byte back or on, and
  `escaped` word by word, its carry from the search's first byte. A mask that reads escapes is read only from a
  token's first byte or the one after: a start is never escaped.
- **A block's token starts** are the block's window of `skip`.

`lex_on` is one loop over the blocks, as lean's `lex`: no stage 1, no `'pass`, no `made` or `wait`, no vector of
masks. The test build's walk keeps its own stage 1 over the file (`stage1_best`), which only its checks read. The form
applies where stage 1 tests nothing of its own (no error tested over the blocks, none after the last) and every mask
the loop reads has a form of the bytes. Elsewhere the pass keeps stage 1. Kotlin's and JSON's files are unchanged.

`rr_parser`'s 51 library tests pass, the corpus's tapes and records included.

The reads are lean's in where they read, not yet in what they read. A run or a closer search makes a window per 64
bytes of its mask, and a test at a token's start reads each byte with a bound. The next steps make them lean's: the
common token's one table load (3), the word's one load (4), and the region readers as lean's micro-lexers (2's
other half).

## Step 4: a word's code from one load (done)

A token's word is `word_at(src, s, len)`. It loads the token's first 8 bytes (the source is padded, so the load needs
no check), masks them to its length, and hashes them with the length by one multiply into `WORDS`, where each of the
grammar's words has a slot of its own. This is the lean lexer's `word_code`, except that lean's table was written by
hand and ours is the generator's: it searches a fixed sequence of multipliers (splitmix64's) for one that gives each
word its own slot, so the table is the same at every build. A word of more than 8 bytes compares its other bytes too,
where lean checks `violation`'s ninth. The `match` on byte strings is gone where the bytes form applies (step 1).
Elsewhere, and where no multiplier is found, it stays.

`rr_parser`'s 51 library tests pass, the corpus's tapes and records included.

## Step 3, first part: the block's masks once a block, as lean's `Window` (done)

- **The block's words.** At each block, the loop makes once, from the block's 64 bytes, the masks its tokens read
  where they start (`k_M`: a word's run, a number's run and dot, the brackets the depth counts, the gap's newline and
  form feed, the skipped bytes). A token reads its bits from the block's word, and goes on in windows only past the
  block. That is lean's `micro::run`, then `word_end` (`brun_last`, `bfirst_in`, `bcount_in`, `bfirst_clear`).
- **The region readers read from the opener.** A region's row (a rule `read …`) reads its closer and its checks in
  windows from the token's first byte, as lean's micro-lexers read from theirs, and the block makes none of their
  masks.
- **A class is made by the nibble tables.** A window's class is stage 1's: its pair of nibble tables (`classify`), then
  one test and one bitmask. The tables are the ones the file holds (`stage1_codes`: stage 1 makes a class's code again
  against the tables of the classes it writes). A first try took a node's own code, which names an earlier table
  layout, and the samples with a `\u` or a `\q` in a string failed. The tests caught it.
- **A read of one byte is a test of the bytes there** (`first_clear(&M, s, s + 1)`: a zero or a minus before a
  digit).

`rr_parser`'s 51 library tests pass, the corpus's tapes and records included.

What the block makes is still more than lean's four masks (`space`, `word`, `rare`, `pair`): a number's three, which
lean reads as a rare token from its start, and the gap's newline and form feed, which lean's parser asks only where a
statement asks it (step 5).

## Step 5, first part: one record a token, written behind a pointer (done)

- **Tier 1 writes a `Rec { start, last, code }` a token**, the lean lexer's `Tok`, in place of three vectors. It
  reserves room for a record a byte (`n + 1`) once. Each token's first byte comes after the last token's last byte,
  so the writes (`out.add(len).write(…)`) need no check, and the vector's length is set once, at the end. A test
  build asserts the bound at each write.
- **Tier 2 reads one record** (`self.recs[j].code`, `.start`, `.last`) where it read three vectors.
- **Kotlin's tier 1 and tier 2 take the same form**, since the generator writes both. A string resumed over a
  template's code (decision 0600) sets its held record's last byte through the same pointer.

`rr_parser`'s 51 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes and records included. JSON's files
are unchanged.

The gap marks (`any_in` of the newline and form feed at each token) are still tier 1's. They move to where tier 2
asks them in step 5's second part.
