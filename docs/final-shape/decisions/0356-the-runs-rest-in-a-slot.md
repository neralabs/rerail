# 0356: The starts after a run are in a slot of their own

Status: decided (the user, 2026-11-17: "go ahead with 2 then 1", item 1). Follows 0355.

## Intro

**The problem.** After 0355 a JSON token takes the fused path in about 41 instructions, the
lean lexer's count. Traced under lldb (`RERAIL_JSON_ONLY="one-digit numbers"`), each token
still loaded one value from the stack and stored it back unchanged at the loop's back edge.

The value is `rest`: the block's starts after the current run (`mid` splits a block's starts
into runs at its rare ones, as the lean lexer's `loop { rare; run; while run != 0 }`). `mid`
makes it once a run, and only the run's end reads it. It was live across the inner loop, and
Cranelift's allocator split its range there: a reload into a register each token, and a spill
again before the back edge. That is a chain through memory, a store then a load of the same
slot, on every token.

## The decision

`mid` stores `rest` in a stack slot of its own, once a run. A new block, `run_end`, reads it
where the run ends (the inner loop's exit to `run_done`). A token of another rule on the fused
side reads it from the slot too. No value is live across the inner loop for it. The lean lexer's
Rust keeps it so: LLVM spills it at the loop's edges, not each time round.

## Measured

`json_tier1_by_pass` (load 53–86: cycles are noisy, the direction held on each input):

| Input | Instructions a byte, 0355 → 0356 | Cycles a byte, 0355 → 0356 |
|---|---|---|
| corpus | 16.64 → 16.81 | 4.11 → 3.40 |
| numbers | 28.49 → 27.38 | 4.79 → 4.46 |
| one-digit numbers | 54.68 → 51.77 | 10.75 → 7.88 |
| short strings | 37.97 → 40.00 | 6.21 → 5.68 |

A string is a rare token, after which the loop goes back to `mid`, which stores `rest` again: a
store a string, against a load and a store a token before. Strings run more instructions and
fewer cycles.

## Tests

rr_parser: `jsons_cpu_records_are_the_interpreters` (789 files the same),
`jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
`rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`, `the_pairs_are_a_blocks_word`,
`the_marks_tests_are_the_masks`, `the_cpus_records_are_the_interpreters`, `a_short_run_*`,
`the_next_start_is_the_blocks`, `the_run_loop_calls_nothing`, `a_numbers_checks_*`.
