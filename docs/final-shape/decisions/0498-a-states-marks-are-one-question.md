# 0498: A state's marks are one question

Numbered 0497 where it was written.

Status: decided (the user, 2026-11-19: "the automatic semicolon as a question asked at the
statement's end, like `asi()`, instead of a tier-1 bit corrected afterwards"). Item 2 of the
lean shape; follows 0492 and 0496.

## Intro

**How the lean parser asks.** It asks `asi()` where a statement may end. `asi()` is one question,
memoized for the position. It reads the token's line end (`nl`, a lexer bit) and the tokens after
it, and returns one of five answers (`No`, `Zero`, `Semi`, `Swallow`, `Ctor`). The rule matches on
the answer.

**What ours had.** Since 0492, the semicolon's marks (`semi`, `semi_close`, `head_ctor`, …) are
asked only where a rule takes them, and from the records. Tier 1 gives the gap's bit, as the lean
lexer gives `nl`; tier 2 tests the `unless before`s on the records after it (0490). But each state
wrote its chain of mark tests inline, `if … else if … else if …`, and the same chain came again in
every state that ends a statement.

## The decision

A state's chain of two or more mark tests is one question, in the generated Rust. A chain is a
`MARK` and its way (`GOTO` or `STOP`), two or more in a row, with no mark test before the first.

- `ask_N(s)` returns the place of the first mark that holds at the token, or the chain's length
  where none does. It takes the mark it answers, as the test did.
- The state matches on the answer: one arm per way, the answers that go one way joined
  (`0 | 2 =>`), and its switch as the default arm.
- Chains with the same tests are one function: Kotlin has 50.
- The emitter does this as a graph change (`mark_chain` in `succ`): the chain's first test is one
  node whose successors are the marks' ways and the word after the chain. The IR and the other
  backends are unchanged.

## Results

- Kotlin: kotlin_aot 4/4 (tapes the JIT's); compiler 140; parser_tests 92. `tier2_aot.rs` is 19,690
  lines.

## Still apart from `asi()`

- **The settles before the question.** The semicolon's `refuses` and `loses` (`SETTLE`, the
  `gaps_N()` tests and `lose()`) are still statements before the match. In the lean parser they are
  part of `asi()`'s answer (its `lost` comments). Next: fold them into the question, so that it
  returns the lost extras with its answer.
- **The memo.** `asi()` is asked once per position, with a memo. Ours is asked by each state that
  reaches the token.
