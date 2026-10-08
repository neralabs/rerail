"""Writes sqlfluff.expected.json: SQLFluff 4.4.0's reports for each suppress/sqlfluff fixture, the T2 oracle for the
SQL pack's suppression capability (SQLFluff's `noqa` comments). Run offline by a maintainer, never in CI, with the
pinned venv (../common.py):

    <checkout>/.rerail/tools/sql/venv/bin/python generate_sqlfluff.py

Each fixture is linted as PostgreSQL (`raw` templater) with two rules that fire on every probe line, CV05
(`= NULL`, reported at the `=`) and CV08 (`RIGHT JOIN`, reported at `RIGHT`): once with noqa comments ignored
(`disable_noqa`, `all`) and once honoured (`kept`). A report is [code, line, column] (SQLFluff's 1-based line and
character position); a malformed directive is reported by SQLFluff itself with code PRS. Suppressed = in `all`, not in
`kept`. Every fixture must also be accepted whole by PostgreSQL 18.6's parser (libpg_query), so a noqa comment is a
real comment and not text inside a literal the grammar misreads.
"""

from __future__ import annotations

import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402
import reference  # noqa: E402

RULES = ("CV05", "CV08")


def report(text: str, *, disable_noqa: bool) -> list[list]:
    from sqlfluff.core import FluffConfig, Linter

    config = FluffConfig(configs={"core": {"dialect": "postgres", "templater": "raw", "disable_noqa": disable_noqa}},
                         overrides={"rules": ",".join(RULES), "dialect": "postgres"})
    linted = Linter(config=config).lint_string(text)
    return sorted([violation.rule_code(), violation.line_no, violation.line_pos]
                  for violation in linted.get_violations())


def main() -> None:
    common.check_pins(need_grammars=False)
    result: dict = {"tool": "sqlfluff", "version": common.TOOLS["sqlfluff"], "dialect": "postgres",
                    "rules": list(RULES)}
    for path in sorted((HERE / "sqlfluff").glob("*.sql")):
        text = path.read_text()
        reference.pg.parse_sql_json(text)  # raises if PostgreSQL rejects the fixture
        everything = report(text, disable_noqa=True)
        assert all(code in RULES for code, _, _ in everything), (path.name, everything)
        result[path.name] = {"all": everything, "kept": report(text, disable_noqa=False)}
    (HERE / "sqlfluff.expected.json").write_text(json.dumps(result, indent=1) + "\n")


if __name__ == "__main__":
    main()
