# Rerail rule pack: Rust, candidates

The ports in `rules/` here are still being evaluated: scored and reported against the tools' recordings, never gated
and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

The four that port a non-default `clippy.toml` (`-threshold`, `-no-fractions`, `-one-allowed`) stay candidates whatever
they score, since the gates judge them against Clippy's defaults. `clippy-option-env-unwrap`'s two disagreements on
`tests/ui/option_env_unwrap.rs` (an `external!` proc macro that rewrites its tokens' spans, which syntax alone cannot
tell from a span-keeping macro) are the only ones `divergences.json` leaves undeclared.

## Scores

From the same run as the pack README's scores. The totals over every port, the 18 shipped and the 16 candidates; FP
and FN count unexplained points only, and an explained point is one a divergence names:

| Source | Rules | TP | FP | FN | Explained FP | Explained FN |
|---|---|---|---|---|---|---|
| fixtures (Clippy's UI tests and Rerail's probes) | 34 | 397 | 2 | 2 | 0 | 7 |
| ripgrep corpus | 34 | 614 | 0 | 0 | 11 | 6 |
| tokio corpus | 34 | 555 | 0 | 0 | 98 | 95 |

Divergences by kind (`divergences.json`), every one a candidate's:

| Kind | fixtures | ripgrep | tokio | Total | What |
|---|---|---|---|---|---|
| `rerail-unknown` | 7 | 2 | 90 | 99 | Code tree-sitter-rust holds as macro token trees or `macro_rules!` bodies, which Clippy lints after expansion; `box` patterns tree-sitter-rust does not parse; an attribute on a tuple element expression; a decimal literal at or above 2^53, which a rule's JSON-number bounds cannot name |
| `not-comparable` | 0 | 15 | 100 | 115 | Items and files a `cfg` leaves out of the recording's build (macOS, `--all-features`, no `tokio_unstable`, `loom`, `docsrs` or `miri`), `cfg_attr` levels whose predicate is false there, `#[tokio::test]`'s expansion, grep-index's crate-root `#![allow(warnings)]` (read per file), and the ui-toml variants on corpora recorded with Clippy's defaults |
| `upstream-wrong` | 0 | 0 | 0 | 0 | |

Each candidate's TP / FP / FN, explained points in parentheses:

| Rule | fixtures | ripgrep | tokio |
|---|---|---|---|
| `clippy-double-parens` | 24 / 0 / 0 (1) | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-redundant-static-lifetimes` | 31 / 0 / 0 | 32 / 0 / 0 (2) | 0 / 0 / 0 |
| `clippy-else-if-without-else` | 4 / 0 / 0 | 23 / 0 / 0 (2) | 14 / 0 / 0 |
| `clippy-unnested-or-patterns` | 32 / 0 / 0 (3) | 4 / 0 / 0 | 1 / 0 / 0 |
| `clippy-option-env-unwrap` | 5 / 2 / 2 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-unreadable-literal` | 16 / 0 / 0 | 5 / 0 / 0 (1) | 0 / 0 / 0 (1) |
| `clippy-unreadable-literal-no-fractions` | 1 / 0 / 0 | 5 / 0 / 0 (1) | 0 / 0 / 0 (1) |
| `clippy-inconsistent-digit-grouping-no-fractions` | 1 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-decimal-literal-representation` | 8 / 0 / 0 | 0 / 0 / 0 | 2 / 0 / 0 (2) |
| `clippy-decimal-literal-representation-threshold` | 1 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 (3) |
| `clippy-needless-raw-strings` | 15 / 0 / 0 | 382 / 0 / 0 (1) | 1 / 0 / 0 |
| `clippy-needless-raw-string-hashes-one-allowed` | 3 / 0 / 0 | 0 / 0 / 0 (4) | 0 / 0 / 0 |
| `clippy-redundant-field-names` | 8 / 0 / 0 (1) | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-single-char-lifetime-names` | 9 / 0 / 0 | 129 / 0 / 0 (4) | 213 / 0 / 0 (61) |
| `clippy-allow-attributes` | 25 / 0 / 0 (1) | 14 / 0 / 0 (1) | 140 / 0 / 0 (48) |
| `clippy-allow-attributes-without-reason` | 47 / 0 / 0 (1) | 16 / 0 / 0 (1) | 176 / 0 / 0 (77) |

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `clippy-allow-attributes`

