# Bandit 1.9.4

Upstream: [Bandit 1.9.4](https://github.com/PyCQA/bandit/tree/1.9.4). License: Apache-2.0 (Copyright 2014 Hewlett-Packard Development Company, L.P.); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below contain material copied from Bandit and remain under its license. Every other file in this pack
is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of Bandit's and
Ruff's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The
rest is under the Apache License 2.0 (the pack's `LICENSE`): the recorders `upstream/common.py` and
`upstream/bandit/generate.py`, and the `*.expected.json` outputs recorded by running Bandit.

Bandit ships no NOTICE file, so Apache-2.0 §4(d) adds nothing to carry beyond the license and this record. The examples are unmodified.

Paths are relative to the pack directory, `packs/python/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/bandit/fixtures/assert.py` | `examples/assert.py` | verbatim |
| `upstream/bandit/fixtures/binding.py` | `examples/binding.py` | verbatim |
| `upstream/bandit/fixtures/cipher-modes.py` | `examples/cipher-modes.py` | verbatim |
| `upstream/bandit/fixtures/ciphers.py` | `examples/ciphers.py` | verbatim |
| `upstream/bandit/fixtures/crypto-md5.py` | `examples/crypto-md5.py` | verbatim |
| `upstream/bandit/fixtures/dill.py` | `examples/dill.py` | verbatim |
| `upstream/bandit/fixtures/django_sql_injection_extra.py` | `examples/django_sql_injection_extra.py` | verbatim |
| `upstream/bandit/fixtures/eval.py` | `examples/eval.py` | verbatim |
| `upstream/bandit/fixtures/exec.py` | `examples/exec.py` | verbatim |
| `upstream/bandit/fixtures/ftplib.py` | `examples/ftplib.py` | verbatim |
| `upstream/bandit/fixtures/hardcoded-tmp.py` | `examples/hardcoded-tmp.py` | verbatim |
| `upstream/bandit/fixtures/huggingface_unsafe_download.py` | `examples/huggingface_unsafe_download.py` | verbatim |
| `upstream/bandit/fixtures/imports-aliases.py` | `examples/imports-aliases.py` | verbatim |
| `upstream/bandit/fixtures/jinja2_templating.py` | `examples/jinja2_templating.py` | verbatim |
| `upstream/bandit/fixtures/jsonpickle.py` | `examples/jsonpickle.py` | verbatim |
| `upstream/bandit/fixtures/marshal_deserialize.py` | `examples/marshal_deserialize.py` | verbatim |
| `upstream/bandit/fixtures/mktemp.py` | `examples/mktemp.py` | verbatim |
| `upstream/bandit/fixtures/new_candidates-all.py` | `examples/new_candidates-all.py` | verbatim |
| `upstream/bandit/fixtures/no_host_key_verification.py` | `examples/no_host_key_verification.py` | verbatim |
| `upstream/bandit/fixtures/os-chmod.py` | `examples/os-chmod.py` | verbatim |
| `upstream/bandit/fixtures/os-exec.py` | `examples/os-exec.py` | verbatim |
| `upstream/bandit/fixtures/os-spawn.py` | `examples/os-spawn.py` | verbatim |
| `upstream/bandit/fixtures/os-startfile.py` | `examples/os-startfile.py` | verbatim |
| `upstream/bandit/fixtures/pandas_read_pickle.py` | `examples/pandas_read_pickle.py` | verbatim |
| `upstream/bandit/fixtures/pickle_deserialize.py` | `examples/pickle_deserialize.py` | verbatim |
| `upstream/bandit/fixtures/pycrypto.py` | `examples/pycrypto.py` | verbatim |
| `upstream/bandit/fixtures/pyghmi.py` | `examples/pyghmi.py` | verbatim |
| `upstream/bandit/fixtures/random_module.py` | `examples/random_module.py` | verbatim |
| `upstream/bandit/fixtures/requests-missing-timeout.py` | `examples/requests-missing-timeout.py` | verbatim |
| `upstream/bandit/fixtures/shelve_open.py` | `examples/shelve_open.py` | verbatim |
| `upstream/bandit/fixtures/ssl-insecure-version.py` | `examples/ssl-insecure-version.py` | verbatim |
| `upstream/bandit/fixtures/telnetlib.py` | `examples/telnetlib.py` | verbatim |
| `upstream/bandit/fixtures/try_except_continue.py` | `examples/try_except_continue.py` | verbatim |
| `upstream/bandit/fixtures/try_except_pass.py` | `examples/try_except_pass.py` | verbatim |
| `upstream/bandit/fixtures/unverified_context.py` | `examples/unverified_context.py` | verbatim |
| `upstream/bandit/fixtures/urlopen.py` | `examples/urlopen.py` | verbatim |
| `upstream/bandit/fixtures/weak_cryptographic_key_sizes.py` | `examples/weak_cryptographic_key_sizes.py` | verbatim |
| `upstream/bandit/fixtures/wildcard-injection.py` | `examples/wildcard-injection.py` | verbatim |
| `upstream/bandit/fixtures/xml_etree_celementtree.py` | `examples/xml_etree_celementtree.py` | verbatim |
| `upstream/bandit/fixtures/xml_etree_elementtree.py` | `examples/xml_etree_elementtree.py` | verbatim |
| `upstream/bandit/fixtures/xml_expatbuilder.py` | `examples/xml_expatbuilder.py` | verbatim |
| `upstream/bandit/fixtures/xml_expatreader.py` | `examples/xml_expatreader.py` | verbatim |
| `upstream/bandit/fixtures/xml_minidom.py` | `examples/xml_minidom.py` | verbatim |
| `upstream/bandit/fixtures/xml_pulldom.py` | `examples/xml_pulldom.py` | verbatim |
| `upstream/bandit/fixtures/xml_sax.py` | `examples/xml_sax.py` | verbatim |
| `upstream/bandit/fixtures/xml_xmlrpc.py` | `examples/xml_xmlrpc.py` | verbatim |
