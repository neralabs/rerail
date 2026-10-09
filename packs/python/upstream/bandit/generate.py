"""Records Bandit 1.9.4 for the Python pack's ported tests (rerail/packs/python/conformance.json).
Run offline, never in CI, under CPython 3.14 (Bandit itself runs from its own install, on CPython 3.13):

    python3.14 generate.py fixtures <Bandit checkout at tag 1.9.4> <bandit executable, 1.9.4>
    python3.14 generate.py corpus <bandit executable, 1.9.4> <name>=<corpus checkout> [...]

`fixtures` copies the examples each test is reported in (`examples/<file>`; see TESTS) to fixtures/<file> (the pack's
licenses/bandit/UPSTREAM.md lists them) and runs Bandit with every ported test over every copy. `corpus` runs the same
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

# Each ported test and the examples Bandit 1.9.4 reports it in, run over every file of examples/ as bare `bandit` runs.
TESTS: dict[str, list[str]] = {
    "B101": ["assert.py"],
    "B102": ["exec.py"],
    "B104": ["binding.py"],
    "B108": ["hardcoded-tmp.py", "os-chmod.py"],
    "B110": ["try_except_pass.py"],
    "B112": ["try_except_continue.py"],
    "B113": ["requests-missing-timeout.py"],
    "B301": [
        "dill.py",
        "imports-aliases.py",
        "jsonpickle.py",
        "pandas_read_pickle.py",
        "pickle_deserialize.py",
        "shelve_open.py",
    ],
    "B302": ["marshal_deserialize.py"],
    "B305": ["cipher-modes.py"],
    "B306": ["mktemp.py"],
    "B307": ["eval.py"],
    "B310": ["urlopen.py"],
    "B311": ["random_module.py"],
    "B312": ["telnetlib.py"],
    "B313": ["xml_etree_celementtree.py"],
    "B314": ["xml_etree_elementtree.py"],
    "B315": ["xml_expatreader.py"],
    "B316": ["xml_expatbuilder.py"],
    "B317": ["new_candidates-all.py", "xml_sax.py"],
    "B318": ["xml_minidom.py"],
    "B319": ["xml_pulldom.py"],
    "B321": ["ftplib.py"],
    "B323": ["unverified_context.py"],
    "B401": ["telnetlib.py"],
    "B402": ["ftplib.py"],
    "B405": ["xml_etree_celementtree.py", "xml_etree_elementtree.py"],
    "B406": ["xml_expatreader.py", "xml_sax.py"],
    "B407": ["xml_expatbuilder.py"],
    "B408": ["xml_minidom.py"],
    "B409": ["xml_pulldom.py"],
    "B411": ["xml_xmlrpc.py"],
    "B413": ["ciphers.py", "crypto-md5.py", "pycrypto.py", "weak_cryptographic_key_sizes.py"],
    "B415": ["pyghmi.py"],
    "B502": ["ssl-insecure-version.py"],
    "B505": ["weak_cryptographic_key_sizes.py"],
    "B507": ["no_host_key_verification.py"],
    "B606": ["os-exec.py", "os-spawn.py", "os-startfile.py", "wildcard-injection.py"],
    "B610": ["django_sql_injection_extra.py"],
    "B615": ["huggingface_unsafe_download.py"],
    "B701": ["jinja2_templating.py"],
}


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
