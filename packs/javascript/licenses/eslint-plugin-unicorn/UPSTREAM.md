# eslint-plugin-unicorn 76.0.0

License: MIT (Copyright (c) Sindre Sorhus <sindresorhus@gmail.com> (https://sindresorhus.com)); [`LICENSE`](LICENSE) is the upstream license file (`license` in the repository), copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/sindresorhus/eslint-plugin-unicorn | `v76.0.0` | `f70155ebe24dbb35b9814c3dbb3ab3620e09c3e1` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/eslint-plugin-unicorn/fixtures/**` | `test/*` | extracted: each file is one test case's `code`, verbatim |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/**` | as its original | renamed: the file of the same path below `fixtures/`, with its names renamed (literals and the names' order kept) |

The files taken from it contain material copied from eslint-plugin-unicorn and remain under its license. The rule documents `rules/unicorn-*.json` and `candidates/rules/unicorn-*.json` are Rerail's re-implementations of four of the plugin's rules (three shipped, one candidate), under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The generator `upstream/eslint-plugin-unicorn/generate.mjs` and the expected findings it writes by running the plugin (`fixtures.expected.json`) are Rerail's, under the Apache License 2.0 (the pack's `LICENSE`).

Each copied file is the `code` of one of the rule's own `valid` or `invalid` test cases, verbatim, written by the generator: cases for another parser or file type are left out, and identical cases are written once. Each file under `upstream/eslint-plugin-unicorn/fixtures/renamed/` is derived from the copied file of the same path below `fixtures/`, with its names renamed (literals and the names' order kept).

Paths are relative to the pack directory, `packs/javascript/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/eslint-plugin-unicorn/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
