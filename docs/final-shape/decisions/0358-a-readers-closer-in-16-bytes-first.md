# 0358: A reader's closer is looked for in 16 bytes first

Status: decided (the user, 2026-11-17: "go ahead with 2 then 1", item 1: short strings).
Follows 0357.

## Intro

**The problem.** After 0357 a string token's arm read the 64 bytes after its opener: four
vector loads, the quotes' and the backslashes' masks (each four compares, four `and`s with the
weights, four `addp`), about 42 instructions. A JSON string is mostly short. The number's run
already reads the 16 bytes at the token first (`short_word`), as the lean lexer does.

## The decision

The arm (`RuleMode::Calls` in the arm, or `RuleMode::Short`) makes the closer's word over the 16
bytes after the opener first (`Bytewise::short_words_carried`: `short_words` with the escapes'
carries in, zero where the opener is apart from them, as 0357). Where it has a bit, that is the
closer. Else the arm reads the 64-byte window, then the whole function, as before.

A closer escaped inside the 16 bytes is no bit there, since the escapes are made in the window
from a carry of zero. A quote past the 16 bytes, or one whose escape straddles their end, is left
to the 64-byte window, which starts at the same byte.

## Measured

`json_tier1_by_pass`, instructions a byte: short strings 34.60 → 31.40, corpus 15.99 → 15.96.
Cycles at load 27–51: short strings 6.7–7.5 → 6.65.

## Tests

rr_parser: `jsons_cpu_records_are_the_interpreters` (789 files, JSONTestSuite's escapes
included, the same), `jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
`rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`, `the_pairs_are_a_blocks_word`,
`the_marks_tests_are_the_masks`, `the_cpus_records_are_the_interpreters`, `a_short_run_*`,
`the_next_start_is_the_blocks`, `the_run_loop_calls_nothing`, `a_numbers_checks_*`.
