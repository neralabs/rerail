# Rerail rule pack: Rust

Ports of 34 Clippy 0.1.99 lints. Seven pass the [port readiness gates](../../docs/port-readiness.md) and ship in `rules/` and `pack.json`: `clippy-many-single-char-names`, `clippy-needless-arbitrary-self-type`, `clippy-needless-else`, `clippy-needless-raw-string-hashes`, `clippy-octal-escapes`, `clippy-redundant-at-rest-pattern` and `clippy-single-component-path-imports`. Clippy's default lint levels leave `many_single_char_names` and `needless_raw_string_hashes` (pedantic) off, so they ship off (`"on": false`). The other 27 are candidates in `candidates/rules/`, judged by the same gates without blocking. Each port is scored against Clippy's recorded findings on its UI tests and on pinned ripgrep and tokio checkouts, through the engine's production workspace path, with every disagreement declared in `divergences.json` (218 entries, each with its evidence).

This pack follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and each shipped rule's document, title, group and whether it is on |
| `rules/` | The code-rule documents (`rerail.policy/v1`) of the ports that pass the port readiness gates |
| `candidates/rules/` | The code-rule documents of the other ports, scored and judged but never shipped |
| `conformance.json` | Each ported lint's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned Clippy tool and corpora |
| `divergences.json` | Declared disagreements with Clippy |
| `upstream/clippy/` | Clippy's copied UI tests, the probes Rerail wrote (`fixtures/rerail/`, listed in `UPSTREAM.md`), recordings and recorder (`generate.py`, sharing `upstream/common.py`); the port readiness gates' `gates.yaml`, `recording.lock` and `goldens.tsv` |
| `licenses/clippy/` | Clippy's licence and `UPSTREAM.md`, listing every copied file |

## Scores

Every port, shipped or candidate, is scored here. Scored with `node --import tsx conformance/languages/cli.ts --pack rust` (Clippy 0.1.99, Rust 1.99.0; fixtures and both
corpora). FP and FN count unexplained points only; an explained point is one a divergence names, and recall counts
an explained FN against the pack.

| Source | Rules | TP | FP | FN | Explained FP | Explained FN |
|---|---|---|---|---|---|---|
| fixtures (Clippy's UI tests and Rerail's probes) | 34 | 393 | 0 | 0 | 0 | 11 |
| ripgrep corpus | 34 | 614 | 0 | 0 | 11 | 6 |
| tokio corpus | 34 | 553 | 0 | 0 | 98 | 95 |

Divergences by kind (`divergences.json`):

| Kind | fixtures | ripgrep | tokio | Total | What |
|---|---|---|---|---|---|
| `rerail-unknown` | 11 | 2 | 90 | 103 | Code tree-sitter-rust holds as macro token trees or `macro_rules!` bodies, which Clippy lints after expansion; `box` patterns tree-sitter-rust does not parse; an attribute on a tuple element expression; a decimal literal at or above 2^52, which a rule's JSON-number bounds cannot name |
| `not-comparable` | 0 | 15 | 100 | 115 | Items and files a `cfg` leaves out of the recording's build (macOS, `--all-features`, no `tokio_unstable`, `loom`, `docsrs` or `miri`), `cfg_attr` levels whose predicate is false there, `#[tokio::test]`'s expansion, grep-index's crate-root `#![allow(warnings)]` (read per file), and the ui-toml variants on corpora recorded with Clippy's defaults |
| `upstream-wrong` | 0 | 0 | 0 | 0 | |

Per rule (TP / FP / FN, explained points in parentheses):