Port of Clippy 0.1.99's allow_attributes: reports the path of every outer #[allow(..)] attribute, one a #[cfg_attr(..)] expands to included. The lint needs Rust 1.81 (lint reasons, `suppress.clippy.msrv`). Inner #![allow] is not reported. What a macro invocation's or macro_rules! body's token tree holds, and whether a #[cfg] predicate holds in a build, are not read. A #[cfg_attr] is read as active unless its first argument is exactly `test`, `any()` or `not(clippy)`, the predicates the suppression engine reads as off whatever the build; every other predicate is read as active.

### `clippy-allow-attributes-without-reason`

Port of Clippy 0.1.99's allow_attributes_without_reason: reports every #[allow(..)] and #[expect(..)] attribute, inner or outer, one a #[cfg_attr(..)] expands to included (at the expanded attribute), whose last item is not `reason = ".."`. The lint needs Rust 1.81 (lint reasons, `suppress.clippy.msrv`). What a macro invocation's or macro_rules! body's token tree holds, and whether a #[cfg] predicate holds in a build, are not read. A #[cfg_attr] is read as active unless its first argument is exactly `test`, `any()` or `not(clippy)`, the predicates the suppression engine reads as off whatever the build; every other predicate is read as active. Each allow or expect a #[cfg_attr] expands to is checked on its own for a last `reason = ".."` item and reported at its own span (`allow(..)`, as Clippy reports the expanded attribute); a list nested deeper than two parentheses inside it is not read.

### `clippy-decimal-literal-representation`

Port of Clippy 0.1.99's decimal_literal_representation (literal-representation-threshold 16384, the default): reports a decimal integer literal whose value is at least the threshold and whose hexadecimal digits (four or more above this threshold) are a power of two with a margin (1, 2, 4 or 8 then zeros, or 1, 3, 7 or F then Fs, the last digit free: a value at most 16 below or 15 above a power of two) or only 0s and Fs after a leading 7 or F (every hexadecimal digit 0 or F, the leading one possibly 7). Values of 2^53 and above are not read: the rule language has no hexadecimal reading of a value, so the digits are read as value ranges and bit masks, and a rule's numbers reach the engine as JavaScript numbers, exact only up to 2^53. Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-decimal-literal-representation-threshold`

Port of Clippy 0.1.99's decimal_literal_representation under literal-representation-threshold = 0xFFFFFF (its ui-toml test): reports a decimal integer literal whose value is at least the threshold and whose hexadecimal digits (four or more above this threshold) are a power of two with a margin (1, 2, 4 or 8 then zeros, or 1, 3, 7 or F then Fs, the last digit free: a value at most 16 below or 15 above a power of two) or only 0s and Fs after a leading 7 or F (every hexadecimal digit 0 or F, the leading one possibly 7). Values of 2^53 and above are not read: the rule language has no hexadecimal reading of a value, so the digits are read as value ranges and bit masks, and a rule's numbers reach the engine as JavaScript numbers, exact only up to 2^53. Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-double-parens`

Port of Clippy 0.1.99's double_parens lint: reports redundant nested parentheses around an expression (only whitespace between the two opening or closing parentheses, as Clippy's source check requires; a call's sole argument in parentheses, reported once where the nested-parentheses clauses already report the same span), and the same two shapes in a macro_rules! transcriber's tokens (a parenthesized group whose only content is another parenthesized group, in expression position after `=`, `=>`, `return`, a delimiter, `;` or `,`, or as the sole parenthesized argument after a call's name; a `$(` repetition is not a parenthesized expression); expansion of macro invocations' arguments and tuple/unit argument semantics are not modelled.

### `clippy-else-if-without-else`

Port of Clippy 0.1.99's else_if_without_else lint: reports the if expression of an else-if branch whose final if has no else, spanning the whole else-if expression; macro expansion and cfg evaluation are not modelled.

### `clippy-inconsistent-digit-grouping-no-fractions`

Port of Clippy 0.1.99's inconsistent_digit_grouping under unreadable-literal-lint-fractions = false (its ui-toml test): reports a decimal literal whose integer part or fraction has groups of different sizes after the first (or a first group longer than the rest), or whose integer and fraction are grouped differently (unreadable-literal-lint-fractions = false: a fraction is never unreadable). Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

### `clippy-needless-raw-string-hashes-one-allowed`

Port of Clippy 0.1.99's needless_raw_string_hashes under allow-one-hash-in-raw-strings = true (its ui-toml test): reports a raw string, byte string or C string literal (up to ten hashes) holding a `\\` or a `"`, whose hashes exceed what its contents require: one more than the most hashes after a quote that starts a run (a quote right after a run's hashes ends it instead). One hash is always allowed. A raw literal holding neither is needless_raw_strings', which the recording enables and which then takes it.

