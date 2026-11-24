# 0480. Bitlex tier 2 keeps the reference's meaning of calls and token marks

Status: built (audited 2026-11-23). Accepted (2026-11-19; the user: "I want to fix the compiler, not work around it").
Follows 0436 (whose grammar met the three limits), 0407 and 0129. Kotlin's `, ends` (Q346) stays open and Kotlin's.

## Intro

Tier 2 is Bitlex's tree compiled to words: each rule a level, a rule used in several places a
level the words call (`PUSH`, `RETURN`). Bitlex's reference (`tools/bitlex/src/tree.rs`) is what
the words must equal.

Writing the preprocessor's grammar (0436 step 3) met three limits of the compiler. Each was a
place where the compiled words did less than the reference, so the grammar worked around it:

1. **A called rule did not end at the hidden marks after it.** In the reference, a rule that is
   not `nested` takes its caller's follow set: a hidden mark after the rule ends it there (doc 24
   §2.3, `sym`'s `follow`). `grammar::shared` makes such a rule a call, "as if marked nested",
   and a call had no follow. So each directive's tail took its line's end itself.
2. **The verifier refused a called level that took only a mark.** It asked each path from a
   `PUSH` to its `RETURN` to take a token. A level whose path takes a mark (and that mark's bit is
   then cleared, so it can not be taken again at that token) also progresses. So each tail took a
   token first.
3. **A token mark could not tell two kinds that share a first byte.** Its elements test a
   record's kind, the class of its first byte. `#` and `##` both start with `#`, so the
   conditional directives were mask marks, their word found by 15 lead masks at most 16 bytes past
   the `#`.

This decision fixes the compiler, so the grammar now says what it means.

## Decision

**1. A called rule ends at its caller's follow** (`grammar::follow_contexts`,
`StateRow::follows`, `bitlex_tier2::state`).
- For each rule tier 2 calls, the compiler finds the hidden marks after each of its uses, as the
  reference's follow sets: a sequence right to left, a loop's own first marks, through the uses
  of the rules that are not `nested`.
- A level ends at the marks every use has after it (`follows`). Where it may end, such a mark is
  an `AtMark::End`.
- A mark only some uses have after it (`split`) is the same ended at or skipped where the level
  may end and takes no token the mark may hold at. Where it may hold at a token the level takes,
  one call can not end after some uses and read on after others, so tier 2 refuses the grammar,
  saying why and how to change it (`nested`, or one follow).
- Kotlin's grammar has eight called rules whose uses differ in their follows. None of them meets
  that case.

**2. A taken mark is progress** (`stream::validate`). From a `PUSH`, each path is
followed with what it has done: a `SPAN`, or a taken `MARK` (its next word, the mark not kept).
A `RETURN` after a taken mark is progress, as after a token.

**3. A token mark tells a kind by its rows' starts where its first byte is shared**
(`TokenElem::rows`, `Records::kind_rows`).
- Such a kind gets a bit of the records' code, after the marks' and the extras': the OR of its
  rows' starts, the mask tier 2's `ROW` reads.
- Its shared bytes are classes of their own, so the element's kinds hold only the bytes no
  other token starts, and the bit takes the rest.
- `Records::mark_bits` ends the marks' bits at the first such bit, as at the extra's.
- The CPU's token loop sets the bit from the bytes at a token's first byte. It no longer drops
  the bit's output from the rows where another mask (a `ROW`'s) names it, since tier 2 reads it
  there (`BlockNative::read_at_tokens`).
- The interpreter, the Cranelift form, the generated Rust, and the `rir` text and assembler all
  read the field.

**Found on the way: Metal generates `early`** (0330). The trunk's 0330 made the C directives'
byte-order-mark test an `early`, and Metal refused every program that had one, so the
directives' Metal tests failed. An `early`'s value lies in a file's first 16 bits, which Metal's
narrowest lane holds, so it is `a >> k` in the lane's word, as the CPU's `ushr`. A prefix (0335)
stays refused (Q280).

## The grammar after

`bitlex_pp/pp.bitlex`:
- **A conditional directive** is a mark at its `#` with its word after it, on its line:
  `mark if_on = dir_hash, before hash "if" | "ifdef" | "ifndef" without cline`. Its word is
  matched whole, at any distance. The lead masks and the 16-byte bound are gone.
- **A directive** is its word, then what it has (optional), then its line's end, once:
  `retag if ('if' as identifier) [expr: operands] eol_mark`. The `cond_tail` and `text_tail`
  rules are gone.

## Measured

| | Before | After |
|---|---:|---:|
| `pp.bitlex` lines | 401 | 309 |
| R4, files with the lean writer's tape | 3,073 of 3,135 | 3,073 of 3,135 |
| R4, refused (by the compiled words) | 62 | 62 |
| R4, differ | 0 | 0 |

None of the 62 refusals came from the 16-byte bound. The new unit cases test the bound: a word
23 bytes past its `#`, a comment before the word, a `#` alone on its line before an `if`, `ifx`.

Tests:
- `ladder_tests::a_called_level_may_take_only_a_mark`
- `ladder_tests::a_called_rule_ends_at_the_hidden_marks_its_caller_takes`
- `ladder_tests::a_call_whose_uses_differ_where_it_can_go_on_is_refused`
- `ladder_tests::a_token_mark_tells_a_kind_by_its_rows_where_a_first_byte_is_shared`

Each test is checked against the reference and every executor. The compiler's 137 tests pass,
as do Kotlin's 51, .rerail's 30 and the directives' 21.

## What is left

- **Q346 (Kotlin's):** the reference does not read a mark's `, ends`.
- **Q280:** Metal runs no prefix, so JSON's ladder steps are refused there.
- **The 62 refused files.** They are the next step of 0436's R4.
