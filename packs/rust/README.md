# Rerail rule pack: Rust

34 code rules for Rust, ported from Clippy 0.1.99. Each is scored against Clippy's recorded findings on its UI tests and on pinned ripgrep and tokio checkouts, through the engine's production workspace path, with every disagreement declared in `divergences.json` (218 entries, each with its evidence).

This pack follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported lint's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned Clippy tool and corpora |
| `divergences.json` | Declared disagreements with Clippy |
| `upstream/clippy/` | Clippy's copied UI tests, recordings and recorder (`generate.py`, sharing `upstream/common.py`) |
| `licenses/clippy/` | Clippy's licence and `UPSTREAM.md`, listing every copied file |

## Scores

Scored with `node --import tsx conformance/languages/cli.ts --pack rust` (Clippy 0.1.99, Rust 1.99.0; fixtures and both
corpora). FP and FN count unexplained points only; an explained point is one a divergence names, and recall counts
an explained FN against the pack.

| Source | Rules | TP | FP | FN | Explained FP | Explained FN |
|---|---|---|---|---|---|---|
| fixtures (Clippy's UI tests) | 34 | 319 | 0 | 0 | 0 | 11 |
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
| `clippy-precedence` | 9 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-precedence-bits` | 8 / 0 / 0 | 0 / 0 / 0 | 2 / 0 / 0 |
| `clippy-double-parens` | 8 / 0 / 0 (5) | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-int-plus-one` | 14 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-redundant-static-lifetimes` | 28 / 0 / 0 | 32 / 0 / 0 (2) | 0 / 0 / 0 |
| `clippy-needless-arbitrary-self-type` | 9 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-else-if-without-else` | 4 / 0 / 0 | 23 / 0 / 0 (2) | 14 / 0 / 0 |
| `clippy-partial-pub-fields` | 4 / 0 / 0 | 0 / 0 / 0 | 4 / 0 / 0 |
| `clippy-unneeded-wildcard-pattern` | 18 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-redundant-at-rest-pattern` | 6 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-needless-else` | 1 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| `clippy-unnested-or-patterns` | 26 / 0 / 0 (3) | 4 / 0 / 0 | 1 / 0 / 0 |
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
| `clippy-allow-attributes` | 16 / 0 / 0 (1) | 14 / 0 / 0 (1) | 140 / 0 / 0 (48) |
| `clippy-allow-attributes-without-reason` | 40 / 0 / 0 (1) | 16 / 0 / 0 (1) | 176 / 0 / 0 (77) |

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
