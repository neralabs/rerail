"""Pack test data for the Python recorders (packs/<id>/upstream/common.py and the generate.py beside them): the messages
scripts/s3/pack-fixtures.mjs gives. The data (a tool's test cases under packs/<id>/upstream/<tool>/ and its recorded
answers on them) lives in S3, not git; .gitignore covers it, so what a recorder writes is invisible to `git status`
until it is added with `git add -f` (CONTRIBUTING.md, "Pack test data")."""

from __future__ import annotations

import os
import pathlib
import shlex
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PULL_INSTRUCTION = ('pack test data lives in S3, not git: run `npm run fixtures:pull` (CONTRIBUTING.md, '
                    '"Pack test data")')


def _repository_path(path: os.PathLike | str) -> str:
    resolved = pathlib.Path(path).resolve()
    try:
        return resolved.relative_to(ROOT).as_posix()
    except ValueError:
        return resolved.as_posix()


def require(path: pathlib.Path) -> pathlib.Path:
    """`path` when it exists; otherwise stops the recorder with the instruction to pull."""
    if not path.exists():
        raise SystemExit(f"{_repository_path(path)} is missing: {PULL_INSTRUCTION}")
    return path


def written(*paths: pathlib.Path) -> None:
    """Tells the contributor how to put the pack test data a recorder wrote (files or directories) in a pull request."""
    listed = sorted({_repository_path(path) for path in paths if path.exists()})
    if not listed:
        return
    print("Wrote pack test data, which git ignores (it lives in S3). Put new or changed files in your pull request with\n"
          f"  git add -f -- {' '.join(shlex.quote(path) for path in listed)}\n"
          'and a maintainer moves them to S3 before merge (CONTRIBUTING.md, "Pack test data").', file=sys.stderr)


def recording_written(path: pathlib.Path, source: dict) -> None:
    """`written` for a recording: a fixtures recording names the case sources beside it too."""
    cases = [path.parent / "fixtures", path.parent / "tests"] if source.get("kind") == "fixtures" else []
    written(path, *cases)
