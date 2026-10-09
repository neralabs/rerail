# Bandit 1.8.6

Upstream: [Bandit 1.8.6](https://github.com/PyCQA/bandit/tree/1.8.6). License: Apache-2.0 (Copyright 2014 Hewlett-Packard Development Company, L.P.); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

Source: https://github.com/PyCQA/bandit
Revision: 2d0b675b04c80ae42277e10500db06a0a37bae17

The files listed below contain material copied from Bandit and remain under its license. Every other file in this
pack is Rerail's. The rule documents in `rules/` are Rerail's re-implementations of Bandit's and Ruff's rules,
published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under
the Apache License 2.0 (the pack's `LICENSE`): the recorders `upstream/common.py` and `upstream/bandit/generate.py`,
and the `*.expected.json` outputs recorded by running Bandit. `upstream/bandit/source.json`, the facts `conformance/recorders/python-source.ts` records from the upstream source and tests at the revision above, holds names, words and regexes extracted from them: it is Bandit's material and Rerail's, under `Apache-2.0`, as `REUSE.toml` annotates it, and the table lists it.

Bandit ships no NOTICE file, so Apache-2.0 §4(d) adds nothing to carry beyond the license and this record. The examples are unmodified.

Paths are relative to the pack directory, `packs/python/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/bandit/source.json` | the rules' implementations and tests, and the helpers they delegate to, at the revision above | extracted: the names, words and regexes `source.json` records |
| `upstream/bandit/fixtures/assert.py` | `examples/assert.py` | verbatim |
| `upstream/bandit/fixtures/django_sql_injection_extra.py` | `examples/django_sql_injection_extra.py` | verbatim |
| `upstream/bandit/fixtures/new_candidates-all.py` | `examples/new_candidates-all.py` | verbatim |
| `upstream/bandit/fixtures/requests-missing-timeout.py` | `examples/requests-missing-timeout.py` | verbatim |
| `upstream/bandit/fixtures/weak_cryptographic_key_sizes.py` | `examples/weak_cryptographic_key_sizes.py` | verbatim |
| `upstream/bandit/fixtures/yaml_load.py` | `examples/yaml_load.py` | verbatim |
