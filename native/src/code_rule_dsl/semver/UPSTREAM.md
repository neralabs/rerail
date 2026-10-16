# node-semver 7.8.5

Upstream: [npm/node-semver](https://github.com/npm/node-semver/tree/6e05b7637396ac66522cff8731f07cfe0ef49a29) at tag `v7.8.5` (commit `6e05b7637396ac66522cff8731f07cfe0ef49a29`), the `semver` package 7.8.5. License: ISC (Copyright (c) Isaac Z. Schlueter and Contributors); the licence text is [`LICENSES/ISC.txt`](../../../../LICENSES/ISC.txt), and the copyright notice and full text are in [`THIRD_PARTY_NOTICES`](../../../../THIRD_PARTY_NOTICES).

Both files below are under ISC, as `REUSE.toml` annotates them. The recorder that writes the cases, `scripts/generate/semver/semver-subset-cases.mjs`, is Rerail's (Apache-2.0).

Paths are relative to this directory, `native/src/code_rule_dsl/semver/`.

| File | Upstream path | Copy |
|---|---|---|
| `mod.rs` | `classes/range.js`, `classes/comparator.js`, `classes/semver.js`, `internal/re.js`, `internal/constants.js`, `internal/identifiers.js`, `ranges/subset.js` | derived: translated to Rust for the default options only (`loose` and `includePrerelease` off), the regexes compiled for whitespace-normalized range text, and only what parsing a range and `subset()` need kept |
| `subset_cases.json` | the outputs of `subset()` and `new Range()` in `semver` 7.8.5 | recorded: node-semver's own answers, written by `node scripts/generate/semver/semver-subset-cases.mjs <tools>/node_modules/semver` |
