"""Pack test data for the Python generators (packs/<id>/upstream/common.py and the generate.py beside them): the messages
scripts/s3/pack-fixtures.mjs gives, and the per-file provenance scripts/pack-data/provenance.mjs writes. The data (a
tool's test cases under packs/<id>/upstream/<tool>/ and its recorded answers on them) is not in git: each tool-pack's is
one archive in S3, which the tool directory's test-data.lock pins by sha256. New or changed data is staged into the
tool-pack's publish/ and published by the publish workflow (CONTRIBUTING.md, "Pack test data")."""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PULL_INSTRUCTION = ('pack test data is not in git: run `npm run fixtures:pull` (each tool-pack\'s archive, which its '
                    'test-data.lock pins, from S3) (CONTRIBUTING.md, "Pack test data")')
PROVENANCE_WRITER = pathlib.Path(__file__).resolve().parents[1] / "pack-data" / "provenance.mjs"


def _repository_path(path: os.PathLike | str) -> str:
    resolved = pathlib.Path(path).resolve()
    try:
        return resolved.relative_to(ROOT).as_posix()
    except ValueError:
        return resolved.as_posix()


def require(path: pathlib.Path) -> pathlib.Path:
    """`path` when it exists; otherwise stops the generator with the instruction to pull and derive."""
    if not path.exists():
        raise SystemExit(f"{_repository_path(path)} is missing: {PULL_INSTRUCTION}")
    return path


def written(*paths: pathlib.Path) -> None:
    """Tells the contributor how the pack test data a generator wrote (files or directories) is published (pack-fixtures.mjs's packFixturesWritten)."""
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


def expected_findings_written(path: pathlib.Path, source: dict) -> None:
    """`written` for expected findings: fixtures expected findings name the case sources beside them too."""
    cases = [path.parent / "fixtures", path.parent / "tests"] if source.get("kind") == "fixtures" else []
    written(path, *cases)


def write_provenance(pack: str, tool: str, entries: dict, commits: dict) -> None:
    """Writes packs/<pack>/upstream/<tool>/provenance.json and LICENSES.md under ROOT through the JavaScript writer
    (scripts/pack-data/provenance.mjs `writeProvenance`): `entries` maps each tool-relative path the generator copied (and
    any Rerail file it describes) to `{source, from?, how, renamedFrom?, detail?}`, and `commits` each project to the
    commit it checked out. Stops the generator with the writer's refusal, or when node is not on PATH."""
    node = shutil.which("node")
    if node is None:
        raise SystemExit(f"packs/{pack}/upstream/{tool}: writing its provenance needs node on PATH")
    run = subprocess.run([node, str(PROVENANCE_WRITER), "write", "--pack", pack, "--tool", tool, "--root", str(ROOT)],
                         input=json.dumps({"entries": entries, "commits": commits}), capture_output=True,
                         encoding="utf-8", check=False)
    if run.returncode != 0:
        raise SystemExit(run.stderr.strip() or f"{PROVENANCE_WRITER} exited with {run.returncode}")
