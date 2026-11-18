# 0357: A reader's window has no escape carry after its opener

Status: decided (the user, 2026-11-17: "go ahead with 2 then 1", item 1: short strings).
Follows 0356.

## Intro

**The problem.** A JSON string token (`"ab"`) took about 100 instructions on the reader's arm.
Traced under lldb (`RERAIL_JSON_ONLY="short strings"`), the closer's window at the byte after
the opener:

1. loaded its four vectors and made the quotes' mask;
2. tested the byte before the window (the opener) for a backslash: the escape's carry in, a
   branch to a cold block that may call (`carry_in`);
3. made the backslashes' mask and the escapes.

The vectors were live across the branch to a block that calls, and aarch64 keeps no vector
register whole across a call. So Cranelift stored all four after their loads and loaded them
again after the branch: 8 vector stack accesses, and the weights loaded from the stack twice.

The byte before the window is the opener. The arm has just tested it is `"`, which is no
backslash. The short reader's other path (`words_gated`) knew that already: it gives the escape
a carry in of zero where the opener and the escape's run are apart (`Bytewise::apart`). The
path JSON takes (`RuleMode::Calls` in the arm, `Bytewise::word`) gave none.

## The decision

That path makes the closer's word with `words_carried`. Each run the closer's escapes read gets a
carry in of zero where the opener is apart from it (`carried_runs`, `apart`), as on
`words_gated`'s path and in the lean lexer's reader. The window's two masks are made from its
four vectors in registers: no test of the byte before, no branch, no spill.

## Measured

`json_tier1_by_pass`, instructions a byte: short strings 40.00 → 34.60, corpus 16.81 → 15.99.
Short strings' cycles a byte were 5.68 before and 6.7–7.5 in three runs after, at load 40–45:
judged by the shape (no spill, no branch), not by those cycles.

What is left on a string is that its next start waits on its end. The loop's starts are made
from the bytes alone (`lean_starts`), so a string's interior bytes are starts, which its end
clears. simdjson's structurals already leave the string's interior out.

## Tests

rr_parser: `jsons_cpu_records_are_the_interpreters` (789 files the same),
`jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
`rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`, `the_pairs_are_a_blocks_word`,
`the_marks_tests_are_the_masks`, `the_cpus_records_are_the_interpreters`, `a_short_run_*`,
`the_next_start_is_the_blocks`, `the_run_loop_calls_nothing`, `a_numbers_checks_*`.
