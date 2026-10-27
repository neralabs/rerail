# eslint-plugin-n 18.4.1

License: MIT (Copyright (c) 2015 Toru Nagashima); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/eslint-community/eslint-plugin-n | `v18.4.1` | `f1d47aff402b9e009578f4b3155b84f7d8beef9b` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/eslint-plugin-n/tests/**` | `tests/lib/rules/**` | verbatim: the rule test files |
| `upstream/eslint-plugin-n/fixtures/**` | `tests/**` | extracted: each file is one RuleTester case's `code`, verbatim; and verbatim, the upstream test data (`tests/fixtures/**`) a case's `filename` reads |
| `upstream/eslint-plugin-n/fixtures/renamed/**` | as its original | renamed: the file of the same path below `fixtures/`, with its names and literals renamed; and verbatim, a copy of the test data beside the variants |

The files taken from it are the upstream rule tests of the rules the Node pack ports (`conformance.json`, `upstream/eslint-plugin-n/generate.mjs`), the upstream test data they read, and one file per test case holding that case's `code` verbatim; they remain under eslint-plugin-n's license. Two files under `upstream/eslint-plugin-n/fixtures/` are Rerail's although the pattern above covers them: `fixtures/package.json` (the fixtures' source type and engines) and its copy `fixtures/renamed/package.json`; `provenance.json` records them as Rerail's. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of eslint-plugin-n's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generator `upstream/eslint-plugin-n/generate.mjs`, `upstream/eslint-plugin-n/stub-test-helpers.js` (stands in for upstream's `tests/test-helpers.js` and only captures cases), and the `*.expected.json` expected findings written by running eslint-plugin-n.

Paths are relative to the pack directory, `packs/node/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/eslint-plugin-n/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept until commit `cbf682862a47`) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
