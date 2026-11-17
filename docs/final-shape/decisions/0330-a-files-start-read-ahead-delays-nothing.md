# 0330: A file's start read ahead delays nothing

Status: decided (2026-11-16); landed on the CPU. Metal refuses the new
operation for now. Part of 0177's token loop, after 0323.

## Intro

**The problem.** JSON's tier 1 on the CPU took 42.8 instructions a byte
over the corpus. simdjson's stage 1, over the same 12 files, takes 2.9.
Tier 1 is one pass: its block masks and its token loop are one function.

Five inputs part the two, each 16 files of about 60 KB.
`rr_parser`'s `json_tier1_by_pass` measures them, least of 5 rounds:

| Input | Tier 1, instructions a byte | simdjson stage 1 |
|---|---|---|
| Strings of 5,000 ASCII bytes (a token every 5,000 bytes) | 23.3 | 2.15 |
| The same in two-byte UTF-8 | 28.2 | 3.26 |
| White space | 28.1 | 2.15 |
| Numbers (`1234,`: a token every 2.5 bytes) | 62.1 | 4.51 |
| Short strings (`"ab",`) | 83.7 | 4.51 |

Where there are almost no tokens, the block masks alone cost about 1,490
instructions per 64 bytes. simdjson spends about 140.

Taking constructs out of `json.bitlex` one at a time showed where those
instructions go:

| Variant | Corpus | Long ASCII strings | White space |
|---|---|---|---|
| As it was | 42.8 | 23.3 | 28.1 |
| No error checks | 42.3 | 23.0 | 28.0 |
| No errors, no byte order mark | 27.5 | 14.8 | 15.0 |

The errors are almost free: their gated regions skip them. The byte order
mark costs a third of tier 1. It is a test of the file's first three
bytes:

```text
mask bom        = ~prev(~empty) & ef & next(bb) & next(bf, 2)
mask bom_bytes  = bom | prev(bom, 1) | prev(bom, 2)
mask word       = code & wordbyte & ~bom_bytes
skip ws | bom_bytes
```

**The cause.** Bitlex lowers `next(x, k)` as a delay (`bitlex_masks`, "Delays: `next`
without lookahead"). A value with a `next` is computed `k` bytes late, and
whatever it is combined with is brought to the same delay by `prev`s. So
`word` and `skip`, and with them every token output, ran two bytes late.
Each of their operands was shifted, with a carried field each, and the
token loop read delayed windows. All of that was for one bit of block 0.

A lean lexer, and simdjson, look at the file's first bytes once, before
the loop.

**The decision.** A value whose bits all lie in a file's first bytes is
brought to an earlier delay by an in-word shift, with no carry: a new
block operation, `early %a, k` (`BlockOp::Early`). Its bit `p` is `a`'s bit
`p + k`, within the word. The shift is exact because `a` has no bits past
the first `EARLY_BITS` (16) positions, which is Metal's narrowest lane. So
no bit crosses a word, and no word but the file's first has any.

`bitlex_masks::first_bits` finds these values from the operations. It
tracks three bounds for each value:
- the position from which it has no bits;
- a run of positions where it is all ones (`within` up to the end, a class
  of every byte past it);
- the position past the end from which it has no bits.

A file's first byte, `~prev(~empty)`, is `within & ~prev(within)`, so it has
no bits from position 1 on. An `and` with it is bounded, and so are a
`prev` of a bounded value and an `or` of two.

`Lowering::two`, the step every two-operand operation takes, now moves a
bounded operand down to the other's delay (`early_to`) instead of moving
the other up. A `prev` of at least `k` folds into an `early` of `k`. The
grammar is unchanged: `bom_bytes` is at delay 0, and so are `word`, `skip`
and the token outputs.

## Each executor

- **The interpreter** (`block::run`) shifts each word by `k`.
- **The CPU's block masks.** An `ushr`. In a full block, `full_words`
  knows the value is zero, so the steady loop never makes it.
- **The CPU's bytewise windows.** The token loop computes the skip and
  run words from the bytes (`native_cpu_bytes`). An `early` is exact in
  any window: it is `a`'s window `k` bytes later. A window that starts at
  `EARLY_BITS` or later has none of its bits, so `mask_at` makes it behind
  a cold branch, with its own windows, which nothing outside reads.
  `scalar` cuts it the same way. Each byte analysis follows it as a
  `prev` mirrored: `needs`, `inst_byte_at`, `needs_by_offset`, `bytes_at`,
  `fold`, `on_run_only`. `pad_reach` counts its reach as it does a delay's.
- **Metal** refuses a program with an `early`
  (`Error::UnsupportedBackend`), at `run_batch` and in the serial runs, as
  it refuses a fixed point. It is to be generated as a shift of a file's
  first lane.
- **The prover and the local cones** (`bitlex_prove`, `token_local`) take
  an `early` as free, as they take a carried value: it reads bytes no
  window holds.

## Measured

`json_tier1_by_pass`, least of 5 rounds, instructions a byte:

| Input | Before | After | simdjson |
|---|---|---|---|
| Corpus (12 files) | 42.8 | 36.5 | 2.9 |
| Long ASCII strings | 23.3 | 19.0 | 2.15 |
| Long UTF-8 strings | 28.2 | 23.3 | 3.26 |
| White space | 28.1 | 22.5 | 2.15 |
| Numbers | 62.1 | 56.4 | 4.51 |
| Short strings | 83.7 | 77.9 | 4.51 |

## What is left

**The skip's form in the token loop.** With the same grammar but `skip ws`
alone, the corpus takes 28.6 and white space 15.4. The remaining 8
instructions a byte come from the skip holding a term of the file's start.

The token loop picks the lean lexer's form (`lean_starts`, the refill that
skips) only where the skip is "a byte class alone" (`needs` at offset 0).
At run time the loop computes the skip from its whole expression, so the
form would be right. But the check is by byte sets, and `bom_bytes` puts
`ef`, `bb` and `bf` at offsets 0 to 2.

The right form is the lean lexer's: the start of the file is a prologue,
done once, and the loop's skip is the class. That is the next step.

**The block masks**, about 15 instructions a byte on ASCII without a BOM,
against simdjson's 2.15. The full ASCII body makes 22 movemasks a block;
simdjson's makes four. Among the causes:
- each class group is its own table pair and test, where simdjson does one
  lookup for white space and structurals;
- a `prev` on the vectors is an `ext` and a movemask of its own;
- the string region is a call a block (`f2`), through the stack.

**The token loop**, about 80 instructions a number token and 135 a short
string, after the masks.

## Tests

- `compiler`'s `a_files_start_read_ahead_delays_nothing`: a test of a
  file's start that looks ahead is at delay 0 through an `early`. The CPU
  equals the interpreter, and both equal a reference made by hand. Files of
  length 0 to 200 over block edges, and a batch.
- `rr_parser`'s `jsons_cpu_records_are_the_interpreters` adds byte order
  marks, whole, cut, doubled and not at the start, before short values and
  values over a block's edge: 789 files, all the same.
- `jsons_tier2_split_is_tier1s` and `json_loop_against_step_14`: 129
  parsed the same, 216 refused by both, none differ.
- rr unchanged (`tmp/verify.sh`).
