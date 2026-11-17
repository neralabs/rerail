# 0350: The body's constants are made at its head

Status: decided (the user, 2026-11-17, item 2: "remove whatever simdjson doesn't compute").
Follows 0349. Keeps 0069.

## Intro

**The problem.** Tier 1 runs without Cranelift's optimizer (0069), so that the optimizer's code
motion can't hoist the loop's vector constants out of the loop. But the block loop took two
of them, the bit weights a bitmask reads and the nibble mask a lookup reads, from
`Consts::make` at the function's entry. So they were hoisted anyway, by the generator. Live
across the token loop and its calls, they were spilled at the entry and reloaded at each use:
129 vector loads from the stack in the JIT's code for JSON.

simdjson's stage 1 makes them in the loop, and clang keeps them in registers. Its loop makes no
call.

## The decision

The hot body makes its constants at its head, each block (`body_consts`): one `movi` for the
nibble mask, and the weights' word made and duplicated (`movz`, three `movk`, `dup`). They live
within the block, where they are used. The general body and the token loop keep theirs.

A Cranelift rule that rematerializes `vconst` and `splat` (as an `iconst` is) was tried first.
It can't apply: rules of the mid-end run only with the optimizer on.

## Measured

`json_tier1_by_pass`, least of 9 rounds, M5 Max at load 30–45:

| | Before | After |
|---|---|---|
| vector loads from the stack, in the JIT's code | 129 | 19 |
| white space, instructions a byte | 4.77 | 4.71 |
| long UTF-8 strings | 9.18 | 8.86 |
| corpus | 21.4 | 21.1 |

Most of the reloads were off the white-space path, in the gated regions.

## What the white-space path is now

The hot loop's blocks on white space, from the JIT's code (`RERAIL_JSON_ASM`): about 194
instructions a block. With the token loop's refill, about 300; simdjson's stage 1 is 137.

- The ASCII test, with the constants: 21.
- The core (backslashes, quotes, the escape, the string's toggle, output 8): 65.
- The control bytes' bitmask: 17.
- The number check's gate: a second class lookup, its bitmask and the packed state's shifts,
  about 50. simdjson computes none of it in stage 1.
- The loop's end: 15.
- The token loop's refill, about 100 a block: it loads the block again and classifies it
  with a table of its own.

## Tests

`rr_parser`: `jsons_cpu_records_are_the_interpreters` (1,165,507 tokens the same),
`jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
`rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`,
`json_loop_against_step_14` (129 the same, 216 refused by both, none differ).
