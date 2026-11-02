# Rerail rule pack: Rust, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

The four that port a non-default `clippy.toml` (`-threshold`, `-no-fractions`, `-one-allowed`) stay candidates whatever
they score, since their expected findings are Clippy's at its defaults. `clippy-option-env-unwrap`'s two disagreements
on `tests/ui/option_env_unwrap.rs` (an `external!` proc macro that rewrites its tokens' spans, which syntax alone cannot
tell from a span-keeping macro) are the only ones `divergences.json` leaves undeclared.

## Scores

From the same run as the pack README's scores. The totals over every port, the 33 shipped and the 1 candidate; FP
and FN count unexplained points only, and an explained point is one a divergence names:

| Source | Rules | TP | FP | FN | Explained FP | Explained FN |
|---|---|---|---|---|---|---|
| fixtures (Clippy's UI tests and Rerail's probes) | 34 | 397 | 2 | 2 | 0 | 7 |

The 7 explained FN are `rerail-unknown` divergences (`divergences.json`), every one a shipped rule's: code
tree-sitter-rust holds as macro token trees or `macro_rules!` bodies, which Clippy lints after expansion; `box` patterns
tree-sitter-rust does not parse; an attribute on a tuple element expression; a decimal literal at or above 2^53, which a
rule's JSON-number bounds cannot name.

The candidate on the fixtures (TP / FP / FN, explained points in parentheses):

| Rule | fixtures |
|---|---|
| `clippy-option-env-unwrap` | 5 / 2 / 2 |

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `clippy-option-env-unwrap`

Port of Clippy 0.1.99's option_env_unwrap: reports an unwrap or expect call on option_env!, spanning the whole call; inside a token tree (a macro invocation's input or a macro_rules! body) it reports the call where its tokens are written, as Clippy does for macros that keep their input's spans (macro_rules!, format macros) and for a macro_rules! body that is expanded; a proc macro that respans its input, which Clippy reports at the invocation, is not modelled.
