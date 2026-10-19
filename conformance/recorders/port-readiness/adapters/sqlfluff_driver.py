"""SQLFluff, as the port readiness gates run it, inside the pinned runtime of conformance/recorders/port-readiness/runtimes/sqlfluff-<version>.

One JSON request on stdin, one JSON response on stdout:
- {"op": "facts", "codes": [...]}: what SQLFluff's own rule registry says about each rule, the regexes its rule
  modules compile, the PostgreSQL and ANSI keyword sets, and which rules the default configuration selects.
- {"op": "cases", "checkout": dir, "codes": [...]}: every case of every rule's YAML test file
  (test/fixtures/rules/std_rule_cases/<CODE>.yml) at the pinned checkout: the SQL texts and configuration words of all
  of them, and for each requested rule its own cases with SQLFluff's findings on them.
- {"op": "lint", "texts": [...], "codes": [...]}: SQLFluff's findings of the named rules on each text, or why it does
  not read the text whole.
- {"op": "verdicts", "texts": [...]}: libpg_query's (pglast) verdict on each text, the independent PostgreSQL parser.

Every text is linted as `dialect: postgres`, `templater: raw`, with the port's rule configuration (PORT_RULES): the
engine reads every .sql file as PostgreSQL, so that is the configuration the ports implement. The case selection,
`fluff_config` and `lint` follow packs/sql/upstream/sqlfluff/generate.py, which records the pack's conformance
fixtures with the same SQLFluff.

Positions are SQLFluff's `line_no` and `line_pos`: 1-based, in characters.
"""

import concurrent.futures
import importlib
import importlib.metadata
import json
import os
import pathlib
import re
import sys

DIALECT = "postgres"
CASES = "test/fixtures/rules/std_rule_cases"
# Above this size SQLFluff's parse is super-linear; such a file is not read (generate.py's SQLFLUFF_MAX_BYTES).
SQLFLUFF_MAX_BYTES = 200_000
JINJA = ("{{", "{%", "{#")

# The port's rule configuration (SQLFluff's `rules:` section), by code; a code not listed runs with SQLFluff's
# defaults. packs/sql/upstream/sqlfluff/generate.py's RULES holds the same: PG01 and PG02 are opt-in upstream
# (`force_enable = False` in SQLFluff's default configuration), and the ports implement them switched on.
PORT_RULES = {
    "PG01": {"postgres.excessive_locks": {"force_enable": True}},
    "PG02": {"postgres.not_valid_foreign_key": {"force_enable": True}},
}


def port_rules(code):
    return PORT_RULES.get(code, {})


def fluff_config(codes, rules_config):
    from sqlfluff.core import FluffConfig

    return FluffConfig(
        configs={"core": {"dialect": DIALECT, "templater": "raw", "large_file_skip_byte_limit": 0}, "rules": rules_config},
        overrides={"rules": ",".join(codes), "dialect": DIALECT},
    )


def merged_port_rules(codes):
    merged = {}
    for code in codes:
        merged.update(port_rules(code))
    return merged


_LINTER = None


def _init_worker(codes, rules_config):
    global _LINTER
    from sqlfluff.core import Linter

    _LINTER = Linter(config=fluff_config(codes, rules_config))


def _lint_one(text):
    """The selected rules' findings on `text`, or why SQLFluff does not read it whole (parse, lex, templating)."""
    from sqlfluff.core.errors import SQLLintError

    if len(text.encode("utf-8")) > SQLFLUFF_MAX_BYTES:
        return {"excluded": f"larger than {SQLFLUFF_MAX_BYTES} bytes, where SQLFluff's parse is super-linear"}
    linted = _LINTER.lint_string(text)
    findings, problems = [], []
    for violation in linted.get_violations():
        if isinstance(violation, SQLLintError):
            if violation.desc().startswith("Unexpected exception"):
                problems.append(f"{violation.rule_code()} {violation.line_no}:{violation.line_pos} {violation.desc()}")
            else:
                findings.append({"rule": violation.rule_code(), "line": violation.line_no, "column": violation.line_pos})
        else:
            problems.append(f"{violation.rule_code()} {violation.line_no}:{violation.line_pos} {violation.desc()}")
    if problems:
        return {"excluded": f"SQLFluff's {DIALECT} dialect does not read it whole: {problems[0][:200]}"}
    findings.sort(key=lambda f: (f["rule"], f["line"], f["column"]))
    return {"findings": findings}


