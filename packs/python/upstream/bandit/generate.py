"""Records Bandit 1.9.4 for the Python pack's ported tests (rerail/packs/python/conformance.json).
Run offline, never in CI, under CPython 3.14 (Bandit itself runs from its own install, on CPython 3.13):

    python3.14 generate.py fixtures <Bandit checkout at tag 1.9.4> <bandit executable, 1.9.4>
    python3.14 generate.py corpus <bandit executable, 1.9.4> <name>=<corpus checkout> [...]

`fixtures` copies the examples each test fires in (`examples/<file>`; see TESTS) to fixtures/<file> (the pack's
licenses/bandit/UPSTREAM.md lists them), writes Rerail's own cases (NOSEC, PROBES) to fixtures/rerail/ (listed in
upstream/bandit/UPSTREAM.md), and runs Bandit with every ported test over every file. `corpus` runs the same
tests over a corpus's pack paths, files CPython 3.14 does not parse left out. Both run as bare `bandit` does: `# nosec`
honoured, every severity and confidence. A file Bandit reports an error for is skipped. Each finding is the test id,
path, line and column (`col_offset` + 1: Bandit's is 0-based)."""

from __future__ import annotations

import json
import pathlib
import shutil
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

VERSION = "1.9.4"
ORIGIN = "https://github.com/PyCQA/bandit@92ae8b82fb422a639f0ed8d99e96cea769594e08"

