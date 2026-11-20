# 0467: The generated Rust read against the lean parser's, and the gaps closed in turn

(Was 0431 on Go-C-and-C-support; renumbered at its roll-up into rr-align, roll-up 12, past 0465.)

Status: decided (2026-11-19). Follows 0429 ("the generated Rust beside the lean parser's") and
0430. The user: "evaluate the Rust code generated. Is it reasonably idiomatic and does it have
the same structure as the known-performant lean code? Address the gaps."

## Intro

Kotlin's Bitlex grammar compiles to two Rust files, `kotlin_aot`'s `out/tier1_aot.rs` (the
lexer) and `out/tier2_aot.rs` (the parser). The lean parser is the hand-written Rust they are
measured against: `kotlin_lean/src/lex.rs` and `block.rs` for the lexer, `parse.rs` for the
parser. This decision reads the two side by side, names each place where the generated code
has another structure, and records each gap as it closes.

## The reading

### Tier 1: not idiomatic, and not the lean lexer's shape

`tier1_aot.rs` (41,235 lines, functions `f0` to `f23`) is written by `rerail_clif::rust` from
the Cranelift IR the JIT compiles. It is that IR line by line:
- numbered SSA variables (`let mut v9254: u64 = 0;`);
- raw `read_unaligned` and `write_unaligned` at byte offsets from a frame pointer;
- labelled blocks for every IR block;
- a static `TABLES_DATA` of 5,864 words.

The lean lexer is one loop, a token at a time (`Lexer::lex_run` over `lex_blocks`). A switch on
the first byte picks the token's reader, and the reader finds the token's end.

| Gap | Generated | Lean |
|---|---|---|
| A. Form | IR translated line by line, no names | named functions and readers |
| B. The block body | every mask of the grammar at every byte; about 95 carried words loaded, shifted and stored back each block | a few masks a block (`lex_blocks`) |
| C. Passes | the body, the token loop, then `late_marks` over every block's outputs for the remaining gap marks | one loop |

A and B are one change: a structured emitter for the block program and the token loop, as
`rust_stream` is for tier 2. It cannot come from the IR, which has lost the structure.

### Tier 2: the lean parser's skeleton, with tests the lean parser never makes

`tier2_aot.rs` (81,675 lines, 212 functions) already has 0135's shape: a function a grammar
rule, a call a Rust call, one `&mut S`, structured control flow. The gaps:

| Gap | Generated | Lean |
|---|---|---|
| D. Names | `fn r{pc}`, kinds and words as numbers (`match kind { 50 => … }`, `s.word() != 22`), no indentation | `import_header()`, `T::Dot`, `w::AS` |
| E. Marks | a chain of mark tests before each token's switch | `semi()` or `ends()` where a semicolon is taken |
| F. A token's kind | a switch on the first byte; where rows share one, a test of a tier-1 start mask per row (`s.bit(25, s.start)`: 3,600 places), then a byte trie, then the word compared anyway | `peek()`: the token's kind, from the lexer |
| G. Depth | `if s.depth == DEPTH { … } s.depth += 1; … s.depth -= 1;` at every call | one `deeper()` |

## Gap F, step 1: a string's tokens are met only inside the string (2026-11-19)

Most of the row tests had one cause. `row_modes` decides which rows a state can meet, and it did
not know that a region read in parts has its own tokens: a string's content, its `$name`, its
`${`. So a state outside any string still counted them as rows beginning at `"` or at a name's
byte. It then tested `simple_identifier`'s start mask before every name (621 places of
`s.bit(25, …)`).

Now each reader in parts is a mode, as a toggle's parity is. Its opener's row sets the mode, and a
row that closes the region (its closer, or a `closes` kind such as `${`) clears it. A row
inside needs it set, and every other row needs it clear. `TokenRule::Inside` carries the
reader and whether the row closes (`Lowering::inside_of`). Where every row a state can meet at
a byte goes to one place (a string's content in each of its four regions), no test is made.

`s.bit` tests: 295, from about 3,600. Tapes unchanged: the compiler, Kotlin and Bitlex tests
pass; the corpus is 447 the same, 44 refused, 0 differ; `aot_tapes_are_the_jits` passes on the
447 files.

What is left of F is rows of different kinds that share a first byte:
- an integer and a hex or binary number;
- `"` and `"""`;
- `$` as content and `$name`;
- a char and `\uXXXX`;
- `punct` and `op3`.

Tier 1 decides each, but the record's kind is still its first byte's. The lean lexer gives the
kind (`Tok::kind`), so step 2 is a record kind per row, the reader's retag (`Reader::retag`)
made general.

## Order of the rest

1. F step 2: the record's kind from the row.
2. D: functions named by rule (`Emit::rule_of`), and kinds and words as named constants.
3. G: the depth guard as one function.
4. E: 0429 item 6's remaining part. `dotted`, `acc`, `generic` and `decl_mod` each decide at the
   token where the lean parser decides, in place of a mark that every nested level must end on.
5. A and B: the structured tier 1 emitter.
