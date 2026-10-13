---
"@neralabs/rerail-pack-rust": minor
---

Rust pack: 34 rules ported from Clippy 0.1.99, each scored against Clippy's own findings on its UI tests and on pinned ripgrep and tokio checkouts, with every disagreement declared with its evidence. Each rule's id is `clippy-<lint>` (a configuration variant adds its setting, as `clippy-unreadable-literal-no-fractions`), the same string as its document's file name and its port's id in `conformance.json`; the package names the repository and directory it is published from, `neralabs/rerail` at `packs/rust`.