# Each ported test, shipped or candidate, and the examples it fires in when `# nosec` is ignored (bare `bandit
# --ignore-nosec` over every file of examples/): its examples include the lines `# nosec` suppresses, which the
# recording, run as bare `bandit` honouring `# nosec`, keeps without a finding.
TESTS: dict[str, list[str]] = {
    "B101": ["assert.py"],
    "B102": ["exec.py"],
    "B103": ["os-chmod.py"],
    "B104": ["binding.py"],
    "B105": ["hardcoded-passwords.py"],
    "B106": ["hardcoded-passwords.py", "pyghmi.py"],
    "B107": ["hardcoded-passwords.py"],
    "B108": ["hardcoded-tmp.py", "os-chmod.py"],
    "B110": ["try_except_pass.py"],
    "B112": ["try_except_continue.py"],
    "B113": ["requests-missing-timeout.py"],
    "B201": ["flask_debug.py"],
    "B202": ["tarfile_extractall.py"],
    "B301": [
        "dill.py",
        "imports-aliases.py",
        "jsonpickle.py",
        "pandas_read_pickle.py",
        "pickle_deserialize.py",
        "shelve_open.py",
    ],
    "B302": ["marshal_deserialize.py"],
    "B303": ["ciphers.py", "crypto-md5.py", "nosec.py"],
    "B304": ["ciphers.py"],
    "B305": ["cipher-modes.py"],
    "B306": ["mktemp.py"],
    "B307": ["eval.py"],
    "B308": ["mark_safe.py", "mark_safe_insecure.py", "mark_safe_secure.py"],
    "B310": ["urlopen.py"],
    "B311": ["mark_safe_insecure.py", "random_module.py"],
    "B312": ["telnetlib.py"],
    "B313": ["xml_etree_celementtree.py"],
    "B314": ["xml_etree_elementtree.py"],
    "B315": ["xml_expatreader.py"],
    "B316": ["xml_expatbuilder.py"],
    "B317": ["new_candidates-all.py", "new_candidates-nosec.py", "new_candidates-some.py", "xml_sax.py"],
    "B318": ["xml_minidom.py"],
    "B319": ["xml_pulldom.py"],
    "B321": ["ftplib.py"],
    "B323": ["unverified_context.py"],
    "B324": ["crypto-md5.py", "hashlib_new_insecure_functions.py", "imports-aliases.py"],
    "B401": ["telnetlib.py"],
    "B402": ["ftplib.py"],
    "B403": [
        "dill.py",
        "imports-aliases.py",
        "imports-function.py",
        "imports-with-importlib.py",
        "imports.py",
        "pandas_read_pickle.py",
        "pickle_deserialize.py",
        "shelve_open.py",
    ],
    "B404": [
        "imports-aliases.py",
        "imports-from.py",
        "imports-function.py",
        "imports-with-importlib.py",
        "imports.py",
        "multiline_statement.py",
        "nosec.py",
        "partial_path_process.py",
        "subprocess_shell.py",
        "wildcard-injection.py",
    ],
    "B405": ["xml_etree_celementtree.py", "xml_etree_elementtree.py"],
    "B406": ["xml_expatreader.py", "xml_sax.py"],
    "B407": ["xml_expatbuilder.py"],
    "B408": ["xml_minidom.py"],
    "B409": ["xml_pulldom.py"],
    "B411": ["xml_xmlrpc.py"],
    "B412": ["httpoxy_cgihandler.py", "httpoxy_twisted_directory.py", "httpoxy_twisted_script.py"],
    "B413": ["ciphers.py", "crypto-md5.py", "pycrypto.py", "weak_cryptographic_key_sizes.py"],
    "B415": ["pyghmi.py"],
    "B501": ["requests-ssl-verify-disabled.py"],
    "B502": ["ssl-insecure-version.py"],
    "B503": ["ssl-insecure-version.py"],
    "B504": ["ssl-insecure-version.py"],
    "B505": ["weak_cryptographic_key_sizes.py"],
    "B506": ["new_candidates-all.py", "new_candidates-nosec.py", "new_candidates-some.py", "yaml_load.py"],
    "B507": ["no_host_key_verification.py"],
    "B508": ["snmp.py"],
    "B509": ["snmp.py"],
    "B601": ["paramiko_injection.py"],
    "B602": [
        "imports-aliases.py",
        "multiline_statement.py",
        "new_candidates-all.py",
        "new_candidates-nosec.py",
        "new_candidates-some.py",
        "nosec.py",
        "subprocess_shell.py",
        "wildcard-injection.py",
    ],
    "B603": ["partial_path_process.py", "skip.py", "subprocess_shell.py", "wildcard-injection.py"],
    "B604": ["subprocess_shell.py"],
    "B605": ["os-popen.py", "os_system.py", "popen_wrappers.py", "subprocess_shell.py", "wildcard-injection.py"],
    "B606": ["os-exec.py", "os-spawn.py", "os-startfile.py", "wildcard-injection.py"],
    "B607": ["nosec.py", "partial_path_process.py"],
    "B608": ["sql_multiline_statements.py", "sql_statements.py"],
    "B609": ["wildcard-injection.py"],
    "B610": ["django_sql_injection_extra.py"],
    "B611": ["django_sql_injection_raw.py"],
    "B612": ["logging_config_insecure_listen.py"],
    "B613": ["trojansource.py"],
    "B614": ["pytorch_load.py"],
    "B615": ["huggingface_unsafe_download.py"],
    "B701": ["jinja2_templating.py"],
    "B702": ["mako_templating.py"],
    "B703": ["mark_safe_insecure.py", "mark_safe_secure.py"],
    "B704": [
        "markupsafe_markup_xss.py",
        "markupsafe_markup_xss_allowed_calls.py",
        "markupsafe_markup_xss_extend_markup_names.py",
    ],
}


