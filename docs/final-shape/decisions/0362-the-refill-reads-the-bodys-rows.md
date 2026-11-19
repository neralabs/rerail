# 0362: The refill reads the body's rows

Status: decided (the user, 2026-11-17: "complete this", of 0360's "not taken": the refill
reading the body's words instead of classifying each block's bytes again). Follows 0355 and
0360.

## Intro

**The problem.** The token loop's refill made a block's words from its bytes: four loads, the
nibble lookups, and a mask each for the skip (white space), the main run (a word's bytes) and
the rare starts (a string's quote). The body had classified the same bytes a moment before,
for its own values. On white space that was about 63 of the refill's 110 instructions a block,
and it is why white space cost 0.37 instructions a byte more after 0355. simdjson classifies
each byte once: stage 2 reads stage 1's index.

## The decision

Where the token loop is the lean lexer's (`RecordTables::lean`, fused rules) and sets no mark,
the body stores the refill's words in its rows (`refill_outs`, `refill_rows`):

- the skip's and the main run's outputs: the CPU's form keeps them (`cpu_form_keeping`), not
  the zero it gives the outputs the record pass makes from the bytes elsewhere;
- the rare starts' word: a value the body makes whose bytes are the rare set's (a skipped
  byte's answer free), stored where it is made (`Sinks::extra`) in the row slot of a total no
  one reads by its rows (0347's `rowless_totals`).

The refill, where it waits at the blocks made (0360's `bounded`, so the rows it reads are
final), loads the three words. No byte is classified twice.

0360 priced this at about 35 a block in the body against 70 saved, and did not take it. It
costs less in the body: the quote's word is the body's own (its strings), and the white space
and word classes share its lookups.

## Measured

`json_tier1_by_pass`, instructions a byte (load about 50):

| Input | Before | After | simdjson stage 1 |
|---|---:|---:|---:|
| corpus | 15.98 | 15.42 | 2.96 |
| white space | 4.66 | 3.98 | 2.15 |
| numbers | 27.45 | 26.76 | 4.51 |
| one-digit numbers | 51.84 | 51.15 | 8.71 |
| 12-digit numbers | 17.64 | 16.95 | 3.05 |
| short strings | 31.26 | 30.98 | 4.51 |
| long ASCII strings | 4.28 | 4.77 | 2.15 |
| long UTF-8 strings | 8.14 | 8.62 | 3.26 |

Long strings pay the body's three words a block (about 30 instructions) where the token loop
reads few blocks. Every other input gains. White space is back below where it was before 0355
(4.30).

## Tests

- rr_parser: `jsons_cpu_records_are_the_interpreters` (789 files the same),
  `jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for`,
  `rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`,
  `the_pairs_are_a_blocks_word`, `the_marks_tests_are_the_masks`,
  `the_cpus_records_are_the_interpreters`, `a_short_run_*`, `the_next_start_is_the_blocks`,
  `the_run_loop_calls_nothing`, `a_numbers_checks_*`.
- parser_tests: the JSON tests, the loop guards (`json_loop_keeps_values_in_registers`
  within 667/4) and the `equal_the` tests.