| Rule | fixtures | ripgrep | tokio |
|---|---|---|---|
| `clippy-precedence` | 30 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-precedence-bits` | 24 / 0 / 0 | 0 / 0 / 0 | 2 / 0 / 0 |
| `clippy-double-parens` | 20 / 0 / 0 (5) | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-int-plus-one` | 14 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-redundant-static-lifetimes` | 31 / 0 / 0 | 32 / 0 / 0 (2) | 0 / 0 / 0 |
| `clippy-needless-arbitrary-self-type` | 9 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-else-if-without-else` | 4 / 0 / 0 | 23 / 0 / 0 (2) | 14 / 0 / 0 |
| `clippy-partial-pub-fields` | 4 / 0 / 0 | 0 / 0 / 0 | 4 / 0 / 0 |
| `clippy-unneeded-wildcard-pattern` | 18 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-redundant-at-rest-pattern` | 6 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-needless-else` | 1 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-unnested-or-patterns` | 32 / 0 / 0 (3) | 4 / 0 / 0 | 1 / 0 / 0 |
| `clippy-option-env-unwrap` | 7 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-unreadable-literal` | 16 / 0 / 0 | 5 / 0 / 0 (1) | 0 / 0 / 0 (1) |
| `clippy-unreadable-literal-no-fractions` | 1 / 0 / 0 | 5 / 0 / 0 (1) | 0 / 0 / 0 (1) |
| `clippy-inconsistent-digit-grouping` | 9 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-inconsistent-digit-grouping-no-fractions` | 1 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-large-digit-groups` | 4 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-unusual-byte-groupings` | 2 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-decimal-literal-representation` | 8 / 0 / 0 | 0 / 0 / 0 | 2 / 0 / 0 (2) |
| `clippy-decimal-literal-representation-threshold` | 1 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 (3) |
| `clippy-mixed-case-hex-literals` | 4 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-zero-prefixed-literal` | 5 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-octal-escapes` | 11 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-needless-raw-strings` | 15 / 0 / 0 | 382 / 0 / 0 (1) | 1 / 0 / 0 |
| `clippy-needless-raw-string-hashes` | 15 / 0 / 0 | 4 / 0 / 0 | 0 / 0 / 0 |
| `clippy-needless-raw-string-hashes-one-allowed` | 3 / 0 / 0 | 0 / 0 / 0 (4) | 0 / 0 / 0 |
| `clippy-builtin-type-shadow` | 1 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-redundant-field-names` | 8 / 0 / 0 (1) | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-single-char-lifetime-names` | 9 / 0 / 0 | 129 / 0 / 0 (4) | 213 / 0 / 0 (61) |
| `clippy-single-component-path-imports` | 5 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-many-single-char-names` | 3 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-allow-attributes` | 25 / 0 / 0 (1) | 14 / 0 / 0 (1) | 140 / 0 / 0 (48) |
| `clippy-allow-attributes-without-reason` | 47 / 0 / 0 (1) | 16 / 0 / 0 (1) | 176 / 0 / 0 (77) |

## Recording and scoring

Re-recording runs offline against Clippy 0.1.99 and Rust 1.99.0:

```sh
python3 upstream/clippy/generate.py fixtures <rust-clippy checkout at tag rust-1.99.0> <scratch directory>
python3 upstream/clippy/generate.py corpus <cargo target root> ripgrep=<ripgrep checkout> tokio=<tokio checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack rust --rule clippy-precedence --fixtures
```

The port readiness gates judge every port against their own recording of Clippy (`upstream/clippy/recording.lock`;
the recorder runs the rustup toolchain `conformance/recorders/port-readiness/runtimes/clippy-1.99.0/` pins, on the
same UI tests and corpora, as `generate.py` does):

```sh
npm run port-readiness:record -- --pack rust --tool clippy   # needs rustup, git and the network
npm run port-readiness -- --pack rust --tool clippy
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `clippy-allow-attributes`

Port of Clippy 0.1.99's allow_attributes: reports the path of every outer #[allow(..)] attribute, one a #[cfg_attr(..)] expands to included. The lint needs Rust 1.81 (lint reasons, `suppress.clippy.msrv`). Inner #![allow] is not reported. What a macro invocation's or macro_rules! body's token tree holds, and whether a #[cfg] predicate holds in a build, are not read. A #[cfg_attr] is read as active unless its first argument is exactly `test`, `any()` or `not(clippy)`, the predicates the suppression engine reads as off whatever the build; every other predicate is read as active.

### `clippy-allow-attributes-without-reason`

Port of Clippy 0.1.99's allow_attributes_without_reason: reports every #[allow(..)] and #[expect(..)] attribute, inner or outer, one a #[cfg_attr(..)] expands to included (at the expanded attribute), whose last item is not `reason = ".."`. The lint needs Rust 1.81 (lint reasons, `suppress.clippy.msrv`). What a macro invocation's or macro_rules! body's token tree holds, and whether a #[cfg] predicate holds in a build, are not read. A #[cfg_attr] is read as active unless its first argument is exactly `test`, `any()` or `not(clippy)`, the predicates the suppression engine reads as off whatever the build; every other predicate is read as active. Each allow or expect a #[cfg_attr] expands to is checked on its own for a last `reason = ".."` item.

### `clippy-decimal-literal-representation`

Port of Clippy 0.1.99's decimal_literal_representation (literal-representation-threshold 16384, the default): reports a decimal integer literal whose value is at least the threshold and whose hexadecimal digits are a power of two with a margin (1, 2, 4 or 8 then zeros, or 1, 3, 7 or F then Fs, the last digit free) or only 0s and Fs after a leading 7 or F. Values of 2^52 and above are not read (the thresholds are JSON numbers); macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-decimal-literal-representation-threshold`

