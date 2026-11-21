# 0405: UTF-8 at simdjson's structure

Status: decided (the user, 2026-11-18: "please do all of these", the three gaps 0404 left).
Follows 0403 and 0404.

## Intro

**The problem.** After 0404, three parts of JSON's UTF-8 check still differed from simdjson's
`lookup4` algorithm:

- **The bytes that never start a sequence** (`c0`, `c1`, `f5` to `ff`) were a class test of
  their own, on a second lookup of the current byte.
- **Each rule was tested** to a lane of zeros or ones. simdjson keeps the ANDed bytes, and any
  nonzero byte is the error.
- **The bytes before the block** were loaded again each block. simdjson carries the block
  before's last chunk in a register.

## The decision

**1. The bytes that never start a sequence are pair rules**, as in simdjson's tables:

```
error "invalid UTF-8" when (prev(c0c1, 1) & cont) | (prev(f5ff, 1) & cont)
                         | (prev(lead, 1) & not_cont) | (cont & ~prev(high, 1))
                         | ...
```

- `c0` or `c1` before a continuation is simdjson's `OVERLONG_2`, and `f5` to `ff` before one is
  too large.
- One of them before a byte that is not a continuation is "too short": every lead byte is.
- So the end-of-file check is `lead & last_byte`.

That makes eight rules, so `native_cpu_pairs` **merges their boxes** (a box is the byte
before's high and low nibbles, by this byte's):

- two boxes that differ in one of the four sets are one, their union in it;
- a box is split where a part of it is one with a second box and the rest with a third.

So `f5..ff × 8x..bx` becomes `f4..ff × 9x..bx` (with `f4`'s too-large rule) and
`f0, f5..ff × 8x` (with `f0`'s overlong rule). These are simdjson's `TOO_LARGE` and
`TOO_LARGE_1000 | OVERLONG_4` bits. The eight rules take seven bits, and `TWO_CONTS` the
eighth.

**A range that runs to `0xff` is one compare** (`Lanes::range`): `prev2 ≥ 0xe0` is one `cmhs`,
as simdjson's saturating subtract. Lags 2 and 3 need no lookup at all.

**2. Rules as bytes** (`Lanes::as_bytes`). A tree of `or`s and `xor`s over pair rules is made
on the bytes, then tested once:

- a pair is its tables' AND, with its bits;
- an `xor` of a one-bit pair with any lane flips that bit where the lane is set: an `and` and
  an `eor`, the rest of the AND as it was;
- an `or` of two with no bit in common, from the same AND, costs nothing.

So `14-json`'s check is `(P & Q) ^ ((prev2 ≥ e0 | prev3 ≥ f0) & 0x80)`, and one `cmtst` a
chunk: simdjson's `special_cases ^ must_be_2_3_continuation`. The pairs of one pair of tables
share their AND (`Lanes::pair_and`).

**3. The carry** (`native_cpu_block::forward`). The hot body takes the bytes before its block
as a vector parameter:

- on the back edge, its own last chunk;
- from the loop's head, a load (the window holds the bytes before every block).

A `prev` byte inside two chunks in registers is an `ext` of them (`Lanes::bytes`), so the first
chunk's byte before is an `ext` of the carried chunk. Chunk 0 is one key with chunk 1's 16
bytes back, so it is read as the carried value everywhere.

**Not carried round the token loop.** Where the CPU's token loop runs after each block (0372),
its calls would spill the vector. There the head loads it.

## Requirements, a test each

- R1. The new rules refuse what the plain form does
  (`utf8_in_simdjsons_form_refuses_as_the_plain_form`, the plain form now with its own `never`).
- R2. Seven boxes of one pair, `TWO_CONTS` another, on shared tables
  (`utf8s_two_byte_rules_are_one_pair_lookup`, now `[7, 1]`).
- R3. `14-json`'s and `15-jsonc`'s hot loops load no byte before their block, and tier 1 agrees
  with the interpreter on UTF-8 placed across the edges of full blocks
  (`the_hot_loop_carries_the_bytes_before_its_block`). A carry of the stale chunk fails it at
  `c3 a9` before the third block.
- R4. The compiler suite (108), the parser_tests guards, and the rr JSON set: 129 parse the same,
  216 are refused by both, 0 differ.

## What changed

| `14-json` | Hot loop | Stack accesses | UTF-8 stretch | Steady chunk |
|---|---:|---:|---:|---:|
| After 0404 | 549 | 1 | 128 | 25 |
| The bytes that never start a sequence as pairs, ranges to `0xff` | 512 | 1 | | |
| The carry | 511 | 1 | | |
| Rules as bytes | **507** | **1** | **85** | **17** |
| simdjson | | | 72 | 17 |

`15-jsonc`: 735 / 8 → **693 / 9**.

The steady chunk is now simdjson's 17 instructions:

- an `ext` for the byte before;
- the pair's two lookups and their AND;
- two `cmhs` and `ext`s for lags 2 and 3;
- `orr`, `and`, `eor`, and one `cmtst`.

What is left of the stretch is the mask (`and`s and `addp`s to a word, about 9). simdjson ORs
its error bytes into one vector and tests it once a file. The error is a row here (0402: a flag
needs totals no one reads by rows).
