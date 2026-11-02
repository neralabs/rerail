# Bladestan 0.11.7

License: MIT; [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/TomasVotruba/bladestan | `0.11.7` | `ceb0823cd4b4f400b64a008cace0d91e786a2859` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/bladestan/fixtures/**` | `tests/skeleton/resources/**` | verbatim |

The files taken from it are the templates of Bladestan's rule test (`tests/Rules/BladeRuleTest.php`, its fixtures under `tests/Rules/Fixture/` render them) and remain under Bladestan's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of the validators and lints of stillat/blade-parser, Bladestan and tlint, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/bladestan/` (`generate.py`, `RecordTest.php`), and the expected findings they write by running Bladestan.

Paths are relative to the pack directory, `packs/blade/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/bladestan/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The expected findings recorded by running the tool are Rerail's.
