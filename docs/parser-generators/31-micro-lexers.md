# Micro-lexers, and a block lexer in Bitlex's shape

## Intro

The lean Rust `.rr` parser (page 30) is the bar the compiled grammar must clear.
Its first lexer read a byte at a time.
This page rewrites that lexer in the shape Bitlex compiles to:
64-byte blocks, masks a bit a byte, the same operations by name.

The aim is convergence.
If the hand-written Rust and Bitlex's output have the same form,
Bitlex becomes a way to generate a known form,
not the place where the form is found.

Two pieces:

- **Micro-lexers** find where one construct ends:
  a comment, a string, a backtick literal.
  They start at the construct's first byte, run 64 bytes a step without a branch inside a step,
  and end at the first bit of a stop mask (a trailing-zero count).
- **A block lexer** finds every token start of a block at once, as simdjson's stage 1 does,
  then writes the tokens one at a time without a branch per token.
  Rare tokens (strings, comments, backtick literals, numbers, `#`, non-ASCII) go to their micro-lexers.

Both are in `native/crates/rr_syntax/src/micro.rs` and `lean.rs`.
Neither is on the product's path.

## The operations

A block is 64 bytes. A mask is one bit a byte.
Past the text's end a block is padded with zeros, which nothing stops at.

| Operation | What it gives | Bitlex's name |
|---|---|---|
| byte compare | bit `i`: byte `i` is `x` | `eq` |
| nibble lookups | each byte's classes, two table lookups | `class` |
| a byte, then another | bit `i`: bytes `i`, `i+1` are `x`, `y` | `pair` |
| escapes | the bytes a run of backslashes escapes | `escaped` |

`escaped` is Bitlex's own formula, simdjson's branch-free escape finder:

```rust
// The bytes a run of backslashes escapes; `cin`: the block's
// first byte is escaped. Returns them and the carry out.
fn escaped(run: u64, cin: u64) -> (u64, u64) {
    const EVEN: u64 = 0x5555_5555_5555_5555;
    let b = run & !cin;
    let follows = (b << 1) | cin;
    let odd_starts = b & !EVEN & !follows;
    let (sequences, overflow) = odd_starts.overflowing_add(b);
    ((EVEN ^ (sequences << 1)) & follows, u64::from(overflow))
}
```

On aarch64 a compare is four NEON `cmeq`s,
packed to 64 bits by simdjson's pairwise adds.
Elsewhere the same masks come 8 bytes a word.
A test checks the two agree.

## The micro-lexers

Each makes a stop mask a block. Its construct ends at the mask's lowest bit.

```rust
// A string's body: its closing quote is the first quote
// not escaped. A `\u` escape before it is noted on the way.
let (esc, out) = escaped(blk.eq(b'\\'), carry);
carry = out;
let quote = blk.eq(b'"') & !esc;
u |= esc & blk.eq(b'u') & below(quote);
quote
```

| Construct | Stop mask |
|---|---|
| line comment | `eq '\n'` |
| block comment | `pair '*' '/'` |
| string | `eq '"' & !escaped(eq '\\')` |
| backtick literal, one tick | `eq '`'` |
| backtick literal, two | `pair '`' '`'` |

Their test runs each against a scalar scan on 20,000 random texts
made of the bytes they stop at.

## The block lexer

```
starts = !space & !(word & word << 1)   // a word's first byte, or any non-space
pairs  = pair & starts                  // `:-`, `=>`, `!=`, `<=`, `>=`, `<:`, `->`
starts &= !(pairs << 1)                 // a pair's second byte starts nothing
```

`space`, `word` and the rare bytes come from one pair of nibble tables.
`pair` comes from a second pair:
a first byte's table names the second bytes it takes,
the next byte's table names which one it is,
and a pair is any bit of both.
All seven pairs are exact nibble products, so this needs no compares.

The rare starts cut the block into runs.
A run's tokens are written straight:

- the kind from a table on the first byte, or the pair's kind by a select;
- the end from the run of `word` bits (a trailing-zero count);
- the word's code (`code`, `min`, `decl`, …) from one 8-byte load,
  masked to the word's length, hashed into a 128-slot table
  where the 30 words take 30 slots.

A token that ends past the block starts the next block where it ends.
Lines are not counted per token:
an index of newlines is made when a line is first asked.

Tests: `lean_tokens_are_the_engines` compares every token and comment
with the engine's lexer on every `.rr` file.
`lean_tape_is_the_engines` compares the whole tape, also on cut files.

## Measured

Same run, interleaved, least of 7 rounds, load 35 to 60.
All the Rust here is at `opt-level=3`, with overflow checks and debug assertions off.

| | instructions | cycles | IPC |
|---|---|---|---|
| block lexer | 156M | 33.2M | 4.7 |
| byte lexer | 132M | 34.2M | 3.9 |
| tier 1 (compiled) | 532M | 106M | 5.0 |

**The block shape is at parity with the byte lexer**:
3% fewer cycles, 19% more instructions.

Why the shape does not win here:

- **The long constructs are few.**
  Strings are 38% of the corpus's bytes and line comments 23%,
  but they are 163,000 of 1.56 million tokens.
  The micro-lexers made those cheap, but they were never where the time went.
  Swapping them in alone changed nothing.
- **The cost is a token, not a byte.**
  97% of whitespace runs are one byte. 47% of words are one letter.
  57% of tokens are punctuation.
  Every form spends about 21 cycles a token.
- **Where the block lexer's instructions go:**
  the block masks are about 17M (measured by making them twice);
  the word codes about 46M (all tokens get one, branch-free);
  the rest is the token write: kind, end, the run's bookkeeping.

## What it says about tier 1

Tier 1 has the same shape and costs 3.2 times the cycles of either lexer.
So the shape is not tier 1's problem. What it computes a block is.

A block lexer needs four masks and per-token tables.
Tier 1 computes about 85 IR instructions a byte:
the keyword words and their marks a byte at a time,
the errors, the fallback's checks, the marks tier 2 reads.

The next step for Bitlex is to emit this form:

- the starts, the pairs and the rare runs,
- the micro-lexers as their stop masks,
- word codes by one load and a hashed table,

and to move what tier 1 now does with masks a byte (keyword words)
to per-token tables, as this lexer does.

### Words' codes for words alone, punctuation by one load

Two changes after the run above:

- A token's kind and length come from one load of a 1024-entry table,
  indexed by its first byte, whether it is a pair's, and whether that pair is `<:`.
  A word's length is its run of word bits instead.
- Only a word token gets a word code: one branch on "is it a word".

| | instructions | cycles | IPC |
|---|---|---|---|
| block lexer, before | 156M | 33.2M | 4.7 |
| block lexer, after | 132.5M | 32.6M | 4.1 |
| byte lexer | 131.6M | 34.3M | 3.8 |

The branch costs some IPC, but less than a code for every token did.
The block lexer is now level with the byte lexer in instructions
and 5% under it in cycles.
