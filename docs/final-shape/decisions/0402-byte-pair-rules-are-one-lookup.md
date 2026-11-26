# 0402: Byte-pair rules are one lookup

Status: built (audited 2026-11-23). Decided (the user, 2026-11-18: "please work through these", the second being page
39's item 2, "the real gap to simdjson's UTF-8 check"). Follows 0389 and 0411.

## Intro

**The problem.** Page 39 measured JSON's UTF-8 check against simdjson's: about 195
instructions a block in `14-json`'s hot loop, against 72. Item 2 put the gap down to the lane
plan making the check's previous-byte values as 64-bit words.

**That premise was wrong.** It came from the program as Bitlex writes it, where every class
holding a byte of the byte order mark is cut by `~prefix`. The prefix is a word, so the whole
check fell to words. The CPU compiles the program the host strips (0389), where the prefix is a
constant zero. There the whole invalid-UTF-8 check is on the vectors already, its `prev`s as
`ext`s, and only its error is made a mask.

**The real gap is the rules' form.** The grammar states four two-byte rules apart:

```
(prev(e0, 1) & below_a0) | (prev(ed, 1) & from_a0)
  | (prev(f0, 1) & below_90) | (prev(f4, 1) & from_90)
```

Each one costs, per 16-byte chunk:

- a compare of this byte, then on the chunk before it;
- an `ext`;
- a test of the class after it;
- an `and`, and an `or` into the rest.

That is about 19 instructions a chunk for the four. simdjson makes all of its two-byte rules
one lookup (`check_special_cases`): a table of the byte before's nibbles and one of this byte's
high nibble, ANDed, each rule a bit.

## The decision

**Byte-pair rules ORed together are one `pair`** (`native_cpu_pairs::fused`, on the CPU's
program after the host strips the prefix). `BlockInst::Pair` was there already, in the
interpreter, the prover, the CPU's lanes and Metal, but only a hand-written program used it.

- **A rule** is `and(prev(L, 1), C)`:
  - `L` is one byte, or a class that is one product (high nibbles by low nibbles), without
    0x00. Before a file a `prev` reads no bit, while a pair reads the byte 0x00.
  - `C` is one bit of a table. That table is the pair's, so its lookup is the one `C` had.
  - An `and_not` of zero is looked through: a class the prefix cut, after stripping.
- **The rules of one OR tree** merge. Every inner `or` must be read by the tree only, and every
  rule by its `or` only; none may be an output or a member of a fixed point.
  - The first rule becomes `Pair { before: P, table: T, bits }`, where the new table `P` holds
    each `L` at its rule's bit.
  - The others become zero, so the tree's value is the same.
- **Two rules at least.** One rule alone would cost a lookup more than it saves.

**The interpreter is the oracle.** It runs the program as Bitlex wrote it. The rewrite is the
CPU's, structural, keyed on no name, and the tests compare the two.

**Rejected: page 39's item 2 as written (errors as flags).** It is built and kept outside the
tree (`tmp/item2-totals-as-one-flag-v2.patch`, now with word leaves, as simdjson's
unescaped-character error is a word). It still fires on no grammar here:

- a flag needs totals that no one reads by their rows (0347's `rowless_totals`);
- those exist only where the CPU's token loop runs, and `14-json`'s tier 1 has none.

And it would save little. The error's reduction is 12 instructions of the loop. The open-sequence
check that page 39 counted is folded away in a full block already.

## Requirements, a test each

- R1. `14-json`'s four rules are one pair of four bits
  (`tier2_tests::utf8s_two_byte_rules_are_one_pair_lookup`).
- R2. The CPU's tier 1 agrees with the interpreter's (the same test). Its documents put every
  lead byte near the rules (`c2` to `f5`) before continuation bytes and others, complete or cut
  short, at offsets across the 16-byte and 64-byte edges. Both accepted and refused files are
  among them. A wrong table, any low nibble for `L`, fails it.
- R3. The rest of the CPU equals the interpreter:
  - the compiler suite: the ladder's every step, JSON's vectors and gated regions;
  - the parser_tests guards;
  - the rr JSON set.

## What changed

| | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| `14-json` after 0411 | 616 | 5 | 195 |
| `14-json` now | **574** | **1** | **153** |
| `15-jsonc` after 0412 | 814 | 27 | |
| `15-jsonc` now | **784** | **11** | |

simdjson's UTF-8 check is 72 instructions a block.

**What is left of the gap: the block's first chunk.** Its previous bytes are the 16 before the
block. The loop loads them again at −1, −2 and −3, and looks each load up again: about 25
instructions a block. simdjson carries the previous block's input vector in a register and
takes `prev1` from it with one `ext`.
