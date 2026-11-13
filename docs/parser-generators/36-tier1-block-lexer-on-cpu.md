# Tier 1 on the CPU in the block lexer's form: the design

Status: decided (2026-11-13, decision 0139); building.

## Intro

Tier 1 is the compiled parser's lexer. It turns a file's bytes into the records tier 2 reads:
each token's first byte, last byte and code (its kind, its word, its marks).

Tier 2 now costs less than the lean parser's parse. Tier 1 does not:

| Over the corpus (1,360 files, 8.8 MB) | Instructions |
|---|---|
| Tier 1, ahead of time | 464.6M |
| &nbsp;&nbsp;pass 0 (forward) | 163.7M |
| &nbsp;&nbsp;pass 1 (backward, one `back_fill`) | 2.1M |
| &nbsp;&nbsp;pass 2 (forward, the extracts and the records) | 298.7M |
| The lean lexer | 132.4M |
| &nbsp;&nbsp;its block masks | about 17M |

The lean lexer (page 31) has the same block shape as tier 1. It makes four masks a block,
then works a token at a time. A string, a comment or a backtick literal is read from its
start by a micro-lexer: a stop mask a 64-byte block, its first bit the construct's end.

Tier 1 makes every check at every byte, about 2,500 instructions a block:

- **Regions by rounds.** Strings, comments and backtick literals hide each other.
  Tier 1 finds them by a `rounds until settled` group: the region masks over the whole block,
  again until they stop changing.
- **A pass for one fact.** `tt_later` ("a `` `` `` later in the file") is a `back_fill`.
  It splits tier 1 into a pass, a backward pass and another pass.
  The first stores 38 mask words a block for the last.
- **Words spelled in bytes.** `input_w` and `output_w` spell `input` and `output`
  with `next()` byte tests at every byte. The record already knows a token's word.
- **Checks at every byte that belong to a token.** JSON's rules (`json_bad`, 16 operations),
  a functor named and not called (7), an I/O directive with no relation (6),
  a number of 19 digits (5), a byte no token takes (7).
- **Two-byte punctuation by compares** (`mask two`, 23 operations).
  The lean lexer finds its seven pairs as one nibble product.

## The decision proposed

On the CPU, tier 1 takes the block lexer's form. Metal keeps today's bit-parallel form.
Both come from the same program: the new pieces are constructs of the block program,
each with two lowerings, not a second module.

### 1. Regions as a construct of their own

A region kind is an opener, a closer, and what happens at the file's end:

```
regions {
  string   = q .. q                 # its closing quote, the first not escaped
  line     = line_open .. lf        # the newline is not the comment's
  block    = block_open .. block_end
  tick1    = tick1 .. tick
  tick2    = tick2 .. tt  else empty    # no `` later: the empty literal
  tick3    = tick3 .. ttt
}
```

It gives the masks the rounds give today: `hidden`, each region's `fill`, its last byte.

- **On Metal**, it lowers to today's rounds, unchanged.
- **On the CPU**, to the lean lexer's micro-lexers. A walk over the block's openers outside
  regions: at each, the first closer after it (a trailing-zero count of the closer's mask).
  A region open at the block's end is carried to the next block, as a string's state is now.
  The walk is a loop over regions, never over bytes.
- **The interpreter** runs the walk. It is the construct's meaning:
  regions taken left to right, as the rounds' fixed point gives them.
  A test checks the rounds give the same masks.
- **`tt_later` goes.** "No `` later" is what the walk meets at the file's end,
  so the `back_fill`, the backward pass and pass 0's 38 stored words go with it.
  Tier 1 on the CPU is one forward pass.

### 2. Checks moved to the token

The prover that moved the depth count and the `\u` check to tier 2 moves the rest it can.
What it cannot prove stays as a mask.

- **Keywords as words.** `input_w`, `output_w` and `dir_start` become token marks
  (`tokens after "." "input"`): the record's word, one compare. The two escapes that read them
  (an I/O directive with no relation, a functor not called) become marks the grammar refuses.
  This is a change to the `.rr` grammar, in the form token marks already have.
- **A token's own checks at its token.** A number of 19 digits is its length.
  JSON's leading zero and lone `-` are its number tokens' first bytes.
  These become local checks at their token, as the `\u` check is.
- **A byte no token takes** becomes a token kind of its own: the kind table's "bad",
  refused where tier 2 meets it.

### 3. The records from the starts alone

Today the records come from two extracts, every token's first byte and its last byte,
zipped, then a pass a token. On the CPU, one walk over the starts makes each record,
as the lean lexer writes a token:

- the kind from one table load on the first byte, as now;
- the end from the token's run: a trailing-zero count of the mask its bytes continue
  (a word's run, a pair's second byte, a region's end from the region walk);
- the word by its hash, as now.

There is no `lasts` extract, and no `lasts` masks.
The aim is the lean lexer's 114 instructions a token, masks included.

### 4. The primitives

- Two-byte punctuation as one nibble product, a table pair (page 31).
- The classes from one table pair where they fit.

These are good for Metal too.

## What stays the same

- The block program is still the one format: digest, `validate`, `rir`, the interpreter,
  Cranelift, Metal. Regions are a new construct in it, printed and assembled as the others.
- Tier 2 reads the same records and mask words.
- Metal's tier 1 stays bit-parallel: rounds for regions, extracts for the records.
  It gains from steps 2 and 4, which take operations out of the program.

## Loops: settled

Page 12 decided the mask passes are loop-free and branch-free: no byte-by-byte work,
and no event-by-event work. On the CPU, this design adds two loops:
one over the block's regions (step 1) and one over its tokens (step 3).

Neither is over bytes. The records are already a loop over tokens. The region walk is
the lean lexer's form, which the request named. Metal keeps page 12's form throughout.

Settled (2026-11-13): acceptable where needed. CPU and GPU diverge as little as they can.

## The order, each step measured and tested

| Step | Takes out | Expected |
|---|---|---|
| 1. Regions, the walk on the CPU | the rounds, the backward pass, pass 0's stores | pass 0 and pass 1 (166M) go; pass 2 makes what pass 0 made for it |
| 2. Checks to the token | keyword spellings, JSON's and the numbers' byte checks | about 60 operations a block |
| 3. Records from the starts | the `lasts` extract and masks | a token toward 114 instructions |
| 4. Primitives | `two`'s compares, extra lookups | about 20 operations a block |

The target: tier 1 on the CPU within 1.5 times the lean lexer, about 200M instructions.

Step 1, measured (2026-11-13). Tier 1 on the CPU is one forward pass: 630.0M instructions
ahead of time, from 700.4M. Pass 0's rounds and stores went; its masks are still made, in
the one pass, since the later pass read most of them. Decision 0139 records how it was built.

Every step keeps:

- the tapes the engine's over the corpus, and the lean parser's byte for byte;
- the ahead-of-time tapes the JIT's;
- the interpreter's, on the `.rr` ladder and JSON;
- on Metal, its tapes the CPU's.

The ladder's shape check changes with it. On the CPU, tier 1 may loop over a block's regions
and its tokens; still never over bytes.
