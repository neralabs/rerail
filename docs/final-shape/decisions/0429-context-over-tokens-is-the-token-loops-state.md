# 0429: Context over tokens is the token loop's state

Status: decided (2026-11-18), 0489's stage 3 made concrete. Follows 0489, 0177 and 0407.

## Intro

Tier 1 finds where each token starts and ends, and the grammar's marks, by testing every byte of
a file with bitmasks: 64 bytes at once, a bit a byte. That suits the things a lexer asks of
bytes: where a word or a string ends. It does not suit the things a parser asks of tokens: "is
this word `private`?", "are we inside a class's header?". Kotlin's grammar asks many of those.
Written as masks, `private` is seven byte compares and six shifts at each of the 64 bytes. "In a
class's header" is a fill, a run of bits from a `class` word to the token that ends the header.

The lean parser asks the same things once a token: it compares the token's word, and carries
"in a header" as its state. Kotlin's tier 1 is 1,500 operations a 64-byte block on the CPU, about
9,200 machine instructions with the spills: 144 a byte, against about 5 for the lean lexer.
About 420 of the operations are keyword tests (`vis_w`, `head_stop`, `jat`, `ctor_w`, …), and
the fills and marks built on them add more (`tier1_cones`).

## The decision

A mask that is a question about tokens is answered once a token, in the token loop, on the CPU.
Metal keeps the masks (0177: one construct, two lowerings).