### `clippy-needless-raw-strings`

Port of Clippy 0.1.99's needless_raw_strings: reports raw literals whose contents do not require raw-string syntax or hashes; macro expansion and format-string preprocessing are not modelled.

### `clippy-option-env-unwrap`

Port of Clippy 0.1.99's option_env_unwrap: reports an unwrap or expect call on option_env!, spanning the whole call; inside a token tree (a macro invocation's input or a macro_rules! body) it reports the call where its tokens are written, as Clippy does for macros that keep their input's spans (macro_rules!, format macros) and for a macro_rules! body that is expanded; a proc macro that respans its input, which Clippy reports at the invocation, is not modelled.

### `clippy-redundant-field-names`

Port of Clippy 0.1.99's redundant_field_names: reports a struct expression's field initializer whose value is a path of one segment, without generic arguments, naming the field (`S { a: a }`). The lint needs Rust 1.17 (field init shorthand, `suppress.clippy.msrv`). Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-redundant-static-lifetimes`

Port of Clippy 0.1.99's redundant_static_lifetimes: reports redundant 'static lifetimes in free const and static declarations, where Clippy's visit_type reaches them (through references, arrays, slices and tuples, not into fn-pointer, generic-argument or trait-object types); associated items are not modelled. The lint needs Rust 1.17 (`suppress.clippy.msrv`).

### `clippy-single-char-lifetime-names`

Port of Clippy 0.1.99's single_char_lifetime_names: reports each single-character lifetime parameter where it is declared, a generic parameter list's or a `for<..>` binder's, not where a bound or a type names it. Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-unnested-or-patterns`

Port of Clippy 0.1.99's unnested_or_patterns: reports a match arm's, let's, if/while let's or parameter's pattern that holds an or-pattern Clippy nests: one with a parenthesised or-pattern among its alternatives, or with two alternatives (adjacent or not) built alike that differ in one sub-pattern at most: `&mut`, the same `x @`, `ref x @` or `mut x @` binding, a tuple, slice or tuple struct (same path) pattern whose differing element is not `..` on either side, among the first six, or a struct pattern (same path, both with or both without `..`, up to six fields in any order) whose other fields are the same, unless the two are the same shorthands only. The lint needs Rust 1.53 (`suppress.clippy.msrv`). Sub-patterns compare by their tokens, parentheses included. Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands; `box` patterns (an unstable feature) do not parse.

### `clippy-unreadable-literal`

Port of Clippy 0.1.99's unreadable_literal: reports an integer or float literal whose digits, as Clippy's NumericLiteral splits them (radix prefix, suffix and the `_` before it, integer, fraction and exponent), hold an ungrouped part longer than five digits (a decimal integer part, a fraction, or a binary, octal or hexadecimal literal whose first group is). Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

### `clippy-unreadable-literal-no-fractions`

Port of Clippy 0.1.99's unreadable_literal under unreadable-literal-lint-fractions = false (its ui-toml test): reports an integer or float literal whose digits, as Clippy's NumericLiteral splits them (radix prefix, suffix and the `_` before it, integer, fraction and exponent), hold an ungrouped part longer than five digits (a decimal integer part, a fraction (never, under unreadable-literal-lint-fractions = false), or a binary, octal or hexadecimal literal whose first group is). Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.
