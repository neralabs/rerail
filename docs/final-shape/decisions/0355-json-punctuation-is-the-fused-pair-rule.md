# 0355: JSON's punctuation is the fused pair rule, and its starts the lean lexer's

Status: built (audited 2026-11-23). Decided (the user, 2026-11-17: "go ahead with 2 then 1", item 1: the token loop's
cost a token on numbers and short strings, read against the lean lexer and `rerail-json-lean`, the shape differences
fixed in the generator). Follows 0381. (Numbered 0354 at first; 0354 was another worktree's.)

## Intro

**The problem.** A pair of tokens `1,` cost tier 1 about 186 instructions; simdjson's stage 1
and walk do it in a fraction of that. Traced instruction by instruction under lldb (the made
input "one-digit numbers", `RERAIL_JSON_ONLY`), the pair's path was:

- the number, the fused run rule: about 70;
- the comma, `token punctuation = start code & op pair empty`: lowered as `EndRule::Pair(empty)`,
  a pair by a mask's word. Each comma called a function that called another, which on a full
  block returns the constant 0 (`empty`), then looked its code up in the word table by hash
  (the punctuation's keywords): about 95.

And each token's next start waited on its end: `left & (-1 << end)`, then a `ctz`. Every token
was one serial chain (the bytes' load, the entry's load, the run, the end, the mask), as the
lean lexer's is not: its starts are the tokens' own (`lean_starts`), and a token clears its
bit alone (`left &= left - 1`).

## The decision

1. **`pair empty` is a pair table of none** (`bitlex_masks`): `EndRule::Next` with no
   spellings, as `pair P` with none. Punctuation is then the fused pair rule beside the number's
   run, its code by first byte in the entry (`TOKEN_PUNCT`), no call and no hash.
2. **A pair table of none reads no row** (`RecordTables::pairless`, `next_last`): the token is
   its first byte.
3. **Its nibble tables are zeros** (`pair_nibbles`), and the block's pair word is the constant
   zero (no lookup in the refill). So the loop has a pair word, and JSON's starts can be the lean
   lexer's.
4. **A run byte may be zero** (`lean_starts`). JSON's `wordbyte` is `~(ws | op | quote)`, which
   holds on a zero byte. The condition kept the bytes past the file out of a run. The run's word
   is cut at the file's end wherever its output holds on a zero byte (`Bytewise::word`'s
   `cut64`), so no run bit, and no put-back start, lies past it. The pair word is not cut, so a
   pair's second byte stays no zero.

## Measured

`json_tier1_by_pass`, tier 1's records pass (load 30–50; cycles move by up to 30% at this
load):

| Input | Instructions a byte, 0381 → 0355 | Cycles a byte, 0381 → 0355 |
|---|---|---|
| corpus (`jsonexamples`) | 20.26 → 16.64 | 4.84 → 4.11 |
| numbers (`1234,`) | 43.07 → 28.49 | 8.86 → 4.79 |
| one-digit numbers (`1,`) | 92.5 → 54.7 | 19.4 → 10.75 |
| 12-digit numbers | 22.84 → 17.95 | 5.28 → 2.99 |
| short strings (`"ab",`) | 50.50 → 37.97 | 12.3 → 6.21 |
| white space | 4.30 → 4.67 | 0.82 → 0.70 |

Step by step, instructions a byte on the corpus: 20.26, then 19.15 (1), then 17.62 (2), then
16.64 (3 and 4). The cycles moved with 3 and 4, which take the chain off the token's end.

White space costs 0.37 more instructions a byte: the refill now makes the lean starts (the run's
interiors, the rare starts' word) on blocks that have none. The refill classifies a block's bytes
again after the body did. Bounding it by the blocks the body has made (`ready`), and reading the
body's words, is the next step. Today a refill on bytes is bounded by the file's end, so its first
call walks the whole file, and its wait would read again from the cursor's block.

## Tests

- rr_parser: `jsons_cpu_records_are_the_interpreters` (789 files, 1,165,507 tokens the same),
  `jsons_tier2_split_is_tier1s`, `json_split_stores_only_what_rows_are_read_for` (10 outputs:
  `empty`'s is gone), `json_loop_against_step_14` (129 the same, 216 refused by both, none
  differ), `rrs_records_are_rows_on_the_cpu`, `the_loop_reads_no_record_back`,
  `the_marks_tests_are_the_masks`, `the_cpus_records_are_the_interpreters`, `a_short_run_*`,
  `the_next_start_is_the_blocks`, `the_run_loop_calls_nothing`, `a_numbers_checks_*`.
- `the_pairs_are_a_blocks_word`: rr's pair tables are `[6, 0]`. rr also has a `pair empty` rule,
  now a table of none.
- parser_tests: the JSON tests, the guards and the `equal_the` tests (every executor, Metal's
  included).
- `json_compiled_path_equals_the_interpreters` was stale since 8a1762be0 (tier 2 writes the tape,
  by four rare functions of its own). It asserted no call at all, and failed before this change.
  It now asserts every call is one of the tape's functions, so a pass over the table (a wrap's or
  a drop's function) still fails it.
- `RERAIL_JSON_ONLY=<name>` runs one made input of `json_tier1_by_pass` alone, to trace it.