# Ours: one case per shipped Bandit test that Bandit reports, `{c}` where the reported line ends.
# fixtures/rerail/nosec/<Bxxx>.py holds it once without a comment and once after each of NOSEC_COMMENTS, so check 4
# fails when a port stops honouring `# nosec` as Bandit does.
NOSEC = {
    "B101": "assert x{c}\n",
    "B102": "exec('x = 1'){c}\n",
    "B104": "host = '0.0.0.0'{c}\n",
    "B108": "path = '/tmp/data'{c}\n",
    "B110": "try:\n    run()\nexcept Exception:{c}\n    pass\n",
    "B112": "for item in items:\n    try:\n        run(item)\n    except Exception:{c}\n        continue\n",
    "B113": "import requests\nrequests.get('https://example.com'){c}\n",
    "B301": "import pickle\npickle.loads(data){c}\n",
    "B302": "import marshal\nmarshal.loads(data){c}\n",
    "B305": "from cryptography.hazmat.primitives.ciphers import modes\nmodes.ECB(){c}\n",
    "B306": "import tempfile\ntempfile.mktemp(){c}\n",
    "B307": "eval('1 + 1'){c}\n",
    "B310": "import urllib.request\nurllib.request.urlopen(url){c}\n",
    "B311": "import random\nrandom.random(){c}\n",
    "B312": "import telnetlib\ntelnetlib.Telnet('example.com'){c}\n",
    "B313": "import xml.etree.cElementTree as ET\nET.parse('data.xml'){c}\n",
    "B314": "import xml.etree.ElementTree as ET\nET.parse('data.xml'){c}\n",
    "B315": "import xml.sax.expatreader\nxml.sax.expatreader.create_parser(){c}\n",
    "B316": "import xml.dom.expatbuilder\nxml.dom.expatbuilder.parse('data.xml'){c}\n",
    "B317": "import xml.sax\nxml.sax.parse('data.xml', handler){c}\n",
    "B318": "import xml.dom.minidom\nxml.dom.minidom.parse('data.xml'){c}\n",
    "B319": "import xml.dom.pulldom\nxml.dom.pulldom.parse('data.xml'){c}\n",
    "B321": "import ftplib\nftplib.FTP('example.com'){c}\n",
    "B323": "import ssl\nssl._create_unverified_context(){c}\n",
    "B401": "import telnetlib{c}\n",
    "B402": "import ftplib{c}\n",
    "B405": "import xml.etree.ElementTree{c}\n",
    "B406": "import xml.sax{c}\n",
    "B407": "import xml.dom.expatbuilder{c}\n",
    "B408": "import xml.dom.minidom{c}\n",
    "B409": "import xml.dom.pulldom{c}\n",
    "B411": "import xmlrpc{c}\n",
    "B413": "import Crypto.Cipher{c}\n",
    "B415": "import pyghmi{c}\n",
    "B502": "import ssl\nssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1){c}\n",
    "B505": "from cryptography.hazmat.primitives.asymmetric import rsa\n"
            "rsa.generate_private_key(public_exponent=65537, key_size=1024){c}\n",
    "B507": "import paramiko\nclient = paramiko.SSHClient()\n"
            "client.set_missing_host_key_policy(paramiko.AutoAddPolicy()){c}\n",
    "B606": "import os\nos.execl('/bin/ls', 'ls'){c}\n",
    "B610": "User.objects.extra(where=[condition]){c}\n",
    "B615": "from transformers import AutoModel\nAutoModel.from_pretrained('org/model'){c}\n",
    "B701": "import jinja2\njinja2.Environment(autoescape=False){c}\n",
}

NOSEC_COMMENTS = ["", "  # nosec", "  # nosec {code}", "  # nosec: {code}", "  # nosec {name}", "  # nosec {other}"]

# The shipped tests' names (Bandit 1.9.4's registry), which a `# nosec` list may name instead of the id.
NAMES = {
    "B101": "assert_used",
    "B102": "exec_used",
    "B104": "hardcoded_bind_all_interfaces",
    "B108": "hardcoded_tmp_directory",
    "B110": "try_except_pass",
    "B112": "try_except_continue",
    "B113": "request_without_timeout",
    "B301": "pickle",
    "B302": "marshal",
    "B305": "cipher_modes",
    "B306": "mktemp_q",
    "B307": "eval",
    "B310": "urllib_urlopen",
    "B311": "random",
    "B312": "telnetlib",
    "B313": "xml_bad_cElementTree",
    "B314": "xml_bad_ElementTree",
    "B315": "xml_bad_expatreader",
    "B316": "xml_bad_expatbuilder",
    "B317": "xml_bad_sax",
    "B318": "xml_bad_minidom",
    "B319": "xml_bad_pulldom",
    "B321": "ftplib",
    "B323": "unverified_context",
    "B401": "import_telnetlib",
    "B402": "import_ftplib",
    "B405": "import_xml_etree",
    "B406": "import_xml_sax",
    "B407": "import_xml_expat",
    "B408": "import_xml_minidom",
    "B409": "import_xml_pulldom",
    "B411": "import_xmlrpclib",
    "B413": "import_pycrypto",
    "B415": "import_pyghmi",
    "B502": "ssl_with_bad_version",
    "B505": "weak_cryptographic_key",
    "B507": "ssh_no_host_key_verification",
    "B606": "start_process_with_no_shell",
    "B610": "django_extra_used",
    "B615": "huggingface_unsafe_download",
    "B701": "jinja2_autoescape_false",
}

