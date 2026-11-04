# Next.js 16.3.8 (@next/eslint-plugin-next)

License: MIT (Copyright (c) 2025 Vercel, Inc.); [`LICENSE`](LICENSE) is the upstream `license.md`, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/vercel/next.js | `v16.3.8` | `b0fad0d45eb4c4430fda5eeeb442e8a5af08a5f6` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/eslint-plugin-next/tests/**` | `test/unit/eslint-plugin-next/*` | verbatim: the rule test files |
| `upstream/eslint-plugin-next/fixtures/**` | `test/unit/eslint-plugin-next/*` | extracted: each file is one RuleTester case's `code`, verbatim |
| `upstream/eslint-plugin-next/fixtures/renamed/**` | as its original | renamed: the file of the same path below `fixtures/`, with its names renamed (literals and the names' order kept) |

The files taken from it are the plugin's rule tests and the test cases extracted from them, and remain under Next.js's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of @next/eslint-plugin-next's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generator `upstream/eslint-plugin-next/generate.mjs` (with its stub `RuleTester` and stub plugin module), `upstream/eslint-plugin-next/fixtures.cases.json`, and the `*.expected.json` expected findings written by running the pinned plugin.

Paths are relative to the pack directory, `packs/next/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/eslint-plugin-next/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/eslint-plugin-next/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.
