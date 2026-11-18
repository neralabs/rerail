# 0400: A token's end is its gap's

Status: decided (the user, 2026-11-18: page 39's item 3, "find token ends without a second
position list", chosen by shape: "what brings us closer to the desired shape"). Follows 0398
and 0399. Built for `14-json`; JSONC's comments keep the extract (below).

## Intro

**The problem.** Tier 1 wrote two position lists for every token: its first byte (the starts)
and its last byte (the lasts). Page 39 measured the lasts' extract at 123 instructions of
`14-json`'s hot loop. simdjson's stage 1 writes starts only, and stage 2 finds where a token
ends as it reads it.

## The decision

**A token ends at the next token's start, less the skip's bytes before it.** The skip is the
grammar's `skip` mask, the bytes between tokens (whitespace outside strings). With `K` the skip
and `S` every row's starts, Bitlex proves three things (`bitlex_masks::gap_ends`):

- no last of a row is in `K`;
- after a last of a row comes a byte of `K`, a start, or the file's end;
- after a byte of `K` comes a byte of `K`, a start, or the file's end.

Then the bytes between a token's last and the next start are a run of `K`, and the last is the
highest byte below the next start that is not in `K`. Trailing spaces inside a string's content
are not in `K` (they are inside a string), so the content keeps them.

The proof need not hold where the file is refused: it excuses every place at or after a
fallback bit (an error, an escape, a byte no token holds), since those files never reach tier 2.

**One rule, no state.** A rule by the token's first byte was tried first: it fails because a
byte does not tell a token's kind. `{` begins both a punctuation token and a string's content,
and they end differently. simdjson's stage 2 knows by its state. The gap rule needs neither.

**Who makes the lasts.** The block program keeps the lasts' extract, as 0177's end rules do, so
Metal (which makes every mask) is unchanged. `Records::gap` names the skip's output. Then:

- the CPU's tier 1 makes no lasts mask and no lasts extract (`BlockNative::ruled_lasts`);
- the CPU's record pass makes each last from the next start and the skip's mask words: one load
  and a count of leading zeros, a word a turn for a gap of 64 bytes or more
  (`native_cpu_block::gap_last`);
- the interpreter makes them the same way (`records::gap_lasts`), so it is the CPU's oracle.

**JSONC keeps the extract** (until 0401, which proves its comments too). A comment's last is followed by a gap only past the escape of a
comment opened inside another. That escape is earlier in the file than the comment's last
byte, by any distance. The induction prover finds no lemma for it within its budget, and a
larger budget made the compile too slow. Tried on documents, the property holds. A row the
proof does not reach keeps the program on the extract (R2).

## Requirements, a test each

- R1. `14-json`'s rows all end at their gaps, and the CPU makes no lasts
  (`tier2_tests::a_tokens_end_is_its_gaps_as_the_lasts_extract`).
- R2. A grammar the proof does not reach keeps the extract (`NO_GAP_ENDS`, the same test's
  oracle; `15-jsonc` was such a grammar until 0401).
- R3. The interpreter's lasts and codes equal the old extract's on every ladder step with gap
  ends, on every document tier 1 accepts (the same test, against the program compiled with
  `NO_GAP_ENDS`).
- R4. The CPU equals the interpreter: the ladder's every step, JSON's vectors and gated regions,
  the compiled path, the corpus of trees. Tier 1's outputs are compared without the ones the
  record pass makes (`BlockRun::without`).
- R5. Every executor: the CPU makes the lasts from the gaps; Metal and the interpreter of masks
  keep the extract, which the proof makes equal.
- R6. The hot loop loses the extract (`json_shapes`).

## What changed

| `14-json` | Hot loop | Stack accesses | Tier 1's operations |
|---|---:|---:|---:|
| After 0399 | 772 | 20 | 122 |
| Now | **616** | **5** | 123 |

The one operation more is the skip's mask, now an output the record pass reads. JSONC is
unchanged (988, 33). The rr JSON set: 129 parse the same, 216 refused by both, 0 differ.
