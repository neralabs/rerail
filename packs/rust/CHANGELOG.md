# @neralabs/rerail-pack-rust

## 0.1.1

### Patch Changes

- b1952f2: Rust pack: ships no rules yet. Its 34 Clippy 0.1.99 ports are candidates until they pass the V1 rule validation gates, scored against Clippy's own findings on its UI tests and on pinned ripgrep and tokio checkouts with every disagreement declared with its evidence, and the pack is never recommended while it has no rules. Each rule's id is `clippy-<lint>` (a configuration variant adds its setting, as `clippy-unreadable-literal-no-fractions`), the same string as its document's file name and its port's id in `conformance.json`; the package names the repository and directory it is published from, `neralabs/rerail` at `packs/rust`.
