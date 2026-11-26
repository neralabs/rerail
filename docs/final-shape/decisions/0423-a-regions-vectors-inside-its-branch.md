# 0423: A gated region's vectors are made inside its branch

Status: built (audited 2026-11-23). Decided (the user, 2026-11-18: "do this: what's left: 14-json is 130 instructions
over stage1.rir … the next part to trace"). Follows 0375, 0399 and 0421.

## Intro

**The question.** After 0421, `14-json`'s hot loop was 462 instructions and `stage1.rir`'s 332.
The listings differ in:

| | `stage1.rir` | `14-json` |
|---|---:|---:|
| `addp` (4 a mask) | 20 | 36 |
| `tbl` | 20 | 28 |
| `cmeq` | 0 | 12 |
| `and`, `orr`, `bic`, `lsr`, `mov` | 103 | 179 |

## What the trace found

`14-json` reduces nine masks a block, stage 1 five:

| Mask | Stage 1 | `14-json` |
|---|---|---|
| Quote, backslash | yes | yes (`cmeq`) |
| Control bytes | yes (class bit) | yes (`cmhs`), in the string-body region |
| Whitespace, operators | yes | yes |
| The `u` of an escape | | yes, in the string-body region |
| `d`, `8 9 a b`, `c`–`f` | | yes: **the surrogate check**, every block |

**The surrogate check** ("a lone high or low surrogate", 0399 kept it in tier 1) is a gated
region: its gate is a `\u` escape in the block, which most blocks have none of.
- Its word logic was behind the gate.
- Its three class masks were not. Each block made them before the branch: a second two-nibble
  table looked up per chunk, three tests, three reductions and three moves to words, about 60
  instructions every block.

**The cause** was the compiler's. The three classes are the region's `vectors`: values on the
vectors that only its members read, which the lane plan (`Region::vectors`) says are "made on
demand inside the branch, and never in a block it skips". Its members read them as words: they
are made from words (`Emit::as_words`), as the ASCII split decides for bit logic that reads a
value it makes. `demand_order` then took each class as an operand from outside the region and
made its mask before the region's branch.

**The rest of the difference:**
- the `u` test, 13 instructions, in the string-body region;
- the token kinds' word logic, about 46: the starts of string content, escapes, punctuation and
  words, and the skip, where simdjson makes one structural mask. Tier 2 reads these kinds. That
  is the grammar's split of the work between the tiers (0398, 0399), not the compiler's.

## The decision

**A region's own values on the vectors are made in its branch.**
- `demand_order` does not make a region's `vectors` before it.
- `region` makes those that a member reads as words at the top of the branch, before the
  members, and before an outlined region's call, which passes them in.

**The compiler refuses a function that breaks this** (`region_vectors_outside`, a shape check).
An instruction made for a region's vector value must be in a block that the region's branch
dominates. Cold blocks are left out: the edge part makes values without the regions. Each
instruction is marked with its value as its source location (`Emit::emit`). The skip block
and the join are made for no value, so `region` clears the location before them.

## Requirements, a test each

- R1. No region's value on the vectors is made outside its branch, in any grammar:
  - the shape check refuses it at compile time, so every ladder step's compile is a test;
  - undoing the `demand_order` part refuses `14-json` ("region 0's value %74 … made outside
    its branch").
- R2. Tier 1 equals the interpreter, surrogates included:
  - the compiler suite, the parser_tests guards (`json_tier1_vectors_equal_the_interpreter`
    has surrogate pairs at block edges), `block_tests` and `compile_tests`;
  - the rr_parser suite (48);
  - the rr JSON set: 789 files, every token the same, and `the_loops_errors_are_the_masks`;
  - `json_corpus_accuracy` on the 17 corpus files within the narrow widths: both forms accept
    each, with tree-sitter-json's nodes.

## What changed

| `14-json` | After 0421 | Now |
|---|---:|---:|
| Hot loop, code | 462 | 470 |
| The surrogate region's branch | (made every block) | 79, behind the gate |
| Tier 1, retired instructions, 17 corpus files | 21.2M | **20.0–20.1M (−5.4%)** |

The code is 8 longer: the region looks its bytes' nibbles up again inside the branch. A block
with no `\u` escape runs about 60 fewer instructions. Instructions were counted twice each way
(`json_optimizer_bench`, optimizer off) at a load of 40–50. Cycles at that load are noise and
are not given. `15-jsonc`'s loop is unchanged (632 / 7).

## Found on the way, not changed

`canada.json`, `citm_catalog.json`, `twitterescaped.json` and seven more corpus files are refused
by the ladder's forms at `ACCEPT`: their node counts or offsets are past tier 2's narrow widths.
That is page 37's "Still to do": the file parsed again at wider widths, in place of the
fallback. It predates today's changes: the same with 0420, 0421 and this change undone.
