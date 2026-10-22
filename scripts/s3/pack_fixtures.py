"""Pack test data for the Python recorders (packs/<id>/upstream/common.py and the generate.py beside them): the messages
scripts/s3/pack-fixtures.mjs gives. The data (a tool's test cases under packs/<id>/upstream/<tool>/ and its recorded
answers on them) is not in git. What Rerail wrote for the tool to score (its tool's UPSTREAM.md lists it as Rerail's,
or it is under upstream/<tool>/authored/) is authored and lives in S3; everything else is derived, regenerated from the
pins by `npm run fixtures:derive` and CI, and never committed (CONTRIBUTING.md, "Pack test data")."""

from __future__ import annotations

import os
import pathlib
import re
import shlex
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PULL_INSTRUCTION = ('pack test data is not in git: run `npm run fixtures:pull` (what Rerail wrote, from S3) and '
                    '`npm run fixtures:derive` (the rest, from the pinned upstreams) (CONTRIBUTING.md, '
                    '"Pack test data")')
# A row of an UPSTREAM.md file table: its first cell names a pack-relative path (pack-fixtures.mjs's UPSTREAM_ROW).
_UPSTREAM_ROW = re.compile(r"^\|\s*`([^`]+)`")


def _repository_path(path: os.PathLike | str) -> str:
    resolved = pathlib.Path(path).resolve()
    try:
        return resolved.relative_to(ROOT).as_posix()
    except ValueError:
        return resolved.as_posix()


def require(path: pathlib.Path) -> pathlib.Path:
    """`path` when it exists; otherwise stops the recorder with the instruction to pull and derive."""
    if not path.exists():
        raise SystemExit(f"{_repository_path(path)} is missing: {PULL_INSTRUCTION}")
    return path


def _is_authored(path: str) -> bool:
    """Whether a repository path is authored pack test data (pack-fixtures.mjs's packFixtureKind)."""
    parts = path.split("/")
    if len(parts) < 5 or parts[0] != "packs" or parts[2] != "upstream":
        return False
    if parts[4] == "authored":
        return True
    record = ROOT.joinpath(*parts[:4], "UPSTREAM.md")
    if not record.exists():
        return False
    prefix = f"upstream/{parts[3]}/"
    listed = {match.group(1) for line in record.read_text(encoding="utf-8").split("\n")
              if (match := _UPSTREAM_ROW.match(line)) and match.group(1).startswith(prefix)}
    return "/".join(parts[2:]) in listed


def written(*paths: pathlib.Path) -> None:
    """Tells the contributor which of the pack test data a recorder wrote (files or directories) goes in a pull request."""
    files = sorted({_repository_path(file) for path in paths if path.exists()
                    for file in ([path] if path.is_file() else [entry for entry in path.rglob("*") if entry.is_file()])})
    if not files:
        return
    own = [path for path in files if _is_authored(path)]
    if len(files) > len(own):
        print(f"Wrote {len(files) - len(own)} derived pack test data file(s), which git ignores: `npm run fixtures:derive` "
              "and CI regenerate them from the pins, so never commit them.", file=sys.stderr)
    if own:
        print(f"Wrote {len(own)} authored pack test data file(s) (their UPSTREAM.md lists them as Rerail's), which git "
              "ignores (they live in S3). Put new or changed ones in your pull request with\n"
              f"  git add -f -- {' '.join(shlex.quote(path) for path in own)}\n"
              'and a maintainer moves them to S3 before merge (CONTRIBUTING.md, "Pack test data").', file=sys.stderr)


def recording_written(path: pathlib.Path, source: dict) -> None:
    """`written` for a recording: a fixtures recording names the case sources beside it too."""
    cases = [path.parent / "fixtures", path.parent / "tests"] if source.get("kind") == "fixtures" else []
    written(path, *cases)
