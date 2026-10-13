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
| `upstream/bandit/fixtures/django_sql_injection_raw.py` | `examples/django_sql_injection_raw.py` | verbatim |
| `upstream/bandit/fixtures/eval.py` | `examples/eval.py` | verbatim |
| `upstream/bandit/fixtures/exec.py` | `examples/exec.py` | verbatim |
| `upstream/bandit/fixtures/flask_debug.py` | `examples/flask_debug.py` | verbatim |
| `upstream/bandit/fixtures/ftplib.py` | `examples/ftplib.py` | verbatim |
| `upstream/bandit/fixtures/hardcoded-passwords.py` | `examples/hardcoded-passwords.py` | verbatim |
| `upstream/bandit/fixtures/hardcoded-tmp.py` | `examples/hardcoded-tmp.py` | verbatim |
| `upstream/bandit/fixtures/hashlib_new_insecure_functions.py` | `examples/hashlib_new_insecure_functions.py` | verbatim |
| `upstream/bandit/fixtures/httpoxy_cgihandler.py` | `examples/httpoxy_cgihandler.py` | verbatim |
| `upstream/bandit/fixtures/httpoxy_twisted_directory.py` | `examples/httpoxy_twisted_directory.py` | verbatim |
| `upstream/bandit/fixtures/httpoxy_twisted_script.py` | `examples/httpoxy_twisted_script.py` | verbatim |
| `upstream/bandit/fixtures/huggingface_unsafe_download.py` | `examples/huggingface_unsafe_download.py` | verbatim |
| `upstream/bandit/fixtures/imports-aliases.py` | `examples/imports-aliases.py` | verbatim |
| `upstream/bandit/fixtures/imports-from.py` | `examples/imports-from.py` | verbatim |
| `upstream/bandit/fixtures/imports-function.py` | `examples/imports-function.py` | verbatim |
| `upstream/bandit/fixtures/imports-with-importlib.py` | `examples/imports-with-importlib.py` | verbatim |
| `upstream/bandit/fixtures/imports.py` | `examples/imports.py` | verbatim |
| `upstream/bandit/fixtures/jinja2_templating.py` | `examples/jinja2_templating.py` | verbatim |
| `upstream/bandit/fixtures/jsonpickle.py` | `examples/jsonpickle.py` | verbatim |
| `upstream/bandit/fixtures/logging_config_insecure_listen.py` | `examples/logging_config_insecure_listen.py` | verbatim |
| `upstream/bandit/fixtures/mako_templating.py` | `examples/mako_templating.py` | verbatim |
| `upstream/bandit/fixtures/mark_safe.py` | `examples/mark_safe.py` | verbatim |
| `upstream/bandit/fixtures/mark_safe_insecure.py` | `examples/mark_safe_insecure.py` | verbatim |
| `upstream/bandit/fixtures/mark_safe_secure.py` | `examples/mark_safe_secure.py` | verbatim |
| `upstream/bandit/fixtures/markupsafe_markup_xss.py` | `examples/markupsafe_markup_xss.py` | verbatim |
| `upstream/bandit/fixtures/markupsafe_markup_xss_allowed_calls.py` | `examples/markupsafe_markup_xss_allowed_calls.py` | verbatim |
| `upstream/bandit/fixtures/markupsafe_markup_xss_extend_markup_names.py` | `examples/markupsafe_markup_xss_extend_markup_names.py` | verbatim |
| `upstream/bandit/fixtures/marshal_deserialize.py` | `examples/marshal_deserialize.py` | verbatim |
| `upstream/bandit/fixtures/mktemp.py` | `examples/mktemp.py` | verbatim |
| `upstream/bandit/fixtures/multiline_statement.py` | `examples/multiline_statement.py` | verbatim |
| `upstream/bandit/fixtures/new_candidates-all.py` | `examples/new_candidates-all.py` | verbatim |
| `upstream/bandit/fixtures/new_candidates-nosec.py` | `examples/new_candidates-nosec.py` | verbatim |
| `upstream/bandit/fixtures/new_candidates-some.py` | `examples/new_candidates-some.py` | verbatim |
| `upstream/bandit/fixtures/no_host_key_verification.py` | `examples/no_host_key_verification.py` | verbatim |
| `upstream/bandit/fixtures/nosec.py` | `examples/nosec.py` | verbatim |
| `upstream/bandit/fixtures/os-chmod.py` | `examples/os-chmod.py` | verbatim |
| `upstream/bandit/fixtures/os-exec.py` | `examples/os-exec.py` | verbatim |
| `upstream/bandit/fixtures/os-popen.py` | `examples/os-popen.py` | verbatim |
| `upstream/bandit/fixtures/os-spawn.py` | `examples/os-spawn.py` | verbatim |
| `upstream/bandit/fixtures/os-startfile.py` | `examples/os-startfile.py` | verbatim |
| `upstream/bandit/fixtures/os_system.py` | `examples/os_system.py` | verbatim |
| `upstream/bandit/fixtures/pandas_read_pickle.py` | `examples/pandas_read_pickle.py` | verbatim |
| `upstream/bandit/fixtures/paramiko_injection.py` | `examples/paramiko_injection.py` | verbatim |
| `upstream/bandit/fixtures/partial_path_process.py` | `examples/partial_path_process.py` | verbatim |
| `upstream/bandit/fixtures/pickle_deserialize.py` | `examples/pickle_deserialize.py` | verbatim |
| `upstream/bandit/fixtures/popen_wrappers.py` | `examples/popen_wrappers.py` | verbatim |
| `upstream/bandit/fixtures/pycrypto.py` | `examples/pycrypto.py` | verbatim |
| `upstream/bandit/fixtures/pyghmi.py` | `examples/pyghmi.py` | verbatim |
| `upstream/bandit/fixtures/pytorch_load.py` | `examples/pytorch_load.py` | verbatim |
| `upstream/bandit/fixtures/random_module.py` | `examples/random_module.py` | verbatim |
| `upstream/bandit/fixtures/requests-missing-timeout.py` | `examples/requests-missing-timeout.py` | verbatim |
| `upstream/bandit/fixtures/requests-ssl-verify-disabled.py` | `examples/requests-ssl-verify-disabled.py` | verbatim |
| `upstream/bandit/fixtures/shelve_open.py` | `examples/shelve_open.py` | verbatim |
| `upstream/bandit/fixtures/skip.py` | `examples/skip.py` | verbatim |
| `upstream/bandit/fixtures/snmp.py` | `examples/snmp.py` | verbatim |
| `upstream/bandit/fixtures/sql_multiline_statements.py` | `examples/sql_multiline_statements.py` | verbatim |
| `upstream/bandit/fixtures/sql_statements.py` | `examples/sql_statements.py` | verbatim |
| `upstream/bandit/fixtures/ssl-insecure-version.py` | `examples/ssl-insecure-version.py` | verbatim |
| `upstream/bandit/fixtures/subprocess_shell.py` | `examples/subprocess_shell.py` | verbatim |
| `upstream/bandit/fixtures/tarfile_extractall.py` | `examples/tarfile_extractall.py` | verbatim |
| `upstream/bandit/fixtures/telnetlib.py` | `examples/telnetlib.py` | verbatim |
| `upstream/bandit/fixtures/trojansource.py` | `examples/trojansource.py` | verbatim |
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
| `upstream/bandit/fixtures/yaml_load.py` | `examples/yaml_load.py` | verbatim |
