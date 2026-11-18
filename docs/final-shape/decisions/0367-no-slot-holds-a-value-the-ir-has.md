# 0367: No slot holds a value the IR has

Status: decided (the user, 2026-11-17: "just fix the shape of the generated code"). Follows 0366.
Amends 0347's "The stack arrays, read again".

## Intro

**The problem.** Tier 1's pass function, read as Rust (`clif_rust`), had four stack arrays. Three
of them held values the IR already had as values:

- **The rest of the starts.** Before the token loop's run (the starts before the next rare one),
  the starts after the run went to a one-word slot, and were read back after it. The lean lexer
  keeps `starts` as a local across its `while run != 0`.
- **The pass's arguments and the row's index** (`outer`). These were stored at the entry, and
  reloaded where a row's work needs them: the row's start, the row's end, the next row. Its comment
  gave the reason: "Held through the block loop, which reads none of them, they took its
  registers (or were spilled and reloaded in it)."
- **The file's columns** (the record slot, `REC_SLOT`). These were stored at each row's start,
  and 13 were loaded back at each token part's entry. 0347 kept this as a choice.

In each case the generator keeps a value in memory so that a register allocator need not decide
where it lives. The lean lexer and simdjson have these as locals. In the Rust form the slot's
address escapes (`ss.as_mut_ptr() as usize`), so LLVM cannot promote the slot as it promotes a
local: the round-trip stays in the AOT's code too.

## The decision

A value the IR has is read where it is needed, as it is:

- the starts after the run are `mid`'s value, which dominates the run's loop (`from_word` is
  entered only from `mid`'s `go_run` and its own back edge);
- the pass's arguments are the entry's parameters, and the row's index is the row's;
- the file's columns are the row's values (`file_cols(…).values()`), which each token part reads
  through `FileCols::of_tables`.

Where those values live across the blocks' loop is the register allocator's choice, in
Cranelift and in rustc alike.

A slot stays where a call reads it by address:

- **the file's end** (`record_end`), which reads the columns and the loop's count and refusal. It
  is now written there, once a file, in place of at the row's start;
- **the general body's call** (`general`), whose parameters go by a slot. It runs only for the
  first block and the blocks past `len / 64`.

## What changed

In JSON's tier 1 read as Rust, the pass function's stack arrays went from 4 to 2, and their
accesses from 74 to 39.

The hot loop's machine code was checked by the existing guards: `json_loop_keeps_values_in_registers`
and `stage1_loop_keeps_values_in_registers`. Both pass, so Cranelift did not move any of these
values into the hot loop as spills.

## Tests

- `rr_parser`: the JSON equality set (`json`, `the_pairs`, `the_marks_tests`,
  `the_cpus_records`, `a_short_run`, `the_next_start`, `the_run_loop_calls_nothing`,
  `a_numbers_checks`, `rrs_records`, `the_loop_reads_no`, `json_tier1_has_no_label_switch`),
  with `RERAIL_JSON_CORPUS` and `RERAIL_JSON_SUITE`.
- `parser_tests`: the `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`
  and `registers` guards.
- `compiler`: `outline`, `every_step_parses_as_the_reference`, `token_local`.
