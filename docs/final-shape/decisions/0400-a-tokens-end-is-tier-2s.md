# 0400: A token's end is tier 2's

Status: decided (the user, 2026-11-18: page 39's item 3, "find token ends without a second
position list", chosen by shape: "what brings us closer to the desired shape"). Follows 0398
and 0399. In progress.

## Intro

**The problem.** Tier 1 writes two position lists for every token: its first byte (the
starts) and its last byte (the lasts). Page 39 measured the lasts' extract at 123
instructions of `14-json`'s hot loop. simdjson's stage 1 writes starts only, and stage 2
finds where each token ends as it parses it.

## The decision

Tier 1 writes the starts only. Tier 2 finds a token's end when it takes the token (`TOKEN`),
from a rule chosen by the token's first byte, evaluated on the bytes, word-parallel.

The rules, each one Bitlex proves for a row from its `start` and `last` masks:
- **Fixed `n`**: the token is `n` bytes long (`"`, punctuation, an escape). Proof: at the
  row's start, its last is `n - 1` bytes on.
- **Next start**: the token ends where the next token starts (string content, which an escape
  or the closing quote always follows). Proof: every last of the row is followed by a start.
- **Run of a class `C`**: the token is the run of bytes in `C` from its first byte (a word: a
  number or a keyword). Proof: the row's bytes are in `C`, and the byte after its last is
  not. Tier 2 makes the run's end with one 16-byte load and a count of trailing ones, then 64
  bytes a turn for a longer run. No loop over bytes.

A row with none of these keeps tier 1's lasts, and the program keeps the extract.

## Requirements, a test each

- R1. Bitlex chooses a rule for each of `14-json`'s rows, and its tier 1 has no lasts' extract
  (`compile_tests`: the block program extracts the starts only).
- R2. The rules are proven, not assumed. A row whose last the rules cannot prove keeps the
  extract (a ladder step with such a row compiles with its lasts).
- R3. The interpreter's records equal the old records on every ladder step and document
  (`tier2_tests`: the nodes equal with and without the lasts).
- R4. The CPU equals the interpreter, with runs longer than 16 and 64 bytes and runs ending at
  the file's end (`tier2_tests`, a long-token test).
- R5. Every executor of tier 2 takes the rules, or the program keeps its lasts: the
  interpreter, Cranelift, and the Rust tier 2 (`rust_stream`, the `.rr` grammar's AOT).
- R6. The hot loop loses the lasts' extract (`json_shapes`: no second position list).
