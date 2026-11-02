# eslint-plugin-react 7.37.5

License: MIT (Copyright (c) 2014 Yannick Croissant); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/jsx-eslint/eslint-plugin-react | `v7.37.5` | `2c98b83c451a4297edf1787d9a616e50687e27e8` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/eslint-plugin-react/tests/**` | `tests/lib/rules/*` | verbatim: the rule test files |
| `upstream/eslint-plugin-react/fixtures/**` | `tests/lib/rules/*` | extracted: each file is one RuleTester case's `code`, verbatim |
| `upstream/eslint-plugin-react/fixtures/renamed/**` | as its original | renamed: the file of the same path below `fixtures/`, with its names and literals renamed |

The files taken from it are the upstream tests of the rules the React pack ports (`conformance.json`, `upstream/common.mjs` `RULES`), and the test cases extracted from them, one case per file, and remain under eslint-plugin-react's license. Every other file under `upstream/eslint-plugin-react/` is Rerail's, under the Apache License 2.0 (the pack's `LICENSE`): `generate.mjs` (with its stub RuleTester), `../common.mjs` and the expected findings they write by running the pinned plugin. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of eslint-plugin-react's and eslint-plugin-react-hooks' rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them.

`provenance.json` locates each extracted case as `valid[i]` / `invalid[i]`, the i-th case of the `valid` / `invalid` list the test file passes to `RuleTester.run`.

Paths are relative to the pack directory, `packs/react/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/eslint-plugin-react/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/eslint-plugin-react/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.
