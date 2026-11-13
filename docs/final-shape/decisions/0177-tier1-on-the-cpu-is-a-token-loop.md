# 0177: Tier 1 on the CPU is a token loop, as the lean lexer is

Status: decided (2026-11-13); stage 1 building.
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

## The stages

Each stage ends with its gate met, or stops (see Governance). Numbers are tier 1 over
the corpus, ahead of time (AOT) and through the JIT, from 465.9M and 501.5M.

1. **Tokens by their runs.** A token kind declared by its first bytes and its run
   (`token identifier = start alpha run ident`) has its end from the run, at its start.
   The run-ended tokens need no `last` mask and no `lasts` extract; the record pass
   computes their ends. Punctuation's end is its start, or the byte after at a pair.
   Gate: no `lasts` extract on the CPU; tier 1 AOT at most 420M.
2. **Rare tokens read at their start.** The walk gives each string, comment and backtick
   literal its record, and the starts after it in its region are not starts. The other
   masks stop reading `code` (the bytes outside regions) where a token loop needs it
   not. Gate: the CPU body below 120 operations; tier 1 AOT at most 330M.
3. **Checks at the token.** A byte no token takes is a kind of its own (`BAD`) in the
   first-byte table. A number's digits and `.`, a `\u` escape, JSON's rules, the
   line marks (a newline between two tokens) are tested at the tokens they concern,
   in tier 1's loop or in tier 2 (`CHECK`, 0133). Gate: the CPU body at most 40
   operations; tier 1 AOT at most 290M, 1.5 times the lean lexer.

After stage 3 the CPU lowering of today's masks is gone from the CPU path.

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
- **Measured one way.** A stage is judged by `jit_against_aot` (nine rounds), AOT and
  JIT both, with the load noted. The split probe (`rr_tier1_split`) moves by about 8M
  under load: it diagnoses, it does not decide.
- **Stop rules.** A stage that misses its gate by more than a tenth stops: the shape
  is looked at again and the user told, not trimmed. A step that makes AOT or JIT
  worse by more than the noise is not committed as progress.
- **Logged.** Each step and each dropped try is a row in docs/parser-generators/35;
  docs/parser-generators/36 holds the design as built; this record its stages' status.
