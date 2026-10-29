# Rerail rule pack: Rust

18 Clippy 0.1.99 lints ported to Rerail, each passing the [conformance check](../../conformance/README.md#the-gate), in `rules/` and `pack.json`: `clippy-builtin-type-shadow`, `clippy-inconsistent-digit-grouping`, `clippy-int-plus-one`, `clippy-large-digit-groups`, `clippy-many-single-char-names`, `clippy-mixed-case-hex-literals`, `clippy-needless-arbitrary-self-type`, `clippy-needless-else`, `clippy-needless-raw-string-hashes`, `clippy-octal-escapes`, `clippy-partial-pub-fields`, `clippy-precedence`, `clippy-precedence-bits`, `clippy-redundant-at-rest-pattern`, `clippy-single-component-path-imports`, `clippy-unneeded-wildcard-pattern`, `clippy-unusual-byte-groupings` and `clippy-zero-prefixed-literal`. Clippy's default lint levels leave `large_digit_groups`, `many_single_char_names` and `needless_raw_string_hashes` (pedantic) and `partial_pub_fields` and `precedence_bits` (restriction) off, so they ship off (`"on": false`). 16 more ports are candidates in `candidates/rules/`, judged by the same checks without blocking ([candidates/README.md](candidates/README.md)). Each port is scored against Clippy's own findings on its UI tests, and against the labelled lines of Clippy's real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace path. The shipped rules agree with Clippy on every fixture finding, with no declared divergence; `divergences.json` holds the candidates' disagreements, each with its evidence.

This pack follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and each shipped rule's document, title, group and whether it is on |
| `rules/` | The code-rule documents (`rerail.policy/v1`) of the ports that pass the conformance check |
| `candidates/rules/` | The code-rule documents of the other ports, scored and judged but never shipped |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `conformance.json` | Each ported lint's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned Clippy tool |
| `divergences.json` | Declared disagreements with Clippy |
| `upstream/clippy/` | Clippy's copied UI tests, the probes Rerail wrote (`fixtures/rerail/`, listed in `UPSTREAM.md`), the expected findings and their generator (`generate.py`, sharing `upstream/common.py`); the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`); the upstream manifest `gates.yaml` (its pins, approved configurations, the file types it lints and the facts of every lint the pack ports) |
| `licenses/clippy/` | Clippy's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

## Scores

Every port is scored against Clippy's findings on the fixtures, and against the labelled lines of Clippy's real-code
samples ([docs/upstream.md](../../docs/upstream.md#samples)). The 18 shipped rules' fixture scores below come from
`node --import tsx conformance/languages/cli.ts --pack rust --fixtures` (Clippy 0.1.99, Rust 1.99.0); the totals sum
the per-rule rows. FP and FN count unexplained points only; an explained point is one a divergence names, and recall
counts an explained FN against the pack. No shipped rule has a declared divergence. The candidates' scores, and the
totals over every port, are in [candidates/README.md](candidates/README.md). Rust has no rename variants
(`holds-under-rename` covers the ECMAScript packs), so the real-code samples are the pack's only evidence that a port is
not fitted to Clippy's UI tests.

| Source | Rules | TP | FP | FN | Explained FP | Explained FN |
|---|---|---|---|---|---|---|
| fixtures (Clippy's UI tests and Rerail's probes) | 18 | 167 | 0 | 0 | 0 | 0 |

Per rule on the fixtures (TP / FP / FN, explained points in parentheses):

| Rule | fixtures |
|---|---|
| `clippy-precedence` | 30 / 0 / 0 |
| `clippy-precedence-bits` | 26 / 0 / 0 |
| `clippy-int-plus-one` | 14 / 0 / 0 |
| `clippy-needless-arbitrary-self-type` | 9 / 0 / 0 |
| `clippy-partial-pub-fields` | 4 / 0 / 0 |
| `clippy-unneeded-wildcard-pattern` | 18 / 0 / 0 |
| `clippy-redundant-at-rest-pattern` | 6 / 0 / 0 |
| `clippy-needless-else` | 1 / 0 / 0 |
| `clippy-inconsistent-digit-grouping` | 9 / 0 / 0 |
| `clippy-large-digit-groups` | 4 / 0 / 0 |
| `clippy-unusual-byte-groupings` | 2 / 0 / 0 |
| `clippy-mixed-case-hex-literals` | 4 / 0 / 0 |
| `clippy-zero-prefixed-literal` | 5 / 0 / 0 |
| `clippy-octal-escapes` | 11 / 0 / 0 |
| `clippy-needless-raw-string-hashes` | 15 / 0 / 0 |
| `clippy-builtin-type-shadow` | 1 / 0 / 0 |
| `clippy-single-component-path-imports` | 5 / 0 / 0 |
| `clippy-many-single-char-names` | 3 / 0 / 0 |

## Deriving and scoring

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/clippy/publish/`. The recipe (`scripts/pack-data/recipes/rust.ts`) runs `upstream/clippy/generate.py`
against Clippy 0.1.99 and Rust 1.99.0 (installed with `rustup`) over the fixtures. The probes Rerail wrote
(`fixtures/rerail/`) are authored: a change to them is staged into `publish/` with the derived data.

```sh
npm run fixtures:derive -- --pack rust
npm run fixtures:stage -- --pack rust    # copy what changed into upstream/clippy/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack rust --rule clippy-precedence --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `clippy-builtin-type-shadow`

Port of Clippy 0.1.99's builtin_type_shadow: reports generic type parameters named like a primitive type (rustc's PrimTy names: the integer types, the float types f16 to f128, bool, char and str); semantic name resolution is not modelled.

### `clippy-inconsistent-digit-grouping`

Port of Clippy 0.1.99's inconsistent_digit_grouping: reports a decimal literal whose integer part or fraction has groups of different sizes after the first (or a first group longer than the rest), or whose integer and fraction are grouped differently. Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.

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

Port of Clippy 0.1.99's needless_raw_string_hashes: reports a raw string, byte string or C string literal (up to ten hashes) holding a `\\` or a `"`, whose hashes exceed what its contents require: one more than the most hashes after a quote that starts a run (a quote right after a run's hashes ends it instead). A raw literal holding neither is needless_raw_strings', which the expected findings enable and which then takes it.

### `clippy-octal-escapes`

Port of Clippy 0.1.99's octal_escapes: reports each \0 followed by one or two octal digits in string, byte-string, or C-string literals; format-string preprocessing and external macros are not modelled.

### `clippy-partial-pub-fields`

Port of Clippy 0.1.99's partial_pub_fields: in a struct whose first field is `pub` (exactly: `pub(crate)` and other restricted visibilities are not), reports the first field that is not, and in one whose first field is not `pub`, the first field that is, at the field's visibility (a zero-width point at the field's start when it has none).

### `clippy-precedence`

Port of Clippy 0.1.99's precedence lint: reports a shift or bitwise operation whose operand is an unparenthesized arithmetic (+ - * / %) operation, at every nesting depth, and a method call whose receiver is an unparenthesized closure (spanning the call); macro expansion and type/value semantics are not modelled.

### `clippy-precedence-bits`

Port of Clippy 0.1.99's precedence_bits lint: reports, at any nesting depth, every bitwise (& | ^) operation with an unparenthesized shift operand and no unparenthesized arithmetic operand (that mix is the precedence lint's), each at its own span, so every operation of a left-nested chain like `a ^ b ^ c >> 1 ^ d >> 2` that has a shift operand is reported; macro expansion and semantic type checks are not modelled.

### `clippy-single-component-path-imports`

Port of Clippy 0.1.99's single_component_path_imports (Rust 2018 and later): reports a module's (the file's or a `mod`'s) non-`pub` `use name;` without a rename, and each `name` of a `use {a, b};` without a prefix, unless a `macro_rules!` of that name comes before it in the module, a non-`pub` `use self::name..` of the module (or `use self::{name..}`) names it, or a path `self::name::..` outside `use` declarations does anywhere in the module. Clippy reads the module after macro expansion; macro arguments and macro_rules! bodies are token trees no syntax-level reader expands.

### `clippy-unneeded-wildcard-pattern`

Port of Clippy 0.1.99's unneeded_wildcard_pattern: reports the run of wildcard elements of a tuple or tuple-struct pattern adjacent to its rest pattern, and the trailing wildcard fields of a struct pattern before its rest, spanning them with their separators up to (or from) the rest pattern as Clippy does; elements nested more than two brackets deep and macro-expanded code are not modelled.

### `clippy-unusual-byte-groupings`

Port of Clippy 0.1.99's unusual_byte_groupings: reports a binary, octal or hexadecimal literal whose digit groups after the first differ in size, or whose first group is longer than the rest. Literals Clippy reads as a mistyped suffix (an unsuffixed literal ending in `_8`, `_16`, `_32` or `_64`, the exponent's `_32` or `_64`) and hexadecimal UUIDs (8-4-4-4-12) are not reported, nor (as Clippy skips code from expansion) literals in a macro_rules! body; a macro invocation's arguments keep their own spans and are read.
