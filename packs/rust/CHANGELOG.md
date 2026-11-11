# @neralabs/rerail-pack-rust

Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.

## 0.1.6 (2026-11-06)

33 rules, 17 of them on by default.

### Changed

- The pack's npm page lists keywords: Rerail, its language and the tools it ports, so a search on npm finds it. Its rules are unchanged.

## 0.1.5 (2026-11-04)

33 rules, 17 of them on by default.

- 15 new rules from Clippy: 4 start in Steer and 11 off.

## 0.1.4

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- 115b81b: Rust pack: eighteen Clippy 0.1.99 lints pass every port readiness gate and ship. On: `builtin_type_shadow`, `inconsistent_digit_grouping`, `int_plus_one`, `mixed_case_hex_literals`, `needless_arbitrary_self_type`, `needless_else`, `octal_escapes`, `precedence`, `redundant_at_rest_pattern`, `single_component_path_imports`, `unneeded_wildcard_pattern`, `unusual_byte_groupings` and `zero_prefixed_literal`. Off, as Clippy's default lint levels leave them (pedantic or restriction): `large_digit_groups`, `many_single_char_names`, `needless_raw_string_hashes`, `partial_pub_fields` and `precedence_bits`. 16 ports remain candidates.

## 0.1.3

### Patch Changes

- e06db22: Rust pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: Rust pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- b1952f2: Rust pack: ships no rules yet. Its 34 Clippy 0.1.99 ports are candidates until they pass the V1 rule validation gates, scored against Clippy's own findings on its UI tests and on pinned ripgrep and tokio checkouts with every disagreement declared with its evidence, and the pack is never recommended while it has no rules. Each rule's id is `clippy-<lint>` (a configuration variant adds its setting, as `clippy-unreadable-literal-no-fractions`), the same string as its document's file name and its port's id in `conformance.json`; the package names the repository and directory it is published from, `neralabs/rerail` at `packs/rust`.
