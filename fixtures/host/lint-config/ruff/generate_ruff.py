"""Writes ruff.expected.json: which of corpus.json's `codes` Ruff 0.16.9 runs, by file, for each config of the corpus.
Each case is a repository of probe.py in every folder of the corpus's `probes` plus the case's own files; Ruff runs on
it as a repository runs it, `ruff check .` from the root, `HOME` and `XDG_CONFIG_HOME` an empty folder, with
`--extend-select RUF100` added. The probe holds one `# noqa: <code>` comment per code on a line no rule reports, so
RUF100 names every code Ruff does not run in that file (`non-enabled`, per-file ignores included); a removed rule's
code on the last line proves RUF100 ran. A case Ruff refuses is recorded as "error". The lint-config test
(test/engine/catalog/lint-config/ruff-oracle.test.ts) reads the same files into the reader and holds it to this.
Run offline, not in CI, from anywhere:
python3 generate_ruff.py <path to a ruff executable, 0.16.9>
(`uvx ruff@0.16.9`'s binary; scripts/pack-data/runtimes/ruff-0.16.9 pins it)."""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys
import tempfile

HERE: pathlib.Path = pathlib.Path(__file__).parent
VERSION: str = "0.16.9"
#: A removed rule's code: RUF100 always reports it as not enabled, so its report proves RUF100 ran in the file.
SENTINEL: str = "ANN101"
NOT_ENABLED: re.Pattern[str] = re.compile(r"non-enabled: ([^)]*)")


def probe_text(codes: list[str]) -> str:
    return '"""Probe."""\n\n' + "".join(f"x = 1  # noqa: {code}\n" for code in [*codes, SENTINEL])


def write_case(root: pathlib.Path, probes: list[str], codes: list[str], files: dict[str, str]) -> None:
    for folder in probes:
        (root / folder).mkdir(parents=True, exist_ok=True)
        (root / folder / "probe.py").write_text(probe_text(codes))
    for name, text in files.items():
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(text)


def not_enabled(diagnostics: list[dict[str, object]]) -> dict[str, set[str]]:
    """Per file, the codes RUF100 reports as not enabled there."""
    found: dict[str, set[str]] = {}
    for diagnostic in diagnostics:
        match = NOT_ENABLED.search(str(diagnostic["message"]))
        if diagnostic["code"] == "RUF100" and match is not None:
            found.setdefault(str(diagnostic["filename"]), set()).update(re.findall(r"`([A-Z]+[0-9]+)`", match.group(1)))
    return found


def probe_results(root: pathlib.Path, probes: list[str], codes: list[str], off: dict[str, set[str]], name: str) -> dict[str, str]:
    """Per probe file (from the root), the codes Ruff runs there, space-separated."""
    results: dict[str, str] = {}
    for probe in probes:
        file: str = f"{probe}/probe.py" if probe else "probe.py"
        disabled: set[str] = off.get(str(root / file), set())
        if SENTINEL not in disabled:
            sys.exit(f"{name}: RUF100 did not run in {file}")
        results[file] = " ".join(code for code in codes if code not in disabled)
    return results


def run_case(ruff: str, probes: list[str], codes: list[str], case: dict[str, object]) -> object:
    with tempfile.TemporaryDirectory() as home, tempfile.TemporaryDirectory() as folder:
        root: pathlib.Path = pathlib.Path(folder).resolve()
        write_case(root, probes, codes, case["files"])  # type: ignore[arg-type]
        args: list[str] = [ruff, "check", ".", "--no-cache", "--extend-select", "RUF100", "--output-format", "json"]
        env: dict[str, str] = {"HOME": home, "XDG_CONFIG_HOME": home, "PATH": "/usr/bin:/bin"}
        done = subprocess.run(args, capture_output=True, text=True, check=False, cwd=root, env=env)
        if done.returncode not in (0, 1):
            return "error"
        return probe_results(root, probes, codes, not_enabled(json.loads(done.stdout)), str(case["name"]))


def main() -> None:
    ruff: str = sys.argv[1]
    found: str = subprocess.run([ruff, "version"], capture_output=True, text=True, check=True).stdout.split()[1]
    if found != VERSION:
        sys.exit(f"ruff {found} is not {VERSION}")
    corpus: dict[str, object] = json.loads((HERE / "corpus.json").read_text())
    probes: list[str] = corpus["probes"]  # type: ignore[assignment]
    codes: list[str] = corpus["codes"]  # type: ignore[assignment]
    results: dict[str, object] = {"ruff": VERSION}
    for case in corpus["cases"]:  # type: ignore[attr-defined]
        results[case["name"]] = run_case(ruff, probes, codes, case)
    (HERE / "ruff.expected.json").write_text(json.dumps(results, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
