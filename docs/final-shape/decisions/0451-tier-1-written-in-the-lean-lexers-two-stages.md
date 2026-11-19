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

## Kinds that share a first byte: the row's kind, not a code bit (2026-11-19)

Go-C-and-C++'s 0445 (point 3) and this branch's 0431 F step 2 solve one problem: a token mark,
or tier 2, telling apart two kinds that start with the same byte (the preprocessor's `#` and
`##`, Kotlin's string parts). 0445 adds a bit to the record's code per such kind, the OR of its
rows' start masks (`Records::kind_rows`): a mask made in stage 1 at every byte, and a code bit
from the 16 the marks share. F step 2 gives the record its row's kind, as the lean lexer's
`Tok::kind` does: the token loop knows the row from the path it took (a pair table's second
byte, a reader's arm, a string's part) or from the token's word (keyword rows), so the kind is
one table lookup and no mask.

**Decided: the row's kind.** It leaves stage 1 the lexer's masks (the direction of this
decision's gap B), spends kind values, which are plentiful, not code bits, which are not, and
is one mechanism: it already removed every `ROW` test from tier 2. At the merge with 0445:
- a keyword row gets a record kind of its own from its word, in the token loop, extending
  `ReadPart::kind` past the string parts;
- `TokenElem` tests that kind; `Records::kind_rows`, its code bit and its masks go, and so does
  `BlockNative::read_at_tokens`'s use for it;
- 0445's test `a_token_mark_tells_a_kind_by_its_rows_where_a_first_byte_is_shared` and the
  preprocessor's cases stay, ported to the kinds (Q282).

## `rust_block` names every field it reads

`Gen::new` takes the records apart field by field (`Records`, `Reader`, `ReadArm`, `ReadPart`,
`TokenCheck`, and each `CheckThen`), with the reason it ignores each lowering hint. A field or
a test's ending added on another branch (0413's `run … once` and `CheckThen::Cut`, 0445's
`kind_rows`) fails the build there until the emitter writes it or refuses it, never a wrong
tier 1 that only `aot_tapes_are_the_jits` would catch.
