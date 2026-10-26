"""SQLFluff, as the port readiness gates run it, inside the pinned runtime of conformance/recorders/port-readiness/runtimes/sqlfluff-<version>.

One JSON request on stdin, one JSON response on stdout:
- {"op": "facts", "codes": [...]}: what SQLFluff's own rule registry says about each rule, the regexes its rule
  modules compile, the PostgreSQL and ANSI keyword sets, and which rules the default configuration selects.
- {"op": "cases", "checkout": dir, "codes": [...]}: every case of every rule's YAML test file
  (test/fixtures/rules/std_rule_cases/<CODE>.yml) at the pinned checkout: the SQL texts and configuration words of all
  of them, and for each requested rule its own cases with SQLFluff's findings on them.
- {"op": "verdicts", "texts": [...]}: libpg_query's (pglast) verdict on each text, the independent PostgreSQL parser,
  on the SQL psql would send for the text read as a psql script (psql_sent_sql).

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
    # Names a rule reads from its dialect by label at run time (`dialect.sets("value_table_functions")`) cannot be read
    # from source either: record the postgres dialect's sets of names, which the adapter credits to code naming a label.
    dialect_sets = {
        label: sorted(members)
        for label, members in sorted(dialect_selector(DIALECT)._sets.items())
        if all(isinstance(member, str) for member in members)
    }
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
        "dialectSets": dialect_sets,
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


# A .sql file is a psql script: psql, not the server, reads its meta-commands (`\echo`, `\set`, `\gset`, `\if`, ...),
# substitutes its variables (`:name`, `:'name'`, `:"name"`) and sends the rows after `COPY ... FROM STDIN;` as data.
# None of that is SQL, so libpg_query, the server's parser, rejects a script that uses it. SQLFluff's postgres dialect
# lexes the same constructs out of the SQL, and so does the engine. psql_sent_sql turns a script into the SQL psql
# would send, position for position, following psql's lexers (src/bin/psql/psqlscanslash.l, src/fe_utils/psqlscan.l).
# psql's own variables: the ones it sets itself (psql's documentation, "Variables"), defined in every script.
PSQL_VARIABLES = frozenset((
    "AUTOCOMMIT COMP_KEYWORD_CASE DBNAME ECHO ECHO_HIDDEN ENCODING ERROR FETCH_COUNT HIDE_TABLEAM "
    "HIDE_TOAST_COMPRESSION HISTCONTROL HISTFILE HISTSIZE HOST IGNOREEOF LASTOID LAST_ERROR_MESSAGE LAST_ERROR_SQLSTATE "
    "ON_ERROR_ROLLBACK ON_ERROR_STOP PIPELINE_COMMAND_COUNT PIPELINE_RESULT_COUNT PIPELINE_SYNC_COUNT PORT PROMPT1 "
    "PROMPT2 PROMPT3 QUIET ROW_COUNT SERVER_VERSION_NAME SERVER_VERSION_NUM SERVICE SHELL_ERROR SHELL_EXIT_CODE "
    "SHOW_ALL_RESULTS SHOW_CONTEXT SINGLELINE SINGLESTEP SQLSTATE USER VERBOSITY VERSION VERSION_NAME VERSION_NUM "
    "WATCH_INTERVAL"
).split())
# Meta-commands that end the query buffer, as `;` does (the ones that return PSQL_CMD_SEND in psql's command.c).
PSQL_SENDS_QUERY = frozenset((
    "g gx gdesc gexec gset crosstabview watch parse close_prepared flush flushrequest getresults sendpipeline "
    "startpipeline syncpipeline endpipeline"
).split())
# Meta-commands whose argument is the rest of the line, backslashes included (OT_WHOLE_LINE in psql's command.c);
# psql reads `\copy` in any case.
PSQL_WHOLE_LINE = frozenset(("!", "copy", "ef", "ev", "sf", "sv", "h", "help"))
# Meta-commands whose argument is a file, or a pipe to a command when it starts with `|` (OT_FILEPIPE).
PSQL_FILE_PIPE = frozenset(("g", "gx", "o", "out", "w", "write"))
PSQL_PIPE = re.compile(r"[ \t\f\v]*\|")
PSQL_VARIABLE = re.compile(r""":(?:'([A-Za-z0-9_\x80-\U0010ffff]+)'|"([A-Za-z0-9_\x80-\U0010ffff]+)"|\{\?([A-Za-z0-9_\x80-\U0010ffff]+)\}|([A-Za-z0-9_\x80-\U0010ffff]+))""")
PSQL_ARGUMENT = re.compile(r"""'(?:[^'\\]|\\.|'')*'|"(?:[^"]|"")*"|`[^`]*`|[^\s'"`]+""")
SQL_WORD_CHAR = re.compile(r"[A-Za-z0-9_$\x80-\U0010ffff]")
SQL_WORD = re.compile(r""""((?:[^"]|"")*)"|([A-Za-z_\x80-\U0010ffff][A-Za-z0-9_$\x80-\U0010ffff]*)""")
DOLLAR_QUOTE = re.compile(r"\$(?:[A-Za-z_\x80-\U0010ffff][A-Za-z0-9_\x80-\U0010ffff]*)?\$")
COPY_FROM_STDIN = re.compile(r"(?is)^\s*copy\b.*\bfrom\s+stdin\b")
COPY_DATA_END = re.compile(r"^\\\.\r?$", re.M)


