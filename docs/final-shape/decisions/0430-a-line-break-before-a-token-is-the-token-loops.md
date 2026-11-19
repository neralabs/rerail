# 0430: A line break before a token is the token loop's, as the lean lexer's `nl`

Status: decided (2026-11-18), the first step of 0429's "marks at every token" and "automatic
semicolons" (items 3 and 6). Follows 0429, 0177 and 0406.

## Intro

Kotlin ends a statement at a line break, unless the next line goes on with it. The lean lexer
notes, for each token, whether its gap held a line break. That is `Tok::nl`, set while
`Lexer::skip` passes the blanks and comments. The parser asks the full question only where its
state takes a semicolon: `t.nl && self.ends()`. Only `asi()` reads the bytes after the token.

The generated parser does it the other way round. Tier 1 computes the semicolon as a mask at
every byte of every block (`lf_end`, and `after_semi` under it). A pass after the tokens then
lifts each bit to the next token (`late_marks`), and tier 2 tests the mark at every token.

## The decision

1. **The grammar says what the lean lexer notes.** `semi`'s mask is a line break in code, the
   gap's (`lf & code`). The `;` before it is the grammar token before (`unless after ";"`), not a
   fill over the bytes after the `;`.
2. **The token loop notes it.** On the CPU, a gap mark whose mask is a class of blank bytes
   in code is set while the token loop skips the gap. That is where the lean lexer sets `nl`;
   there is no mask and no late pass for it. The compiler finds such a mark by its mask's shape
   (a byte class, and `code`), never by its name. Metal and Bitlex's reference keep the mask
   lifted to the next token, as now.
3. **Tier 2 asks where its rule takes one.** This comes after steps 1 and 2: the mark is tested
   where a rule can take a semicolon, as `ends()` is, not at every token.

Each step keeps the tapes the same: the kotlin-parser tests, the corpus against the lean
parser's tapes, and `aot_tapes_are_the_jits`.

## Step 1b: the class header's line break is the class rule's (2026-11-18)

Step 2 needs `semi` to be the line break alone. It was two marks, `lf & code & ~class_head` and
`lf & code & class_head`, where `class_head` is a fill from `class` over the header: context
over tokens, made at every byte. The lean lexer has no such mask. After a class's name, the
parser asks `asi` with `AsiCtx::ctor`, and a next line that starts with `constructor`, or a
modifier or annotations before it, goes on.

Now the class rule asks the same, once, after the name: `[head_ctor] [primary_constructor]`.
`head_ctor` is a hidden mark of the line break (`lf & code`), declared before `semi`, with the
lean lexer's two shapes as its `before`s. `semi` is one mark. The escape "`constructor` after
modifiers at a line's start" and its `ctor_reach` are gone. `class_head` is left only in the
escape for a block comment before a token that goes on (0429's going-on comments).

Two constructs made it work:

1. **A level that took a mark at a token does not end there at a hidden `ends` mark** (Q228).
   After `head_ctor` is taken, `semi` is still live at the same token, and `class_rest` may end
   there (all its parts after are optional). So `semi`'s `ends` ended the level, and the
   statement took `semi` instead. The lean parser makes one decision at a line break. So in a
   state that the level enters only by taking a mark (no call, no return lands there), a hidden
   mark that ends levels does not end it (`bitlex_tier2`, `after_mark_only`). A mark that is
   not hidden still does. Ordering every end after every arm was tried first and is wrong: a
   token mark's arm (`primary_constructor`'s modifiers at `internal class B`) then wins where
   the level must end.
