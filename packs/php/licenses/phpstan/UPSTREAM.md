# PHPStan 2.2.16

License: MIT (Copyright (c) 2016 Ondřej Mirtes, Copyright (c) 2025 PHPStan s.r.o.); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/phpstan/phpstan-src | `2.2.16` | `08e3736efd29631ab8f32596c5dd2362dec8dc26` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/phpstan/fixtures/**` | `tests/PHPStan/**` | verbatim |

The files taken from it are test data of the rules the PHP pack ports (the pack's `conformance.json`), analysed by their RuleTestCase tests, and remain under PHPStan's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of PHP_CodeSniffer's, PHPStan's and Psalm's rules, published under the pack's licence expression, `Apache-2.0 AND BSD-3-Clause AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/phpstan/generate.py`, `upstream/phpstan/read_tests.php` and the expected findings they write from the tests' expectations and by running PHPStan.

Paths are relative to the pack directory, `packs/php/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/phpstan/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
