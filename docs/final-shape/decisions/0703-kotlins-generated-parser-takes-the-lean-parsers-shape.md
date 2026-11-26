# 0703: Kotlin's generated parser takes the lean parser's shape

Status: being built (the user, 2026-11-25: "We will need to make the code match more to get our performance back,
without making the Rerail or Json code match less", then "address all of these"). Follows 0681, 0685 and 0696.

## Intro

Kotlin's generated parser (`kotlin_aot`) is about 1.7 times the lean parser's CPU (`kotlin_lean`), its tier 2 1.4 to
1.65 times the lean parser's own work and its tier 1 about 6.5 times the lean lexer (8251c7bb7b). A count of the
two codes' shapes, tier by tier, found the rules themselves the lean parser's size (137 functions in 4,000 lines
against 151 in 3,522) and five differences, all Kotlin's but the last: rr's and JSON's generated code has none of
them.

- Tier 1 is the two-pass form: a whole-file stage 1 of 144 masks, then a token loop over them, then about 30 more
  whole-file passes for its refusals. rr and JSON get the lean lexer's one-pass form. Kotlin's strings and templates
  trip every condition that refuses it (`bitlex_lex_rust`, `fused`).
- Tier 1 works out ten marks a token (the line breaks, the semicolons, `as_open`, …) and tier 2 checks and tracks
  them in every rule (523 tests, 693 writes of `asks` and `marked`) and on every token (`open_mark`). The lean lexer
  gives a token one line-break flag, and the parser asks the automatic semicolon only where it reads one.
- The marks' lookahead is a function a state: 2,164 functions, 76% of tier 2. The lean parser asks the same with a
  test of a few tokens and, for the hard case, a trial read of its own expression, rewound.
- Precedence is ten functions, each its own copy of the operator loop. The lean parser has one loop with a
  minimum precedence. rr's lean parser has a function a level too (`term`, `product`): that shape stays where the
  grammar is written so.
- A name that is no hard keyword was tested against a list of 26 words, 673 times. The lean parser numbers its
  words hard keywords first and tests one compare (`ident_word`).

## Decisions

1. **A row's hard keywords are numbered together** (`bitlex_tier2::lower`: the keywords' strings hard before soft,
   row by row, each row's in the order of its kinds). A name's test against a row's hard keywords that are one run
   is one range test (`bitlex_rust::kind_test`), as the lean parser's `ident_word`; a row whose are not keeps its
   list. Kotlin's 673 lists are each `!(w::AS_1..=w::NULL_26).contains(&word)`, tier 2's Rust 1.74 MB to 1.52 MB. The
   runs of kinds that make a keyword's leaf one add (`word_kind`) are within a row, so they stay. rr's and JSON's code
   changes only in its word numbers.
2. **An operator ladder is one loop** (`G::ladders`, `G::ladder`). Three or more nested choices `H (op | …)*` of one
   head whose loops end alike (their exits, the marks after them) are one method, `H_ladder(set)`. It is the loop over
   every level's operators in one order that keeps each level's, each operator's tests guarded by its bit of `set` (an
   `unless` of the ladder's: a flag named `#i`, `flag_cond`), so an operator a level does not take falls through to the
   next arm as the level's own decision did. Each level keeps its head's call and its depth, then calls the ladder
   with its set. The lean parser's `expr(min)` takes a minimum, since its levels nest. Kotlin's do not (`gen_op` and
   `kw_infix_op` are some middle levels' only), so the set is a bit set. Kotlin's eleven levels of `unary_tight` are
   one ladder. rr's levels each have their own head (`term` over `product`, `product` over `primary`), as rr's lean
   parser has them, so it has no ladder, and neither does JSON. Their code is unchanged. Both of tier 2's Rust forms
   that call methods (`source`, `tier2_calls`) take the ladders; the one loop's form (rr's `tier2_loop`) has none.
   `RERAIL_BITLEX_LADDERS` prints the candidate groups.
3. **A token pays for the marks no rule takes only where one may hold** (`open_mark`). A mark each of whose
   definitions is a code bit at the token is asked behind one test of those bits of the token's code (with
   `tier1::ASKED` and `TESTED`, constants: where a bit is asked of the gap or the bytes, the test folds away). A mark
   each of whose definitions' runs starts with a kind is asked behind that kind's test, the marks of one kind together.
   Kotlin's plain token (the `_` arm) was five hold tests, each a `taken` bit and a function: now one code-bit test and
   two kind compares, as the lean parser asks a label only at a name. rr's arms keep their tests, its bits being asked of
   the gap; JSON has no such mark.
4. **The escapes after the token loop are passes over the words** (`one_pass_after`). Each escape made of `code`
   (`code_escape`) was its own pass over the file per mask, each a vector of the file's: 26 in Kotlin's tier 1, beside
   the test build's own. Now they are loops over the words, each mask a word: its `prev` its word and the word before's,
   a `fill` its carry, an `any` the word tested. A mask read past its word (`next`) starts a new loop before the read,
   and a mask a later loop reads is kept as a vector as well. Kotlin's are two loops, with one vector (`t108`, read two
   bytes ahead). rr and JSON have no escape made of `code`. A check's reason with a line break in it is written with
   `\n` in its comment.

What stays: Kotlin's tier 1 keeps its two passes. Stage 1 keeps the 144 masks its token loop reads, for the whole
file, and tier 1 computes the token's marks into its code (its line breaks, the semicolons, `as_open`, ...). Its
strings trip every condition of 0619's one pass: templates that resume a string (`tail ... or t_dollar resumes str`),
a fill kept over a region (`assume ... at ends t_brace`), and strings that tier 2 reads in parts, from tier 1's
masks (`rd1_open`). The lookahead of `as_split` and `is_split` stays a function a state (`next_*`).
