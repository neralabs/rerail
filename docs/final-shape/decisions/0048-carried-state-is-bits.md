# 0048: Carried state is bits, packed

Status: accepted (user, 2026-11-07: "there should not be anything like the number of carried
words"; "I want this pinned down with TDD tests. The shape HAS to be correct"; "storing 1 bit
in 64 bits is 4x worse than the still very naive approach")

## Intro

A block program runs block by block. What one block passes to the next is its carried state.
simdjson's stage 1 carries three things: whether the next byte is escaped, whether it is in a
string, and whether the last byte was a scalar. Each is one bit of information.

Our JSON program carried 78 64-bit words. Each carried operation (`toggle`, `fill`,
`after_odd_run`) took a word for its one-bit carry. Each value that a `prev` looks back on took
another word, for the 1 to 5 bits that the look-back reads. That is 4,992 bits held for about
104 bits of state.

This record makes the state a count of bits, and pins every executor to it.

```
before: a word per carried op, a word per looked-back value   78 words  (JSON)
after:  one field per carry, its width in bits, packed        104 bits = 2 words
```

## Prior work consulted

- **ast-index-spike indexes:** the spike ran no block programs. Its rule of measuring against
  the real reference holds: simdjson's carries are the reference here.
- **Lessons ledger #28** (proxies are not the cost; only end-to-end CPU time counts). **Agrees.**
  The guard counts bits, but the change was kept only after paired CPU timings on the same
  load. Packing added 24 instructions, so the instruction count alone would have refused it.
- **Lessons ledger #10** (measure the code that runs). The guards read Cranelift's IR and machine
  code.
- **Doc 12 §2–§3** (block programs with "typed carries" on every backend). **Agrees**: the
  carries keep their types, and only their storage changes.

## Decision

1. **A program's state is bits** (`block_tests::state_bits`):
   - one bit per `toggle`, `fill` and `after_odd_run`;
   - for each value its `prev`s read, the top `k` bits of its previous word, `k` being the
     farthest look-back;
   - a `toggle`'s and a `fill`'s carry is their own top bit, so a look-back on one adds only
     `k - 1`.
2. **A look-back stays a carry of `k` bits on the CPU.** Rebuilding a byte class from the bytes
   `k` before costs a load, a lookup and a bitmask: 15 to 40 vector operations. Reading `k`
   packed bits costs two scalar operations, and writing them costs two. The GPU is different:
   a carry there is a look-back across lanes, so its generated code rebuilds local look-backs
   from the bytes (its windowed `prev`s).
3. **Cranelift packs the state.** Each carry is a field of exactly its width, placed first-fit
   in 64-bit loop-carried words. A field is ORed into its word as soon as its value is made, so
   no value stays live to the back edge for its carry.
4. **The guards:**
   - `stage1_loop_keeps_values_in_registers` and `json_loop_keeps_values_in_registers`: the
     body's carried parameters are ⌈state bits / 64⌉ plus one count per extract;
   - `json_state_is_bits`: JSON's state bits are a ratchet, lowered as the compiler stops
     restating state.

## Results

| JSON | before | after |
|---|---:|---:|
| carried words | 78 | 2 (104 bits) |
| loop instructions | 1915 | 1939 |
| loop stack loads and stores | 405 | 164 |

Stage 1 carries 1 word. Its loop still has no stack traffic.

Paired CPU timings, Cranelift over the 774 data-layer JSON files, best of 3, load 70–126:

| run | unpacked | packed |
|---|---:|---:|
| 1 | 141.9 ns/block | 113.8 ns/block |
| 2 | 140.8 | 138.2 |
| 3 | 138.1 | 134.8, 87.6 |

## Next, under the same guards

- **Less state.** About 18 bits of information cross a JSON block boundary:
  - 4 bits of lexical state: in a string, number or literal, or between tokens;
  - 14 bits of nesting: a depth up to 13, and whether each open bracket is '{'.

  The compiler restates some of it. A token's run over `starts(x) through x` is `x`. The
  literal mask is made twice. `aligned` repeats the escaped carry's parity. Two exclusive
  `fill`s make a three-way choice.
- **Metal holds the same bits.** Today each carried step has its own look-back state and each
  `prev` a three-word halo.
- **The remaining stack traffic** is live values in the body, not the state.
