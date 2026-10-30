# @neralabs/rerail-pack-rust

## 0.1.5

### Patch Changes

- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- 3fb36af: The README describes only the rules the pack ships. What it said about the candidate ports moves to `candidates/README.md`, which is not published.
- c8352a7: README and licence notes: the ports are scored on labelled real-code samples instead of pinned corpora; no rule changes.
- c8352a7: Rust pack: eighteen Clippy 0.1.99 lints pass the conformance check and ship. On: `builtin_type_shadow`, `inconsistent_digit_grouping`, `int_plus_one`, `mixed_case_hex_literals`, `needless_arbitrary_self_type`, `needless_else`, `octal_escapes`, `precedence`, `redundant_at_rest_pattern`, `single_component_path_imports`, `unneeded_wildcard_pattern`, `unusual_byte_groupings` and `zero_prefixed_literal`. Off, as Clippy's default lint levels leave them (pedantic or restriction): `large_digit_groups`, `many_single_char_names`, `needless_raw_string_hashes`, `partial_pub_fields` and `precedence_bits`. 16 ports remain candidates.
- e1b9a49: Rust pack: ships 15 more rules, 33 in all, now that they pass every conformance check. Start on: `clippy-double-parens`, `clippy-inconsistent-digit-grouping-no-fractions`, `clippy-redundant-field-names` and `clippy-redundant-static-lifetimes`. Start off, as no approved upstream configuration turns them on: `clippy-allow-attributes-without-reason`, `clippy-allow-attributes`, `clippy-decimal-literal-representation-threshold`, `clippy-decimal-literal-representation`, `clippy-else-if-without-else`, `clippy-needless-raw-string-hashes-one-allowed`, `clippy-needless-raw-strings`, `clippy-single-char-lifetime-names`, `clippy-unnested-or-patterns`, `clippy-unreadable-literal-no-fractions` and `clippy-unreadable-literal`.

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
