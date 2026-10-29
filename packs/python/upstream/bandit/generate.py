"""Records Bandit 1.9.4 for the Python pack's ported tests (rerail/packs/python/conformance.json).
Run by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI),
under CPython 3.14 (Bandit itself runs from its own install, on CPython 3.13):

    python3.14 generate.py fixtures <Bandit checkout at tag 1.9.4> <bandit executable, 1.9.4>

`fixtures` copies the examples each test fires in (`examples/<file>`; see TESTS) to fixtures/<file>, writes Rerail's own
cases (NOSEC, PROBES) to fixtures/rerail/ (listed in upstream/bandit/UPSTREAM.md), runs Bandit with every ported test
over every file, as bare `bandit` does (`# nosec` honoured, every severity and confidence), and writes provenance.json
and LICENSES.md (each copy's upstream path; `common.write_provenance`). A file Bandit reports an error for is skipped.
Each finding is the test id, path, line and column (`col_offset` + 1: Bandit's is 0-based). `source.units` names each
copied example's origin: the tests TESTS lists it for (Rerail's own cases have none)."""

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
# expected findings, run as bare `bandit` honouring `# nosec`, keeps without a finding.
# The list is fixtures.json ({"rules": {test: [upstream path, ...]}}); here each path is under examples/.
TESTS: dict[str, list[str]] = {
    test: [path.removeprefix("examples/") for path in paths]
    for test, paths in json.loads((HERE / "fixtures.json").read_text())["rules"].items()}


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
    "B303": "from Crypto.Hash import MD5\nMD5.new(){c}\n",
    "B305": "from cryptography.hazmat.primitives.ciphers import modes\nmodes.ECB(){c}\n",
    "B306": "import tempfile\ntempfile.mktemp(){c}\n",
    "B307": "eval('1 + 1'){c}\n",
    "B308": "from django.utils.safestring import mark_safe\nmark_safe(text){c}\n",
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
    "B303": "md5",
    "B305": "cipher_modes",
    "B306": "mktemp_q",
    "B307": "eval",
    "B308": "mark_safe",
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
    # From Moti's review of #29. Each port still misreads its case, so it stays a candidate until it agrees.
    # B104: a string statement is skipped, and an implicit concatenation is one string.
    "b104-docstring-concatenation.py": "\"0.0.0.0\"\nx = \"not-\" \"0.0.0.0\"\n",
    # B108: a parenthesized string statement is still a statement.
    "b108-parenthesized-statement.py": "(\"/tmp/x\")\n",
    # B701: Bandit exempts any call named `select_autoescape`, however it is imported.
    "b701-select-autoescape-import.py": "import jinja2\nfrom jinja2.utils import select_autoescape\n"
                                        "env = jinja2.Environment(autoescape=select_autoescape())\n",
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
    copied = sorted({file for paths in TESTS.values() for file in paths})
    for file in copied:
        shutil.copyfile(checkout / "examples" / file, target / file)
    ours = common.write_authored(target, authored())
    common.write_record("bandit", f"Bandit {VERSION}", ours, "`NOSEC`, `PROBES`")
    files = sorted(copied + ours)
    findings, errors = bandit(executable, files, target)
    assert not errors, errors
    common.write(HERE / "fixtures.expected.json", "bandit", version,
                 {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(TESTS), "files": files,
                  "units": common.origins(TESTS)}, findings)
    common.write_provenance("bandit", ORIGIN, "examples", copied)


def main() -> None:
    common.require_python()
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), sys.argv[3])
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
