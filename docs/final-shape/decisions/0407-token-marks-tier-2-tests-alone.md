# 0407: Token marks that tier 2 tests alone hold no bit of the code

Status: decided (2026-11-18, while moving Kotlin off `back_fill`, 0416). Changes 0129 (token
marks tested from the records).

## Intro

**The problem.** A record's code is 32 bits: the token's kind in the low 16, and one bit for each
mark in the high 16 (`records::MARKS`). Kotlin's step 35 uses all 16: 14 marks and the 2 extras'
bits. Five of the 14 are token marks (`mark K = tokens …`, 0129). Their bit was the grammar's
token starts, set at every token, and tier 2 tested their run of tokens anyway. The bit carried
nothing, but tier 1 made it and the code held it. With no bits left, the grammar merged unrelated
cases into one mark (`dotted`, `named`, `generic`), and 0416 cannot turn the back fills into
more token marks.

The lean parser keeps nothing per token for its peeks: it reads the tokens ahead where a rule asks.

## The decision

The code holds the marks up to the last one that tier 1 makes (a gap mark, from a mask) or that
refuses wherever it holds (`TokenMark::refuses`, tested once a token by its bit). A token mark
after them has no bit and no mask (`Records::mark_bits` counts the marks in the code). Tier 2
makes its bit of `live` at every grammar token, as it does for a mark past the code
(`rest_bits`), and tests its tokens where a state asks. Its first token is an element of its
pattern too (offset 0), so no word test is needed beside it. At the stream's end it does not
hold.

A mark's order still decides which marks are done when one is taken: declaration order. So a
token mark that must come before a gap mark (Kotlin's `blk`, before the lambda's marks) stays in
the code. The grammar puts the others last.

The rule applies only with the token loop's records. Without them every mark is a mask tier 2
reads, and the any-mark mask (`TOKEN m=1+k`) would leave a mark with no mask never live.

`live` is 32 bits in the interpreter and 64 in the compiled code, so a grammar has up to 32 marks
in all. Up to 16 of those are in the code, with the extras' bits.

## Kotlin

Step 36 puts `decl_mod`, `use_site`, `file_at` and `trail` after `generic`. The code holds 10 marks
and the 2 extras' bits, and 4 bits are free. The corpus is unchanged: 429 files the same as the
lean parser's tapes, 62 refused, 0 differ.

## Tests

- `ladder_tests::a_gap_mark_is_decided_by_the_token_after`: its `wild` (`tokens "." "*"`), after
  the gap marks, is past the code (`mark_bits` 2, its pattern order 2). The reference's nodes are
  the interpreter's and Cranelift's on the grammar's sentences, their mutations and fixed
  documents (`import a.b.*`, and `import a`, a comment and `.*` on the next line, refused).
- `stream::validate_token_marks` refuses a refusing token mark past the code.
