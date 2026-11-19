# 0431: The generated Rust read against the lean parser's, and the gaps closed in turn

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

## G and D: one `deeper()`, `else if`, indentation and the grammar's names (2026-11-19)

These two change only the emitter (`rust_stream`), and the tapes are the same.
- A call is `s.deeper(move |s| rule(s))?`, one function as the lean parser's `deeper()`.
- A test whose else is one `if` chain is written `else if`.
- Each rule's body is indented by its braces.
- The program carries the grammar's names (`Words::names`, text `name rule R "N"`, `name kind
  #K "N"`, `name token K "N"`). A rule's function is named for its rule (`fn import_header`,
  `fn class_body`). Node kinds are `k::…`, record kinds `t::…` and words `w::…`, and bytes are
  written as literals (`b'.'`).

Indentation shows what is left. Before E, the median function nested 22 levels deep and the
deepest 43; the lean parser stays near 5.

## E: each decision at the token where the lean parser makes it (2026-11-19)

The four marks that are not hidden (`dotted`, `acc`, `generic`, `decl_mod`) each made every
nested level that may end test them before its switch (0430 step 3). The lean parser tests each
only where its loop would take the token. `user_type` asks `i != type_stop` at a `.`.
`declaration_ahead` decides at a modifier's word, and `accessor_ahead` at `get` or `set`. Three
changes give tier 2 the same shape:

1. **A mark's words.** A mark holds, as tier 2 tests it, only where one of its `before`s does
   (`patterns_hold`). So its first bytes are within the first tokens of its `before`s, and where
   each of those is a word, the mark holds only at those words (`Mark::words`). A token mark's
   words are its first element's. `acc` says its words: `before "get" | "set"`.
2. **A test in the arm of its word.** An End test of a mark with words moves into the switch,
   onto those words' entries. A one-byte word (`.`, `{`) goes on its byte's target instead. A
   refusal of such a mark moves the same way. A word is named in the switch only to hold its
   test, and is taken out again where it holds none. A token that no arm takes already ends a
   nested level (the switch's default). A test moves only past the tests of marks apart from it:
   their first bytes, or their words, never meet.
3. **Two decisions apart.** `dotted` also held at a receiver's first token, and `generic` at a
   function type's receiver's first name. Each is a decision a rule makes where the declaration
   or the type starts, as `receiver_end` does. Each is now a hidden mark of its own: `recv`,
   taken by `receiver`, and `fn_recv`, taken by `type`. What is left of `dotted` holds at `.`
   and `{`, and of `generic` at `<`, `(` and `suspend`.

The four marks' tests went from 750–1,060 places each to 283 (`dotted`), 205 (`acc`), 381
(`generic`) and 290 (`decl_mod`), each in the arm of its own word. Tier 2's program went from
17,815 words to 10,765. `tier2_aot.rs` went from 71,093 lines to 53,721. `import_header` now
begins with its token's switch. Tapes unchanged: the compiler tests (128) and the Kotlin tests
(51) pass; the corpus is 447 the same, 44 refused, 0 differ; `aot_tapes_are_the_jits` passes.

The names are still split into record kinds by first letter (`SIMPLE_IDENTIFIER_15`, …). One
switch causes it: in a string after `$`, `interpolated_identifier` and `string_content` both
begin at a letter, and tier 2 tells them apart with `ROW` tests, a byte at a time. That is F
step 2.

## F, step 2: the token's kind from its row, as the lean lexer's `Tok::kind` (2026-11-19)

After step 1, tier 2 still told rows that share a first byte apart by a test of tier 1's start
mask per row (`ROW`). There were two kinds of such rows.

1. **Rows of keywords beside one other row.** At `.` these are `punct`'s `.`, `op3`'s `...` and
   a real literal's `.5`. At `<` and `=` they are `punct` and `op3`. Where every row met at a byte
   but one holds only keywords, the record's word already tells them apart. So the switch names
   each keyword, and a token that is no keyword is the other row's (`by_words_of`). This is the
   lean parser's `peek() == T::Dot`.
2. **A string's parts.** After `$`, a name (`interpolated_identifier`) and the string's text
   both begin at a letter. A `$` alone is text too. A part whose first byte another token of its
   region also begins with now has a record kind of its own (`ReadPart::kind`). That is the
   lean lexer's string reader, which says what it read. The compiler gives the kind a byte that
   no token begins with, one byte per part's name, whichever reader reads it. The CPU's walk
   writes that kind into the part's code and keeps its word. The interpreter and Metal take it
   from a retag (`Retag::part`) that keeps the word too. Tier 2 meets the part's keywords at that
   byte (`Row::moved`). A keyword read as a part keeps the part's kind, so the row reserves none
   for the state's end. A state that cannot meet the part (outside its region, by
   `row_modes`) refuses that byte, so the emitter names its words only where the part comes.

`ROW` tests: none, from 295. The names are one record kind, `t::SIMPLE_IDENTIFIER`: the kinds
`SIMPLE_IDENTIFIER_15` and its kin are gone. Each reader's and part's kind is named for its
row (`t::INTERPOLATED_IDENTIFIER`, `t::LINE_COMMENT`). Tier 2's program went from 10,765 words
to 9,514, and `tier2_aot.rs` from 53,721 lines to 40,574. The 96 `s.bit` tests left are marks'
`drops`, which test where a comment lies, not a token's kind.

Tapes unchanged: the compiler tests (128), Kotlin (51), Bitlex (11) and `rr_aot` (15) pass; the
corpus is 447 the same, 44 refused, 0 differ; `aot_tapes_are_the_jits` passes.

## Order of the rest

1. A and B: the structured tier 1 emitter.

D, G, E and F are done (above).