# Ours: inputs a port once misread, each now a golden case (fixtures/rerail/probes/<name>.py).
PROBES = {
    # B502: Bandit reads `method` and `ssl_version` as keywords only.
    "b502-keyword-only.py": "import ssl\nfrom OpenSSL import SSL\nSSL.Context(SSL.SSLv23_METHOD)\n"
                            "ssl.SSLContext(ssl.PROTOCOL_TLSv1)\nSSL.Context(method=SSL.SSLv23_METHOD)\n"
                            "ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)\n",
    # B108: a docstring is skipped, and an implicit concatenation is one string.
    "b108-docstring-concatenation.py": "\"\"\"/tmp/foo is where things live.\"\"\"\n\n\ndef f():\n"
                                       "    \"\"\"/tmp/bar too.\"\"\"\n\n\nx = \"/t\" \"mp/x\"\ny = \"x\" \"/tmp\"\n"
                                       "z = \"/tmp/z\"\n",
    # `# nosec` on a multi-line node: the reported line and the first line of the node carrying one.
    "nosec-lines.py": "import requests\nrequests.get(\n    \"https://example.com\",  # nosec B113\n)\n"
                      "requests.get(\n    \"https://example.com\",\n)  # nosec\nrequests.get(\n"
                      "    \"https://example.com\",  # nosec B101\n)\ntry:\n    run()\nexcept Exception:\n"
                      "    pass  # nosec B110\n",
}


def authored() -> dict[str, str]:
    """Rerail's own cases, by path under fixtures/."""
    nosec = common.suppression_cases(NOSEC, NOSEC_COMMENTS, "bandit", NAMES)
    cases = {f"rerail/nosec/{path}": text for path, text in nosec.items()}
    cases.update({f"rerail/probes/{name}": text for name, text in PROBES.items()})
    return cases


def bandit(executable: str, paths: list[str], cwd: pathlib.Path) -> tuple[list[dict], set[str]]:
    """The ported tests' findings over `paths` (relative to `cwd`), and the files Bandit could not analyse."""
    findings = []
    errors: set[str] = set()
    for start in range(0, len(paths), 500):
        run = subprocess.run(
            [executable, "-q", "-f", "json", "-t", ",".join(sorted(TESTS)),
             *paths[start:start + 500]],
            capture_output=True, text=True, cwd=cwd,
        )
        report = json.loads(run.stdout)
        errors |= {pathlib.Path(error["filename"]).as_posix() for error in report["errors"]}
        for result in report["results"]:
            assert result["test_id"] in TESTS, result
            findings.append({"rule": result["test_id"], "path": pathlib.Path(result["filename"]).as_posix(),
                             "line": result["line_number"], "column": result["col_offset"] + 1})
    return [finding for finding in findings if finding["path"] not in errors], errors


def fixtures(checkout: pathlib.Path, executable: str) -> None:
    version = common.tool_version(executable, VERSION)
    head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != ORIGIN.rsplit("@", 1)[1]:
        raise SystemExit(f"{checkout} is at {head}, not Bandit {VERSION}")
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    target.mkdir()
    files = sorted({file for paths in TESTS.values() for file in paths})
    for file in files:
        shutil.copyfile(checkout / "examples" / file, target / file)
    ours = common.write_authored(target, authored())
    common.write_record("bandit", f"Bandit {VERSION}", ours, "`NOSEC`, `PROBES`")
    files = sorted(files + ours)
    findings, errors = bandit(executable, files, target)
    assert not errors, errors
    common.write(HERE / "fixtures.expected.json", "bandit", version,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(TESTS), "files": files}, findings)


def corpus(executable: str, name: str, root: pathlib.Path) -> None:
    version = common.tool_version(executable, VERSION)
    files = common.corpus_files(root, name)
    skipped = set(common.unparsable(root, files))
    findings, errors = bandit(executable, sorted(set(files) - skipped), root)
    common.write(HERE / f"corpus-{name}.expected.json", "bandit", version,
                 common.corpus_source(name, sorted(TESTS), sorted(skipped | errors)), findings)


def main() -> None:
    common.require_python()
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), sys.argv[3])
    else:
        for argument in sys.argv[3:]:
            name, _, path = argument.partition("=")
            corpus(sys.argv[2], name, pathlib.Path(path).resolve())


if __name__ == "__main__":
    main()
