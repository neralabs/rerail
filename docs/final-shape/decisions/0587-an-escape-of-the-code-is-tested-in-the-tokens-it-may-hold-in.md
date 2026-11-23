# 0587: An escape of the code is tested in the tokens it may hold in

Status: decided (the user, 2026-11-21: "maybe compare the rust code", on choosing the next step by the lean
parser's Rust). Follows 0609 and 0608 (the stretch escape).

## Intro

**The problem.** Compared with `kotlin_lean`'s lexer, two things in tier 1's Rust were not the lean form:
- "a `0x` or `0b` with no digit" was tested over each stretch of code between region tokens (`code_from`, 0608).
  The lean lexer decides `0x` in `number()`, at the number token's first byte.
- An escape outside the regions (`code_checks`: this one, "a `$` outside a string") was `any_in(&v, s, l)`
  in every row of code, the punctuation's and the names' included. The lean lexer tests each in the one token
  that reads it.

## The decision

- **An escape `prev(X, j) & Y` over the tokens of code.** Where `X₀` (its value outside the regions) is at no
  skipped byte, every byte it holds at is a token of code's byte. So it is `X₀ & next(Y, j)`, one stage-1 mask,
  tested over those tokens (`code_checks`). The stretches stay the form for an `X₀` at a skipped byte.
- **Each row tests it where it may hold** (`may_hold`):
  - at the token's first byte, with its start bytes;
  - over the bytes after, with its tail bytes and the token's bytes before each;
  - so one `bit(&v, s)`, an `any_in(&v, s + 1, l)`, both, or no test.

  `may_hold` asks whether some way the mask holds (`cubes`) needs only bytes the context allows at each offset.
- **`cubes` knows a shifted class's complement.** `~prev(c)` and `~next(c, k)` for a mask `c` of exact bytes
  are `c`'s complement at the byte read. So `num_start`'s `~prev(word)` says no number starts inside a name.

Now:
- the hex and binary literals test the escape at `s`, as `number()` does;
- the integer literal tests it over its bytes;
- the real literal tests it over its bytes after the first (`1e+0x`);
- the names test it at `s` (their start bytes are the grammar's, digits among them);
- the punctuation does not test it;
- "a `$` outside a string" is one `bit(&m_dollar, s)` in the punctuation's row.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. `number()` reads
`0x` at the token's first two bytes. A test over each stretch between region tokens, and one over every token's
bytes, asked at every byte whether a number started there.

## What changed

- `bitlex_lex_rust.rs`:
  - `stretch_escape` gives a `code_checks` test where it can;
  - `code_checks` carries the escape's mask;
  - `may_hold` is new;
  - `cubes` knows a shifted class's complement.
- `tier1.rs` regenerated: no `code_from` stretches.
- The test `the_0x_escape_is_the_interpreters`. Its `1e+0x` is also refused as a name touching a number, so no
  text tells the real literal's tail test from that escape; the test checks that tier 1 agrees with the
  interpreter.

Kotlin: 55/55 with the corpus; bitlex 11/11; the compiler's bitlex 9/9.
