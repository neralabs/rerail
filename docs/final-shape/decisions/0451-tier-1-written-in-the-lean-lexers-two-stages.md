# 0451: Tier 1 written as Rust in the lean lexer's two stages

Status: decided (2026-11-19). It closes gap A of 0431 and states what is left of gap B. The
user: "please redo the tier 1 rust generation".

## Intro

Kotlin's tier 1 ahead of time (`kotlin_aot`'s `out/tier1_aot.rs`) used to be the JIT's Cranelift
IR translated line by line (`clif_rust`), in 41,235 lines:
- numbered SSA variables;
- loads and stores at frame offsets;
- a label for each IR block;
- a static table of 5,864 words.

It now comes from the block program and its records (`rerail_compiler::rust_block`). It has the
two stages of the lean lexer (`kotlin_lean`'s `block.rs` and `lex.rs`) and is 3,948 lines.

## The shape

1. **Stage 1, the masks.** A loop over the 64-byte blocks, dispatched over `rerail_simd8`'s
   backends as `block.rs` is (`dispatch!`, `S::load`, `S::classify`, `S::eq_mask`). A block's
   values are named `let`s:
   - `is_quote` for a byte compare, `class3_05` for a class;
   - a stored output by what reads it: `blank`, `simple_identifier_run`, `quote_opens`,
     `line_comment_tag`, `mark4`, `tier2_17`;
   - `v{i}` for any other value.

   A carried operation keeps its state in a local across the loop: `last_x` for `prev`, a
   carry for each `fill` and `escaped`. The masks come out as `struct Masks`, a `Vec<u64>` each,
   with each output's delay taken off.
2. **The regions** (strings, comments, templates) come from a walk of their own between two
   block loops. Stage 1a makes only the walk's openers and closers. `regions()` goes from
   region to region, as `BlockProgram::walk_group` does. Stage 1b reads what the walk made.
3. **Stage 2, the token loop** (`Tokens::run`). It goes past the blanks (`next_clear`). Then
   `match END[first byte]` picks an arm a rule, commented with its token's name: a run, a pair, a
   last mask, a pair table, or a reader. A reader has a function of its own (`read_quote`), and a
   region read in parts has one more (`read_quote_parts`, as the lean string reader).
   `Tokens::token` writes a record:
   - its kind, from `KIND` or a retag;
   - its word, from the perfect hash;
   - its marks;
   - its gap marks: `hits`, `unless`, and the hits carried to the next grammar token.

   The bits tier 2 tests itself are left out, as the CPU's tier 1 leaves them
   (`lazy_code_bits`). A token the file is refused at ends the loop before its record, as the
   JIT's loop ends.
4. **The entry.** `tier1_entry` has `native_cpu_block::Entry`'s signature and its own
   `repr(C) Row`. It writes the `Batch` the JIT's tier 1 writes:
   - the records, by the layout;
   - the counts and the refusal flag;
   - the masks tier 2 reads from tier 1's words (`BlockNative::masks`), leaving out those tier 2
     reads from the bytes (`records_made`).

   The JIT also writes masks that nothing reads. The generated code makes none of them.
5. **One source, two readers.** The build script and `aot()` write the source with the same
   expression (`src/tier1_source.rs`), and `aot()` checks the fingerprint `SOURCE`. The table
   check and the `STRUCTURED` assertion are gone. `rust_block` refuses what it does not write yet:
   - records without a skip;
   - a prefix, a gap rule, or rows;
   - a fixed point that is no walk;
   - `BackFill`, `Pair`, `Prefix`, `Phase`, `Position`, `Cmp`, `Bit`.

## A bug the comparison found

`aot_tapes_are_the_jits` now compares two independent tier 1s, so it found a JIT bug. At `/`,
the comments' reader goes on to its other rule, a pair table. But the JIT's token entry took
the pair row from the byte's own rule, which is the reader's, so the row was 0. `/=` was then
read as `/` and `=`. The entry now takes the row of the rule the reader goes on to, as
`pair_of` already did for the codes. The case is in `statements_and_expressions_are_the_lean_parsers`.
The one corpus file with `/=` now parses: 448 the same, 43 refused, 0 differ.

## What is left: gap B

Stage 1 still makes about 1,700 values a block, 383 of them carried words. Its 99 stored outputs
are the marks, the token tests, the untaken bytes, tier 2's masks and the totals, and between
them they reach almost every operation of the program. The lean lexer's `lex_blocks` makes a few
class masks. Closing B moves those questions out of the block loop into the token loop, at the
token where the lean lexer asks them. That is 0429's order of work, and it is now visible in the
generated source, a named `let` each.

## Tests

- `aot_tapes_are_the_jits` passes on all 495 corpus files: the records, counts and masks tier 2
  reads are the JIT's, and the tapes are the same on the 448 it parses. For a file tier 1
  refuses, the records are compared only up to the count.
- `structure` and `tier1_alone` pass.
- The compiler tests (128) and the Kotlin tests (51) pass.
