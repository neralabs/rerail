# 0351: A constant is made where it is read

Status: decided (the user, 2026-11-17, item 1: the token loop's cost a token, read against
the lean lexer). Follows 0350. Keeps 0069.

## Intro

**The problem.** The token loop's entry makes the constants its tokens read: a table's
address, the word hash's multiplier, the slots' base, zero, all ones. Tier 1 runs without
Cranelift's optimizer (0069), and nothing else makes such a constant again near its reader. So
each was one value live through the whole loop, its cold calls included (the string reader, a
run past its window), and the register allocator put it on the stack. A number token's path
loaded and stored 9 words on the stack (`RERAIL_JSON_ASM`, blocks 17 to 56). The lean lexer's
has two loads: its constants are immediates or a `movz` beside their readers.

## The decision

Before the JIT compiles a tier-1 function (`native_cpu_codegen::define_function`), each integer
constant that a block reads but another block made is made again in that block, before its first
reader there (`remat_consts`). This is the optimizer's `remat` of an `iconst`, without the
optimizer's code motion that 0069 keeps out. The AOT's IR is kept before it (`keep`), and rustc
does the same there.

## Measured

`json_tier1_by_pass`, least of 9 rounds, M5 Max at load 24–30:

| | Before | After |
|---|---|---|
| stack accesses in the JIT's code | 331 | 301 |
| a number token's stack accesses | 9 | 4 |
| numbers (4 digits), instructions a byte | 44.3 | 43.8 |
| short strings | 51.1 | 50.3 |
| corpus | 21.1 | 20.9 |

Left on a number's path: the refill's run word and the cursor. Both are loop values, live
across the loop's cold calls, which leave Cranelift only the callee-saved registers for them.

## A number's cost, parted

Synthetic inputs of numbers 1, 4 and 12 digits long (`json_tier1_by_pass` gains the first and
the last) give, for a number and its comma, instructions against the number's length `L`:

| | A pair | 1 digit | 4 digits | 12 digits |
|---|---|---|---|---|
| tier 1 | about 178 + 11 L | 189 | 222 | 310 |
| simdjson's stage 1 | about 15 + 2 L | 17 | 23 | 40 |

- About 89 a token is the token loop: its paths for a number and a comma are about 66 and 54.
- About 11 a digit is the block's, against 4.7 a byte on white space. The rest is the number
  check's masks, whose region runs in every block of numbers (about 300 instructions a block).

**The number check stays tier 1's masks** (`json_number_check_is_tier1_masks`, doc 18 §4), as
decided against 0086. Its 44 operations run once per 64 bytes, for every number in them. Run per
token, in the token's window, they would run once per number: 12 times as often on dense numbers.
That was 100 operations a number when tier 2 ran it. Where numbers are sparse, the gate (0337)
costs about 1 instruction a byte, a class lookup and its bitmask. That is the cost of keeping it.

## Tests

- `rr_parser`: `jsons_cpu_records_are_the_interpreters` (1,165,507 tokens the same),
  `jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
  `rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`,
  `json_loop_against_step_14` (129 the same, 216 refused by both, none differ).
- `parser_tests`: `json_loop_keeps_values_in_registers` (704 and 10),
  `stage1_loop_keeps_values_in_registers` (609 and 9), `json_state_is_bits`,
  `json_tier1_vectors_equal_the_interpreter`, `json_gated_regions_equal_the_interpreter`,
  `json_stream_equals_the_evaluator_on_every_executor`, `stage1_equals_simdjson_on_every_executor`.
- `json_tier1_by_pass` gains `RERAIL_JSON_CLIF=<path>`: each pass's Cranelift IR.
