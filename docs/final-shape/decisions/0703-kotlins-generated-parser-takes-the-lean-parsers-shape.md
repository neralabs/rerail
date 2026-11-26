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