def lint_texts(texts, codes, rules_config):
    if not texts:
        return []
    workers = max(1, min(len(texts) // 8 + 1, os.cpu_count() or 1))
    if workers == 1:
        _init_worker(codes, rules_config)
        return [_lint_one(text) for text in texts]
    with concurrent.futures.ProcessPoolExecutor(workers, initializer=_init_worker, initargs=(codes, rules_config)) as pool:
        return list(pool.map(_lint_one, texts, chunksize=4))


def facts(codes):
    from sqlfluff.core import FluffConfig, Linter
    from sqlfluff.core.dialects import dialect_selector
    from sqlfluff.core.plugin.host import get_plugin_manager

    registered = {}
    for plugin_rules in get_plugin_manager().hook.get_rules():
        for rule in plugin_rules:
            registered[rule.code] = rule
    rules = {}
    for code in codes:
        rule = registered.get(code)
        if rule is None:
            continue
        module = importlib.import_module(rule.__module__)
        rules[code] = {
            "name": rule.name,
            "module": rule.__module__,
            "file": module.__file__,
            "groups": list(rule.groups),
            "aliases": list(rule.aliases),
            "configKeywords": list(getattr(rule, "config_keywords", []) or []),
        }
    # The rules `rules = all` (SQLFluff's default configuration) selects for the postgres dialect.
    default = Linter(config=FluffConfig(overrides={"dialect": DIALECT})).get_rulepack()
    selected = sorted(rule.code for rule in default.rules)
    # Patterns upstream builds at run time cannot be read from source; read the compiled objects.
    regexes = {}
    for name in sorted({rule["module"] for rule in rules.values()}):
        module = importlib.import_module(name)
        regexes[name] = [
            {"pattern": value.pattern, "flags": value.flags}
            for value in vars(module).values()
            if isinstance(value, re.Pattern) and isinstance(value.pattern, str)
        ]
    keywords = set()
    for dialect in ("ansi", DIALECT):
        selected_dialect = dialect_selector(dialect)
        for kind in ("reserved_keywords", "unreserved_keywords"):
            keywords |= set(selected_dialect.sets(kind))
    metadata = importlib.metadata.metadata("sqlfluff")
    classifiers = [value for key, value in metadata.items() if key == "Classifier" and value.startswith("License :: OSI Approved :: ")]
    import sqlfluff

    return {
        "version": importlib.metadata.version("sqlfluff"),
        # The wheel's License field is the licence text; its SPDX name is in the trove classifier (`... :: MIT License`).
        "license": metadata.get("License-Expression") or ", ".join(value.removeprefix("License :: OSI Approved :: ").removesuffix(" License") for value in classifiers),
        "python": ".".join(str(part) for part in sys.version_info[:3]),
        "package": os.path.dirname(sqlfluff.__file__),
        "rules": rules,
        "defaultSelected": selected,
        "regexes": regexes,
        "keywords": sorted(keywords),
    }


def config_words(value):
    """Every string in a test configuration, keys and values, which the gates read words from."""
    if isinstance(value, dict):
        return [word for key, inner in value.items() for word in [str(key), *config_words(inner)]]
    if isinstance(value, list):
        return [word for inner in value for word in config_words(inner)]
    return [] if value is None or isinstance(value, bool) else [str(value)]


def load_yaml(path):
    import yaml

    return yaml.safe_load(path.read_text(encoding="utf-8"))


def case_entries(code, data):
    """The rule's cases in file order: SQL, expectation and why the port's configuration does not run it."""
    data = dict(data)
    assert data.pop("rule") == code, code
    shared = data.pop("configs", None)
    entries = []
    for name, case in data.items():
        configs = dict(case.get("configs", shared) or {})
        core = dict(configs.pop("core", None) or {})
        dialect = core.pop("dialect", "ansi")
        rules = configs.pop("rules", None) or {}
        sql = case.get("fail_str", case.get("pass_str"))
        skipped = None
        if case.get("skip"):
            skipped = f"upstream skips it: {case['skip']}"
        elif sql is None:
            skipped = "the case has no SQL"
        elif any(marker in sql for marker in JINJA):
            skipped = "Jinja: the ports read raw SQL, and SQLFluff runs this case through its jinja templater"
        elif core or configs:
            skipped = f"it configures SQLFluff beyond the dialect and rules ({', '.join(sorted([*core, *configs]))}), which the port does not"
        elif rules != port_rules(code):
            skipped = f"its rule configuration ({json.dumps(rules, sort_keys=True)}) is not the port's ({json.dumps(port_rules(code), sort_keys=True)})"
        violations = None
        if "fail_str" in case and case.get("violations"):
            violations = [[v["start_line_no"], v["start_line_pos"]] for v in case["violations"] if v.get("code", code) == code]
        entries.append({
            "case": name,
            "dialect": dialect,
            "sql": sql if sql is not None else "",
            "expect": "fail" if "fail_str" in case else "pass",
            "skipped": skipped,
            # Upstream's written expectation counts only where the case's own configuration is the port's.
            "markupApplies": skipped is None and dialect == DIALECT,
            "violations": violations,
        })
    return entries


def cases(checkout, codes):
    directory = pathlib.Path(checkout) / CASES
    texts, words = [], []
    for path in sorted(directory.glob("*.yml")):
        data = load_yaml(path)
        words += config_words(data.get("configs"))
        for name, case in data.items():
            if not isinstance(case, dict):
                continue
            words += config_words(case.get("configs"))
            for key in ("fail_str", "pass_str"):
                if isinstance(case.get(key), str):
                    texts.append(case[key])
    rules = {code: case_entries(code, load_yaml(directory / f"{code}.yml")) for code in codes if (directory / f"{code}.yml").exists()}
    # One run over every runnable case of every rule, configured as the ports are.
    runnable = [entry for entries in rules.values() for entry in entries if entry["skipped"] is None]
    results = lint_texts([entry["sql"] for entry in runnable], codes, merged_port_rules(codes))
    for entry, result in zip(runnable, results):
        if "excluded" in result:
            entry["skipped"] = result["excluded"]
            entry["markupApplies"] = False
            entry["findings"] = []
        else:
            entry["findings"] = result["findings"]
    # A rule with `force_enable` in its configuration may be switched off by SQLFluff's default configuration for the
    # postgres dialect ("Run this rule even for dialects where this rule is disabled by default"): it is off when, with
    # `force_enable` at its default, it reports nothing on any of its own runnable cases where `force_enable = True`
    # reports something.
    default_off = {}
    for code, entries in rules.items():
        own = [entry for entry in entries if entry["skipped"] is None]
        if not own:
            continue
        from sqlfluff.core.plugin.host import get_plugin_manager

        rule = next(rule for plugin_rules in get_plugin_manager().hook.get_rules() for rule in plugin_rules if rule.code == code)
        if "force_enable" not in (getattr(rule, "config_keywords", []) or []):
            continue
        sqls = [entry["sql"] for entry in own]
        forced = lint_texts(sqls, [code], {rule.name: {"force_enable": True}})
        defaulted = lint_texts(sqls, [code], {})
        count = lambda results: sum(len(result.get("findings", [])) for result in results)
        default_off[code] = count(defaulted) == 0 and count(forced) > 0
    return {"texts": texts, "configWords": sorted(set(words)), "rules": rules, "defaultOff": default_off}


def verdicts(texts):
    from pglast import parser

    version = ".".join(str(part) for part in parser.get_postgresql_version())
    out = []
    for text in texts:
        try:
            parser.parse_sql(text)
            syntax = []
        except parser.ParseError as error:
            syntax = [str(error).splitlines()[0][:300]]
        out.append([{"parser": f"libpg_query (PostgreSQL {version})", "syntax": syntax, "grammar": None}])
    return out


def main():
    request = json.load(sys.stdin)
    if request["op"] == "facts":
        response = facts(request["codes"])
    elif request["op"] == "cases":
        response = cases(request["checkout"], request["codes"])
    elif request["op"] == "lint":
        response = lint_texts(request["texts"], request["codes"], merged_port_rules(request["codes"]))
    elif request["op"] == "verdicts":
        response = verdicts(request["texts"])
    else:
        raise SystemExit(f"unknown op {request['op']}")
    json.dump(response, sys.stdout)


if __name__ == "__main__":
    main()
