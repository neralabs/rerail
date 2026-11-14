# 0177: Tier 1 on the CPU is a token loop, as the lean lexer is

Status: decided (2026-11-13); stage 1 landed (the rules, their meaning, the CPU's ends from
them; see Stage 1); the token loop runs in the block loop; the starts, ends and the
loop's masks are the lean lexer's (see The shape); the marks and checks at the token next.
Replaces 0139's way on the CPU (its constructs stay; its "take operations out of the one
pass" does not). Metal keeps the bit-parallel form.
The design: docs/parser-generators/36; the log: docs/parser-generators/35.

## Intro

**The problem.** Tier 1 is the compiled parser's lexer: it turns a file's bytes into
records, one a token (its first byte, its last byte, its code). On the CPU it computes
every mask of the grammar at every byte, and only then reads the tokens off them.

Over the corpus (1,461 files, 13.9 MB, 2.56M tokens, 217K blocks of 64 bytes),
measured the same way (`jit_against_aot`, nine rounds):

| | Lean lexer | Tier 1 (JIT) |
|---|---|---|
| A block's masks | 4 (17M, measured on an 8.8 MB corpus) | about 350 operations, about 340M (the split probe) |
| A token's work | about 72 instructions | about 83 (record pass 62, extracts 21) |
| In all | 192.9M | 501.5M (AOT 465.9M) |

A token costs about the same in both. The gap is the block's masks. Taking operations
out of them, one mask at a time (0139's steps, and the tries of 2026-11-13: rounds,
carries, regrouping), moved tier 1 by a few millions each, some of them noise.
The form itself is the cost: **today's tier 1 on the CPU is a dead end.**

**The decision.** On the CPU, tier 1 is the lean lexer's form, generated from the grammar:

- **A few masks a block**: the bytes no token starts at (blanks), the runs a token
  is (a word's bytes), the rare starts, a two-byte punctuation's first byte.
- **A loop over a block's starts**: each token's kind by one table load of its first
  byte, its end by its run, its word by the hash table (as the record pass now).
- **A rare token read at its start**: a string, a comment, a backtick literal, by the
  walk (0139's step 1, a generated function), which jumps the loop past it.
- **The checks at the token**: what a byte, a number, an escape or a mark must be,
  tested where the token is, not at every byte.

Each is a construct of the block program with two lowerings: this token loop on the
CPU, today's masks on Metal (0139's rule: one program, not a second module).

## The shape

The lean lexer has no stages: one loop over a file's blocks, and in it, a block at a time,
its few masks, then the loop over its tokens, each written where it is found, a rare one read
by its own reader, its checks made at it. The CPU's tier 1 is generated to that one shape:

| Part | The lean lexer | The CPU's tier 1 | Built |
|---|---|---|---|
| A block's masks | space, word, rare, pair | `skip`, the runs, the pair (1, 1 and 9, 31 operations) | yes (2c) |
| The token loop | in the block loop | a part after each block, over the blocks whose outputs are final (`part_fn`) | yes |
| A token's start | the first non-space after the one before | `skip`'s words after the token before (`records::walk`) | yes (2a) |
| A token's end | its run, its pair | its first byte's rule (`run`, `pair`, `last`) | yes (1) |
| A rare token | its reader, which moves the cursor | its region's last byte, by the walk's masks | masks for now |
| The checks | at the token | the fallback and the marks, masks at every byte | no |
| What tier 2 reads | the tokens | the records; the marks as bits at a token's start | marks as masks |

What is left is the last three rows: the marks and checks made at the token (a byte no token
takes a `BAD` kind; a mark a token's bit, from the bytes between it and the token before), and
the rare tokens by their readers. With them the CPU makes no mask but the loop's, and the body
is the lean lexer's.

## Governance

- **The meaning is the interpreter's.** Each new construct is defined first in the
  interpreter (`stream.rs`, `block.rs`), with a test that it equals today's masks over
  the corpus sample and over the block's edges, before any lowering.
- **One program, two lowerings.** A construct is lowered for the CPU and for Metal.
  No grammar gets a CPU-only or Metal-only statement.
- **Byte-identical, always.** Each step keeps, with no conversion:
  the corpus's tapes the engine's (`corpus_tapes_are_the_engines`);
  the lean tapes (`corpus_images_are_the_lean_tapes`, 1,452 files taken: a step that
  takes fewer says so and why);
  AOT equal to the JIT (`aot_tapes_are_the_jits`).
- **The contract between the tiers is the records.** Metal against the CPU
  (`tier1_on_metal_equals_cranelift`) compares the records (positions, codes), the
  totals, and the outputs tier 2 reads, no longer every mask word: the CPU will not
  make them.
- **Structural guards, as tests**, each added with its stage and kept:
  stage 1, no `lasts` extract in the CPU's tier 1 and no `last` mask of a run-ended
  token; stage 2, the CPU body's operation count under its budget; stage 3, no
  per-byte check in the CPU body that a token's test replaced. The verifier's
  no-per-byte-loop rule holds: the loops are over a block's tokens and regions.
- **No second path.** What a stage replaces on the CPU is deleted when the stage lands:
  no flag between the old form and the new.
- **Judged by shape.** A step lands when it makes the generated code more like the
  lean lexer's and keeps the tapes byte-identical. A half-built shape's cost is not a
  verdict on it: an intermediate step may cost more until the stage that completes it.
- **Measured one way, at the end.** The whole shape is measured by `jit_against_aot`
  (nine rounds), AOT and JIT both, with the load noted. The split probe moves by about
  8M under load: it diagnoses, it does not decide.
- **Stop rule.** A stage that cannot reach its shape gate stops: the shape is looked at
  again and the user told.
- **Logged.** Each step and each dropped try is a row in docs/parser-generators/35;
  docs/parser-generators/36 holds the design as built; this record its stages' status.

## Stage 1

Built: the syntax (`token K = start S run R`, `pair P`), its mask meaning
(`TokenRule::last`), the records' end rules (`Records::ends`, `rules`), the interpreter's
lasts from them, and the CPU's: the record pass finds each run- or pair-ended token's last
from its first byte, and the CPU makes no `lasts` extract (guard:
`the_cpu_makes_no_lasts_mask`). Over the corpus the rules' lasts are the masks' lasts
(1,473 files, 2,567,918 tokens). The `.rr` grammar ends its words, numbers, `#` words and
punctuation by rules, and its strings, comments and backtick literals by their masks.

In between: the ends are found in the record pass, after the blocks, which reloads the run
mask's word. The lean lexer finds them inside the block loop, from the word mask in a
register (`end = pos + max(run, n1)`); that is where the token loop goes once stages 2 and
3 have shrunk the body (the decision's "loop over a block's starts").

## Stage 2a

Built: the records' `skip` (an output: the bytes between tokens), the interpreter's loop
(`records::walk`: each token at the first byte after the one before that `skip` does not
hold), and the CPU's: the record pass finds each start in `skip`'s words and writes the
starts column, so the CPU makes no starts extract (guard: `the_cpu_makes_no_starts_mask`).
A byte no token begins with has no rule (`NO_RULE`): its token is the byte alone. The loop's
tokens are the masks' over the corpus (1,473 files, 2,567,918 tokens,
`the_loops_tokens_are_the_masks`). The CPU's form: 329 operations, from 334; the start masks
are still made for their other readers (marks, row modes), which 2b takes on.

## Stage 2c, and 2b's readers

Built: `skip` is `ws`, the word runs `word`, the number's run its digits and a float's `.`
between two (`num_run`), and `two` reads no `code`. The CPU's lean-lexer masks are now what
the lean lexer's are: `skip` one operation, the word run one, the number's run 9, the pair 31
(from 111). `second` keeps `code`, so the masks' meaning (Metal's `p_last`) is unchanged
(`token_rules_are_their_last_masks`).

The census (`cpu_outputs_by_operations`) shows what still reads the token start masks and
`code`, and so holds the body at 331 operations: the fallback (the gap check, a byte no token
covers; the escapes and errors), reading 199, 32 of them its own; the marks tier 2 reads
(`newline` through `after_comma` and `starts_g`, `json_bad`, `float_after`), each reading 70 to
250, mostly shared; and the regions' last bytes. Each is a check of stage 3, made at the token:
2b (the start masks go) is done there, not apart.

## The marks

Where the loop finds the starts, no mark is a start mask. A mark that holds between tokens
is its own mask, and the record pass lifts each of its bits to the first grammar token at it
or after (an extra's token passed over by its mark): `Records::gaps`, `records::gap_marks`,
the CPU's late marks, Metal's record kernel. A mark of one word is its other terms at the
token (none: the word alone), a token mark of one first word the word alone. Tier 2 takes
every mark from the codes: the any-mark mask goes (`the_marks_are_not_start_masks`). The CPU's
form: 285 operations, from 331. What still reads `code` and the regions among the marks:
`newline` (its `after_comma`, a fill to the next start), `float_after` and `json_bad`.