1. **Token-level masks.** A mask is *token-level* when, in the program, its value at a byte
   matters only at grammar token starts, and there it is a function of the tokens:
   - a word test: `kw_start & text(W) & ~next(w, |W|)`, a word token whose text is W, is the
     token's record's word, compared (the lean parser's `is_id`). The compiler proves the term
     holds exactly at those tokens (`never`, as 0407's mark words);
   - a token's kind, or its first byte's class;
   - `&`, `|` and `~` of token-level masks, and `prev(…)` of one taken at the token before;
   - a fill whose start and stop are token-level (`fill(A, S)`): one bit of state the loop
     carries over tokens, set at A and cleared after S, as `AsiCtx` is.

   The compiler finds them from the outputs back: each mark and output the program reads at
   token starts only (a mark, a gap mark lifted to the next token, a token test) pulls its
   operands in. A mask read at any other byte keeps its bytewise form, and so do its operands.
2. **The token loop's step.** The token-level masks become a small program the token loop runs
   at each grammar token, after it has the token's kind and word. The program has straight-line
   word compares and bit logic, and a few state bits for the fills. Its results are the marks'
   bits in the token's code, where `late_marks` would set them now.
3. **Gap marks.** A gap mark read at the token after its gap (`Records::gaps`) reads the state
   as it was after the token before, as the lean parser's `asi` reads its context.
4. **Proved, not assumed.** Each lowering rests on a proved fact: that a word test holds only at
   that word's start; that a fill's start and stop hold only at token starts; and that a mask's
   readers read it at token starts only. Where a proof fails, the mask stays bytewise. That is
   a cost, never a wrong tape.

The gate is structural. The CPU body loses each token-level mask's operations (`structure`
prints them), the token loop gains the step's few, and the tapes stay byte-identical
(`aot_tapes_are_the_jits`, the corpus against the lean tapes). Measurements are information
(`jit_against_aot`).

## Order of the work

1. The analysis alone: which masks are token-level, and the operations they would remove.
   `tier1_cones` prints it.
2. Word tests and their `&`, `|` and `~`: the step in the token loop, and the marks set from it.
3. Fills over tokens: the state bits.
4. Gap marks read through the state.

## Step 1's finding (2026-11-18)

`kotlin_parser`'s `token_level_probe` reads the grammar and finds the masks that are token-level
(by the forms above) and that only token-level readers read. **None qualify as the grammar
stands.** The word tests are token-level: `jat`, `ctor_w`, `finally_w`, `where_w`, `else_w`,
`catch_w` and their kin. But their readers are byte idioms that mean token facts, for example
`jat_last = at_after & prev(fill(jat, at_sign))` for "the token before is `return@`…". The
readers fall into three kinds:

| Kind | Examples | What it means | The lean parser's form |
|---|---|---|---|
| A lexing decision | `jat`, `jat_last`, `p_start`, `three`, `asq` | whether `return@` and `===` are one token | the lexer's reader for the token |
| The fallback's comment placement | `goes_now`, `awe_word`, `else_odd`, `ctor_reach`, `ann_ctor` (the escapes "a block comment before a token that goes on the line", …) | where tree-sitter places a comment that the parser would place otherwise | the parser places the comment as tree-sitter does; since 0490 a mark's `settles` and `drops` can say that |
| Context over tokens | `class_head`, `line_mod`, `hdr_line`, `decl_stop` | being in a class's header, or on a line that starts with modifiers | a rule's level in tier 2 |

So 0429's lowering applies only once each reader says what it means. That is the grammar's work,
as 0490 did for the next token's back fills. The order of the work becomes:

1. The escapes of comment placement go: the grammar places those comments as tree-sitter does,
   by `settles` and `drops` where the lean parser decides. Their cones leave tier 1, and the files
   they refuse now parse.
2. The lexing decisions become their tokens' rules.
3. The context over tokens becomes tier 2's levels, or token marks.
4. What is left of the word tests and fills becomes the token loop's step, as above.

`tier1_cones` gives each step's gate: the operations each removes from the CPU body.

## Where it stands

Before the work (2026-11-18, after the late marks' merge walk, load about 12): the CPU form has
1,500 operations a block. Tier 1 ahead of time takes 443M instructions and 88M cycles, against the
lean lexer's 54M and 15M. Tiers 1 and 2 take 799M instructions and 179M cycles, against the lean
parser's 299M and 60M.

## The swallowed comments (2026-11-18)

The lean lexer's `asi` swallows a block comment on the line after a line break when two things
hold: the parser's state takes a semicolon there, and a word with `a`, `e` or `w` follows. The
grammar's `semi` already dropped it (`drops swallow_at`), and a mark's `drops` applies only where
the mark is taken. So `swallow_at` no longer asks what ends the line before. The escape's other
cases are gone:
- after a header's line (`package`, `import`);
- after a line that ends with `?`, `>` or an operator;
- in a call's arguments, where no semicolon comes and the comment is any other.

The masks that made those cases (`ok_lf`, `brace_lf`, `sw_blk`, `br_blk`, `hdr_line`,
`hdr_gap`) are gone too.

One more change follows the lean parser. With none other pending, a drop now ends the nodes
closed next at the token after the comment, where the semicolon of no width is (`Asi::Swallow`
sets `last_end` there). They used to end at the comment's end. A header's node shows it: it
holds its semicolon, so it ends at the word. The change is the same in the interpreter, the Rust
stream, Cranelift and Bitlex's reference.

The corpus: 442 the same, 49 refused, 0 differ, from 431 and 60. The CPU form: 1,413 operations
a block, from 1,500. Measured (load about 16, over the 442 files): tier 1 ahead of time took 416M
instructions and 85M cycles. The lean lexer took 55M and 16M.

The escape for comments that go on the line had the same breadth. Its `after_blk` held after any
`*/`, one in a string (`"*/*"`) or a comment inside a line (`0 /*repeat*/,`) included. The
comment the lean lexer reads is one that starts the line after a line break (`asked_blk`). All
its tests now ask for that comment (`after_asked`). The corpus: 447 the same, 44 refused,
0 differ.

## What is left, and the construct each needs (2026-11-18)

`tier1_cones` now counts what the CPU body keeps: the cones of every output except the record
pass's starts and lasts. It found 1,417 operations, against 1,413 in the CPU form.
`RERAIL_CONES_OF=<statement>` lists the outputs that read a statement's operations. The
measurements behind the table:

- The settling marks' masks (`fill(m, gs)`) and the code's token marks keep `grammar_starts`,
  but only 5 operations depend on it there.
- `jat` and `two_start` are read by the token end rules' masks (`mask_of`, under the "stream
  mark" label), not by the marks.

| What is left | Operations | What it needs |
|---|---|---|
| The `return@` and `as?` tokens: `simple_identifier` ends at the mask `last name_last` (`jat`, `asq`, `name_tok`, `name_last`) | about 90 | two tokens, as the lean lexer reads them, joined by tier 2 where its rule takes them (the lean parser's `adjacent(0)`): done by 0451's gap B step 1. The lean lexer has no such word reader; this row's first plan was wrong |
| `punct`'s pairs where another kind starts at the same byte (`.` with a number): `EndRule::Pair` reads `two_start`, plus `three`, `p_start` | about 80 | the byte's rule picked by its next byte (a number's digit or a pair's second byte), as the lean lexer's switch does |
| The class header's context (`class_head`, `head_stop`, `class_kw`, `ctor_w`, `vis_w`, `line_mod`, `ctor_stop`) | about 250 | `semi` split no more. The class rule takes a hidden mark before `[primary_constructor]`, as the lean parser asks once after the name. `class A⏎private⏎constructor()` (the lean lexer reads a modifier and `constructor` on one line) still needs a refusal, so a token pattern needs a test for a line break between two tokens |
| The comments that go on the line (`goes_now`, `ann_blk`, `ann_ctor`, `else_next`) | about 65 | a mark clause that makes the pending comment zero-width at the token where the mark is tested and not taken, as the lean lexer's `lost` does. No corpus file hits the escape now |
| The regions and strings (`regions hidden`, `string_content`, the template's tokens) | about 250 | already walked (`Walk`). What remains is the masks the readers' parts read; not yet looked at |

## The generated Rust beside the lean parser's (2026-11-18)

This section replaces the operation counts above as the measure of the work (the user: the
goal is generated Rust shaped like the lean parser's, "not operations, not instructions, not
cycles, not time, not MB/s"). Read side by side: `kotlin_aot`'s `out/tier1_aot.rs` and
`out/tier2_aot.rs` against `kotlin_lean/src/lex.rs` and `parse.rs`.

The lexer:

1. **Passes.** `lex.rs` is one loop, a token at a time. A switch on the token's first byte
   picks its reader (`word_end`, a number's, a string's, a comment's). The generated tier 1 has
   three parts. First, a body for each 64-byte block that computes every mask of the grammar at
   every byte. Then the token loop, which jumps on the record's kind and reads ends from those
   masks. Last, a pass over every block's outputs that sets the marks' bits on the tokens
   (`late_marks`).
2. **Keywords.** `lex.rs` reads a word and looks it up once (`w::…`). The generated body spells
   each keyword the context needs as byte compares and shifts (`text("private")`, `vis_w`,
   `head_stop`, …), beside the record's word.
3. **Automatic semicolons.** The lean parser calls `asi()` where its state takes one, and that
   reads the bytes after the token then. Tier 1 computes the semicolon's mask at every line
   break, its comment cases and the class header's context (`class_head`) included, before
   the parser runs.
4. **`return@`, `as?`.** The lean lexer's word reader decides them. Tier 1 decides them by the
   end mask `name_last`.

The parser:

5. **Rules.** Both are a function a rule (`r<pc>` against `import_header`, `class_rest`, …).
6. **Marks at every token.** At a token, the generated rule tests each live mark in turn, each
   by its pattern function (`patterns_9`, `patterns_13`, …). The refusing ones come first, then
   the rest. The lean parser asks nothing it does not need. It asks `semi()` or `ends()` where
   it takes a semicolon, and peeks where a rule chooses (`peek() == T::Dot`, `is_word(w::AS)`).
7. **A name.** Where the lean parser calls `simple_identifier()`, one function (`t.kind ==
   Name`, `soft(t.word)`), the generated rule inlines a `match kind { … match word { … } }` over
   the word-bearing kinds (34 to 48) and their words, at each place a name may come.

What unblocks what: 6 and 3 are the same change seen from the two tiers. Where tier 2 asks for a
semicolon as `asi()` does, tier 1 needs no gap marks, and no `class_head` or comment context
under them. Without those, tier 1's body keeps the readers' masks (1, 2). Then 7: a name as one
call.
