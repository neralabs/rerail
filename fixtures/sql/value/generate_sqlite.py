"""Writes sqlite.expected.json: SQLite 3.50.4's runtime value of every literal in sqlite.cases.json, the T2 oracle for
the SQL pack's `values` capability on the SQLite dialect (the runtime answers, so no parser reading is needed). Run
offline by a maintainer, never in CI, with the pinned venv (../common.py):

    <checkout>/.nera/tools/sql/venv/bin/python generate_sqlite.py

Each case is evaluated alone as `SELECT typeof(<literal>), quote(<literal>)` in a fresh in-memory database holding
one table `t(a)`, so a double-quoted token resolves to a column where one exists (`"a"`) and otherwise falls back to a
string literal, as SQLite does by default (SQLITE_DQS). A case SQLite rejects records the error instead. `quote()`
renders the value as SQLite's own literal: integers and reals in their canonical text, strings quoted, blobs as X'..'.
"""

from __future__ import annotations

import json
import pathlib
import sqlite3
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402


def evaluate(literal: str) -> dict:
    connection = sqlite3.connect(":memory:")
    try:
        connection.execute("CREATE TABLE t(a)")
        connection.execute("INSERT INTO t VALUES ('column a')")
        kind, quoted = connection.execute(f"SELECT typeof({literal}), quote({literal}) FROM t").fetchone()
        return {"type": kind, "quote": quoted}
    except sqlite3.Error as error:
        return {"error": str(error)}
    finally:
        connection.close()


def main() -> None:
    common.check_pins(need_grammars=False)
    cases = json.loads((HERE / "sqlite.cases.json").read_text())
    result = {"tool": "sqlite3", "version": sqlite3.sqlite_version,
              "cases": [{"literal": literal, **evaluate(literal)} for literal in cases]}
    (HERE / "sqlite.expected.json").write_text(json.dumps(result, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
