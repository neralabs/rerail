# SQLFluff 4.4.0

License: MIT (Copyright (c) 2018-2026 Alan Cruickshank); [`LICENSE.md`](LICENSE.md) is the upstream license file, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/sqlfluff/sqlfluff | `4.4.0` | `0348222a3bcdb0226563f7349c5a39beb0641955` | MIT |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/sqlfluff/fixtures/**` | `test/fixtures/rules/std_rule_cases/*` | extracted: each file is one YAML case's `pass_str` or `fail_str`, byte for byte, named after the case |

The files taken from it are SQL strings taken from the YAML test cases of the SQLFluff rules the SQL pack ports (the pack's `conformance.json`) and remain under SQLFluff's license. Each file is one case's `pass_str` or `fail_str`, written byte for byte; the case's name is the file name. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of SQLFluff's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. Every other file under `upstream/sqlfluff/`, including the cases Rerail wrote itself (`fixtures/rerail/`), is Rerail's, under the Apache License 2.0 (the pack's `LICENSE`): `generate.py` and the expected findings it writes by running SQLFluff (`fixtures.expected.json`, `fixtures.cases.json`).

Paths are relative to the pack directory, `packs/sql/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/sqlfluff/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept until commit `cbf682862a47`) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/sqlfluff/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.
