# 0360: The refill follows the body

Status: built (audited 2026-11-23). Decided (the user, 2026-11-17: "go ahead with 2 then 1": what tier 1 computes that
simdjson does not). Follows 0349 and 0355–0358. (Numbered 0359 at first; 0359 was another worktree's.)

## Intro

**The problem.** 0349 runs the token loop after each run of 16 full blocks, "the run's bytes
still in L1". It was not so for JSON. A refill whose words are all the bytes' (`refill_limit`
none) was bounded by the file's end, not by the blocks made (`ready`). So the first part walked
the whole file from its bytes, and every later part had nothing to do. The two stages were two
passes over the file, the second at L2 or beyond for a large one.

A refill bounded by `ready` has to wait at it. The wait kept the state as the head had it, and
the next part read again from the cursor's block. On blocks with no start (white space, a long
gap) that is quadratic: measured, white space went from 4.66 to 60.1 instructions a byte with
the bound and the old resume.

## The decision

Where the refill skips spent blocks (the lean lexer's starts, `skips`) and folds no gap marks,
in a part of the fused pass (`bounded`):

- it reads blocks up to the file's end as before, and past the blocks made it waits (`rwait`);
- the wait keeps the block it would read next as the state's spent block (`h[4]`, `spent_q`):
  the blocks before it were read and had no start after the cursor;
- the next part's refill reads `h[4]` whole wherever it is past the cursor's block
  (`h[4] > qc`). Without such a wait `h[4]` is at most the block after the cursor's, since a
  token starts in the block the refill read. So the wider test changes no other case.

Each part of the token loop now reads the 16 blocks the body has just made.

## Measured

`json_tier1_by_pass` (load 65): instructions a byte about the same, as expected: corpus 15.96 →
15.98, white space 4.705 → 4.66, long strings 4.39 → 4.28. Cycles are noisy at this load; the
corpus files are small, so the locality gain is for large files.

**The refill's classification, read and not changed.** On white space a block costs the body
about 178 instructions and the refill about 110 (lldb). Of the refill's, about 63 classify the
block's bytes again: four loads, the nibble lookups, the skip, run and rare starts' words. The
rest is the lean starts, the bounds and spills (`q` in a stack slot: five loads and a store a
block). The body makes the skip and run words only where they are stored (Cranelift lowers no
unused value). To give the refill its words, the body would store them, and make the rare
starts' set as an extra: about 35 a block to save about 70, half an instruction a byte. Not
taken.

## Tests

- rr_parser: `jsons_cpu_records_are_the_interpreters` (789 files the same),
  `jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
  `rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`, `the_pairs_are_a_blocks_word`,
  `the_marks_tests_are_the_masks`, `the_cpus_records_are_the_interpreters`, `a_short_run_*`,
  `the_next_start_is_the_blocks`, `the_run_loop_calls_nothing`, `a_numbers_checks_*`.
- parser_tests: the JSON tests, the guards and the `equal_the` tests.
- The wait is exercised. With the old resume test the made white space goes to 60.1
  instructions a byte, so the bounded refill waits on it.
