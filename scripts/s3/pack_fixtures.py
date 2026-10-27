"""Pack test data for the Python recorders (packs/<id>/upstream/common.py and the generate.py beside them): the messages
scripts/s3/pack-fixtures.mjs gives. The data (a tool's test cases under packs/<id>/upstream/<tool>/ and its recorded
answers on them) is not in git: each tool-pack's is one archive in S3, which the tool directory's test-data.lock pins by
sha256. New or changed data is staged into the tool-pack's publish/ and published by the publish workflow
(CONTRIBUTING.md, "Pack test data")."""

from __future__ import annotations

import os
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PULL_INSTRUCTION = ('pack test data is not in git: run `npm run fixtures:pull` (each tool-pack\'s archive, which its '
                    'test-data.lock pins, from S3) (CONTRIBUTING.md, "Pack test data")')


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


def written(*paths: pathlib.Path) -> None:
    """Tells the contributor how the pack test data a recorder wrote (files or directories) is published (pack-fixtures.mjs's packFixturesWritten)."""
    files = sorted({_repository_path(file) for path in paths if path.exists()
                    for file in ([path] if path.is_file() else [entry for entry in path.rglob("*") if entry.is_file()])})
    counts: dict[tuple[str, str], int] = {}
    for file in files:
        parts = file.split("/")
        if len(parts) < 5 or parts[0] != "packs" or parts[2] != "upstream":
            continue
        counts[(parts[1], parts[3])] = counts.get((parts[1], parts[3]), 0) + 1
    for (pack, tool), count in sorted(counts.items()):
        print(f"Wrote {count} pack test data file(s) under packs/{pack}/upstream/{tool}/, which git ignores (the tool-pack's "
              f"data is an archive in S3). To publish new or changed data, stage it with `npm run fixtures:stage -- --pack "
              f"{pack} --tool {tool}`, commit packs/{pack}/upstream/{tool}/publish/ in the pull request, and run the publish "
              "workflow (https://github.com/neralabs/rerail/actions/workflows/pack-data-publish.yml) for it "
              '(CONTRIBUTING.md, "Pack test data").', file=sys.stderr)


def recording_written(path: pathlib.Path, source: dict) -> None:
    """`written` for a recording: a fixtures recording names the case sources beside it too."""
    cases = [path.parent / "fixtures", path.parent / "tests"] if source.get("kind") == "fixtures" else []
    written(path, *cases)
