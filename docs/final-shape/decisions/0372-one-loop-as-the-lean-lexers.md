# 0372: One loop, as the lean lexer's

Status: decided (the user, 2026-11-17: "go to one loop if it's closer to lean"). Replaces 0349.
Follows 0362 and 0367.

## Intro

**The problem.** The lean lexer (`rr_syntax/src/lean.rs`, `lex`) has one loop over 64-byte
windows. Each iteration:

1. classifies the window: `starts`, `rare`, the word and pair masks;
2. takes its tokens at once, from those masks in registers;
3. goes on to the next window.

Tier 1's pass had two loops:

- the full-block body, which looped on itself for 16 blocks (0349's `TOKEN_RUN`) and stored each
  block's refill words in its row;
- then a token part, which read the 16 rows back and took their tokens.

0349 chose the run of 16 to pay the switch between the two once a kilobyte. That switch cost
spills and reloads: the file's columns from the record slot, the pass's arguments, the body's
values. 0367 removed those slots. What was left was the shape: the body's words went to memory and
came back a run later, where the lean lexer's never leave registers.

## The decision

The pass is one loop. Each full block runs its body, then the token part over the blocks now
final, then goes back by the head to the next block (`forward`, `tokens`).

**The block's words are handed on, not read back.** The body stores the refill's rows where it
makes them (`Emit::sink`). Those words are the skip's, the main run's and the rare starts'
(`refill_rows`, 0362). The sink now keeps each word it stores (`Emit::stored_words`). `step` hands
them to the token part as values (`Hand`), with the block they belong to. The refill reads block
`q`'s words:

- `q` the handed block: as the body made them, with no row read;
- any other block: its row, on a cold path. That is a block the general body made (the first
  block, the last), or one a token's cursor came back to.

**The block is read at once.** If no stored output is delayed, a block's row is final once its
body made it. Then `ready`, the bound on the blocks the token part may read, includes the block just
made (`at_once`), and its tokens are taken in the same iteration, as lean takes its window's. The
last block is the file end's part's, so `ready` stops before it. If a stored output is delayed, a
block's delayed bits go into the row before it. Then `ready` stays one block behind, and the handed
words are not used: their block is read in the next iteration, from its row.

## The shape, in JSON's tier 1 read as Rust

- One loop (`'b1`): a block's body, its tokens, the next block. Before, the body was a loop of its
  own inside the blocks' loop.
- The refill's words: `if q == block { the body's } else { cold: the row's }`. Before, three row
  loads each refill.
- The pass function is 3,358 lines; it was 3,418.

Still unlike lean: the body stores the refill's rows, for the cold path. Lean stores no masks.
The rows could go only if the general body also handed its words (it is an outlined call, through
a slot), and if a token's cursor never went back to an earlier full block.

## Tests

- `rr_parser`, the JSON equality set: `json`, `the_pairs`, `the_marks_tests`,
  `the_cpus_records`, `a_short_run`, `the_next_start`, `the_run_loop_calls_nothing`,
  `a_numbers_checks`, `rrs_records`, `the_loop_reads_no`, `json_tier1_has_no_label_switch`. They
  were run with `RERAIL_JSON_CORPUS` and `RERAIL_JSON_SUITE`.
  - `jsons_tier2_split_is_tier1s` found the first version's fault: a sunk output's word is zero
    past its sink (`Emit::finish`), so the handed words were zeros. They are now the words the
    sink stored.
- `parser_tests`: the `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`
  and `registers` guards.
- `compiler`: `outline`, `every_step_parses_as_the_reference`, `token_local`.
