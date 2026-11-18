# 0374: The refill's words have no row

Renumbered from 0373, which two other worktrees took.

Status: decided (the user, 2026-11-17: "please do it", on 0372's "still unlike lean"). Follows
0372. Amends 0362.

## Intro

**The problem.** After 0372, the pass is one loop, and each full block hands its refill words to
the token part as values. But the body still stored those words in its row each block: the skip's,
the main run's, and the rare starts'. Only a cold path read them back:

- a block the general body made (the first block, the last), whose words reached the token part
  through no value;
- a block a token's cursor came back to.

The lean lexer stores no masks: a window's `starts` and `rare` live in registers and are gone after
it. simdjson's stage 1 stores only its index.

## The decision

Where nothing else reads them, the refill's words have no row (`forward`'s `rowless`). Then:

- **The body does not store them.** The outputs stay unstored (`Loop::stored`), and the rare
  starts' word is kept for the token part, not stored (`Sinks::extra_stored`).
- **Every block hands its words on.** The general body is an outlined call, so it writes its block's
  index and words after its parameters in the call's slot (`general`). The pass reads them back
  after the call, as it reads the parameters, and hands them to the token part. A block with no
  words passes all ones as its index.
- **The cold way classifies again.** A block that is not the handed one has its bytes classified
  by the refill's own classes (`Bytewise::words_with`), as before 0362. It is the rare path, behind
  `q == block`.
- **The file's end is told, not found.** The token part's last block was the file end's part's,
  because `ready` stopped one block short. Now `ready` takes every block made, and the part is told
  it is the file end's (`closing`). A part that reads the file to its end goes back by the head,
  which sends it to `finish`.

The words have no row only where all of these hold. Otherwise they are stored as 0362's:

- the token loop is lean's, with a rare starts' set (`RecordTables::lean`, `rare`);
- no one else reads those outputs (tier 2, a mark, a test), and the body can make them from the
  bytes;
- no stored output is delayed, so each block is final once made (0372's `at_once`).

## The shape, in JSON's tier 1 read as Rust

| `f0`, the pass function | 0372 | Now |
|---|---:|---:|
| row reads (`* 80`) | 1 (cold) | 0 |
| stores | 47 | 44 |
| lines | 3,358 | 3,534 |

The lines added are the cold re-classify. It is behind `core::hint::cold_path()`.

## Tests

- `rr_parser`: the JSON equality set (`json`, `the_pairs`, `the_marks_tests`, `the_cpus_records`,
  `a_short_run`, `the_next_start`, `the_run_loop_calls_nothing`, `a_numbers_checks`,
  `rrs_records`, `the_loop_reads_no`, `json_tier1_has_no_label_switch`,
  `json_split_stores_only_what_rows_are_read_for`), with `RERAIL_JSON_CORPUS` and
  `RERAIL_JSON_SUITE`.
  - The cold way is reached. With its words made zeros (a probe, removed),
    `jsons_tier2_split_is_tier1s` fails.
- `parser_tests`: the `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`
  and `registers` guards.
- `compiler`: `outline`, `every_step_parses_as_the_reference`, `token_local`.
