# eslint-plugin-n 18.4.1 `getNpmignore`, node-ignore 5.3.2, globrex 0.1.2

Upstream:

- [eslint-community/eslint-plugin-n](https://github.com/eslint-community/eslint-plugin-n/tree/f1d47aff402b9e009578f4b3155b84f7d8beef9b) at tag `v18.4.1` (commit `f1d47aff402b9e009578f4b3155b84f7d8beef9b`). License: MIT (Copyright (c) 2015 Toru Nagashima).
- [kaelzhang/node-ignore](https://github.com/kaelzhang/node-ignore/tree/cfd0fce1258fe87950d8ce3d18b345bf7d4ed780), the `ignore` package 5.3.2 (commit `cfd0fce1258fe87950d8ce3d18b345bf7d4ed780`). License: MIT (Copyright (c) 2013 Kael Zhang <i@kael.me>, contributors).
- [terkelg/globrex](https://github.com/terkelg/globrex/tree/9f6521b4a97664b2270b01de1f44a371a425e298), the `globrex` package 0.1.2 (commit `9f6521b4a97664b2270b01de1f44a371a425e298`). License: MIT (Copyright (c) 2018 Terkel Gjervig Nielsen).

The licence text is [`LICENSES/MIT.txt`](../../../../LICENSES/MIT.txt); the copyright notices and full texts are in [`THIRD_PARTY_NOTICES`](../../../../THIRD_PARTY_NOTICES).

Both files below are under MIT, as `REUSE.toml` annotates them. The tests (`tests.rs`) and the recorder that writes the cases, `scripts/generate/npm-ignore/npm-ignore-cases.mjs`, are Rerail's (Apache-2.0).

Paths are relative to this directory, `native/src/code_rule_dsl/npm_ignore/`.

| File | Upstream path | Copy |
|---|---|---|
| `mod.rs` | eslint-plugin-n `lib/util/get-npmignore.js`; node-ignore `index.js`; globrex `index.js` | derived: translated to Rust: `getNpmignore`'s `match` for one package (its `files`, `main`, `.npmignore` and `.gitignore` passed in, the never-ignored names tested as when ESLint runs in the package folder), node-ignore's `ignore()` with its default options (`add`, `ignores`, the `REPLACERS` chain run on `fancy_regex`), globrex with `extended` and `globstar` on a POSIX host; JavaScript class syntax that Rust reads differently (`[`, `&`, `~` in a class) is escaped, brace sequences expand to at most 4,096 patterns, and a pattern whose regex does not compile is left out |
| `cases.json` | the outputs of `getNpmignore(...).match()` in eslint-plugin-n 18.4.1 and of `ignore().add().ignores()` in `ignore` 5.3.2 | recorded: the packages' own answers, written by `node scripts/generate/npm-ignore/npm-ignore-cases.mjs <tools>/node_modules` |
