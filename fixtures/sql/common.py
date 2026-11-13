"""Pins shared by the SQL pack's offline generators (fixtures/sql/). Run offline by a maintainer, never in CI.

Tools live under the gitignored `<checkout>/.rerail/tools/sql/` and corpora under `<checkout>/.rerail/corpora/sql/`, where
`<checkout>` is the nearest ancestor holding `.rerail/corpora/sql` (sub-worktrees under `.rerail/wt/` find it this way),
or under `$RERAIL_SQL_CORPORA` when it is set (`npm run fixtures:derive` provisions them there, by checkout name).
Every generator calls `check_pins()` first and refuses to run against any other tool version.

Setup (from `<checkout>/.rerail/tools/sql`):

    VIRTUAL_ENV=$PWD/venv uv pip install sqlfluff==4.4.0 pglast==8.5 tree-sitter==0.26.0 pyyaml==6.0.3 pymysql==1.2.3
    # grammars, built with tree-sitter CLI 0.27.0 (`tree-sitter build -o <name>.dylib`) from the commits in GRAMMARS
    # MySQL reference: docker image mysql:8.4 (8.4.11, digest in TOOLS)

Dialects are chosen by path, never detected: each corpus maps globs to one dialect (`CORPORA[...]["units"]`), the way
the pack would read a per-path dialect setting.
"""

from __future__ import annotations

import ctypes
import glob
import hashlib
import os
import pathlib
import sys
import warnings
from importlib.metadata import version as dist_version

HERE = pathlib.Path(__file__).resolve().parent


def _checkout() -> pathlib.Path:
    if os.environ.get("RERAIL_SQL_CORPORA"):
        return HERE.parents[1]
    for parent in HERE.parents:
        if (parent / ".rerail/corpora/sql").is_dir():
            return parent
    raise SystemExit("no ancestor holds .rerail/corpora/sql; clone the corpora first (see CORPORA)")


CHECKOUT = _checkout()
CORPUS_ROOT = pathlib.Path(os.environ.get("RERAIL_SQL_CORPORA") or CHECKOUT / ".rerail/corpora/sql").resolve()
TOOLS_ROOT = CHECKOUT / ".rerail/tools/sql"
GRAMMAR_ROOT = TOOLS_ROOT / "grammars"

# The build too: python-downloads.json pins uv's CPython 3.12.13 to python-build-standalone 20260408 (each download's
# sha256), the build linking TOOLS["sqlite"]; a newer uv's 3.12.13 is a later build linking another SQLite.
PYTHON = "3.12.13"
TOOLS = {
    "sqlfluff": "4.4.0",
    "pglast": "8.5",  # libpg_query for PostgreSQL 18 (parser version 180006)
    "postgresql-parser": "180006",
    "sqlite": "3.50.4",  # the SQLite CPython 3.12.13 build 20260408 links (python-downloads.json)
    "mysql": "8.4.11",  # docker mysql:8.4@sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242
    "tree-sitter": "0.26.0",  # py-tree-sitter; grammars built with tree-sitter CLI 0.27.0 (Rerail's runtime crate)
    "pyyaml": "6.0.3",
    "pymysql": "1.2.3",
}
MYSQL_IMAGE = "mysql:8.4@sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242"

GRAMMARS = {
    "derekstride": {
        "repository": "https://github.com/DerekStride/tree-sitter-sql",
        "commit": "7b51ecda191d36b92f5a90a8d1bc3faef1c7b8b8",
        "version": "0.3.11",
        "crate": "tree-sitter-sequel",
        "symbol": "tree_sitter_sql",
        "license": "MIT",
        "dialects": ["postgres", "mysql", "sqlite"],
        "statement": "statement",
    },
    "gmr-postgres": {
        "repository": "https://github.com/gmr/tree-sitter-postgres",
        "commit": "59d0d8cd7506d68de1229fb4bbce838c83b60c8a",
        "version": "19.0.0-beta.4.1",
        "crate": "tree-sitter-postgres",
        "symbol": "tree_sitter_postgres",
        "license": "BSD-3-Clause",
        "dialects": ["postgres"],
        "statement": "toplevel_stmt",
        # postgres/src/parser.c is a Git LFS object; a clone without git-lfs holds a pointer file. Fetch it from
        # https://media.githubusercontent.com/media/gmr/tree-sitter-postgres/<commit>/postgres/src/parser.c
        "files": {"postgres/src/parser.c": "a9090d5082ae5c23892d05aa59e61476f9bd39ad634228f2046024debdf815b5"},
    },
    "dhcmrlchtdj-sqlite": {
        "repository": "https://github.com/dhcmrlchtdj/tree-sitter-sqlite",
        "commit": "993be0a91c0c90b0cc7799e6ff65922390e2cefe",
        "version": "0.0.2",
        "crate": None,
        "symbol": "tree_sitter_sqlite",
        "license": "MIT",
        "dialects": ["sqlite"],
        "statement": "sql_stmt",
    },
}

