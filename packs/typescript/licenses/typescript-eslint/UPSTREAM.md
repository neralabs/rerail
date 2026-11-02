# typescript-eslint 8.70.1

License: MIT (Copyright (c) 2019 typescript-eslint and other contributors); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/typescript-eslint/typescript-eslint | `v8.70.1` | `23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/typescript-eslint/fixtures/**` | `packages/eslint-plugin/tests/rules/**` | extracted: each file is one RuleTester case's `code`, verbatim |
| `upstream/typescript-eslint/fixtures/renamed/**` | as its original | renamed: the file of the same path below `fixtures/`, with its names and literals renamed |

The files taken from it contain material copied from typescript-eslint and remain under its license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/` are Rerail's re-implementations of typescript-eslint's rules (and, in `candidates/rules/vitest-*.json`, of three @vitest/eslint-plugin 1.6.27 rules, which are MIT too), under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generator `upstream/generate.mjs`, and the `*.expected.json` expected findings it writes by running typescript-eslint on ESLint.

Each copied file is the `code` of one of the rule's own `valid` or `invalid` RuleTester cases, verbatim, written by `upstream/generate.mjs`: cases with rule options, a custom parser, or type information for a rule that requires type checking are left out, and identical code is written once. Each file under `upstream/typescript-eslint/fixtures/renamed/` is derived from the copied file of the same path below `fixtures/`, with its names and literals renamed.

Paths are relative to the pack directory, `packs/typescript/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/typescript-eslint/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
