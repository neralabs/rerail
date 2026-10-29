# Ruff 0.16.9

License: MIT (Copyright (c) 2022 Charles Marsh); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/astral-sh/ruff | `0.16.9` | `0be08a206f9c3180afd3e93bcc792ed5cb1f4db1` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/ruff/fixtures/**` | `crates/ruff_linter/resources/test/fixtures/**` | verbatim: each file keeps its path below upstream's `crates/ruff_linter/resources/test/fixtures/` |

The files taken from it contain material copied from Ruff and remain under its license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of Bandit's and Ruff's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/ruff/generate.py`, and the `*.expected.json` expected findings written by running Ruff.

Ruff's `LICENSE` also carries the notices of the projects Ruff derives from, Pyflakes among them. Ruff's pyflakes fixtures come with that notice.

Paths are relative to the pack directory, `packs/python/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/ruff/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/ruff/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.

## Not copied

Some of Ruff's rules are ports of plugins under copyleft licences, and Ruff's fixtures for them carry those plugins' test code. Nothing under these upstream paths is copied, so their rules have no expected findings, and no pack test data holds their text (`upstream/ruff/generate.py` reads the upstream prefixes, and the pack check refuses any pulled file under the pack paths):

- Not copied: `crates/ruff_linter/resources/test/fixtures/flake8_builtins/` as `upstream/ruff/fixtures/flake8_builtins/**` — flake8-builtins, GPL-2.0
- Not copied: `crates/ruff_linter/resources/test/fixtures/flake8_django/` as `upstream/ruff/fixtures/flake8_django/**` — flake8-django, GPL-3.0-or-later
- Not copied: `crates/ruff_linter/resources/test/fixtures/flake8_executable/` as `upstream/ruff/fixtures/flake8_executable/**` — flake8-executable, LGPL-3.0-or-later
- Not copied: `crates/ruff_linter/resources/test/fixtures/pylint/` as `upstream/ruff/fixtures/pylint/**` — pylint, GPL-2.0-or-later
- Not copied: `crates/ruff_linter/resources/test/fixtures/refurb/` as `upstream/ruff/fixtures/refurb/**` — refurb, GPL-3.0-only
