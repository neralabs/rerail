"""Writes ruff.expected.json: where Ruff reports each Pyflakes rule on its fixtures ruff/*.py, as byte ranges (start,
end). The `%` rules (F501-F509) run on every `%` fixture (F502, F503, F504, F50x) and the `.format` rules (F521-F525) on
every `.format` fixture (F521-F525); the string annotation rules (F401, F632, F722, F821, F841, F842) each on its own
fixtures. The fixtures are Ruff 0.16.9's own (crates/ruff_linter/resources/test/fixtures/pyflakes, MIT), but for
`*_strings.py`, written for string type definitions, `F50x_operands.py`, written for the operands Ruff types,
`F5[02]x_parenthesized.py`, written for format strings in parentheses, `F821_typing_settings.py(i)`, written for the
typing settings, `F722_semantic.py`, written for semantic syntax errors in string type definitions,
`F722_nested.py`, written for such errors in strings nested in one, and `F811_typing_modules.py` /
`F722_typing_modules.py`, written for a typing module's members as typing's own. Fixtures Ruff's
own tests read under settings (`CONFIGURED`) are also read under each, keyed `<fixture>:<test name>`; `project/` keeps
Ruff's package layout, which names the modules its relative imports resolve against. Run offline, not in CI:
python3 generate_ruff.py <path to a ruff executable, 0.16.9>"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

HERE: pathlib.Path = pathlib.Path(__file__).parent
PERCENT: list[str] = ["F501", "F502", "F503", "F504", "F505", "F506", "F507", "F508", "F509"]
BRACE: list[str] = ["F521", "F522", "F523", "F524", "F525"]
# Fixtures whose codes their name does not give.
CODES: dict[str, list[str]] = {
    "typing_modules.py": ["F821"],
    "project/foo/bar.py": ["F821"],
    "project/foo/bop/baz.py": ["F821"],
    "F722_semantic.py": ["F722", "F821"],
}
TYPICAL: list[str] = ["--config", 'lint.typing-modules = ["foo.typical"]']
STD: list[str] = ["--config", 'lint.typing-modules = ["std"]']
SETTINGS: list[str] = ["--config", 'lint.typing-modules = ["compat"]', "--config", 'lint.pyflakes.extend-generics = ["lib.Box"]']
# The settings of Ruff's tests (rules/pyflakes/mod.rs), by fixture and test name.
CONFIGURED: dict[str, dict[str, list[str]]] = {
    "typing_modules.py": {"extra_typing_modules": ["--config", 'lint.typing-modules = ["airflow.typing_compat"]']},
    "project/foo/bar.py": {"relative_typing_module": TYPICAL},
    "project/foo/bop/baz.py": {"nested_relative_typing_module": TYPICAL},
    "F401_15.py": {
        "extend_immutable_calls": ["--config", 'lint.pyflakes.extend-generics = ["django.db.models.ForeignKey"]']
    },
    "F821_typing_settings.py": {"settings": SETTINGS},
    "F821_typing_settings.pyi": {"settings": SETTINGS},
    "F811_typing_modules.py": {"typing_modules": STD},
    "F722_typing_modules.py": {"typing_modules": STD},
}


def byte_offset(lines: list[str], row: int, column: int) -> int:
    return len(("".join(lines[: row - 1]) + lines[row - 1][: column - 1]).encode())


def check(ruff: str, path: pathlib.Path, codes: list[str], settings: list[str]) -> dict[str, list[list[int]]]:
    """Where Ruff reports each code on the fixture under the settings, as sorted byte ranges."""
    lines: list[str] = path.read_text().splitlines(keepends=True)
    output: str = subprocess.run(
        [ruff, "check", "--isolated", "--no-cache", *settings, "--select", ",".join(codes), "--output-format", "json", str(path)],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    found: dict[str, list[list[int]]] = {code: [] for code in codes}
    for report in json.loads(output):
        start: int = byte_offset(lines, int(report["location"]["row"]), int(report["location"]["column"]))
        end: int = byte_offset(lines, int(report["end_location"]["row"]), int(report["end_location"]["column"]))
        if report["code"] in found:
            found[str(report["code"])].append([start, end])
    return {code: sorted(ranges) for code, ranges in found.items()}


def main() -> None:
    ruff: str = sys.argv[1]
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
    results: dict[str, object] = {"ruff": version}
    root: pathlib.Path = HERE / "ruff"
    paths: list[pathlib.Path] = sorted([*root.glob("*.py"), *root.glob("*.pyi")]) + sorted(root.glob("project/**/*.py"))
    for path in paths:
        name: str = path.relative_to(root).as_posix()
        if path.stat().st_size == 0:
            continue
        stem: str = path.stem.split("_")[0]
        codes: list[str] = CODES.get(
            name, BRACE if stem.startswith("F52") else PERCENT if stem.startswith("F50") else [stem]
        )
        results[name] = check(ruff, path, codes, [])
        for test, settings in CONFIGURED.get(name, {}).items():
            results[f"{name}:{test}"] = check(ruff, path, codes, settings)
    (HERE / "ruff.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
