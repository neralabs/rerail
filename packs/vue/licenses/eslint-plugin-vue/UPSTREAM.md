# eslint-plugin-vue 10.11.1

License: MIT (Copyright (c) 2017 Toru Nagashima); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/vuejs/eslint-plugin-vue | `v10.11.1` | `ec9c53a6fbed637d7b0fbcd6893b0206ae32839f` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/eslint-plugin-vue/fixtures/**` | `tests/lib/rules/*` | verbatim, one RuleTester case's `code`; or adapted, a script case wrapped in a `<script>` element so it is a `.vue` file |

The files taken from it are the code of RuleTester cases of the rules the Vue pack ports (`../../conformance.json`), and remain under eslint-plugin-vue's license. Each is one case's `code`, copied verbatim, or wrapped in a `<script>` element when the case is a script (a non-`.vue` filename, a parser other than vue-eslint-parser, or code with no SFC block and no filename), so that it is a `.vue` file. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of eslint-plugin-vue's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. Every other file under `upstream/` is Rerail's, under the Apache License 2.0 (this repository's `LICENSE`): `read_tests.mjs`, `generate.mjs`, `authored.mjs`, `cases.json`, the fixtures `rerail-*.vue` (written by Rerail; `upstream/eslint-plugin-vue/UPSTREAM.md` lists them) and the expected findings they write by running ESLint 10.11.0 with eslint-plugin-vue 10.11.1.

mjs fixtures`. Paths are relative to the pack directory (`packs/vue/`).

Paths are relative to the pack directory, `packs/vue/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/eslint-plugin-vue/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept until commit `cbf682862a47`) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/eslint-plugin-vue/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.
