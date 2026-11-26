# 0338: A string is a toggle, as simdjson's `prefix_xor`

Status: built (audited 2026-11-23). Decided (2026-11-17); landed. Follows 0337. Judged by the generated code's
structure against simdjson's stage 1, not by counts.

## Intro

**The problem.** simdjson's stage 1 finds the bytes inside strings in one
expression, with one carried bit:

```cpp
// simdjson, json_string_scanner::next (arm64)
const uint64_t quote = in.eq('"') & ~escaped;
const uint64_t in_string = prefix_xor(quote) ^ prev_in_string;
prev_in_string = uint64_t(static_cast<int64_t>(in_string) >> 63);
```

JSON's grammar writes its string as a region:

```text
regions hidden {
  str = q .. q
}
```

A `regions` group is a fixed point (its openers are taken only outside the
regions, which its carry `hidden` says), so the CPU made it a walk
(`block::Walk`). That was a function of its own, called once a block
through a stack slot, with a loop over each string's opener and closer
(`trailing_zeros`, the walk's state read and written in memory). Nothing in
simdjson's block has that shape.

**The decision.** A `regions` group of one kind whose opener is its closer,
one byte long, with no `else` and not `outside`, is lowered as a toggle,
with no fixed point and no walk. With `t = toggle(q)`:

```text
hidden = prev(t)          # the bytes after an opener, through its closer
o_str  = q & ~prev(t)     # an opener: a quote outside a string
f_str  = t                # from the opener up to the closer
```

By induction over the bytes, with the walk's fill a byte before equal to
`t` there: the walk's hidden byte is `prev(t)`, its opener `q & ~prev(t)`,
and its fill `open | (prev(fill) & ~q) = (q & ~t') | (t' & ~q) = t`. So
the two are the same values on every stream, the stream's cut included (`q`
is cleaned to the stream first, as the walk's inputs are).

The toggle is also tier 2's mode for the string's rows (`row_modes`), as a
`toggle` statement's is.

Regions of several kinds, or whose closer is not their opener (a comment's
`/*` and `*/`), keep the walk: their openers depend on the carry.

## The generated code

The full body's ASCII path now has simdjson's string scanner's form inline:
the quote and backslash masks, `escaped`, the prefix XOR, the carried
parity. There is no call, and no walk state in memory. The prefix XOR is six
shift-XOR rounds, since Cranelift has no carry-less multiply, where simdjson
has one `vmull_p64`.

## What is left, by structure

- **The classes' tables.** The masks' products are given table bits first
  come, first served (`Lowering::product`), so one class can straddle two
  tables: `ws | op | quote`, the word's complement, takes two table pairs
  a vector. simdjson tests its classes with one lookup each. `class_pack`
  already packs a body's classes that way, but only for the token loop's
  windows.
- **The prefix XOR** as a carry-less multiply.
- **Per block, the outputs go to memory** (an 88-byte record per block)
  and the token loop reads them back. The delayed outputs are fixed by
  reading the block before's word back, shifting it and storing it again.
  The token loop's state crosses each block through a 19-word stack array.

## Tests

- `rr_parser`'s `jsons_cpu_records_are_the_interpreters` (789 files),
  `jsons_tier2_split_is_tier1s` (386), `json_loop_against_step_14` (129 the
  same, 216 refused by both, none differ).
- rr unchanged (`tmp/verify.sh`; its AOT source the same).
