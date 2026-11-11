# stillat/blade-parser 2.1.0

License: MIT (Copyright (c) Johnathon Koster); [`license.md`](license.md) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/stillat/blade-parser | `v2.1.0` | `23a962f99bca451f8738ed2475a70129fb33f07c` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/blade-parser/fixtures/**` | `tests/Validation/*` | extracted: each file is the template a test hands `Document::fromText()`, captured verbatim as the parser receives it |

The files taken from it are the templates the tests of the validators the Blade pack ports (the pack's `conformance.json`) hand the parser, and remain under stillat/blade-parser's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of the validators and lints of stillat/blade-parser, Bladestan and tlint, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py`, `upstream/blade_capture.php` and `upstream/blade-parser/` (`generate.py`, `read_tests.php`, `run.php`), and the expected findings they write by running stillat/blade-parser 2.1.0.

Each fixture is written by `upstream/blade-parser/generate.py fixtures`, which runs the test with the template captured as the parser receives it; a template several dataset rows of a test file build alike is written once, under the first.

Paths are relative to the pack directory, `packs/blade/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/blade-parser/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
