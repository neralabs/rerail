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

## Open

- **Names are now the lexer's largest cost**: about 35 instructions of word hash and 30 for the
  run's end, for every name. A name's word code could be computed only where the parser asks for
  a keyword. The parser asks often, so this needs counting first.
- **Cycles: 44 a token against the floor's 4.** At about 2.7 instructions a cycle, the gap is
  branches: the per-token choice between table, name, number and the rest depends on the data.
  simdjson's stage 2 branches a token too; a table of the start byte's class (one indirect jump)
  is the next form to measure.
- **Strings and comments** (197 and 163 instructions) go through the scalar path: page 36's region
  walk would take them off the masks.
- **The other lean parsers.** Java, Rust, C# and PHP each keep their own copy of `Blocks::next`.
  `rerail_simd8::blocks` replaces it, and `run`'s form applies to each, behind each one's digests
  and oracles.
