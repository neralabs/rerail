# 0417: UTF-8 and errors in simdjson's registers

Status: built (audited 2026-11-23). Decided (the user, 2026-11-18: "please address these", the two items 0406 left).
Follows 0405 and 0406. First numbered 0411, which Final-shape-quality's record has (commit 44efa560b names it so).

## Intro

**The problem.** 0406 left two things:

- **JSONC's stack accesses: 11**, two more than after 0405. Its loop is short of integer
  registers.
- **The UTF-8 stretch: 76 instructions against simdjson's 72.** 0406 put the gap down to the
  ASCII way's `mov` of the error vector and to a chain of ORs.

**The ORs were not the gap.** The listing has four ORs a block, one for each chunk's error
bytes, as simdjson's four `error |=`. What differed:

- **The ASCII way copied the vector.** Both ways of the ASCII branch join with the error
  vector as a parameter. The fast way passed the vector it took, and the register allocator
  copied it there.
- **Lags 2 and 3 were compares then `ext`s.** The grammar's `prev(ge_e0, 2)` compared each
  chunk with `0xe0`, then took the bytes 2 back with an `ext` of the compares. It also compared
  chunk 0, the carried one, again each block. simdjson takes the bytes 2 back first
  (`prev<2>(input)`), then compares.

**JSONC's spills** came from words made early and read only at the block's end:

- a word leaf of the flag (the unescaped-control check, made behind the ASCII branch), held
  until the flag's root ORed it;
- the edge part's totals, which 0406 kept out of the running word, held to the end for the
  edge join.

## The decision

**1. A value read only by `prev`s of one lag is made from the bytes that far back**
(`Lanes::only_prev`, `Prev`). `prev(x, k)`, where nothing else reads `x`, is `x` made from the
`ext` of the input chunks `k` back, as simdjson does it. The chunks' compares then read the
`ext`s, and chunk 0 is no longer compared.

**2. The error vector passes no join.** In a full block, the vector the block took goes on as
it is (`Emit::err`). An edge block is entered from the loop's head, so its vector is zero, and
its edge part passes zero on: the flag's totals are made exactly there, as words. The ASCII
way's `mov` is gone.

**3. A flag's word leaf is ORed into the running word where it is made** (`Emit::sink`), as
simdjson ORs each error word into its register. This applies where the flag's roots are ORed
into the running word. The root then ORs only what is left.

**4. Every rowless total goes into the running word where it is made, the edge part's
too** (`Sinks::ors`). 0406 kept the edge part's totals out, since an OR can't be taken back.
Now an edge block makes its running word anew. It starts from the word the block took (the
loop's parameters, in the edge part's slot) and ORs in each such total, made exactly by the
edge part and cut to the stream. The join takes the full body's running word or that one. No
total's word is held to the block's end.

**Rejected on the way: the edge block ORing only the totals it makes again into the word it
took.** The others, which the full body made rightly, were lost: nine ladder steps differed
from the interpreter.

**Left: the constants** (done in 0418). The stretch builds six constants each block: `movi` of `0xe0`,
`0xf0` and `0x80`, and three table loads. simdjson keeps them in registers across its loop.
Here Cranelift builds them where they are used: the mid-end's code motion is off (0069),
because it hoists every vector constant and spills them. Holding just the UTF-8 check's six
would raise the vector pressure in the rest of the loop (its peak is 32, at the number and
string checks).

## Requirements, a test each

- R1. Lags 2 and 3 are made from the bytes 2 and 3 back: one `ext` of each a chunk, read first
  by a compare (`the_hot_loop_keeps_simdjsons_utf8_and_error_registers`). Turning `only_prev`
  off fails it.
- R2. The hot loop copies no vector but the carried chunk (the same test). 0406's ASCII way
  fails it.
- R3. A stack ratchet: at most 1 for `14-json` and 9 for `15-jsonc`, as that test's loop counts
  them (the same test). `json_shapes`'s loop gives 1 and 7.
- R4. Tier 1 agrees with the interpreter:
  - on every ladder step (`every_ladder_step_compiles_here_and_equals_the_interpreters`,
    `every_step_parses_as_the_reference`, which caught the rejected edge form);
  - on UTF-8 at block edges (`the_hot_loop_carries_the_bytes_before_its_block`);
  - on the compiler suite, the parser_tests guards, the rr_parser suite (48) and the rr JSON
    set (789 files, every token the same, and `the_loops_errors_are_the_masks`).

## What changed

| | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| `14-json` after 0406 | 485 | 1 | 76 |
| `14-json` now | **481** | **1** | **74** |
| `15-jsonc` after 0406 | 667 | 11 | |
| `15-jsonc` now | **651** | **7** | |
| simdjson | | | 72 |

The stretch is now simdjson's form chunk by chunk: 16 instructions a chunk, the same as
simdjson's. Our compares are `cmhs`, where simdjson uses saturating subtracts, at the same
count. That is 64 of the 74. The other 10 are the four ORs and the six constants built in the
loop. simdjson's other 8 are its ORs and its incomplete-sequence check (`ldr`, `cmhi`, `mov`),
which is folded away here in a full block. The constants are the gap.
