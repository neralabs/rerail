# 0349: The token loop runs after a run of blocks

Status: built (audited 2026-11-23). Decided (the user, 2026-11-17: "go ahead with 2 then 1", item 2 being the block
cost on plain bytes). Amends 0177's "a part after each block".

**Replaced by 0372** (2026-11-17): the pass is one loop again, a block's body then its tokens,
its words handed on as values.

## Intro

**The problem.** On white space, JSON's tier 1 retired 7.5 instructions a byte, about 480 a
64-byte block. simdjson's stage 1 retires 2.15 (138 a block). Reading the JIT's machine code
(`json_tier1_by_pass`, `RERAIL_JSON_ASM`), about half of the 480 was not the block's work but
the switch between the block body and the token loop, which 0177 ran after every block:

- the body's loop values to the stack before the token loop, and back after it (10 stores, 10
  loads);
- at the token loop's entry: the file's 13 columns from the record slot, the per-file
  constants made and stored again, the values derived from the file's length made again;
- the token loop's exit.

simdjson's stage 1 has no such switch: its indexer writes in the same loop. A lean lexer has
one loop. Ours pays the switch every 64 bytes, whether a token is there or not.

## The decision

The hot loop goes to the token loop after a run of `TOKEN_RUN` full blocks (16, a kilobyte), at
the last full block, and from every block the general body makes (the first, the last). Between
those points, the body loops on itself, as it does where no token loop runs.

The token loop already runs over every block before `ready` (the blocks that are final). It
now finds up to 16 there, where it found one. Its refill reads the bytes itself, which are
still in L1. The rows it might read are final, as before.

## Measured

`json_tier1_by_pass`, least of 9 rounds, M5 Max at load 40–60, instructions a byte (cycles):

| Input | Before | After | simdjson |
|---|---|---|---|
| corpus | 23.9 (5.5) | 21.5 (4.9) | 2.9 |
| white space | 7.48 (1.39) | 4.78 (0.64) | 2.15 |
| long ASCII strings | 7.49 (0.83) | 4.78 (0.59) | 2.15 |
| long UTF-8 strings | 11.9 (2.6) | 9.2 (1.5) | 3.26 |
| numbers | 47.6 (9.0) | 44.9 (9.5) | 4.5 |
| short strings | 54.2 (12.4) | 51.4 (11.5) | 4.5 |

## Left of item 2, read from the same code

- **The number check is tier 1's masks.** The grammar gives it to tier 2 ("tier 2 over every
  token (kinds, keywords, the number check …)"), and 0071 counts a `fill` in tier 1 as a
  violation. Without it (a copy of the grammar with no `check number`) white space took 1.0
  instruction a byte less and numbers 7 less. Its states are zero on the byte before a token,
  and each self-loop's fill restarts at the token's first byte, so a window from the token's
  first byte gives the same bits.
- **The refill classifies the bytes again.** The body loads the block and looks its bytes
  up; the token loop's refill loads them again and looks them up in a table of its own.
- **The vector constants are reloaded from the stack.** The bit weights and the nibble mask
  are live across the token loop's calls, so they are spilled, and reloaded at each use (about
  20 loads a block).

## Tests

- `rr_parser`: `jsons_cpu_records_are_the_interpreters` (789 files, 1,165,507 tokens the same),
  `jsons_tier2_split_is_tier1s`, `rrs_records_are_rows_on_the_cpu`,
  `the_loop_reads_no_record_back`, `json_loop_against_step_14` (129 the same, 216 refused by
  both, none differ).
- `json_tier1_by_pass` gains `RERAIL_JSON_ASM=<path>`: tier 1's machine code.
