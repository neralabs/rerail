# 0623: `.rerail`'s generated Rust, read beside the lean parser

Status: the comparison is made, and the order below is done. The item is the user's (2026-11-22): "please
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

## Step 5, second part: the gap marks asked where tier 2 asks them (done)

`.rerail`'s gap marks are `line` and `newline` (a newline since the grammar token before) and `json_bad`'s form feed.
Tier 1 searched each gap for both bytes at every token, and made their two words at every block. Lean's lexer sets no
mark: the few statements that ask whether a token is on a line read the bytes from the last token to it (`on_line`).

- **Tier 1 leaves a gap mark of one byte to tier 2**, in the bytes form, where no region is read in parts. That holds
  where the loop keeps no accumulator of the mark, no extra keeps it, and no `else escape` reads it at the file's end.
  Its records hold only the hits of the mark that the token tests give (`json_bad` at a number's `0` or a bare `-`).
  The block makes neither byte's word.
- **Tier 1's file says which marks, and how to ask them**: `ASKED` (their bits), and `gap(src, from, to, before, b)`,
  generated from the grammar. It tests `src[from..to].contains(&byte)`, and the mark's `unless after` on the word
  before.
- **Tier 2 asks them where it asks a mark**: `bit_at`, a gap question's `with` and `without` (`marks_hold`), and the
  marks a level refuses at the token it takes. Each asks the bytes from the grammar token before's first byte, as
  tier 1's gap did. A mark is asked once a token (`held`).
- **Kotlin's files** carry `ASKED = 0` and a `gap` that holds nowhere, so its tier 2 reads its records as before.

`rr_parser`'s 51 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes and records included. The records
test leaves the asked bits out of what it compares (`tier2_asked_marks`); the tapes test them.

## Step 6: tier 2 at `Narrow` where the widths are narrow (done)

Tier 2's parser is generic over the tape's layout (`P<'a, L: Layout>`, `Writer<L>`), as the lean parser's
`tape_at<L>`. `parse` parses at `Narrow` when the widths it guesses are `Narrow::WIDTHS`: the writer's `push` and `close`
are then written for constant widths. Otherwise it parses at the widths read at run time, and again at wider ones
where the writer asks (`Redo`). Kotlin's tier 2 takes the same form.

`rr_parser`'s 51 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes included.

## Step 7: a taken token checks only what the grammar has (done)

Tier 2's runtime is written for every grammar, and patched where a grammar lacks a feature, as step 3 of 0619 does for
the masks:

- **No region read in parts: no split string.** There are no pieces (`sub`, `k`, `piece`). A token's code is its
  record's (`code_at`), its start is its record's (`start`), and `bump` takes the record. The refused marks' test does
  not ask which piece is the token.
- **No mark that `drops` or `loses` an extra: no extra gone or lost.** There is no `gone`, `lost` or `moved`. `flush`
  writes each comment before the token with no `gone.contains` test and no lost comment first. `bump` keeps no lost
  comment's start, and `settle` moves no extra. The helpers that dropped, lost or moved one are gone.

`.rerail` has neither, so its `bump` is lean's: the refused marks where a level refuses one, then the record's span.
Kotlin has both, and its files are unchanged.

`rr_parser`'s 51 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes included.

## Step 8: tier 2 finishes as the grammar's lean parser finishes (done)

Tier 2's `parse` takes a `Finish` (`rerail_tape_format::Finish`), as the lean parsers' `tape_into` does. The crate's
`parse` picks it:

- `rerail-rr-aot` finishes into format 15's image (`Writer::finish`), as `rr_syntax::lean::tape` does. There is no
  `Columns`, so there is no code column copied out of the image, and no column cut to its length.
- `rerail-kotlin-aot` finishes into the columns (`Writer::finish_columns`), as `kotlin_lean` does: its tape is
  `kotlin_lean::Tape`, and its tests read it as that.

`rr_parser`'s tests read the parser as Rust's tape as they read the JIT's, with `image_nodes`. The reader of the columns
(`columns_nodes`) is gone. `rr_parser`'s 51 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes included.

That is the last step of the order.

## Item 5's first half: tier 1 writes only the token marks tier 2 reads (done)

Tier 1 set eleven token marks by a chain of compares on the word (`if word == 40 { marks |= 1 << 2 }`, …). `.rerail`'s
tier 2 asks each of them of the tokens themselves (`pat_fix_on`, … read the words), so it read none of those bits.
Lean's lexer sets no mark.

- **Tier 2's generator keeps the mark bits its code reads** (`G::reads`): a mark asked by its bit (`bit_at`), a token
  question's `with` and `without`, a refused mark, a settling mark, and the extras' bits. `tier2_loop_reads` returns
  them with the code.
- **Tier 1 writes only those**, where its loop reads the bytes and no region is read in parts. It lists the others
  with the asked gap marks in `UNRECORDED`, which the records test leaves out of what it compares.

`.rerail`'s tier 1 now sets no token mark by its word. Its records hold `float_after` and the token tests' hits of
the asked gap marks. Kotlin's files are unchanged but for `UNRECORDED = 0`.

`rr_parser`'s 51 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes and records included.

What 0623's list still has for tier 1 is the rest of step 3. A common token is still sent through the dispatch's rows,
where lean reads its kind and length from one table load (`TOKEN`).

## The depth count: tier 2's stack bounds it (done)

The user (2026-11-22): "Our goal is that we can't recurse infinitely and that we eventually fail on a too-deeply-nested
structure. It doesn't matter if we do some extra work first."

Tier 2 never recurses. It is one loop over its own stack (0618), and it refuses a level past its `DEPTH`. So where
tier 1 reads the bytes, it keeps no count of the grammar's `scan … max N` (`strip_scans`). No token counts its
brackets, and the block makes no word of them. A file nested too deep still ends in a refusal, in tier 2. This takes
back the part of 0619 step 2 that made the depth count the token loop's, for the bytes form.

The difference from the JIT is a file nested a little past 127. The JIT's tier 1 refuses it, and the parser as Rust may
parse it. The shared samples keep the depths of 126 and 127, whose tapes are the JIT's. `deep_nesting_ends_in_a_refusal`
nests brackets and JSON objects 128, 200, 3,000 and 100,000 deep, and checks the last two are refused.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes and records included. Kotlin's files
are unchanged.

## Step 3, second part: a token's row and kind by one load (done)

Each token found its row in three steps. It took its first byte's group from two nibble tables (`DISPATCH_LO`,
`DISPATCH_HI`, simdjson's `classify`), then matched the group to a row, then loaded its kind (`KINDS`). Lean reads a
token's kind and length from one load (`TOKEN`).

Now, where tier 1 reads the bytes and the rows' pick opens no region of its own, a token reads `TOKEN[c]` once:

- its row, where the first byte picks one with no test (a word, a number, an operator, a string, `#`);
- else the arm of its group's tests (`/`, which may open a comment, and the backtick);
- and its record kind, which the rows used to load from `KINDS`.

The dispatch keeps each byte's pick (`Picks::by_byte`), and the generator writes the table from it. The nibble tables
and `KINDS` go where nothing else reads them, as in `.rerail`'s file. Kotlin's file is unchanged.

A word's length is its run of the block's word bits (`brun_last`), as lean's.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes and records included.

## An operator's length from the block's pairs mask (done)

An operator found its length by `pair_6`, a `match` on its first two bytes. Lean reads it from its block's `pair` mask.

Now, where tier 1 reads the bytes, each block makes `k_pair_6` once: the bytes where one of the grammar's pairs
(`pairs two_start`) begins. It is made from two loads, the block and the block one byte on, one equality test of each
pair's bytes. An operator's last byte is `s` plus its bit there. The grammar's mask reads a run of pairs left to right
(`escaped`, lean's `left_to_right`). A token never starts at a pair's second byte, and tokens are taken one after the
other, so at a token's start the raw mask is that mask. A pairs row with a triple keeps its function. `.rerail`'s
`pair_6` is gone. Kotlin's file is unchanged.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass, the corpus's tapes and records included.

## Why the AOT's tier 1 was slower than the JIT's: a string's `\u` test read in full (done)

A line-level profile of `rr-aot-speed` (xctrace's Time Profiler on an M3 Max, the leaf addresses resolved by `atos`
with inlined frames) put over half of tier 1's time in one window word, `w_t13`: the test for a `\u` escape the engine
checks. Every string token ran it over its bytes. Its expression reads `\u`, then the hex digits and braces at up to
eight offsets on. Each class at an offset (`cls_hex(p + 3)`) is a window loaded, classified and turned into bits, so
the test read about a dozen windows for each 64 bytes of every string, `\u` or none. Lean scans a string once
(`micro::string_end`) and unescapes it only when it saw a `\u`.

Now the bytes form reads an `and`'s right side only where its left side holds any bit, when the right side reads three
classes or more and carries no escapes (a side with `escaped`'s carry is always read: the next window starts from it).
A string with no `\u` then costs the test its backslash and `u` windows. On spark2, over the 1,831 files, tier 1 went
from 33.3 ms to 18.9 ms (the JIT's: 17.0 ms). Kotlin's file is unchanged.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## A mark's pattern finds the token before only where it reads it (done)

Each token tier 2 takes asks whether a mark no rule takes holds there (`open_mark`: `.rerail`'s `end_on`,
`fn_uncalled`, `io_bare` and `no_value`). Each pattern's first function was called with the token before
(`prev_tok`, a scan back over the extras), which only a first element `(C)*` with an ending reads. Rust's loops are not
known to end, so the optimizer kept every scan. Now the call passes `self.n` (no token) where the first element does
not read it. On spark2 tier 2 went from 42.9 ms to 41.0 ms. Kotlin's file changes the same way.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## A mark asks its token's word before its cache (done)

At each token taken, `open_mark` tests the marks no rule takes. Each test went through the mark's once-a-token cache
(`held`: a bit set in `known`, the answer stored in `holds`) and then compared the token's word. Lean tests the word.
Now a mark whose every definition's run starts with words (`.`, `@`, `(`) asks first whether the token is one of them
(`matches!(self.word(), w::DOT_13) && …`). Only those tokens reach the cache, and the answer is the same. On spark2
tier 2 went from 41.0 ms to 37.5 ms. Kotlin's file gets the same guards.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## Tier 2 holds its token's code, as lean holds its token (done)

Each `word()` read the token's record: a bounds check on the record index, then the load. The rules ask it hundreds
of times (318 reads in `.rerail`'s file). Where no region is read in parts, a token is its record. There tier 2 now
reads the code once, as it reaches the token (`arrive`, into `cur`), and `word()` takes the word from it. The marks'
tests (`m_*`) are `#[inline]`, so `open_mark`'s word guards read that once. On spark2 tier 2 went from 37.5 ms to
35.9 ms. Kotlin's file gets the `#[inline]` only: its tokens can be a record's pieces.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## A token's tests of its bytes in one window loop (done)

A string ran three tests over its bytes (`str_bad`, a bad `\u`, a `\u` the engine checks), each a loop of its own
windows (`wcount_in`), each loading and classifying the string's bytes again. Lean reads a string once. Now, in the
bytes form, a run of tests of the same span (`if wcount_in(n, s, l, w_A) >= 1 { … }`, one after another) is one loop
(`wany_in`). It calls each test's window function on the same window, so their shared classes (the backslash) are
loaded once a window, and each keeps its escape carry. On spark2 tier 1 went from 18.4 ms to 17.7 ms. Kotlin's file is
unchanged.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## `open_mark` asks by the token's word (done)

`open_mark` called each open mark's test at every token taken, each a function with its own word guard. Now it is one
`match` on the token's word. A word some mark's runs start with calls those marks, plus the marks whose runs start
otherwise. Every other word calls only the latter, and in `.rerail` there are none, so `false`. On spark2 tier 2 went
from 35.7 ms to 35.1 ms. Kotlin's `open_mark` is the same `match`, its four unguarded marks in every arm.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## A block's masks from one classify of it, as lean's `Window` (done)

Each block word (`k_m_word`, `k_m_num_run`, `k_m_dot`, `k_m_digit`, `k_m_ws`) was its window function at the block.
Each class in it, at each offset, was a window loaded, classified, tested, turned into bits and masked to the file
(`cls_digit(p - 1)`, `cls_digit(p + 1)`). Lean classifies a block once and derives its masks from that.

Now, where every block word reads only classes within 63 bytes and no escapes, the bytes form writes
`block_classes`. It loads the block at `base` once, classifies it once a pair of nibble tables, and returns each
class's bits there, masked to the file. A block word is an expression of those bits. A class `d` bytes on is the
block's bits shifted down, the next block's shifted in (`cur[i] >> d | next[i] << (64 - d)`), and one back the same
way from the block before. Each block's classes are made once. The next block's, made for the shifts, are its
`ahead`. The block's own are the next block's `behind`, where the loop goes on to the next block rather than past it
(after a token over several blocks, the loop makes the neighbour's again). A word with escapes, or a class 64 bytes
away, keeps its window function. The windows past the block, which a token's reads go on to, are unchanged.

`.rerail`'s blocks now take one load and one classify (`LO_0`/`HI_0`) and four masks. `num_run` is
`digit | dot & prev(digit) & next(digit)` of those bits. On spark2 tier 1 went from 18.0 ms to 16.7 ms (the JIT's:
16.6 ms). Kotlin's file is unchanged.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

The pairs mask (`w_pair_6`) is now made in the vectors too: each pair's two equality tests joined (`S::and`), the
pairs joined (`S::or`), then one mask, as lean's `pair`. It made two 64-bit masks a pair before. Built side by side on
spark1, A/B in turns, tier 1 is the same (16.9 ms): the optimizer had already joined them. It stays for the shape.

## Tier 2 writes a leaf, and sets a field, in one writer call (done)

A token tier 2 takes was a leaf in two writer calls, `push` then `close`. A field set on the node made last was two,
`key` then `set_key`. Each call checks again whether the node is a held chain's (`chain_at`) or before the window
(`base`), and the leaf's size column was written twice. The writer now has `push_leaf`, a closed leaf at the end (its
tail from its start, as `insert_leaf`'s), and `set_field`, the field's bits where the key holds none. Each checks the
chain and the window once, and takes the two calls where the node is a chain's or before the window. Tier 2's leaves
(`tk`, `merge`, the comments `flush` and `settle` write) and `field_made` use them. A test
(`a_leaf_at_once_and_a_field_are_the_calls_they_join`) gives the same tape both ways, wide tails included.

Built side by side on spark1 and run in turns, three rounds each, tier 2 went from 34.1–37.6 ms to 32.7–33.1 ms (the
JIT's: 32.9 ms). Kotlin's file uses them the same way.

`rr_parser`'s 52 library tests, `kotlin_parser`'s 70 and `rerail-tape-format`'s 43 pass.

A wrap (`wrap`: the wrapped node's key read and its field moved to the wrap, its start read, the wrap inserted) and a
node's close (`close`: its key read for `UNSET`, its start read, the close) are now one writer call each too
(`Writer::wrap`, `Writer::close_from`, `#[inline(always)]` as `close` is), with the same fallback at a chain's node or
before the window. The node is closed before its `UNSET` refusal: a refused file's tape is not read. The test above
covers them. Side by side on spark1 they are neutral (34.5 ms both, the host at a load of 9). With `#[inline]` only
they were 1.5 ms slower, left as calls.

## The one loop's stack is its own local (done)

Every call and return of the one loop (0618) pushed and popped `self.stack`. After any `&mut self` call between them,
the Vec's length, room and pointer were read from memory again. Only `run` reads the stack, so it now takes it into a
local (`std::mem::take`) as it starts, and the code reads `stack`. Side by side on spark1 (at a load of 9), tier 2 went
from 34.9–35.6 ms to 34.2–34.8 ms. Kotlin's tier 2 is not this loop, and its file is unchanged.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## One stack of levels, not two (done)

Tier 2 kept the nodes open (`open`) and the node made last at each (`made`) in two vectors, always pushed and popped
together: a node opened, a wrap, a close. Each level was two pushes and two pops, each with its own length and room.
Now they are one vector of pairs (`levels`: the node open, the node made last in it). Side by side on spark1 (at a load
of 14), tier 2 went from 33.1–34.3 ms to 31.9–33.3 ms. Kotlin's file changes the same way.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## `open_mark` asks a token-only mark afresh (done)

At each token taken, `open_mark` asked its marks through their once-a-token cache (`held`: `known` and `holds` read,
set and written), for an answer only `open_mark` asks there. A mark whose test reads only the tokens (no flag, word or
refusal the rules set) gives the same answer fresh. `open_mark` now asks such a mark's test directly, its `taken` bit
first: `(self.taken >> 11 & 1 == 0 && self.h_end_on())`. The tests (`h_*`) are `#[inline]`. Side by side on spark1,
tier 2 went from 32.4–32.9 ms to 31.6–32.5 ms. Kotlin's `open_mark` asks its token-only marks the same way.

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## A leaf takes its field as it is written (done)

A leaf in a field (`path: string`) was written with no field (`tk(…, 0)`), then labelled by `field_made`: the node
made last read from the levels, its key read and set again. Lean writes a token's field with the token. Now a pass at
the end of tier 2's code (`leaf_fields`) folds a leaf of no field (`tk`, `word_leaf`) followed by `field_made(F)` into
one `tk(…, F)`: the node made last after a leaf is that leaf, and its field is none. In `.rerail`'s file 46 of the 128
`field_made` calls go. Kotlin's has no such pair and is unchanged. Side by side on the M3 Max, four rounds in turns,
tier 2 went from 41.1–41.4 ms to 40.2–40.6 ms (spark1, at a load of 9, could not tell them apart).

`rr_parser`'s 52 library tests and `kotlin_parser`'s 70 pass.

## Tier 2's writer calls check for no chain and no passed node (done)

Every call of the writer tier 2 made still checked whether its node was a held chain's (`chain_at`) or passed on to a
format-16 target (`base`): `key`, `start`, `set_key`, `close`, and the joined calls once each. Tier 2 opens no chain
and writes an image or the columns, never format 16, so neither case can arise. The writer now has `plain_` calls for
such a tape (`plain_key`, `plain_start`, `plain_set_key`, `plain_set_start`, `plain_close`, `plain_leaf`,
`plain_field`, `plain_close_from`, `plain_wrap`). They check neither, read a node's cell at its index, and assert both
in a test build (`Writer::plain`). The checked calls share their bodies. A pass at the end of tier 2's code
(`plain_writer`) makes every such call the plain one. The writer's test builds the same tape the three ways: the calls,
the joined calls, the plain ones.

Side by side on spark1 (at a load of 21), three rounds in turns, tier 2 went from 34.6–34.7 ms to 29.9–31.6 ms. The
checks were cheap branches, but each kept the writer's fields read through the loop. Kotlin's file uses the plain calls
the same way.

`rr_parser`'s 52 library tests, `kotlin_parser`'s 70 and `rerail-tape-format`'s 43 pass.

`size`, `end` and `truncate` (the relabels, the readings again, `merge`) have plain calls too (`plain_size`, `plain_end`,
`plain_truncate`), so every writer call tier 2 makes is plain but `push` and `len`, which check nothing.

### A token with no comment after it reads one record

Comments stay records between the tokens (tier 1 writes them in order), but the walk past them is off a token's path
when none follows it. `advance` reads the record after the token once: a comment sends it to `next_tok`'s walk, else
that record is the next token, and nothing is pending (`pend` moves with `i`), so the next take's `flush` is one
compare instead of a step over the token taken last. Lean's take is `i += 1` beside a separate comment list; this is the
same work on a token with no comment. On spark1 tier 2 was 30.3 ms (the plain calls' 29.9–31.6 ms); all tests pass.
