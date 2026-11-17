# 0318: The lean lexers take tokens off stage 1's start masks, in simdjson's shape, with shared primitives

Status: built for C, C++ and Go (user, 2026-11-16: "go ahead / make sure that the primitives are
reusable so that we can apply the same tricks to the other parsers"). Follows 0313's "simdjson, the
gold standard". Numbered past the highest number in any worktree (0317).

## Intro

simdjson reads JSON in two stages. Stage 1 classifies 64 bytes at a time into bit masks and flattens
the token starts into positions. Stage 2 walks those positions and writes its tape without bounds
or capacity checks. 0313 measured that shape on C's byte classes: 15 instructions and 4 cycles a
token. The lean C lexer cost 145 instructions and 53 cycles a token. Its stage 1 already was
simdjson's, so the difference was all in stage 2.

This decision rebuilds stage 2 of the lean C, C++ and Go lexers in that shape. The pieces any lean
lexer needs now live in `rerail-simd8`, so the other lean parsers (Java, Rust, C#, PHP, which each
keep a copy of the same mask scan) can take them up.

## Where the instructions were

The lexer was timed one token shape at a time (`c_costs` with `RERAIL_COSTS_ALL`, over texts of one
shape: 64 files of 16 KB each), and the per-token paths were counted instruction by instruction
under lldb.

| shape | before: instructions a token | of them |
|---|---:|---|
| `(` | 124 | `token_at` 56 (16 of them saving and restoring 12 registers; the `Err` result written before it is known), the loop 61 (the result through the stack, copied twice; the vector's length and capacity reloaded and checked) |
| a name after a blank | 160 | the gap's walk 37, the name's end 30, the word hash 35, the write 8; the rest mask pointers reloaded and bounds checks |
| a decimal integer | 306 | `number`'s digit loops, each byte bounds-checked |

Stage 1 was 3 instructions a byte, as simdjson's. No shape's cost was the classes; it was the code
around each token.

## What was built

**In `rerail-simd8` (any lean lexer):**
- `blocks::Masks<N>`: a text's masks, `N` a block, scanned without bounds checks (each loop keeps
  its block in range):
  - `next`: the first byte from a position in (or out of) one mask;
  - `first_in` and `last_in`: the first and the last byte of a mask in a range, scanning only the
    blocks of that range. A gap's line ends are looked for between the token before and the next
    start, never past it.
- `blocks::starts`: a block's token starts. A start is any byte that is not a gap and is not a word
  byte after a word byte (simdjson's `follows`).
- `blocks::Cursor`, with `Masks::next_start`: a walk over the start masks. It holds the last block's
  start mask, recomputed from stage 1's masks when the walk enters a block. Its value is a function
  of the position only, so a relex or a parser's rewind resets nothing.
- `swar::digits`: whether 8 bytes (or the first `n`) are all digits, in one test.

**In each lexer (`Lexer::run`, a batch of 16 tokens, the parser's buffer refill):**
- **Writing.** The tokens go into room reserved ahead and the length is set once, as simdjson's tape
  writer does.
- **The gap.**
  - None: one table load, as before.
  - One blank before a token: two loads.
  - Any other blanks and line ends: the cursor finds the next start. The first line end is a scan
    bounded by that start; Go's last one (`nl_last`, where its semicolon goes) is another.
  - A comment (and C's `\`): the scalar `token` from the gap's start.
- **The token.**
  - The ten one-byte punctuators: their kind from a table.
  - An ASCII name: its word run.
  - A decimal integer of up to 8 digits with nothing after it: its word run and one 8-byte test.
  - Other punctuation: `token_at`, now split. The punctuation `match` is inlined. Names with a
    literal's prefix, numbers, literals, directives and the rest go to an outlined `token_rare`, so
    a punctuator is read with no frame to save.

Each crate has a test (`lex::tests::run_reads_as_token_does`) that reads random texts of the
language's pieces by `run` and by `token`. It reads from position 0 of every text, and from every
position of some of them, as a relex may start anywhere. The batches are of 1, 3 and 16 tokens. The
test checks every token, line end, comment and refusal. It found a difference the oracles' corpus
had not: in C a number before a byte past ASCII is refused, and the integer path did not check for
one. It now does.

A first form that only took starts off the cursor, with every token still going through `token_at`,
gained 1.5%. The table, the write and the split made the rest.

## Results

On the M1 Max (load 2.5 to 4), the best of three rounds, interleaved, against the commit before
(`440109790`):

| | instructions | | cycles | |
|---|---:|---:|---:|---:|
| C, the parse | 10.97 → 9.77 G | −11.0% | 2.88 → 2.60 G | −9.7% |
| C++, the parse | 16.16 → 14.72 G | −8.9% | 4.02 → 3.70 G | −7.8% |
| Go, the parse | 13.79 → 12.57 G | −8.8% | 3.67 → 3.34 G | −8.9% |
| C, the lexer alone, a token | 145 → 119 | | 52.9 → 44.3 | |
| Go, the lexer alone, a token | 133 → 117 | | 48.7 → 40.8 | |

By shape, C's lexer alone, this machine (instructions a token): `(` 124 → 44 (and 38 → 6 cycles);
operators 110 → 77; a name after a blank 167 → 136; a decimal 306 → 133; a string 251 → 197.

The digests of every tape are unchanged (C 0dd40a1d1f557d70, C++ 606c5e56ae76bb1f, Go
a18630f041d13f2e). The oracles are the same under Relax: C 829/0/0 and 1,161/0/10, C++ 937/0/63 and
1,271/0/113, Go 860/0/0 (9 error texts recovered as tree-sitter does) and 1,575/0/0 (11). The
engine's 31 lean-parser tests pass.

## Then: micro-lexers shared, stage 1 cut to three masks (2026-11-16)

User, on the open items below: "keep working through this … We created primitives for matching
keywords and similar, please find them and use them where possible".

**The primitives found.** Every lean parser with keywords already reads them by
`rerail_simd8::keyword_switch!`; the C and Go parsers read every keyword by word code, never by
text. The micro-lexers of page 31 (`find`, `find_pair`, `string_end`, and the newline scans) lived in
`rr_syntax`, private to the `.rr` lexer.

**What changed:**
- `rerail_simd8::micro` holds the micro-lexers now; `rr_syntax` uses them from there. New:
  `first_of(text, from, [a, b, c])`, the first of three bytes. It runs a 16-byte SWAR test first,
  for the short string or argument, before a block scan.
- C and Go stage 1 keep three masks (word, blank, line end), not six. Strings, block comments,
  directive arguments, raw strings and Go's raw strings are read by the micro-lexers where they
  start (`first_of`, `find_pair`, `find`), not from masks made for every block.
- `blocks::Masks::run_end`: a word run's end in its own block by one load and a trailing-zero
  count, scanning on only for a run past it.
- `swar::load8` takes any position, past the text's end too (zeros).

**Results**, this machine (load 9 to 16), instructions, digests unchanged:

| | before | after |
|---|---:|---:|
| C, the parse | 9.77 G | 9.76 G |
| C, the lexer alone, a token | 119 | 114 |
| stage 1, a token | | about 35% fewer |
| a string, a token | 197 | 234 |

The string is the cost of reading its end from the bytes, not a mask. Strings are 1.4% of C's
tokens, and stage 1 runs for every block. The oracles are the same as above, and so are the
engine's 31 lean-parser tests.

### Then: the gap, the start byte, the name's probe

- **A gap** that ends in its own block (nearly all) is one run of its block's blank and line-end
  masks (`Masks::run_in_block`): the next token starts at the run's end, and the gap's line ends
  are the line-end bits inside it. The start cursor and the line-end scan are left for gaps that
  run past their block.
- **The start byte's class** is one load (`LEAD`): a one-byte punctuator with its kind, a name,
  a byte for `token` (a comment's `/`, C's `\`, a NUL, or the text's end read as a 0), or the rest.
  This replaces two table loads and three compares. With no gap, the byte already read for the gap
  test is the start byte. A gap's one-blank and comment tests are `FIRST` bits.
- **The token's write** is a field at a time. A write of the whole `Tok` copied its padding from a
  stack slot.
- **The name's probe.** `words.rs` kept its own copy of the 8-byte load. Both copies checked the
  load's end with an overflow test as well. Now there is one load (`swar::load8`, one compare in a
  loop). The direct shape's key takes the length by an xor, not a multiply by a constant: the slot's
  multiply carries the low bits up. Two words with the same first 8 bytes still differ by their
  lengths, and two words of different lengths up to 8 bytes differ in their highest byte. C and Go
  assert at compile time that their lists keep the direct shape.

**Results** on the M1 Max (load 2 before the run, 5 to 8 during it), the best of three rounds,
interleaved, against `81d1d4fe7` (0318 as first built):

| | instructions | | cycles | |
|---|---:|---:|---:|---:|
| C, the parse | 9.77 → 9.31 G | −4.7% | 2.59 → 2.47 G | −4.8% |
| C++, the parse | 14.72 → 14.16 G | −3.8% | 3.70 → 3.58 G | −3.5% |
| Go, the parse | 12.56 → 12.03 G | −4.3% | 3.34 → 3.21 G | −3.7% |
| C, the lexer alone, a token | 119.3 → 102.7 | | 44.3 → 38.8 | |
| Go, the lexer alone, a token | 117.4 → 105.6 | | 40.7 → 36.4 | |

By shape, C's lexer, this machine: `(` 42 → 34 instructions a token; other operators 75 → 66;
a name after a blank 129 → 115; after a line end and indent 188 → 150. The digests, the oracles,
the engine's 31 lean-parser tests and the tests of every lean parser that shares `Words` are
unchanged.

On this machine (an M5, often at loads of 30 to 100) the lexer's instruction counts moved by up
to 5% between runs, the fixed cost of a run most of all. Only the farm's figures are taken.

### Then: strings and comments, and the other lean parsers' scans

- **A string's quote** has its own class in `LEAD`: the run calls `string_end` at once, not
  `decimal`, `token_at` and `token_rare` first. A C string token: 230 → 142 instructions.
- **A gap with a comment** (`skip`) passes its blanks and line ends by the block's run, as the run
  does. Go's last line end keeps its rule: no comment closes inside a run, so the run's last line
  end is taken before a comment has closed, and its first after one where none is set. A C line
  comment: 152 → 128 instructions, 64 → 43 cycles.
- **The other twelve lean parsers** (C#, CSS, ECMAScript, HTML, Java, Markdown and its inline
  grammar, PHP, Python, Rust, SQL, Vue) each kept a copy of the mask scan. Their `Blocks` now
  holds `rerail_simd8::blocks::Masks` and scans by it (one definition, its loads unchecked). Eight
  dropped the text they held only for its length. Their own tests and the engine's 31 lean-parser
  tests pass unchanged.

### Measured and dropped

- **One path for every token** (page 31's run in full): the starts a block at a time, a
  punctuator's kind and length from `OPS[first byte][second byte's class]` with its third byte by
  a select, a name's keyword probed on the lanes loaded at its start, and one branch out for rare
  starts. It was built, and it matched `token` on the random-text test. It cost far more: `(`
  42 → 117 instructions a token, a name after a blank 129 → 183, C's parse 9.76 → 11.05 G. Each
  token computed both the name's and the punctuator's answers. The test for a rare start took
  about 20 instructions. Eight values spilled to the stack. The branchy run predicts well on this
  mix (0313 found the same of the punctuation table).
- **A keyword probe with no branch** (`Words::find_lanes`, on the two lanes as loaded): a name
  129 → 151, C's parse 9.76 → 9.94 G. `find` returns early on a length outside the list's span,
  and loads the second lane only past 8 bytes. Both were removed.

## Open

- **Cycles: 39 a token against the floor's 4** (44 before the steps above). At about 2.7
  instructions a cycle, the gap is branches: the per-token choice between punctuator, name,
  string and the rest depends on the data. One path for every token cost far more (above).
  `LEAD`'s class by one indirect jump, as simdjson's stage 2 takes its byte, is not to be had
  in Rust: a `match` on the five classes compiles to the same compare chain, and a table of
  functions adds a call to every token.
- **Strings and comments** (142 and 128 instructions) are read by the micro-lexers. Page 36's region
  walk would take them off the masks, but that needs stage 1's stop masks back for every block,
  for 3% of the tokens.
- **Names** (115). Every identifier's code is read (`is_name` asks `reserved(t.word)`), so a code
  made only where the parser asks would save nothing. What remains is the run's end, the check of
  the byte after it (a `\` or a byte past ASCII), the length mask and the probe.
- **The other lean parsers** scan by `rerail_simd8::blocks` now (above). `run`'s form (a batch
  of tokens, `LEAD`, the gap's run, the micro-lexers in place of stop masks) applies to each,
  behind each one's digests and oracles; it is not built for them yet.
