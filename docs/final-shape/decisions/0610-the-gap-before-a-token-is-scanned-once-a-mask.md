# 0610: The gap before a token is scanned once a mask

Numbered 0591 where it was written (Swift-Kotlin-Scala-support); 0610 at its merge into Parser-generators-final-shape, whose 0591 is another.

Status: decided (the user, 2026-11-21: "now please again compare the rust code"). Follows 0590.

## Intro

**The problem.** The lean lexer finds whether a line end is in the white space before a token once (`nl`, from
the block's line-end mask). Tier 1 scanned the gap for one three times a token:
- in a kept fill's value before the token;
- in a gap mask's accumulator;
- in a gap mark's test.

Each was `any_in(&m_lf, at, s - 1)`. And it scanned the gap for `ws & code` (`any_in(&m_ws, at, s - 1)`), which
holds at every byte of a gap, since a gap's bytes are the ones the loop skips (`m_ws`).

## The decision

- **A gap mask is read against the skipped bytes** (`gap_code`): one at none of them is `false`; one at all of
  them is `s > at`.
- **Each mask the loop scans a gap of is scanned once** at the loop's top (`gap_NAME = s > at && any_in(&NAME,
  at, s - 1)`), and each test reads that.
- **A gap accumulator's test is the gap test alone.** Every gap test is false over an empty gap, so it needs no
  `s > at` of its own.

Kotlin's loop scans its gap once, for a line end (`gap_m_lf`).

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The lean lexer reads
a gap once.

## What changed

- `bitlex_lex_rust.rs`: `gap_code` reads the skipped bytes; the loop's gap scans are shared (`gap_NAME`); the
  accumulators' tests lose their `s > at`.
- `tier1.rs` regenerated.

Kotlin: 55/55 with the corpus; the compiler's bitlex 9/9.