2. **`E without M` in a token pattern.** `class A⏎private⏎constructor(x: Int)` is refused by
   the lean parser: `modifier_then_constructor` reads the modifier and `constructor` on one
   line. A pattern sees tokens, not line breaks, so `"private" "constructor"` took it. An
   element may now say `without M`: it holds only at a token whose record has no bit of mark
   `M`, the lean parser's `!peek_at(j).nl`. Kotlin's modifier shape is
   `"private" … "constructor" without semi`. This is one more term in each executor's element
   test: the interpreter (`TokenElem::holds`), Cranelift, the generated Rust, and the reference
   (the mark's bit as the record holds it: its mask lifted to the next grammar token, `unless
   after` applied). A mark `without` reads stays in the code on every target (`Records::read`,
   text `recread`): `lazy_marks` leaves it out. `ladder_tests::a_token_without_a_mark` checks
   every executor against the reference; `a_taken_mark_is_the_line_breaks_decision` checks
   rule 1 (it fails without it).

Tests: `constructor_lines_are_the_lean_parsers` (six more cases), and
`a_constructor_off_its_modifiers_line_is_refused`. The corpus is unchanged: 447 the same, 44
refused, 0 differ. `aot_tapes_are_the_jits` passes on the 447 files.

Bitlex's reference does not implement `, ends` (Q229). It accepts the refused texts above,
because its `past_marks` skips the hidden `semi` inside `class_rest`, and its
`primary_constructor` reads `private⏎constructor`.

## Step 2: the token loop notes the line break while it skips the blanks (2026-11-18)

On the CPU, `semi` and `head_ctor` are now the token loop's, as `Tok::nl` is the lean lexer's.
At each token the loop tests the line break class over the blank run before it: from the end
of the token before to the token's first byte. That is one word's range in the block in
registers, and a word at a time from the bytes when the run started in an earlier block. An
extra's bits go on to the grammar token after it, as `skip` ORs `nl` over the comments it
passes. The body makes no mask for the two marks, and the pass after the tokens (`late_marks`)
no longer sets them. Metal and the interpreter keep the mask lifted to the next token.

The compiler finds such a mark by its mask's shape, never by its name, and proves it
(`bitlex_masks`, `Records::blanks`, text `recblank`). A gap mark qualifies when its mask `M`
holds only at bytes of one class `C` (its first bytes), and two things are proved, up to a
refusal:

1. No token's byte holds `M`. A token's bytes are known from its end rule, as the lean
   lexer's readers know them. A `run`'s are its first byte and its run mask's. A `pair`'s are
   its first and the next. A reader's are its regions' fills. A token inside a region (a
   string's part) is its first and last bytes and the region between. A token of fixed length
   is its bytes. For a lexer's token the class must miss them; for a region's, the mark.
2. Each byte of `C` that `M` leaves out is a region's, that is, a token's.

Then `M` between tokens is `C`, and the loop's test is the mask's value. A `last L` token's
bytes are a fill from its first byte to its last, which the prover cannot show. So Kotlin's
names and numbers are now `run` tokens, the lean lexer's readers' shape:
`simple_identifier = start word_start run name_more`, where `name_more` is the word's bytes
after the first, plus the `@` of `return@` and the `?` of `as?`. The numbers run over `nb`, or
over a `0x`/`0b` letter and digits.

The proof found one real difference from the lean lexer. A character's `\u` took any four
bytes after it, so `'\u1'` and a line break after it made one token. The lean lexer's
`char_escape_end` refuses a `\u` without four hex digits, and now the grammar does too.

The loop tells an extra by its mark in the code, so a blank mark is read so only where the
extra's mark is the loop's: its reader's, or its first byte's. A mark set after the tokens is
too late (`blank_marks`). The lifted gap marks the loop sets from the bytes (`loop_marks`) rest
on the same condition; they now check it too (`extra_known`, Q230).

Test: `ladder_tests::a_gap_mark_read_from_the_blanks`. Its grammar has line breaks in block
comments and strings, between extras and after `;`. The test checks the reference, the
interpreter and Cranelift on its sentences, mutations and fixed texts, and that the CPU loop
reads the mark from the blanks. Kotlin: its tests pass, and the corpus is unchanged (447 the
same, 44 refused, 0 differ). `aot_tapes_are_the_jits` passes on the 447 files.

The other gap marks (`semi_close` and the escapes' marks) are still lifted after the tokens.
Step 3 (tier 2 tests `semi` where a rule takes a semicolon) is next.
