"""Records SQLFluff 4.4.0 for the SQL pack's chosen T4 rules (RULES; the pack's conformance.json lists them). Run
by `npm run fixtures:derive` (a maintainer, or the bump workflow for an upstream release; never pull request CI), with
the pinned venv (fixtures/sql/common.py):

    <checkout>/.rerail/tools/sql/venv/bin/python generate.py fixtures

`fixtures` copies every case of each chosen rule's own YAML test file (`test/fixtures/rules/std_rule_cases/<CODE>.yml`
of the pinned sqlfluff checkout, tag 4.4.0) that the port can reproduce: the case's dialect is one the pack targets
(ansi, postgres, mysql, sqlite), its rule configuration is the port's (RULES; SQLFluff's defaults except where
noted) and its SQL holds no Jinja (Rerail reads raw SQL; the cases run here with the `raw` templater). Each case's
`pass_str` or `fail_str` becomes `fixtures/<dialect>/<CODE>/<case>.sql` (provenance.json names its YAML file and case).
SQLFluff then lints every copy with every chosen rule, and the run is required to reproduce the upstream test
on a rule's own cases: no finding of the rule on a `pass_str`, at least one on a `fail_str`, and exactly the
`violations` positions (or `line_numbers`) where the case lists them. So the expected findings on a rule's own cases are the
upstream test's expectation, and on the other rules' cases the tool's output. `source.units` names each upstream copy's
origin: the rule whose YAML test file it came from (Rerail's own cases have none). `fixtures.cases.json` keeps per case
what the standard expected findings have no field for: the upstream expectation, the reference parser's verdict on the copy
(PostgreSQL 18.6 for ansi and postgres cases, SQLite 3.50.4 for sqlite ones) and, for ansi cases, whether SQLFluff's
postgres dialect reports the same findings (a pack without an ansi dialect reads them as PostgreSQL).

Each finding is the rule code, path, line and column (SQLFluff's `start_line_no` and `start_line_pos`: 1-based,
in characters).
"""

from __future__ import annotations

import collections
import json
import pathlib
import shutil
import sys

import yaml

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[3] / "fixtures" / "sql"))
sys.path.insert(0, str(HERE.parents[3] / "scripts" / "s3"))
import common  # noqa: E402
import pack_fixtures  # noqa: E402
import reference  # noqa: E402

TOOL = "sqlfluff"
ORIGIN = f"{common.CORPORA['sqlfluff-dialects']['repository']}@{common.CORPORA['sqlfluff-dialects']['commit']}"
CASES = "test/fixtures/rules/std_rule_cases"
DIALECTS = ("ansi", "postgres", "mysql", "sqlite")

# code -> the port's rule configuration (SQLFluff's `rules:` section). Empty: SQLFluff's defaults.
RULES: dict[str, dict] = {
    "AL04": {},
    "AL05": {},
    "AL08": {},
    "AL09": {},
    "AM01": {},
    "AM02": {},
    "AM05": {},  # fully_qualify_join_types = inner (default): a bare JOIN
    "AM09": {},
    "CV02": {},
    "CV04": {},  # prefer_count_1 = prefer_count_0 = False (default): COUNT(*)
    "CV05": {},
    "CV08": {},
    "PG01": {"postgres.excessive_locks": {"force_enable": True}},  # opt-in upstream
    "PG02": {"postgres.not_valid_foreign_key": {"force_enable": True}},  # opt-in upstream
    "RF01": {},
    "RF04": {},
    "RF05": {},
    "RF06": {},
    "ST01": {},
    "ST02": {},
    "ST08": {},
    "ST10": {},
}
JINJA = ("{{", "{%", "{#")