# Each unit glob is relative to the corpus checkout; the first matching glob decides a file's dialect.
CORPORA = {
    "postgres": {
        "repository": "https://github.com/postgres/postgres",
        "commit": "724edf9bde9d356724ad384a2e196edc3c9f80f7",
        "tag": "REL_18_6",
        "license": "PostgreSQL",
        "units": [
            ("src/test/regress/sql/*.sql", "postgres"),
            ("src/pl/plpgsql/src/sql/*.sql", "postgres"),
            ("contrib/**/*.sql", "postgres"),
            ("src/backend/catalog/*.sql", "postgres"),
        ],
    },
    "gitlab": {
        "repository": "https://github.com/gitlabhq/gitlabhq",
        "commit": "635bee939f2834806197545264ebd6542ddf5364",
        "tag": "v18.4.2",
        "license": "MIT (CE)",
        "units": [("db/*.sql", "postgres")],
    },
    "mattermost": {
        "repository": "https://github.com/mattermost/mattermost",
        "commit": "613bb616cd62c584a606919e6978688e7b87d81e",
        "tag": "v10.11.9",
        "license": "AGPL-3.0 / Apache-2.0 (see LICENSE.txt)",
        "units": [
            ("server/channels/db/migrations/postgres/*.sql", "postgres"),
            ("server/channels/db/migrations/mysql/*.sql", "mysql"),
        ],
    },
    "sqlfluff-dialects": {
        "repository": "https://github.com/sqlfluff/sqlfluff",
        "commit": "0348222a3bcdb0226563f7349c5a39beb0641955",
        "tag": "4.4.0",
        "license": "MIT",
        "checkout": "sqlfluff",
        "units": [
            ("test/fixtures/dialects/postgres/*.sql", "postgres"),
            ("test/fixtures/dialects/mysql/*.sql", "mysql"),
            ("test/fixtures/dialects/sqlite/*.sql", "sqlite"),
        ],
    },
    "anki": {
        "repository": "https://github.com/ankitects/anki",
        "commit": "d52ca669f6deac5966b1c5035bc2dc77c78d3260",
        "tag": "25.09.4",
        "license": "AGPL-3.0-or-later",
        "units": [("rslib/src/**/*.sql", "sqlite")],
    },
    "application-services": {
        "repository": "https://github.com/mozilla/application-services",
        "commit": "c19ea2c24eb2f116dae170f5f82054d325e94a42",
        "tag": "v149.0",
        "license": "MPL-2.0",
        "units": [("components/**/*.sql", "sqlite")],
    },
}


def corpus_dir(name: str) -> pathlib.Path:
    return CORPUS_ROOT / CORPORA[name].get("checkout", name)


def corpus_units(name: str) -> list[tuple[str, str]]:
    """(corpus-relative path, dialect) for every scored file, sorted, each file once (first glob wins)."""
    root = corpus_dir(name)
    seen: dict[str, str] = {}
    for pattern, dialect in CORPORA[name]["units"]:
        for path in sorted(glob.glob(str(root / pattern), recursive=True)):
            relative = pathlib.Path(path).relative_to(root).as_posix()
            seen.setdefault(relative, dialect)
    return sorted(seen.items())


def check_pins(*, need_grammars: bool = True, corpora: tuple[str, ...] | None = None) -> None:
    """Refuses any tool, interpreter or checkout but the pinned ones: every corpus, or only `corpora` when given."""
    import sqlite3

    import pglast

    problems = []
    if sys.version.split()[0] != PYTHON:
        problems.append(f"python {sys.version.split()[0]} != {PYTHON}")
    for dist in ("sqlfluff", "pglast", "tree-sitter", "pyyaml", "pymysql"):
        if dist_version(dist) != TOOLS[dist]:
            problems.append(f"{dist} {dist_version(dist)} != {TOOLS[dist]}")
    if str(pglast.parser.get_postgresql_version()) != "(18, 6)":
        problems.append(f"libpg_query PostgreSQL {pglast.parser.get_postgresql_version()} != (18, 6)")
    if sqlite3.sqlite_version != TOOLS["sqlite"]:
        problems.append(f"sqlite {sqlite3.sqlite_version} != {TOOLS['sqlite']}")
    import subprocess

    for name, corpus in CORPORA.items():
        if corpora is not None and name not in corpora:
            continue
        head = subprocess.run(["git", "-C", str(corpus_dir(name)), "rev-parse", "HEAD"], capture_output=True,
                              text=True).stdout.strip()
        if head != corpus["commit"]:
            problems.append(f"corpus {name} at {head or 'missing'} != {corpus['commit']}")
    if need_grammars:
        for name, grammar in GRAMMARS.items():
            checkout = GRAMMAR_ROOT / name
            head = subprocess.run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True,
                                  text=True).stdout.strip()
            if head != grammar["commit"]:
                problems.append(f"grammar {name} at {head or 'missing'} != {grammar['commit']}")
            for relative, digest in grammar.get("files", {}).items():
                path = checkout / relative
                actual = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else "missing"
                if actual != digest:
                    problems.append(f"grammar {name} {relative} sha256 {actual} != {digest}")
    if problems:
        raise SystemExit("pins do not match:\n  " + "\n  ".join(problems))


def load_parser(name: str):
    import tree_sitter

    grammar = GRAMMARS[name]
    library = ctypes.cdll.LoadLibrary(str(GRAMMAR_ROOT / f"{name}.dylib"))
    function = getattr(library, grammar["symbol"])
    function.restype = ctypes.c_void_p
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        return tree_sitter.Parser(tree_sitter.Language(function()))
