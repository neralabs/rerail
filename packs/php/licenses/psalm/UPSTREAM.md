# vimeo/psalm 6.19.1

License: MIT (Copyright (c) 2016 Vimeo); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/vimeo/psalm | `6.19.1` | `e2ca44251c1f1aa2e35452d426895746b47de64e` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/psalm/fixtures/**` | `tests/*` | extracted: each file is the `code` of one case of a test's `providerValidCodeParse()` or `providerInvalidCodeParse()`, verbatim |

The files taken from it contain material copied from vimeo/psalm and remain under its license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of PHP_CodeSniffer's, PHPStan's and Psalm's rules, published under the pack's licence expression, `Apache-2.0 AND BSD-3-Clause AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/psalm/generate.py`, `upstream/psalm/read_tests.php` and the expected findings they make by running Psalm 6.19.1.

Each fixture is the `code` string of one case of a test's `providerValidCodeParse()` or `providerInvalidCodeParse()`, copied verbatim (`upstream/psalm/generate.py fixtures`).

Paths are relative to the pack directory, `packs/php/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/psalm/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept until commit `cbf682862a47`) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
