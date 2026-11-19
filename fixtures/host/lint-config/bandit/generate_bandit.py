"""Writes bandit.expected.json: what Bandit 1.9.4 reports, test id by file, for each config of corpus.json. Each case
is a repository of probe.py in every folder of the corpus's `probes` (one snippet per test the python pack ports, and
a few more) plus the case's own files; Bandit runs on it as a repository runs it, `bandit -r .` from the root with the
case's `args`, `HOME` an empty folder. A case Bandit stops on is recorded as "error". The lint-config test
(test/engine/catalog/lint-config/bandit-oracle.test.ts) reads the same files into the reader and holds it to this.
Run offline, not in CI, from anywhere:
python3 generate_bandit.py <path to a bandit executable, 1.9.4>
(scripts/pack-data/runtimes/bandit-1.9.4 pins it: `uv sync` there, then .venv/bin/bandit)."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile

HERE: pathlib.Path = pathlib.Path(__file__).parent
VERSION: str = "1.9.4"


def write_case(root: pathlib.Path, probes: list[str], files: dict[str, str]) -> None:
    probe: str = (HERE / "probe.py").read_text()
    for folder in probes:
        (root / folder).mkdir(parents=True, exist_ok=True)
        (root / folder / "probe.py").write_text(probe)
    for name, text in files.items():
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(text)


def run_case(bandit: str, probes: list[str], case: dict[str, object]) -> object:
    with tempfile.TemporaryDirectory() as home, tempfile.TemporaryDirectory() as folder:
        root: pathlib.Path = pathlib.Path(folder)
        write_case(root, probes, case["files"])  # type: ignore[arg-type]
        args: list[str] = [bandit, "-r", ".", "-f", "json", "-q", *case.get("args", [])]  # type: ignore[list-item]
        done = subprocess.run(args, capture_output=True, text=True, check=False, cwd=root, env={"HOME": home, "PATH": "/usr/bin:/bin"})
        if done.returncode not in (0, 1):
            return "error"
        reported: dict[str, set[str]] = {}
        for result in json.loads(done.stdout)["results"]:
            reported.setdefault(str(result["filename"]).removeprefix("./"), set()).add(str(result["test_id"]))
        return {name: " ".join(sorted(ids)) for name, ids in sorted(reported.items())}


def main() -> None:
    bandit: str = sys.argv[1]
    found: str = subprocess.run([bandit, "--version"], capture_output=True, text=True, check=True).stdout.split()[1]
    if found != VERSION:
        sys.exit(f"bandit {found} is not {VERSION}")
    corpus: dict[str, object] = json.loads((HERE / "corpus.json").read_text())
    probes: list[str] = corpus["probes"]  # type: ignore[assignment]
    results: dict[str, object] = {"bandit": VERSION}
    for case in corpus["cases"]:  # type: ignore[attr-defined]
        results[case["name"]] = run_case(bandit, probes, case)
    (HERE / "bandit.expected.json").write_text(json.dumps(results, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