Port of Clippy 0.1.99's decimal_literal_representation under literal-representation-threshold = 0xFFFFFF (its ui-toml test): reports a decimal integer literal whose value is at least the threshold and whose hexadecimal digits are a power of two with a margin (1, 2, 4 or 8 then zeros, or 1, 3, 7 or F then Fs, the last digit free) or only 0s and Fs after a leading 7 or F. Values of 2^52 and above are not read (the thresholds are JSON numbers); macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-double-parens`

Port of Clippy 0.1.99's double_parens lint: reports redundant nested parentheses around an expression (only whitespace between the two opening or closing parentheses, as Clippy's source check requires; a call's sole argument in parentheses, reported once where the nested-parentheses clauses already report the same span); macro expansion and tuple/unit argument semantics are not modelled.

### `clippy-inconsistent-digit-grouping`

Port of Clippy 0.1.99's inconsistent_digit_grouping: reports a decimal literal whose integer part or fraction has groups of different sizes after the first (or a first group longer than the rest), or whose integer and fraction are grouped differently. Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

### `clippy-inconsistent-digit-grouping-no-fractions`

Port of Clippy 0.1.99's inconsistent_digit_grouping under unreadable-literal-lint-fractions = false (its ui-toml test): reports a decimal literal whose integer part or fraction has groups of different sizes after the first (or a first group longer than the rest), or whose integer and fraction are grouped differently (unreadable-literal-lint-fractions = false: a fraction is never unreadable). Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

### `clippy-int-plus-one`

Port of Clippy 0.1.99's int_plus_one lint: reports inequalities equivalent to a strict comparison through addition or subtraction by one; constant evaluation and type semantics are not modelled.

### `clippy-large-digit-groups`

Port of Clippy 0.1.99's large_digit_groups: reports a decimal literal whose integer part, or else its fraction (grouped from the right), is grouped consistently in groups longer than four digits. Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

### `clippy-many-single-char-names`

Port of Clippy 0.1.99's many_single_char_names (threshold 4): in a function with a body (not a trait's default method, not #[test]), Clippy stacks the single-character, non-uppercase, non-underscore names its parameters, `let` patterns and match arm patterns bind (`binds`), a block or arm opening a scope, a name already stacked not again; the lint fires where a scope ends holding one of them and the stack holds more than four, at the stack's first name. This port reports at the function's first such name at its own level, where the function's level holds five, or a block or arm after that name holds one and, with the names before it, five; a function whose own level binds none is not reported. Shorthand struct fields, `if let`, `while let`, `for` and closure parameters bind nothing Clippy counts; macro arguments are token trees.

### `clippy-mixed-case-hex-literals`

Port of Clippy 0.1.99's mixed_case_hex_literals: reports a hexadecimal integer literal whose digits (after `0x`, before the type suffix and the `_` before it) hold both a lowercase and an uppercase letter. A type suffix starts with `i` or `u`, which no digit run crosses.

### `clippy-needless-else`

Port of Clippy 0.1.99's needless_else lint: reports an empty else block after an if statement when it has no meaningful comment or code; macro and cfg expansion are not modelled.

### `clippy-needless-raw-string-hashes`

Port of Clippy 0.1.99's needless_raw_string_hashes: reports a raw string, byte string or C string literal (up to ten hashes) holding a `\\` or a `"`, whose hashes exceed what its contents require: one more than the most hashes after a quote that starts a run (a quote right after a run's hashes ends it instead). A raw literal holding neither is needless_raw_strings', which the recording enables and which then takes it.

### `clippy-needless-raw-string-hashes-one-allowed`

Port of Clippy 0.1.99's needless_raw_string_hashes under allow-one-hash-in-raw-strings = true (its ui-toml test): reports a raw string, byte string or C string literal (up to ten hashes) holding a `\\` or a `"`, whose hashes exceed what its contents require: one more than the most hashes after a quote that starts a run (a quote right after a run's hashes ends it instead). One hash is always allowed. A raw literal holding neither is needless_raw_strings', which the recording enables and which then takes it.

### `clippy-needless-raw-strings`

Port of Clippy 0.1.99's needless_raw_strings: reports raw literals whose contents do not require raw-string syntax or hashes; macro expansion and format-string preprocessing are not modelled.

### `clippy-octal-escapes`