def _blank(text):
    return "".join(char if char in "\r\n" else " " for char in text)


def psql_sent_sql(text):
    """The SQL psql sends for the script `text`, with every character where the script has it.

    - A meta-command runs from its backslash to the end of the line, to the next unquoted backslash, or through `\\\\`;
      it becomes spaces, or `;` and spaces when it ends the query buffer (`\\g`, `\\gset`, `\\gexec`, ...), which then
      ends a statement as `;` does. `\\r` discards the query buffer, which becomes spaces. `\\;` and `\\:` are psql's
      literal `;` and `:`.
    - A variable psql defines at that point (its own, or set by `\\set`, `\\getenv`, `\\prompt` or `\\gset`) is replaced
      by a stand-in for its value of the same length: `:'name'` by the string ` 'name'`, `:"name"` by the identifier
      ` "name"`, `:name` (its raw value) by the identifier `_name`, and `:{?name}` by `TRUE` or `FALSE`. A `:name`
      psql does not define stays as it is, as psql sends it. The names `\\gset` defines are the result's column names,
      which come from the words of its query: every word of the query, under the command's prefix, counts as one. The
      value of a `:name` is not known here, so a value that is not an identifier where one cannot stand (a number
      where only a number may, or a statement) is read as an error.
    - The lines after a `COPY ... FROM STDIN` statement (or `\\copy ... from stdin`) through the line `\\.` (or the end
      of the script) are the copied data, and become spaces.
    Quoted strings and identifiers, dollar quotes and comments are copied unchanged, as psql reads none of this there.
    """
    out = []
    sent = 0  # where in `out` the query buffer starts
    query = []  # the query buffer, strings and comments blanked
    defined = set(PSQL_VARIABLES)
    data = None  # (start, end) of a COPY data block still ahead
    i, n = 0, len(text)

    def emit(piece, sql=None):
        out.append(piece)
        query.append(piece if sql is None else sql)

    def data_after_line(at):
        newline = text.find("\n", at)
        if newline < 0:
            return None
        end = COPY_DATA_END.search(text, newline + 1)
        return (newline + 1, end.end() if end else n)

    def end_statement(at):
        nonlocal data, sent
        if COPY_FROM_STDIN.match("".join(query)):
            data = data_after_line(at)
        query.clear()
        sent = len(out)

    while i < n:
        if data is not None and i >= data[0]:
            out.append(_blank(text[i:data[1]]))
            i, data = max(i, data[1]), None
            continue
        char, two = text[i], text[i:i + 2]
        if two == "--":
            end = text.find("\n", i)
            end = n if end < 0 else end
            emit(text[i:end], _blank(text[i:end])); i = end; continue
        if two == "/*":
            depth, end = 1, i + 2
            while end < n and depth:
                if text.startswith("/*", end): depth, end = depth + 1, end + 2
                elif text.startswith("*/", end): depth, end = depth - 1, end + 2
                else: end += 1
            emit(text[i:end], _blank(text[i:end])); i = end; continue
        if char == "'":
            # E'...' takes backslash escapes; every other string only doubled quotes.
            escapes = i > 0 and text[i - 1] in "eE" and (i < 2 or not SQL_WORD_CHAR.match(text[i - 2]))
            end = i + 1
            while end < n:
                if escapes and text[end] == "\\": end += 2; continue
                if text[end] == "'":
                    if text.startswith("'", end + 1): end += 2; continue
                    break
                end += 1
            emit(text[i:end + 1], _blank(text[i:end + 1])); i = end + 1; continue
        if char == '"':
            end = i + 1
            while end < n:
                if text[end] == '"':
                    if text.startswith('"', end + 1): end += 2; continue
                    break
                end += 1
            emit(text[i:end + 1]); i = end + 1; continue
        if char == "$" and not (i > 0 and SQL_WORD_CHAR.match(text[i - 1])):
            tag = DOLLAR_QUOTE.match(text, i)
            if tag:
                end = text.find(tag.group(0), tag.end())
                end = n if end < 0 else end + len(tag.group(0))
                emit(text[i:end], _blank(text[i:end])); i = end; continue
        if two == "::":
            emit(two); i += 2; continue
        if char == ":":
            variable = PSQL_VARIABLE.match(text, i)
            if variable:
                literal, identifier, test, raw = variable.groups()
                token = variable.group(0)
                if test is not None:
                    emit(("TRUE" if test in defined else "FALSE").ljust(len(token)))
                elif (literal or identifier or raw) in defined:
                    emit(("_" if raw else " ") + token[1:])
                else:
                    emit(token)
                i = variable.end(); continue
        if char == ";":
            emit(";"); i += 1; end_statement(i); continue
        if char == "\\":
            if text[i + 1:i + 2] in (";", ":"):
                emit(" " + text[i + 1]); i += 2; continue
            # The command name runs to whitespace or a backslash; its arguments to the end of the line or to an
            # unquoted backslash, and `\\` after them is consumed with them.
            end = i + 1
            while end < n and not text[end].isspace() and text[end] != "\\":
                end += 1
            name = text[i + 1:end]
            name = "copy" if name.lower() == "copy" else name
            whole_line = name in PSQL_WHOLE_LINE or (name in PSQL_FILE_PIPE and PSQL_PIPE.match(text, end) is not None)
            quote = None
            while end < n and text[end] != "\n":
                if quote:
                    if quote == "'" and text[end] == "\\": end += 1
                    elif text[end] == quote: quote = None
                elif text[end] in "'\"`": quote = text[end]
                elif text[end] == "\\" and not whole_line: break
                end += 1
            command = text[i:end]
            if text.startswith("\\\\", end):
                end += 2
            arguments = [argument[1:-1] if argument[0] == "'" else argument for argument in PSQL_ARGUMENT.findall(command[1 + len(name):])]
            if name in ("set", "getenv") and arguments:
                defined.add(arguments[0])
            elif name == "unset" and arguments:
                defined.discard(arguments[0])
            elif name == "prompt" and arguments:
                defined.add(arguments[-1])
            elif name == "gset":
                prefix = arguments[0] if arguments else ""
                for quoted, word in SQL_WORD.findall("".join(query)):
                    defined.add(prefix + (quoted.replace('""', '"') if quoted else word.lower()))
            if name == "copy" and re.search(r"(?i)\bfrom\s+stdin\b", command):
                data = data_after_line(end)
            if name in PSQL_SENDS_QUERY:
                out.append(";" + _blank(text[i + 1:end])); i = end; end_statement(i); continue
            out.append(_blank(text[i:end]))
            if name in ("r", "reset"):
                out[sent:] = [_blank(piece) for piece in out[sent:]]
                query.clear()
            i = end; continue
        emit(char); i += 1
    return "".join(out)


def verdicts(texts):
    from pglast import parser

    version = ".".join(str(part) for part in parser.get_postgresql_version())
    out = []
    for text in texts:
        try:
            parser.parse_sql(psql_sent_sql(text))
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
    elif request["op"] == "verdicts":
        response = verdicts(request["texts"])
    else:
        raise SystemExit(f"unknown op {request['op']}")
    json.dump(response, sys.stdout)


if __name__ == "__main__":
    main()
