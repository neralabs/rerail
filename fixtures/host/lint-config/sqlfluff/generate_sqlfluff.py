"""Writes sqlfluff.expected.json: the rules SQLFluff 4.4.0 runs, by folder, for each config of corpus.json. Each case is
a repository of probe.sql in every folder of the corpus's `probes` and the case's files, under an empty home folder,
SQLFluff run at its root. SQLFluff's own discovery (`paths_from_path`, which applies `.sqlfluffignore` and
`ignore_paths`) gives the SQL files it lints; for every probe folder whose probe.sql it lints, `FluffConfig.from_path`
reads the configs from the root down to the folder, and `get_rulepack` selects the rules, as `sqlfluff lint` does for
a file there. A rule counts as run when it is in the rule pack and, for the rules that report nothing without it
(AL07, CV13, PG01, PG02, TQ04), its `force_enable` is true. A probe folder SQLFluff ignores is recorded as "ignored",
one it stops on as "error: …", and each SQL file of the case's own as "linted" or "ignored". The lint-config test
(test/engine/catalog/lint-config/sqlfluff-oracle.test.ts) reads the same files into the reader and holds it to this.
Run offline, not in CI, with the Python that has SQLFluff 4.4.0 (scripts/pack-data/runtimes/sqlfluff-4.4.0 pins it:
`uv sync` there), from anywhere:
<that folder>/.venv/bin/python generate_sqlfluff.py"""

from __future__ import annotations

import configparser
import json
import os
import pathlib
import sys
import tempfile

HERE: pathlib.Path = pathlib.Path(__file__).parent
VERSION: str = "4.4.0"
# The rules whose `_eval` returns nothing unless `force_enable` is true (rules/<group>/<code>.py at the pin).
FORCED: frozenset[str] = frozenset({"AL07", "CV13", "PG01", "PG02", "TQ04"})


def run_folder(root: pathlib.Path, folder: str) -> str:
    from sqlfluff.core import FluffConfig
    from sqlfluff.core.errors import SQLFluffUserError
    from sqlfluff.core.rules import get_ruleset

    try:
        config = FluffConfig.from_path(str(root / folder), require_dialect=False)
        rules = get_ruleset().get_rulepack(config).rules
    except (SQLFluffUserError, ValueError, configparser.Error) as error:
        return f"error: {type(error).__name__}"
    return " ".join(sorted(rule.code for rule in rules if rule.code not in FORCED or bool(getattr(rule, "force_enable", False))))


def write_case(root: pathlib.Path, probes: list[str], files: dict[str, str]) -> None:
    for folder in probes:
        (root / folder).mkdir(parents=True, exist_ok=True)
        (root / folder / "probe.sql").write_text("select 1\n")
    for name, text in files.items():
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(text)


def linted_files(root: pathlib.Path) -> set[str] | str:
    """The SQL files `sqlfluff lint .` lints at `root`, after `.sqlfluffignore` and `ignore_paths`; the error it stops on."""
    from sqlfluff.core.errors import SQLFluffUserError
    from sqlfluff.core.linter.discovery import paths_from_path

    try:
        return {pathlib.Path(path).as_posix() for path in paths_from_path(".", working_path=str(root))}
    except (SQLFluffUserError, ValueError, configparser.Error) as error:
        return f"error: {type(error).__name__}"


def record(root: pathlib.Path, probes: list[str], files: dict[str, str], linted: set[str]) -> dict[str, str]:
    results: dict[str, str] = {name: "linted" if name in linted else "ignored" for name in files if name.endswith(".sql")}
    for folder in probes:
        probe: str = (pathlib.PurePosixPath(folder) / "probe.sql").as_posix()
        results[folder] = run_folder(root, folder) if probe in linted else "ignored"
    return results


def run_case(home: pathlib.Path, index: int, probes: list[str], files: dict[str, str]) -> dict[str, str]:
    root: pathlib.Path = home / f"case-{index}"
    write_case(root, probes, files)
    os.chdir(root)
    linted: set[str] | str = linted_files(root)
    # Where discovery stops, SQLFluff lints nothing; each folder records why, as `run_folder` would.
    return {folder: linted for folder in probes} if isinstance(linted, str) else record(root, probes, files, linted)


def main() -> None:
    corpus: dict[str, object] = json.loads((HERE / "corpus.json").read_text())
    with tempfile.TemporaryDirectory() as folder:
        home: pathlib.Path = pathlib.Path(folder).resolve()
        # Before SQLFluff is imported: the person's own config, in their home folder, must not count.
        os.environ["HOME"] = str(home)
        os.environ.pop("XDG_CONFIG_HOME", None)
        import sqlfluff

        if sqlfluff.__version__ != VERSION:
            sys.exit(f"sqlfluff {sqlfluff.__version__} is not {VERSION}")
        results: dict[str, object] = {"sqlfluff": VERSION}
        for index, case in enumerate(corpus["cases"]):  # type: ignore[arg-type]
            results[case["name"]] = run_case(home, index, corpus["probes"], case["files"])  # type: ignore[arg-type]
        os.chdir(HERE)
    (HERE / "sqlfluff.expected.json").write_text(json.dumps(results, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
