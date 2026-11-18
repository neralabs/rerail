# 0347: A total has no row

Status: decided (the user, 2026-11-17: "also do the two state-placement fixes", the second of
which is the outputs that go to memory each block). Follows 0346 and 0341's "What is left".

## Intro

**The problem.** 0341 listed what was left between tier 1 and simdjson's stage 1:

> The outputs go to memory each block (an 88-byte record), and the token loop's state
> crosses each block through a stack array (0177's boundary).

simdjson's stage 1 writes one thing to memory: the structural index. Its error flags
(unescaped characters, the UTF-8 check) stay in registers, OR-ed each block, and are read
once at the end.

The split JSON tier 1 (`json_split(Split::Tier2)`, the program 0341 and 0343 read) has 11
outputs. Its token loop makes 6 of them from the bytes and stores none of those (0177). A full
block still stored 5 words, and 3 of them were delayed: each read the row before, shifted, and
stored it again (`delayed_store`). Four of the five were **totals**: the file's refusals, each
read as a single flag.

## The decision

A total's per-block words have no reader where the token loop runs:

- the file's flag is the running word, OR-ed each block and stored once at the row's end, and
  `Batch::results` reads it from there;
- the rows are read only by tier 2 (`masks`), by a mark at a token's start (`Records::marks`),
  and by the token loop's reads that it can't make from the bytes (the skip, the end rules', the
  readers', the tests').

So a total that none of these reads is not stored (`BlockNative::rowless_totals`). It joins
`records_made` in `forward`'s `stored`, with the same condition: the last pass, with the token
loop. The total is still computed for its running word.

For the split JSON tier 1, a full block now stores one word: output 8, a mask tier 2 reads (the
interface between the tiers, as simdjson's index is between its stages). It made no delayed
store.

| A full block | Before | Now |
|---|---:|---:|
| outputs stored | 5 | 1 |
| delayed stores (a load, a shift, a store) | 3 | 0 |

## The stack arrays, read again

The generated Rust has three stack arrays in the pass function. None of them is a value
crossing every block in place of a register:

- **`ss2`, the loop's parameters for the general body's call.** It runs only for the first
  block and the blocks past `len / 64` (`is_full` false), never in the hot loop.
- **`ss1`, the row's record slot (`REC_SLOT`).** It holds the file's 13 columns (bytes, records,
  counts, tables). It is written once a file. The token loop's entry loads them each block, and
  the token loop's state goes round the block loop as block parameters, in registers.
- **`ss0`, the pass's arguments.** It is written once a call and read once a file.

The record slot is a choice: 13 values live through the block body would need 13 registers
there, or spills. Its cost is 13 loads a block from L1, against about 1,500 instructions a
block. It stays as it is, and is recorded here.

## Tests

- `rr_parser`: `json_split_stores_only_what_rows_are_read_for` (new): in the split program, each
  stored output is read by tier 2 or by a mark, and no total is stored. It prints
  `stored a block: [8]`.
- `rr_parser`:
  - `jsons_cpu_records_are_the_interpreters`: 789 files, 1,165,507 tokens the same;
  - `jsons_tier2_split_is_tier1s`: 386 files;
  - `json_loop_against_step_14`: 129 the same, 216 refused by both, none differ;
  - `the_loop_reads_no_record_back`.
- `parser_tests`: `json_loop_keeps_values_in_registers`, `stage1_loop_keeps_values_in_registers`,
  `json_state_is_bits`, `json_tier1_vectors_equal_the_interpreter`,
  `json_gated_regions_equal_the_interpreter`,
  `json_stream_equals_the_evaluator_on_every_executor`, `stage1_equals_simdjson_on_every_executor`.
