# tlint 9.7.0

License: MIT (Copyright (c) Tighten Co.); [`LICENSE.md`](LICENSE.md) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/tighten/tlint | `v9.7.0` | `e768bedce2bd84930712f9412ecbf2d4c586735c` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/tlint/fixtures/**` | `tests/Linting/Linters/*` | extracted: each file is the template a test hands the lint, captured verbatim as the linter receives it |

The files taken from it are the templates the tests of the lints the Blade pack ports (the pack's `conformance.json`) hand those lints, and remain under tlint's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of the validators and lints of stillat/blade-parser, Bladestan and tlint, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/tlint/` (`generate.py`, `read_tests.php`, `run.php`), and the expected findings they write by running tlint 9.7.0.

Each fixture is written by `upstream/tlint/generate.py fixtures`, which runs the test with the template captured as the linter receives it.

Paths are relative to the pack directory, `packs/blade/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/tlint/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept until commit `cbf682862a47`) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