# Rerail's own cases, written to `fixtures/rerail/<dialect>/<CODE>/<name>.sql`: shapes the review of the port found
# SQLFluff's YAML does not cover for PostgreSQL. SQLFluff 4.4.0 (postgres dialect) must give the stated verdict, as for
# upstream cases: `(line, column)` of each finding of the rule on a fail case, none on a pass case.
RERAIL_CASES: dict[str, list[tuple[str, str, list[tuple[int, int]]]]] = {
    "AM05": [
        ("fail_bare_join_lateral", "SELECT * FROM t JOIN LATERAL (SELECT 1) AS s ON TRUE;\n", [(1, 17)]),
        ("pass_inner_join_lateral", "SELECT * FROM t INNER JOIN LATERAL (SELECT 1) AS s ON TRUE;\n", []),
        ("pass_left_join_lateral", "SELECT * FROM t LEFT JOIN LATERAL (SELECT 1) AS s ON TRUE;\n", []),
        ("pass_cross_join_lateral", "SELECT * FROM t CROSS JOIN LATERAL (SELECT 1) AS s;\n", []),
    ],
    "RF01": [
        ("pass_insert_select_on_conflict_excluded",
         "INSERT INTO t (x, y) SELECT s.a, s.b FROM s ON CONFLICT (x) DO UPDATE SET y = EXCLUDED.y;\n", []),
        ("pass_insert_select_on_conflict_excluded_lower",
         "INSERT INTO t (x, y) SELECT s.a, s.b FROM s ON CONFLICT (x) DO UPDATE SET y = excluded.y;\n", []),
    ],
    "RF04": [
        ("fail_postgres_and_mysql_unreserved_column_names",
         "CREATE TABLE t (hash bigint NOT NULL, list text, PRIMARY KEY (hash));\n", [(1, 17), (1, 39)]),
        ("pass_quoted_keyword_column_name", 'CREATE TABLE t ("hash" bigint);\n', []),
    ],
    "RF06": [
        ("fail_quoted_index_column", 'CREATE INDEX i ON t (url, "timestamp");\n', [(1, 27)]),
        ("fail_quoted_index_column_with_order", 'CREATE INDEX i ON t ("a" DESC, "B");\n', [(1, 22)]),
    ],
    "ST02": [
        ("fail_quoted_same_case_then", 'SELECT CASE WHEN "a" IS NOT NULL THEN "a" END FROM t;\n', [(1, 18)]),
        ("pass_quoted_different_case_then", 'SELECT CASE WHEN "a" IS NOT NULL THEN "A" END FROM t;\n', []),
        ("pass_quoted_and_naked_then", 'SELECT CASE WHEN "a" IS NOT NULL THEN a END FROM t;\n', []),
        ("fail_naked_different_case_then", "SELECT CASE WHEN a IS NOT NULL THEN A END FROM t;\n", [(1, 18)]),
        ("fail_quoted_same_case_else", 'SELECT CASE WHEN "a" IS NULL THEN 1 ELSE "a" END FROM t;\n', [(1, 18)]),
        ("pass_quoted_different_case_else", 'SELECT CASE WHEN "a" IS NULL THEN 1 ELSE "A" END FROM t;\n', []),
        ("fail_qualified_quoted_part_same_case", 'SELECT CASE WHEN x."B" IS NOT NULL THEN X."B" END FROM t;\n', [(1, 18)]),
        ("pass_qualified_quoted_part_different_case", 'SELECT CASE WHEN x."B" IS NOT NULL THEN X."b" END FROM t;\n', []),
    ],
}


def rule_configs() -> dict:
    merged: dict = {}
    for config in RULES.values():
        merged.update(config)
    return merged


def fluff_config(dialect: str):
    from sqlfluff.core import FluffConfig

    return FluffConfig(
        configs={"core": {"dialect": dialect, "templater": "raw", "large_file_skip_byte_limit": 0},
                 "rules": rule_configs()},
        overrides={"rules": ",".join(RULES), "dialect": dialect},
    )


def lint(text: str, dialect: str) -> tuple[list[dict], list[str]]:
    """The chosen rules' findings on `text`, and SQLFluff's non-rule errors (parse, lex, templating)."""
    from sqlfluff.core import Linter
    from sqlfluff.core.errors import SQLLintError

    linted = Linter(config=fluff_config(dialect)).lint_string(text)
    findings, problems = [], []
    for violation in linted.get_violations():
        if isinstance(violation, SQLLintError):
            code = violation.rule_code()
            assert code in RULES, code
            assert not violation.desc().startswith("Unexpected exception"), violation.desc()
            findings.append({"rule": code, "line": violation.line_no, "column": violation.line_pos})
        else:
            problems.append(f"{violation.rule_code()} {violation.line_no}:{violation.line_pos} {violation.desc()}")
    return findings, problems


def load_cases(code: str) -> tuple[list[dict], dict[str, int]]:
    """The rule's cases the port reproduces, in file order, with the reason every other case is left out."""
    path = common.corpus_dir("sqlfluff-dialects") / CASES / f"{code}.yml"
    data = yaml.safe_load(path.read_text())
    assert data.pop("rule") == code
    shared = data.pop("configs", None)
    kept, left = [], collections.Counter()
    for name, case in data.items():
        configs = dict(case.get("configs", shared) or {})
        core = dict(configs.pop("core", None) or {})
        dialect = core.pop("dialect", "ansi")
        rules = configs.pop("rules", None) or {}
        sql = case.get("fail_str", case.get("pass_str"))
        if case.get("skip"):
            left["skipped upstream"] += 1
        elif dialect not in DIALECTS:
            left[f"dialect {dialect}"] += 1
        elif core or configs:
            left["core or templater configuration"] += 1
        elif rules != RULES[code]:
            left["rule configuration other than the port's"] += 1
        elif sql is None:
            left["no SQL"] += 1
        elif any(marker in sql for marker in JINJA):
            left["Jinja"] += 1
        else:
            kept.append({"rule": code, "case": name, "dialect": dialect, "sql": sql,
                         "expect": "fail" if "fail_str" in case else "pass",
                         "violations": [[v["start_line_no"], v["start_line_pos"]] for v in case["violations"]
                                        if v.get("code", code) == code]
                         if case.get("violations") else None,
                         "lineNumbers": case.get("line_numbers") or None})
    return kept, dict(left)


