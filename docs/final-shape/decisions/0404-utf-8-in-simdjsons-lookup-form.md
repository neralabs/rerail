# 0404: UTF-8 in simdjson's lookup form

Status: built (audited 2026-11-23). Decided (the user, 2026-11-18: "go ahead with this", the three gaps of 0402 to
simdjson's check). Follows 0402 and 0403.

## Intro

**The problem.** After 0402, the UTF-8 check's steady chunk was 29 instructions, against about
17 for simdjson's `lookup4` algorithm. 0402 had found three gaps:

- **The two-byte rules.** simdjson's one lookup holds all of them. Ours held four. "A
  continuation is needed after a lead byte" and "the bytes that never start a sequence" were
  separate tests.
- **The current byte.** It was looked up in two tables, because its classes need more than 8
  bits.
- **Lags 2 and 3.** They were tests of class bits. simdjson uses two saturating subtracts of the
  raw bytes.

## The decision

**The grammars state the check in simdjson's form**: `14-json`, `15-jsonc` and
`json_parser/json.bitlex`.

```
error "invalid UTF-8" when never
    | (prev(lead, 1) & not_cont) | (cont & ~prev(high, 1))
    | (prev(e0, 1) & below_a0) | (prev(ed, 1) & from_a0)
    | (prev(f0, 1) & below_90) | (prev(f4, 1) & from_90)
    | ((prev(cont, 1) & cont) ^ (prev(third, 2) | prev(fourth, 3)))
```

The rules, in simdjson's names:

- `TOO_SHORT`: a lead byte before a byte that is not a continuation.
- `TOO_LONG`: a continuation after a byte that is neither a lead byte nor a continuation, or
  first in the file.
- The overlong, surrogate and too-large pairs, as before.
- `TWO_CONTS` against `must_be_2_3_continuation`: two continuations in a row exactly where a
  byte 2 back is at least 0xe0, or a byte 3 back at least 0xf0.

The plain form said where a continuation is needed, from the lead bytes 1, 2 and 3 back, and
XORed that with the continuations. The two forms refuse the same files (the test below).

**The pairs are general** (`native_cpu_pairs`, after 0402):

- **Every rule a bit of two new tables.** `P` holds the byte before's classes and `Q` this
  byte's, eight rules at most. All the rules of the program share the two tables, so their
  lookups are made once a chunk.
- **Rules of one OR tree are one pair**, as in 0402. The six here are one pair; `TWO_CONTS`,
  under the XOR, is a pair of its own on the same tables.
- **A rule is also `and_not(C, prev(L, 1))`**: the byte before not in `L`. Its byte-before set
  is the complement of `L`, which holds 0x00. So it holds at the stream's first byte, where the
  pair reads the byte 0x00 and the `prev` reads no bit.

**A table of high nibbles is one lookup** (`Lanes::lookup`). Every rule's set of this byte here
is a set of high nibbles, so `Q`'s low half is all ones, and its lookup is one `tbl` of the
high nibbles, as simdjson's `byte_2_high`.

**Lags 2 and 3 are simdjson's tests**, `prev2 ≥ 0xe0` and `prev3 ≥ 0xf0`: classes of one range
each, read with `ext`s of the chunk before (0403).

## Requirements, a test each

- R1. The simdjson form refuses exactly what the plain form does
  (`tier2_tests::utf8_in_simdjsons_form_refuses_as_the_plain_form`):
  - every string of up to 3 bytes from the classes' edges, and of 4 from fewer;
  - each at the file's start and inside a string.
- R2. The six rules are one pair, `TWO_CONTS` another, on one pair of tables, and this byte's
  table is of high nibbles (`utf8s_two_byte_rules_are_one_pair_lookup`).
- R3. The CPU's tier 1 agrees with the interpreter on lead bytes before continuations and
  others, an extra continuation, a lead byte after a lead byte, and a continuation first in the
  file, across the edges (the same test). A wrong rewrite fails it: the `and_not` rule without
  its complement.
- R4. The compiler suite (107), the parser_tests guards, and the rr JSON set: 129 parse the
  same, 216 are refused by both, 0 differ.

## What changed

| | Hot loop | Stack accesses | UTF-8 stretch | Steady chunk |
|---|---:|---:|---:|---:|
| `14-json` after 0402 | 574 | 1 | 153 | 29 |
| `14-json` now (0403, 0404) | **549** | **1** | **128** | **25** |
| `15-jsonc` after 0402 | 784 | 11 | | |
| `15-jsonc` now | **735** | **8** | | |
| simdjson | | | 72 | about 17 |

## What is left

- **The bytes that never start a sequence**: `c0`, `c1` and `f5` to `ff`. They are two
  products, which no single bit of a pair table holds, so they keep a class test of this byte.
  That costs a second lookup of the current byte, about 5 instructions a chunk, which the
  lag 2 and 3 tests then share.
- **Rules as lanes.** A rule's AND is tested (`cmtst`) to a 0 or 1 lane. simdjson keeps the
  ANDed bytes, and any nonzero byte is the error.
- **The carry** of the previous block's last chunk (0403).
