# @neralabs/rerail-pack-rust

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
