# @vitest/eslint-plugin 1.6.27

License: MIT (Copyright (c) 2022 - PRESENT Verité Mugabo Makuza); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/vitest-dev/eslint-plugin-vitest | `v1.6.27` | `aad57d121647efe764463c37414a8cd67ec9cf92` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/eslint-plugin-vitest/fixtures/**` | `tests/*` | extracted: each file is one RuleTester case's `code`, verbatim |
| `upstream/eslint-plugin-vitest/fixtures/renamed/**` | as its original | renamed: the file of the same path below `fixtures/`, with its names and literals renamed |

The files taken from it contain material copied from @vitest/eslint-plugin and remain under its license. The rule documents in `candidates/rules/vitest-*.json` are Rerail's re-implementations of three of the plugin's rules, under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The generator `upstream/eslint-plugin-vitest/generate.mjs` and the `*.expected.json` expected findings it writes by running the plugin are Rerail's, under the Apache License 2.0 (the pack's `LICENSE`).

Each copied file is the `code` of one of the rule's own `valid` or `invalid` RuleTester cases, verbatim, written by the generator: cases for another parser or file type are left out, and identical cases are written once. Each file under `upstream/eslint-plugin-vitest/fixtures/renamed/` is derived from the copied file of the same path below `fixtures/`, with its names and literals renamed.

Paths are relative to the pack directory, `packs/typescript/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/eslint-plugin-vitest/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/eslint-plugin-vitest/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.
