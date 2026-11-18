# 0403: A `prev` reads the chunk before, chunk 0 included

Status: decided (the user, 2026-11-18, after a spike measured this step at ±1% and it was taken
out: "profitable is not 'less cycles'. It's 'allows us to move closer to the correct structure'
too … How can we get to SIMDJSON performance if we can't get to the same structure?"). Follows
0402.

## Intro

**The problem.** simdjson's stage 1 makes every `prev<N>` the same way: an `ext` of the chunk
before and this chunk. For the block's first chunk, the chunk before is the previous block's
last, which it carries in a register (`prev_input_block`).

Tier 1 made chunks 2 to 4 that way. Chunk 1 was different: each `prev(a, k)` made `a` again,
from its own load at −k and its own lookups. With three lags, that is three loads and three of
each lookup.

## The decision

**Chunk 1's chunk before is chunk 0: the 16 bytes before the block** (`native_cpu_lanes`,
`Lanes::make` for `Prev`).

- `a` is made over chunk 0 once, as any chunk is, with one load at −16, and every `prev` of
  chunk 1 is then an `ext`, as in chunks 2 to 4. The bytes before a file are zeros, read as no
  bits (the same cut as before, `from`).
- Only where `a` reads its own bytes only: no `prev`, `pair` or `plane_before` inside it
  (`reads_here`). Made over chunk 0, such a value would need bytes before the window
  (`MAX_LAG`), so it keeps the lags.
- Nothing is carried, so the gated regions can still skip a `prev`.

**The next step is simdjson's carry.** Chunk 0 is the previous block's chunk 4. Handed on in a
register, it would not be made at all.

## What changed

| Hot loop (instructions / stack accesses) | `14-json` | `15-jsonc` |
|---|---:|---:|
| Before | 574 / 1 | 784 / 11 |
| Chunk 0 | 568 / 1 | 792 / 13 |

On its own it is a gain only where the `prev`s share a table's lookup: a `prev` of one byte's
compare costs a load and a compare at −k, about what an `ext` costs. It is kept as the
structure the carry needs.

## Tests

- The compiler suite: the ladder's every step, compiled against the interpreter, JSON's
  vectors and gated regions included.
- The parser_tests guards, and the rr JSON set.
