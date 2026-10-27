# ESLint 10.11.0

License: MIT (Copyright OpenJS Foundation and other contributors, <www.openjsf.org>); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/eslint/eslint | `v10.11.0` | `3c0b7c6e1fe2dec778b97a996018126f22d437ae` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/eslint/fixtures/**` | `tests/lib/rules/*` | extracted: each file is one RuleTester case's `code`, verbatim |
| `upstream/eslint/fixtures/renamed/**` | as its original | renamed: the file of the same path below `fixtures/`, with its names and literals renamed |

The files taken from it contain material copied from ESLint and remain under its license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of ESLint's rules (and, for `candidates/rules/unicorn-*.json`, of eslint-plugin-unicorn's, from which no file is copied), published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generator `upstream/generate.mjs`, and the `*.expected.json` expected findings it writes by running ESLint.

Each copied file is the `code` of one of the rule's own `valid` or `invalid` RuleTester cases, verbatim, written by `upstream/generate.mjs`: cases with a custom parser or type information, and performance stress cases over 64 KiB, are left out, and identical code in the same language is written once. Each file under `upstream/eslint/fixtures/renamed/` is derived from the copied file of the same path below `fixtures/`, with its names and literals renamed.

Paths are relative to the pack directory, `packs/javascript/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/eslint/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