def reference_verdict(sql: str, dialect: str) -> dict:
    if dialect in ("ansi", "postgres"):
        try:
            reference.pg.parse_sql_json(sql)
            return {"tool": "libpg_query (PostgreSQL 18.6)", "accepts": True}
        except reference.pg.ParseError as error:
            return {"tool": "libpg_query (PostgreSQL 18.6)", "accepts": False, "error": str(error)[:160]}
    if dialect == "sqlite":
        statements = reference.sqlite_statements(sql)
        return {"tool": "sqlite3 3.50.4", "accepts": bool(statements) and all(s.accepted for s in statements)}
    return {"tool": "none"}


def check_case(case: dict, findings: list[dict]) -> None:
    own = [(f["line"], f["column"]) for f in findings if f["rule"] == case["rule"]]
    where = f"{case['rule']} {case['case']}"
    if case["expect"] == "pass":
        assert not own, f"{where}: upstream passes, SQLFluff reports {own}"
        return
    assert own, f"{where}: upstream fails, SQLFluff reports nothing"
    if case["violations"] is not None:
        assert sorted(own) == sorted(map(tuple, case["violations"])), (where, own, case["violations"])
    if case["lineNumbers"] is not None:
        assert [line for line, _ in own] == case["lineNumbers"], (where, own, case["lineNumbers"])


def write_expected_findings(path: pathlib.Path, source: dict, findings: list[dict]) -> None:
    findings = sorted(findings, key=lambda f: (f["rule"], f["path"], f["line"], f.get("column", 0)))
    path.write_text(json.dumps({"tool": TOOL, "version": common.TOOLS["sqlfluff"], "source": source,
                                "findings": findings}, indent=1, ensure_ascii=False) + "\n")
    pack_fixtures.expected_findings_written(path, source)


def fixtures() -> None:
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    files, findings, cases, left_out, units, copies = [], [], [], {}, {}, {}
    for code in RULES:
        kept, left = load_cases(code)
        left_out[code] = left
        for name, sql, positions in RERAIL_CASES.get(code, []):
            kept.append({"rule": code, "case": name, "dialect": "postgres", "sql": sql, "own": True,
                         "expect": "fail" if positions else "pass", "violations": positions or None,
                         "lineNumbers": None})
        for case in kept:
            relative = f"{'rerail/' if case.get('own') else ''}{case['dialect']}/{code}/{case['case']}.sql"
            (target / relative).parent.mkdir(parents=True, exist_ok=True)
            with open(target / relative, "w", encoding="utf-8", newline="") as handle:
                handle.write(case["sql"])
            found, problems = lint(case["sql"], case["dialect"])
            assert not problems, (relative, problems)
            check_case(case, found)
            row = {"file": relative, "rule": code, "case": case["case"], "dialect": case["dialect"],
                   "expect": case["expect"], "upstreamViolations": None if case.get("own") else case["violations"],
                   "upstreamLineNumbers": case["lineNumbers"],
                   "reference": reference_verdict(case["sql"], case["dialect"])}
            if case.get("own"):
                row["rerail"] = True
            if case["dialect"] == "ansi":
                as_postgres, problems = lint(case["sql"], "postgres")
                row["postgresDialect"] = {"parses": not problems,
                                          "sameFindings": not problems and sorted(map(_key, as_postgres))
                                          == sorted(map(_key, found))}
            cases.append(row)
            files.append(relative)
            if not case.get("own"):
                units[relative] = {"rules": [code]}
                copies[f"fixtures/{relative}"] = {
                    "source": TOOL, "from": f"{CASES}/{code}.yml", "how": "extracted",
                    "detail": f"case `{case['case']}`; `{case['expect']}_str`, verbatim"}
            findings += [{"rule": f["rule"], "path": relative, "line": f["line"], "column": f["column"]}
                         for f in found]
    write_expected_findings(HERE / "fixtures.expected.json",
                    {"kind": "fixtures", "origin": ORIGIN, "rules": sorted(RULES), "files": files,
                     "units": dict(sorted(units.items()))}, findings)
    (HERE / "fixtures.cases.json").write_text(json.dumps(
        {"tool": TOOL, "version": common.TOOLS["sqlfluff"], "origin": ORIGIN, "portConfiguration": RULES,
         "leftOut": left_out, "cases": cases}, indent=1, ensure_ascii=False) + "\n")
    pack_fixtures.written(HERE / "fixtures.cases.json")
    pack_fixtures.write_provenance("sql", TOOL, copies, {TOOL: common.CORPORA["sqlfluff-dialects"]["commit"]})
    print(f"{len(files)} fixture files, {len(findings)} findings", file=sys.stderr)


def _key(finding: dict) -> tuple:
    return finding["rule"], finding["line"], finding["column"]


def main() -> None:
    if sys.argv[1:2] == ["fixtures"]:
        common.check_pins(need_grammars=False, corpora=("sqlfluff-dialects",))
        fixtures()
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
