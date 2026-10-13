"""Write postgres.expected.json, an oracle of PostgreSQL 18.6's reading of SQL call arguments.

The pinned offline environment is pglast 8.5 (libpg_query PostgreSQL 18.6), Python 3.12.13,
and SQLFluff 4.4.0. Run this file with the checkout's ``.rerail/tools/sql/venv/bin/python``;
corpora are not required and ``common.check_pins`` is intentionally not called.

The source is parsed whole. Every FuncCall, CoalesceExpr, MinMaxExpr and NullIfExpr is
recorded in source order. Argument positions are the expression's start; NamedArgExpr
positions are the name and additionally carry the value position. A node without a
location uses the first descendant location (a rare parser representation detail).
PostgreSQL's JSON tree does not retain quoting metadata for String nodes, so function
name spelling is recovered from the source token at the node location: unquoted names
are lower-cased, while quoted names are preserved (including doubled quotes decoded).
"""

from __future__ import annotations

import json
import pathlib
import platform
import re
import sys
from importlib.metadata import version as dist_version

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402
import reference  # noqa: E402
from pglast import parser as pg  # noqa: E402


def position(data: bytes, offset: int) -> tuple[int, int]:
    line_start = data.rfind(b"\n", 0, offset) + 1
    return data.count(b"\n", 0, offset) + 1, len(data[line_start:offset].decode("utf-8")) + 1


def first_location(node: object) -> int:
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "location" and isinstance(value, int) and value >= 0:
                return value
            found = first_location(value)
            if found >= 0:
                return found
    elif isinstance(node, list):
        for item in node:
            found = first_location(item)
            if found >= 0:
                return found
    return -1


def function_names(data: bytes, location: int, names: list[dict]) -> list[str]:
    """Recover quotedness from the source, which libpg_query omits from String nodes."""
    end = data.find(b"(", location)
    text = data[location:end].decode("utf-8")
    tokens = re.findall(r'"((?:""|[^"])*)"|([A-Za-z_\x80-\uffff][A-Za-z_0-9\x80-\uffff]*)', text)
    result = []
    for index, (quoted, bare) in enumerate(tokens):
        if index >= len(names):
            break
        result.append(quoted.replace('""', '"') if quoted else bare.lower())
    return result


def arg_record(data: bytes, arg: dict) -> dict:
    kind, body = next(iter(arg.items()))
    location = body.get("location", -1)
    if location < 0:
        location = first_location(body)
    line, column = position(data, location)
    result = {"line": line, "column": column}
    if kind == "NamedArgExpr":
        result["name"] = body["name"]
        value = body["arg"]
        value_location = first_location(value)
        value_line, value_column = position(data, value_location)
        result["valueLine"] = value_line
        result["valueColumn"] = value_column
    return result


def call_record(data: bytes, kind: str, body: dict) -> dict:
    location = body.get("location", -1)
    if location < 0:
        location = first_location(body)
    line, column = position(data, location)
    if kind == "FuncCall":
        raw_names = body.get("funcname", [])
        names = [item["String"] for item in raw_names]
        name = function_names(data, location, names)
        args = [arg_record(data, arg) for arg in body.get("args", [])]
        result = {
            "kind": kind,
            "line": line,
            "column": column,
            "name": name,
            "star": bool(body.get("agg_star", False)),
            "distinct": bool(body.get("agg_distinct", False)),
            "variadic": bool(body.get("func_variadic", False)),
            "order": bool(body.get("agg_order")),
            "filter": bool(body.get("agg_filter")),
            "over": bool(body.get("over")),
            "args": args,
        }
        if body.get("funcformat") != "COERCE_EXPLICIT_CALL":
            result["syntax"] = "sql"
        return result

    args = [arg_record(data, arg) for arg in body.get("args", [])]
    if kind == "CoalesceExpr":
        name = ["coalesce"]
    elif kind == "MinMaxExpr":
        name = ["greatest" if body.get("op") == "IS_GREATEST" else "least"]
    else:
        name = ["nullif"]
    return {
        "kind": kind,
        "line": line,
        "column": column,
        "name": name,
        "star": False,
        "distinct": False,
        "variadic": False,
        "order": False,
        "filter": False,
        "over": False,
        "args": args,
    }


def main() -> None:
    assert platform.python_version() == "3.12.13", platform.python_version()
    assert dist_version("pglast") == "8.5", dist_version("pglast")
    assert dist_version("sqlfluff") == "4.4.0", dist_version("sqlfluff")
    assert tuple(pg.get_postgresql_version()) == (18, 6), pg.get_postgresql_version()
    assert common.TOOLS["pglast"] == "8.5" and common.TOOLS["sqlfluff"] == "4.4.0"

    source = HERE / "postgres.sql"
    data = source.read_bytes()
    tree = json.loads(reference.pg.parse_sql_json(data.decode("utf-8")))
    statements = [line for line in data.decode("utf-8").splitlines() if line.strip()]
    assert len(tree["stmts"]) == len(statements), (len(tree["stmts"]), len(statements))

    calls = []
    for node in tree["stmts"]:
        def visit(value: object) -> None:
            if isinstance(value, dict):
                for key, child in value.items():
                    if key == "A_Expr" and child.get("kind") == "AEXPR_NULLIF":
                        synthetic = {"location": child.get("location", -1),
                                     "args": [{"ColumnRef": child["lexpr"]}, {"ColumnRef": child["rexpr"]}]}
                        calls.append((first_location(child), call_record(data, "NullIfExpr", synthetic)))
                    elif key in {"FuncCall", "CoalesceExpr", "MinMaxExpr", "NullIfExpr"}:
                        calls.append((first_location(child), call_record(data, key, child)))
                    visit(child)
            elif isinstance(value, list):
                for child in value:
                    visit(child)
        visit(node)
    calls.sort(key=lambda item: item[0])
    result = {"tool": "libpg_query", "version": "pglast 8.5, PostgreSQL 18.6", "source": source.name,
              "calls": [record for _, record in calls]}
    (HERE / "postgres.expected.json").write_text(json.dumps(result, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
