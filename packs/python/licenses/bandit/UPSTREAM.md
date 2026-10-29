# Bandit 1.9.4

License: Apache-2.0 (Copyright 2014 Hewlett-Packard Development Company, L.P.); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/PyCQA/bandit | `1.9.4` | `92ae8b82fb422a639f0ed8d99e96cea769594e08` | Apache-2.0 |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/bandit/fixtures/**` | `examples/*` | verbatim |

The files taken from it contain material copied from Bandit and remain under its license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of Bandit's and Ruff's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the generators `upstream/common.py` and `upstream/bandit/generate.py`, and the `*.expected.json` expected findings written by running Bandit.

Bandit ships no NOTICE file, so Apache-2.0 §4(d) adds nothing to carry beyond the license and this record. The examples are unmodified.

Paths are relative to the pack directory, `packs/python/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/bandit/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept before the archive took the files out of git) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/bandit/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.
