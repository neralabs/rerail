# 0398: A number's check runs over its token, in tier 2

Status: decided (the user, 2026-11-18: "Check numbers and \u escapes once per token in tier 2,
as simdjson does in stage 2", page 39's item 1). Supersedes 0351's note that the number check
stays tier 1's masks. The `\u` escapes' part is not done here.

## Intro

**The problem.** JSON's number check (doc 18 §4) was tier 1's: masks over every 64-byte block,
a state's mask per state of the check's byte machine, in a region gated on the number bytes.
Page 39 measured it as 318 of the 1,078 instructions in `14-json`'s hot loop. simdjson's stage 1
has none of this: stage 2 checks a number once, from the token's own text.

## The decision

A check whose byte machine starts at a token's first byte is a **token's check**
(`LocalCheck::token`). Tier 2 runs it at each token of the row it is in (`CHECK`), over the
token's own bytes, and tier 1 makes none of it.

- **Its ops** are evaluated in the token's frame: place 0 is the token's first byte.
  - `Fill(s, e)`: `f[i] = s[i] | (f[i-1] & !e[i])`, the token's own fill.
  - `Start`: the token's first place.
  - `Last`: its last place.
  - No `Within`: nothing outside the token is read.
  - The check refuses where its last op is 1 at any place of the token.
- **Bitlex** (`token_check_ops`): `Lowering::check`'s masks over the machine, in that frame. A
  row whose keyword starts with a byte the machine takes keeps tier 1's masks, as before.
- **The CPU** (`native_cpu_stream`):
  - A token of 16 bytes or fewer is checked in its arm (`token_check_short`). One 16-byte load,
    one vector a table, the byte sets' masks made by one tree of pairwise adds
    (`bitmasks16`), then the ops on words. No call, no loop.
  - A longer token goes to an outlined loop over its 64-byte windows (`token_check_word`). Each
    `prev` carries its operand's word, and each `fill` its carry. A number over 64 bytes is
    checked 64 bytes at a time (0086), never handed to tree-sitter.
  - The file is readable 64 bytes past its end; the places past the token are cut from the
    test.
- **The interpreter** (`program::token_value`) evaluates the same ops over the token's bytes.
- **Rust** (`rust_stream`) refuses a token's check for now: the only grammar built through Rust
  is the `.rr` grammar, which has no check. Open.

## Measured

`14-json`'s hot loop (`json_tests::json_shapes`): **1,078 → 802 instructions**, 31 → 17 stack
loads or stores. Tier 1's block program: 186 → 138 operations.

Retired instructions a pass, the empty run's taken off (`json_bench` with `RERAIL_BENCH_RUN`,
100 passes, M5 Max):

| Files | Tier 1 | Tier 2 | Both |
|---|---|---|---|
| `numbers.json` + a synthetic file of 60,000 numbers (604 KB) | 13.3M → 10.9M | 16.5M → 24.6M | **+19%** |
| `twitter.json` (632 KB) | 10.4M → 8.8M | 11.0M → 11.6M | **−4.4%** |

**Dense numbers cost more, as 0351 said they would.** Tier 1 saves about 255 instructions a
block on them. A number's check costs about 116 in tier 2. Dense files hold about 7 numbers a
block, so the check would have to cost under about 35 to break even. Its 8 byte sets and 37
logic ops are more than that. simdjson's stage 2 pays the same kind of cost, but it converts
the number's value there too, which tier 2 does not do.

The first CPU form called the outlined loop at every number. It cost 2.0 times tier 2's time
on dense numbers. The arm's short path brought that to about 1.5 times.

## Tests

- `compiler`:
  - `tier2_tests::a_numbers_check_runs_over_its_tokens_bytes` (new). Valid and invalid numbers
    at lengths of 1 to 200 bytes: one window, a window's edge, two and three windows, and a
    number ending at the file's end. The interpreter must equal RFC 8259's grammar, and the CPU
    must equal the interpreter.
  - every test but `random_needs_are_answered`: 103 passed.
- `parser_tests`:
  - `json_number_check_is_a_tokens_check` (was `json_number_check_is_tier1_masks`): tier 1's
    program is the one with no check, there is one token's check, with 4 fills and ops of 48
    or fewer.
  - `json_tier2_is_transducer_words`: a `CHECK` is allowed, and every check is a token's.
  - `json_compiled_path_equals_the_interpreters`: one outlined function past the tape's, the
    check's.
  - `json_msl_is_one_kernel`: the line budget counts the instructions of the form it
    generates (`Planes` writes its classes out as logic). With fewer instructions, the old
    count was under its fixed part.
  - the `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`, `registers`,
    `stage1`, `vectors` and `on_every_executor` guards pass.
- `rr_parser`: the JSON equality set with `RERAIL_JSON_SUITE` and `RERAIL_JSON_CORPUS`, and
  `json_loop_against_step_14`: 129 files parse the same, 216 are refused by both, none differ.

Already failing at HEAD before this change, unchanged by it:
- `only_the_compiler_reads_tier2s_states`;
- `every_step_on_metal_equals_the_interpreter` (`UnsupportedBackend`);
- `corpus_cranelift_equals_the_interpreter`;
- `rr_cases_parse_from_their_steps` and `rr_steps_parse_as_the_reference_and_the_engine`.

`json_bench` refuses `canada.json`, `citm_catalog.json` and `mesh.json` at HEAD as well.