Port of Clippy 0.1.99's octal_escapes: reports each \0 followed by one or two octal digits in string, byte-string, or C-string literals; format-string preprocessing and external macros are not modelled.

### `clippy-partial-pub-fields`

Port of Clippy 0.1.99's partial_pub_fields: in a struct whose first field is `pub` (exactly: `pub(crate)` and other restricted visibilities are not), reports the first field that is not, and in one whose first field is not `pub`, the first field that is, at the field's visibility (its start when it has none).

### `clippy-precedence`

Port of Clippy 0.1.99's precedence lint: reports a shift or bitwise operation whose operand is an unparenthesized arithmetic (+ - * / %) operation, at every nesting depth; macro expansion and type/value semantics are not modelled.

### `clippy-precedence-bits`

Port of Clippy 0.1.99's precedence_bits lint: reports, at any nesting depth, a bitwise (& | ^) operation with an unparenthesized shift operand and no unparenthesized arithmetic operand (that mix is the precedence lint's). Clippy reports every such operation, but operations sharing a start (a left-nested chain like `a ^ b ^ c >> 1 ^ d >> 2`) are one position, so one finding is made per chain, at the outermost operation; macro expansion and semantic type checks are not modelled.

### `clippy-redundant-field-names`

Port of Clippy 0.1.99's redundant_field_names: reports a struct expression's field initializer whose value is a path of one segment, without generic arguments, naming the field (`S { a: a }`). The lint needs Rust 1.17 (field init shorthand, `suppress.clippy.msrv`). Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-redundant-static-lifetimes`

Port of Clippy 0.1.99's redundant_static_lifetimes: reports redundant 'static lifetimes in free const and static declarations, where Clippy's visit_type reaches them (through references, arrays, slices and tuples, not into fn-pointer, generic-argument or trait-object types); associated items are not modelled. The lint needs Rust 1.17 (`suppress.clippy.msrv`).

### `clippy-single-char-lifetime-names`

Port of Clippy 0.1.99's single_char_lifetime_names: reports each single-character lifetime parameter where it is declared, a generic parameter list's or a `for<..>` binder's, not where a bound or a type names it. Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-single-component-path-imports`

Port of Clippy 0.1.99's single_component_path_imports (Rust 2018 and later): reports a module's (the file's or a `mod`'s) non-`pub` `use name;` without a rename, and each `name` of a `use {a, b};` without a prefix, unless a `macro_rules!` of that name comes before it in the module, a non-`pub` `use self::name..` of the module (or `use self::{name..}`) names it, or a path `self::name::..` outside `use` declarations does anywhere in the module. Clippy reads the module after macro expansion; macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-unnested-or-patterns`

Port of Clippy 0.1.99's unnested_or_patterns: reports a match arm's, let's, if/while let's or parameter's pattern that holds an or-pattern Clippy nests: one with a parenthesised or-pattern among its alternatives, or with two alternatives (adjacent or not) built alike that differ in one sub-pattern at most: `&mut`, the same `x @`, `ref x @` or `mut x @` binding, a tuple, slice or tuple struct (same path) pattern whose differing element is not `..` on either side, among the first six, or a struct pattern (same path, both with or both without `..`, up to six fields in any order) whose other fields are the same, unless the two are the same shorthands only. The lint needs Rust 1.53 (`suppress.clippy.msrv`). Sub-patterns compare by their tokens, parentheses included. Macro arguments and macro_rules! bodies are token trees no syntax-level reader expands; `box` patterns (an unstable feature) do not parse.

### `clippy-unreadable-literal`

Port of Clippy 0.1.99's unreadable_literal: reports an integer or float literal whose digits, as Clippy's NumericLiteral splits them (radix prefix, suffix and the `_` before it, integer, fraction and exponent), hold an ungrouped part longer than five digits (a decimal integer part, a fraction, or a binary, octal or hexadecimal literal whose first group is). Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

### `clippy-unreadable-literal-no-fractions`

Port of Clippy 0.1.99's unreadable_literal under unreadable-literal-lint-fractions = false (its ui-toml test): reports an integer or float literal whose digits, as Clippy's NumericLiteral splits them (radix prefix, suffix and the `_` before it, integer, fraction and exponent), hold an ungrouped part longer than five digits (a decimal integer part, a fraction (never, under unreadable-literal-lint-fractions = false), or a binary, octal or hexadecimal literal whose first group is). Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

### `clippy-unusual-byte-groupings`

Port of Clippy 0.1.99's unusual_byte_groupings: reports a binary, octal or hexadecimal literal whose digit groups after the first differ in size, or whose first group is longer than the rest. Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.
