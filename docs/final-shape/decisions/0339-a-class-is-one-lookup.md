# 0339: A class is one lookup

Status: built (audited 2026-11-23). Decided (2026-11-17); landed. Follows 0338. Judged by the generated code's
structure against simdjson's stage 1.

## Intro

**The problem.** simdjson classifies a block's bytes with one lookup per
class: white space by one table, the structural bytes by another.

```cpp
// simdjson, json_character_block::classify (arm64), per 16 bytes
match_op = vceqq_u8(vqtbl1q_u8(op_table, vshrq_n_u8(vaddq_u8(d, vdupq_n_u8(3)), 4)), d);
match_ws = vqtbx1q_u8(vceqq_u8(d, vdupq_n_u8(' ')), ws_table, d);
```

Bitlex tests a class with a pair of nibble tables: each bit of the pair is a
product (a set of high nibbles times a set of low nibbles), and a class is
some bits, read by one `vtst`. But the lowering gave products their bits
first come, first served (`Lowering::product`: product `n` at table
`n / 8`, bit `n % 8`), in the order the classes are declared. A class made
later, a union, reused its products wherever they were. So one class could
straddle two tables and take two table pairs each 16 bytes. JSON's
`ws | op | quote`, the complement of a word's byte and the only class its
ASCII body reads unconditionally, took bits `0xc0` of table 4 and `0x07`
of table 5.

`class_pack` already packs a body's classes as simdjson does, each in one
table, but only the token loop's windows use it.

**The decision.** A class's products are placed together (`Lowering::place`):
in a table that holds them all, else in the one with room for the fewest
missing, else in a new table. A product two classes share in one table
serves both, and a product may also be in two tables. A class of eight
products or fewer is then one `class` operation, one lookup. More than
eight: eight to a table, as before.

## The generated code

JSON's ASCII body now reads one table pair for `ws | op | quote`, and has
simdjson's classification shape:

| simdjson stage 1 | Tier 1's ASCII body |
|---|---|
| `eq('\\')`, `eq('"')`: two compares, two movemasks | the same |
| `escaped`, `prefix_xor`, carried bits | the same (0338) |
| `lteq(0x1F)`: one movemask | the same |
| op: one table; ws: one table; two movemasks | `ws \| op \| quote`: one table pair, one movemask |
| UTF-8: an ASCII test, the check behind it | the same (0139) |

Every other class of JSON's is one `class` operation too. JSON has seven
tables, not six: the union has a table of its own. rr's generated Rust
changes the same way.

## What is left, by structure

- The prefix XOR is six shift-XOR rounds, not a carry-less multiply
  (Cranelift has none).
- Per block, tier 1 stores its outputs to a record in memory, which the
  token loop reads back; a delayed output is fixed by reading the block
  before's word back, shifting it and storing it again. The token loop's
  state crosses each block through a stack array. This is the boundary
  0177 drew between the masks and the token loop.

## Tests

- `rr_parser`'s `jsons_cpu_records_are_the_interpreters` (789 files),
  `jsons_tier2_split_is_tier1s` (386), `json_loop_against_step_14` (129 the
  same, 216 refused by both, none differ).
- rr: `tmp/verify.sh`, all named tests; the AOT's tapes are the JIT's.
