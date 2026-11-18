# 0383: The full body runs on the edge blocks

Status: decided (the user, 2026-11-17: "go ahead in that order", on making tier 1's block work
simdjson's shape). Follows 0372, 0374 and 0375.

## Intro

**The problem.** A full block lies inside the stream, so in its body every `within` is all ones,
and the values made only of constants and `within` fold away (`Plan::full`). The stream's first
block and its last ones are edge blocks, where that is not true. Tier 1 had a second copy of
the body for them, the general body: an outlined function of 2,800 lines in JSON's tier 1 read as
Rust, called on the first block and on the blocks past `len / 64`.

simdjson's stage 1 has one body. The stream's end is handled by a `finish()` that does only the
end's own checks. The lean lexer is the same: one loop body, with its end's work after it.

## The decision

The full body runs on every block. On an edge block, a part at the body's end (the **edge part**,
`Emit::edge_part`) makes again the values the full body gets wrong there, and nothing else.

**Which values.** `native_cpu_edge::edge_values` runs both bodies at once over the bits of a
stream, 64 positions a word. Each bit is the general body's and the full body's: 0, 1, or
unknown, and whether the two are one value.

- Outside the stream the bytes are the padding's zeros, so a byte test is known there.
- The full body's side is as it is made: a value it folds is that word everywhere.
- Each block starts from the general body's carries (below). A difference on a full block's
  positions is refused: then the program keeps the general body.
- It runs on the stream's start, on 64 alignments of its end, and on every short stream whole.
  It takes 117 ms for JSON and 131 ms for JSONC in a debug build, once per program.

**The edge part.**

- **One cold branch.** It sits at the body's end. Its test compares the block's index with the
  one the head's cold path stored in a slot. Every file's first block is an edge block, so a full
  block never finds its own index there. The head's test is one compare (`edge_block`).
- **It remakes only the differing values.** It reads the body's words. It makes again each byte
  test it needs, from the block's bytes, and each value the body did not keep. Every block it
  creates is cold, so Cranelift lays all of it out after the loop.
- **Its outputs replace the full body's.** An output cut to the stream (`within_word`) is stored
  over the body's word, or leaves by the join.
- **Its carries replace the full body's.** The carried words that hold a differing value's bits
  leave by the join. So every block, full or edge, starts from the general body's carries, as it
  did when the general body was a call. A full block's own carries agree with the general body's,
  as no difference reaches its positions.

A program the analysis cannot bound keeps the general body. So does one with groups, walks,
loads or comparisons.

## What changed in JSON's tier 1, read as Rust

| | Before | Now |
|---|---:|---:|
| The pass's lines | 6,651 | 6,341 |
| The general body (`f1`) | 2,800 lines | none |
| The pass function (`f0`) | 3,297 lines | 5,787 lines (the edge part is cold) |
| The hot loop (`json_loop_keeps_values_in_registers`) | 667 instructions, 4 stack accesses | 673, 7 |
| `vqtbl1q_u8` lookups in the file | 100 | 86 |

The hot loop has 6 more instructions:

- the load of the edge slot for the branch: +1;
- the edge part reads two of the body's early words, the string's, which then live to the body's
  end: one spill and its reload, +2;
- 3 more, not traced instruction by instruction.

**What it remakes.** 90 of JSON's 186 values, and 63 of JSONC's 248. All are word operations; no
byte test differs. The checks at the stream's ends are a few of them: the byte order mark, a
UTF-8 sequence or a number cut off by the end. The rest read the byte order mark's check. The
full body takes that check as zero, and in the general body it masks the file's first three
positions. Everything that reads it differs through the rest of block 0, the strings' `fill`s
included. A first version of the analysis ignored the full body's folds and found 46 values.
That missed this case and two compiler tests (`delay_tests`), where an output reads a folded
value directly.

## Rejected

- **The full cone outlined** (a call that made every value the differing ones read): 2,419
  lines, the general body under a new name. The 53 values that differed then depended on 177 of
  the 186.
- **The edge part at the body's start**, where the values it reads are fresh: it held them
  through the body, and the hot loop spilled.
- **Carries kept apart in a slot**, for the next edge block only: the full body's own carries
  then went on into block 1, and the start's differences never provably ended.

## The escapes (step 3)

The escapes were already branch-free in lean's form: the backslash runs' parity is a carry-less
multiply, as the lean lexer's `PMULL_RAW`. The code did not change.

## Tests

- `parser_tests`:
  - `json_tier1_has_no_general_body` (new): no pass of JSON's or JSONC's tier 1 calls a general
    body.
  - `json_edge_values_are_its_ends_checks` (new): no byte test differs, fewer than half the
    values do, and the analysis bounds both programs.
  - `json_tier1_vectors_equal_the_interpreter` gains files that start with a byte order mark,
    then a string, a comment, a run of backslashes or a number that runs into block 1.
  - The `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`, `registers`,
    `stage1`, `vectors` and `on_every_executor` guards: 54 passed.
- `rr_parser`: the JSON equality set, with `RERAIL_JSON_CORPUS` and `RERAIL_JSON_SUITE`.
- `compiler`: every test but `random_needs_are_answered` (101). The outline tests now hold
  whether or not a pass has a general body (`PassShape::general`).

## Found on the way

The `on_every_executor` tests compiled their program once for each input, and one of them went
past two minutes. They now compile once per program (`Executors::every`), and the group takes
36 s.
