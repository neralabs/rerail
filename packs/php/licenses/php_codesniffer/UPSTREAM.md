# PHP_CodeSniffer 4.0.4

License: BSD-3-Clause (Copyright (c) 2012, Squiz Pty Ltd (ABN 77 084 670 600), Copyright (c) 2023, PHPCSStandards and contributors); [`licence.txt`](licence.txt) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/PHPCSStandards/PHP_CodeSniffer | `4.0.4` | `bbdc3d0532623e21838b7041a4364383a8126f96` | BSD-3-Clause |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/phpcs/fixtures/**` | `src/Standards/**` | verbatim, renamed with a `.php` suffix so the host routes it to PHP |

The files taken from it are the unit-test fixtures of the sniffs the PHP pack ports (the pack's `conformance.json`) and remain under PHP_CodeSniffer's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of PHP_CodeSniffer's, PHPStan's and Psalm's rules, published under the pack's licence expression, `Apache-2.0 AND BSD-3-Clause AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/phpcs/generate.py`, `upstream/phpcs/error_lists.php`, `upstream/phpcs/ruleset.xml` and the expected findings they write by running PHP_CodeSniffer.

Paths are relative to the pack directory, `packs/php/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/phpcs/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
