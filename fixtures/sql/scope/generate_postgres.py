"""Writes postgres.expected.json: PostgreSQL 18.6's own reading (libpg_query through pglast 8.5) of every name and
literal in postgres.sql, the T2 oracle for the SQL pack's `names` and `values` capabilities on the PostgreSQL dialect.
Run offline by a maintainer, never in CI, with the pinned venv (../common.py):

    <checkout>/.rerail/tools/sql/venv/bin/python generate_postgres.py

The file is parsed whole. Per statement (its libpg_query node kind and line), the recording lists:
- `relations`: every RangeVar, as the server's identifier parts ([catalog,] [schema,] relation): unquoted identifiers
  folded to lower case (ASCII letters only), quoted ones verbatim, Unicode escapes decoded, every part truncated to
  63 bytes (NAMEDATALEN - 1).
- `columns`: every ColumnRef, its parts the same way (`*` for a star).
- `constants`: every A_Const, its value as the server stores it: ["number", canonical decimal], ["string", text],
  ["bits", "b..." | "x..."], ["bool", true|false] or ["null"]. String continuation, E'' escapes, U&'' escapes and
  dollar quotes are already decoded. A constant the server builds from a keyword or identifier (`SET x = on`) has no
  literal in the source; there are none in this fixture.
Each item sits at its 1-based line and column (Unicode code points from the line's start) in postgres.sql.
"""

from __future__ import annotations

import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402
import reference  # noqa: E402


def position(data: bytes, offset: int) -> tuple[int, int]:
    line_start = data.rfind(b"\n", 0, offset) + 1
    return data.count(b"\n", 0, offset) + 1, len(data[line_start:offset].decode("utf-8")) + 1


def main() -> None:
    common.check_pins(need_grammars=False)
    source = HERE / "postgres.sql"
    data = source.read_bytes()
    tree = json.loads(reference.pg.parse_sql_json(data.decode("utf-8")))
    statements = []
    for raw in tree["stmts"]:
        kind = next(iter(raw["stmt"]))
        reading = reference.pg_names_values({"stmts": [raw]})
        entry: dict = {"kind": kind, "line": position(data, raw.get("stmt_location", 0))[0]}
        for category in ("relations", "columns", "constants"):
            items = []
            for item in reading[category]:
                assert item["location"] >= 0, (kind, category, item)
                line, column = position(data, item["location"])
                value = item["parts"] if category != "constants" else item["value"]
                items.append({"line": line, "column": column, "value": value})
            entry[category] = sorted(items, key=lambda item: (item["line"], item["column"]))
        statements.append(entry)
    result = {"tool": "libpg_query", "version": f"pglast {common.TOOLS['pglast']}, PostgreSQL 18.6",
              "source": source.name, "statements": statements}
    (HERE / "postgres.expected.json").write_text(json.dumps(result, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
