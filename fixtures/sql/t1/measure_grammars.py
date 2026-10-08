"""T1 grammar measurement for the SQL pack: candidate tree-sitter grammars against the reference parsers, on the
pinned corpora (common.py). Run offline by a maintainer, never in CI:

    <checkout>/.nera/tools/sql/venv/bin/python measure_grammars.py --out <report.json> [--mysql-port 3307] [--jobs 10]

Per file (dialect from the corpus's path globs, never detected):
- reference: PostgreSQL 18.6's parser (libpg_query) or SQLite 3.50.4, statement by statement (reference.py); MySQL
  8.4.11, file by file: each Mattermost migration is sent as one multi-statement query, ups in order then downs in
  reverse, as Mattermost's migrator runs them (needs `docker run -d -p 3307:3306 -e MYSQL_ROOT_PASSWORD=t <MYSQL_IMAGE>`;
  the root credential is read from `MYSQL_ROOT_PASSWORD`, `t` when unset, for that throwaway container).
- each candidate grammar for that dialect parses the file (error-file rate, throughput) and every statement the
  reference accepts, alone (error-statement rate, by statement kind).
- structural agreement, where the trees are comparable: top-level statement count; PostgreSQL statement kind
  (gmr-postgres names its nodes after the server's); and, for every relation, column reference and constant the
  server's tree holds, whether the grammar has a node starting at that byte whose text reads as the same name or value.
- SQLFluff 4.4.0 parses each file too (`unparsable` segments), as the T4 reference linter's own reading of the corpus.
"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures
import datetime
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402
import reference  # noqa: E402

SQLFLUFF_MAX_BYTES = 200_000  # SQLFluff's parse is super-linear on a 9 MB pg_dump; larger files are recorded as skipped
LITERAL_TOKEN = re.compile(rb"[-+]?\s*[0-9.]|'|[eEbBxXnN]'|[uU]&'|\$|(?i:true|false|null)\b")
STRING_TOKEN = re.compile(rb"'|[eEnN]'|[uU]&'|\$")
SAMPLES = 8


def grammars_for(dialect: str) -> list[str]:
    return [name for name, grammar in common.GRAMMARS.items() if dialect in grammar["dialects"]]


_PARSERS: dict = {}


def parser(name: str):
    if name not in _PARSERS:
        _PARSERS[name] = common.load_parser(name)
    return _PARSERS[name]


def first_error(node):
    """The first ERROR or MISSING node in document order, or None."""
    if node.type == "ERROR" or node.is_missing:
        return node
    if not node.has_error:
        return None
    for child in node.children:
        found = first_error(child)
        if found is not None:
            return found
    return node


def starts(root) -> dict[int, list]:
    index: dict[int, list] = collections.defaultdict(list)
    stack = [root]
    while stack:
        node = stack.pop()
        if node.is_named:
            index[node.start_byte].append(node)
        stack.extend(node.children)
    return index


def statement_kind(tree, grammar: str) -> str | None:
    """gmr-postgres: toplevel_stmt > stmt > <libpg_query node name>."""
    if grammar != "gmr-postgres":
        return None
    for top in tree.root_node.children:
        if top.type == "toplevel_stmt":
            stmt = top.named_children[0] if top.named_children else None
            if stmt is not None and stmt.type == "stmt" and stmt.named_children:
                return stmt.named_children[0].type
            return stmt.type if stmt is not None else None
    return None


def count_statements(tree, grammar: str) -> int:
    kind = common.GRAMMARS[grammar]["statement"]
    return sum(1 for child in tree.root_node.children if child.type == kind)


def measure_file(job: tuple[str, str, str]) -> dict:
    corpus, relative, dialect = job
    path = common.corpus_dir(corpus) / relative
    raw = path.read_bytes()
    result: dict = {"corpus": corpus, "path": relative, "dialect": dialect, "bytes": len(raw)}
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        text = raw.decode("latin-1")
        result["encoding"] = "latin-1"
    if dialect == "postgres":
        text, meta, copy = reference.clean_psql(text)
        result["blankedLines"] = {"psqlMeta": meta, "copyData": copy}
    elif dialect == "sqlite":
        text, dot = reference.clean_sqlite(text)
        result["blankedLines"] = {"sqliteDot": dot}
    data = text.encode("utf-8")

    statements: list[reference.Statement] | None = None
    if dialect == "postgres":
        try:
            statements = reference.postgres_statements(text)
        except reference.pg.ParseError as error:  # the scanner rejects a token (`""`, `123abc`): no statement split
            result["referenceError"] = f"{type(error).__name__}: {error}"
    elif dialect == "sqlite":
        statements = reference.sqlite_statements(text)
    if statements is not None:
        result["statements"] = len(statements)
        result["acceptedStatements"] = sum(1 for statement in statements if statement.accepted)
        result["referenceAccepts"] = bool(statements) and all(statement.accepted for statement in statements)
        result["rejectedSamples"] = [
            {"line": text.count("\n", 0, statement.start) + 1, "error": (statement.error or "")[:160]}
            for statement in statements if not statement.accepted][:3]
    if dialect == "postgres" and "referenceError" not in result:
        # The file is accepted when the server parses it whole (a statement split by the scanner alone can differ).
        begin = time.perf_counter()
        try:
            reference.pg.parse_sql_json(text)
            result["referenceSeconds"] = time.perf_counter() - begin
        except reference.pg.ParseError as error:
            if result.get("referenceAccepts"):
                result["wholeFileRejected"] = str(error)[:160]
            result["referenceAccepts"] = False

    result["grammars"] = {}
    for grammar in grammars_for(dialect):
        begin = time.perf_counter()
        tree = parser(grammar).parse(data)
        seconds = time.perf_counter() - begin
        error = first_error(tree.root_node)
        row: dict = {"seconds": seconds, "fileError": error is not None}
        if error is not None:
            row["firstError"] = {"line": error.start_point[0] + 1, "column": error.start_point[1] + 1}
        if statements is not None and error is None and result.get("referenceAccepts"):
            row["statementCount"] = [len(statements), count_statements(tree, grammar)]
        if statements is not None:
            row.update(measure_statements(text, statements, grammar, dialect))
        result["grammars"][grammar] = row
    return result


def measure_statements(text: str, statements: list[reference.Statement], grammar: str, dialect: str) -> dict:
    accepted = [statement for statement in statements if statement.accepted]
    errors: collections.Counter = collections.Counter()
    error_samples: dict[str, list] = collections.defaultdict(list)
    kinds = [0, 0]
    agreement = {"relations": [0, 0], "columns": [0, 0], "constants": [0, 0]}
    misses: dict[str, list] = collections.defaultdict(list)
    for statement in accepted:
        body = statement.text(text)
        data = body.encode("utf-8") + b";"
        tree = parser(grammar).parse(data)
        line = text.count("\n", 0, statement.start) + 1
        if tree.root_node.has_error:
            errors[statement.kind or "statement"] += 1
            if len(error_samples[statement.kind or "statement"]) < 3:
                error_samples[statement.kind or "statement"].append(line)
            continue
        if dialect != "postgres":
            continue
        kind = statement_kind(tree, grammar)
        if kind is not None:
            kinds[1] += 1
            kinds[0] += kind == statement.kind
        index = starts(tree.root_node)
        reading = reference.pg_names_values(statement.tree)
        for category in ("relations", "columns"):
            for item in reading[category]:
                if item["location"] < 0:
                    continue
                agreement[category][1] += 1
                if any(reference.identifier_parts(data[node.start_byte:node.end_byte].decode("utf-8", "replace"))
                       == item["parts"] for node in index.get(item["location"], [])):
                    agreement[category][0] += 1
                elif len(misses[category]) < SAMPLES:
                    misses[category].append({"line": line, "expected": item["parts"]})
        for item in reading["constants"]:
            if item["location"] < 0 or item["value"][0] == "unknown":
                continue
            if not LITERAL_TOKEN.match(data, item["location"]) or (
                    item["value"][0] == "string" and not STRING_TOKEN.match(data, item["location"])):
                continue  # a constant the server builds from an identifier or keyword (`SET x = on`), not a literal
            agreement["constants"][1] += 1
            candidates = index.get(item["location"], [])
            if any(reference.literal_value(data[node.start_byte:node.end_byte].decode("utf-8", "replace"))
                   == item["value"] for node in candidates):
                agreement["constants"][0] += 1
            elif len(misses["constants"]) < SAMPLES:
                misses["constants"].append({"line": line, "expected": item["value"],
                                            "source": data[item["location"]:item["location"] + 40].decode(
                                                "utf-8", "replace"),
                                            "nodes": [node.type for node in candidates]})
    row = {"acceptedStatements": len(accepted), "statementErrors": sum(errors.values()),
           "statementErrorsByKind": dict(errors), "statementErrorSamples": dict(error_samples)}
    if dialect == "postgres":
        row["kindAgreement"] = kinds
        row["agreement"] = agreement
        row["agreementMisses"] = dict(misses)
    return row


def sqlfluff_file(job: tuple[str, str, str]) -> dict:
    corpus, relative, dialect = job
    from sqlfluff.core import FluffConfig, Linter

    path = common.corpus_dir(corpus) / relative
    raw = path.read_bytes()
    if len(raw) > SQLFLUFF_MAX_BYTES:
        return {"corpus": corpus, "path": relative, "skipped": f"larger than {SQLFLUFF_MAX_BYTES} bytes"}
    text = raw.decode("utf-8", "replace")
    if dialect == "postgres":
        text = reference.clean_psql(text)[0]
    elif dialect == "sqlite":
        text = reference.clean_sqlite(text)[0]
    config = FluffConfig(overrides={"dialect": dialect, "templater": "raw", "large_file_skip_byte_limit": 0})
    begin = time.perf_counter()
    parsed = Linter(config=config).parse_string(text)
    seconds = time.perf_counter() - begin
    variant = parsed.root_variant()
    if variant is None or variant.tree is None:  # SQLFluff could not lex or parse the file at all
        unparsable = [0]
    else:
        unparsable = [segment.pos_marker.source_slice.start for segment in variant.tree.recursive_crawl("unparsable")]
    return {"corpus": corpus, "path": relative, "seconds": seconds, "bytes": len(raw),
            "unparsable": [text.count("\n", 0, start) + 1 for start in unparsable],
            "violations": len(parsed.violations), "unparsableChars": unparsable}


def mysql_reference(port: int, units: list[tuple[str, str, str]]) -> dict[str, dict]:
    """Mattermost's MySQL migrations through MySQL 8.4: ups in order, then downs in reverse, each file one
    multi-statement query. 1064 (ER_PARSE_ERROR) rejects a file; another error leaves the rest of it undetermined."""
    import pymysql
    from pymysql.constants import CLIENT

    connection = pymysql.connect(host="127.0.0.1", port=port, user="root",
                                 password=os.environ.get("MYSQL_ROOT_PASSWORD", "t"),
                                 client_flag=CLIENT.MULTI_STATEMENTS, autocommit=True)
    with connection.cursor() as cursor:
        cursor.execute("SELECT VERSION()")
        server = cursor.fetchone()[0]
        if not server.startswith(common.TOOLS["mysql"]):
            raise SystemExit(f"MySQL {server} != {common.TOOLS['mysql']}")
        cursor.execute("DROP DATABASE IF EXISTS mm; CREATE DATABASE mm")
        while cursor.nextset():
            pass
    connection.select_db("mm")
    ups = sorted(path for corpus, path, dialect in units if dialect == "mysql" and corpus == "mattermost"
                 and path.endswith(".up.sql"))
    downs = sorted((path for corpus, path, dialect in units if dialect == "mysql" and corpus == "mattermost"
                    and path.endswith(".down.sql")), reverse=True)
    verdicts = {}
    for relative in ups + downs:
        text = (common.corpus_dir("mattermost") / relative).read_text()
        if not text.strip():
            verdicts[relative] = {"verdict": "empty"}
            continue
        try:
            with connection.cursor() as cursor:
                cursor.execute(text)
                while cursor.nextset():
                    pass
            verdicts[relative] = {"verdict": "accepted"}
        except pymysql.err.MySQLError as error:
            code = error.args[0]
            verdicts[relative] = {"verdict": "rejected" if code == 1064 else "undetermined", "code": code,
                                  "message": str(error.args[1])[:200]}
            connection.ping(reconnect=True)
            connection.select_db("mm")
    connection.close()
    return {"server": server, "files": verdicts}


def manifest_sha(rows: list[dict], corpus: str) -> str:
    digest = hashlib.sha256()
    for row in sorted((row for row in rows if row["corpus"] == corpus), key=lambda row: row["path"]):
        data = (common.corpus_dir(corpus) / row["path"]).read_bytes()
        digest.update(row["path"].encode() + b"\0" + hashlib.sha256(data).hexdigest().encode() + b"\n")
    return digest.hexdigest()


def ratio(numerator: float, denominator: float) -> float | None:
    return round(numerator / denominator, 4) if denominator else None


def summarize(rows: list[dict], fluff: dict, mysql: dict | None) -> dict:
    summary: dict = {}
    keys = sorted({(row["corpus"], row["dialect"]) for row in rows})
    for corpus, dialect in keys:
        group = [row for row in rows if row["corpus"] == corpus and row["dialect"] == dialect]
        entry: dict = {"files": len(group), "bytes": sum(row["bytes"] for row in group)}
        if dialect == "mysql" and corpus == "mattermost" and mysql:
            verdicts = mysql["files"]
            accepted = {path for path, verdict in verdicts.items() if verdict["verdict"] == "accepted"}
            entry["reference"] = {"tool": "mysql", "version": mysql["server"], "unit": "file",
                                  "verdicts": dict(collections.Counter(v["verdict"] for v in verdicts.values())),
                                  "undetermined": {path: v for path, v in verdicts.items()
                                                   if v["verdict"] != "accepted"}}
        elif dialect == "mysql":
            accepted = None
            entry["reference"] = {"tool": "none", "note": "no MySQL server verdict for SQLFluff's MySQL fixtures; "
                                  "only SQLFluff's own parse is reported"}
        else:
            accepted = {row["path"] for row in group if row.get("referenceAccepts")}
            entry["reference"] = {
                "tool": "libpg_query (PostgreSQL 18.6)" if dialect == "postgres" else "sqlite3 3.50.4",
                "unit": "statement",
                "statements": sum(row.get("statements", 0) for row in group),
                "acceptedStatements": sum(row.get("acceptedStatements", 0) for row in group),
                "acceptedFiles": len(accepted),
                "scannerFailures": [row["path"] for row in group if "referenceError" in row],
                "blankedLines": dict(sum((collections.Counter(row.get("blankedLines", {})) for row in group),
                                         collections.Counter())),
            }
            seconds = sum(row.get("referenceSeconds", 0) for row in group)
            accepted_bytes = sum(row["bytes"] for row in group if row["path"] in accepted)
            if dialect == "postgres" and seconds:
                entry["reference"]["throughputMiBps"] = round(accepted_bytes / seconds / 2**20, 2)
        entry["grammars"] = {}
        for grammar in grammars_for(dialect):
            cells = [row["grammars"][grammar] for row in group]
            error_files = [row["path"] for row in group if row["grammars"][grammar]["fileError"]]
            seconds = sum(cell["seconds"] for cell in cells)
            grammar_entry: dict = {
                "errorFiles": len(error_files),
                "errorFileRate": ratio(len(error_files), len(group)),
                "throughputMiBps": round(entry["bytes"] / seconds / 2**20, 2) if seconds else None,
            }
            if accepted is not None:
                accepted_errors = [path for path in error_files if path in accepted]
                grammar_entry["referenceAcceptedFiles"] = len(accepted)
                grammar_entry["errorFilesAmongAccepted"] = len(accepted_errors)
                grammar_entry["errorFileRateAmongAccepted"] = ratio(len(accepted_errors), len(accepted))
                grammar_entry["errorFilesAmongAcceptedSample"] = sorted(accepted_errors)[:SAMPLES]
                accepted_seconds = sum(row["grammars"][grammar]["seconds"] for row in group if row["path"] in accepted)
                accepted_bytes = sum(row["bytes"] for row in group if row["path"] in accepted)
                if accepted_seconds:
                    grammar_entry["throughputOnAcceptedMiBps"] = round(accepted_bytes / accepted_seconds / 2**20, 2)
            if dialect in ("postgres", "sqlite"):
                total = sum(cell.get("acceptedStatements", 0) for cell in cells)
                failed = sum(cell.get("statementErrors", 0) for cell in cells)
                by_kind: collections.Counter = collections.Counter()
                samples: dict[str, list] = {}
                for row in group:
                    cell = row["grammars"][grammar]
                    by_kind.update(cell.get("statementErrorsByKind", {}))
                    for kind, lines in cell.get("statementErrorSamples", {}).items():
                        samples.setdefault(kind, [])
                        if len(samples[kind]) < 3:
                            samples[kind].append(f"{row['path']}:{lines[0]}")
                grammar_entry["acceptedStatements"] = total
                grammar_entry["statementErrors"] = failed
                grammar_entry["statementErrorRate"] = ratio(failed, total)
                grammar_entry["statementErrorsByKind"] = [
                    {"kind": kind, "errors": count, "samples": samples.get(kind, [])}
                    for kind, count in by_kind.most_common(25)]
                counts = [cell["statementCount"] for cell in cells if "statementCount" in cell]
                grammar_entry["statementCountAgreement"] = {
                    "files": len(counts), "equal": sum(1 for ref, got in counts if ref == got)}
            if dialect == "postgres":
                agreement = {key: [0, 0] for key in ("relations", "columns", "constants")}
                kinds = [0, 0]
                misses: dict[str, list] = collections.defaultdict(list)
                for row in group:
                    cell = row["grammars"][grammar]
                    if "agreement" not in cell:  # the server's scanner failed on the file: no statements to compare
                        continue
                    for key in agreement:
                        agreement[key][0] += cell["agreement"][key][0]
                        agreement[key][1] += cell["agreement"][key][1]
                    kinds[0] += cell["kindAgreement"][0]
                    kinds[1] += cell["kindAgreement"][1]
                    for key, items in cell["agreementMisses"].items():
                        for item in items:
                            if len(misses[key]) < SAMPLES:
                                misses[key].append({"path": row["path"], **item})
                grammar_entry["structuralAgreement"] = {
                    "scope": "statements the server accepts and the grammar parses without error",
                    **{key: {"matched": value[0], "total": value[1], "rate": ratio(*value)}
                       for key, value in agreement.items()},
                    "statementKind": {"matched": kinds[0], "total": kinds[1], "rate": ratio(*kinds)} if kinds[1]
                    else None,
                    "missSamples": dict(misses),
                }
            entry["grammars"][grammar] = grammar_entry
        fluff_rows = [fluff[(corpus, row["path"])] for row in group if (corpus, row["path"]) in fluff]
        parsed = [cell for cell in fluff_rows if "skipped" not in cell]
        unparsable_files = [cell["path"] for cell in parsed if cell["unparsable"]]
        fluff_seconds = sum(cell["seconds"] for cell in parsed)
        fluff_entry: dict = {
            "files": len(parsed), "skipped": len(fluff_rows) - len(parsed),
            "unparsableFiles": len(unparsable_files),
            "unparsableFileRate": ratio(len(unparsable_files), len(parsed)),
            "throughputMiBps": round(sum(cell["bytes"] for cell in parsed) / fluff_seconds / 2**20, 3)
            if fluff_seconds else None,
        }
        if accepted is not None:
            among = [path for path in unparsable_files if path in accepted]
            considered = [cell for cell in parsed if cell["path"] in accepted]
            fluff_entry["unparsableAmongAccepted"] = len(among)
            fluff_entry["referenceAcceptedFiles"] = len(considered)
            fluff_entry["unparsableRateAmongAccepted"] = ratio(len(among), len(considered))
            fluff_entry["unparsableAmongAcceptedSample"] = sorted(among)[:SAMPLES]
        entry["sqlfluff"] = fluff_entry
        summary[f"{corpus}/{dialect}"] = entry
    return summary


def maintenance() -> dict:
    status = {}
    for name, grammar in common.GRAMMARS.items():
        slug = grammar["repository"].removeprefix("https://github.com/")
        repo = json.loads(subprocess.run(["gh", "api", f"repos/{slug}"], capture_output=True, text=True,
                                         check=True).stdout)
        releases = json.loads(subprocess.run(["gh", "api", f"repos/{slug}/releases?per_page=1"],
                                             capture_output=True, text=True, check=True).stdout)
        issues = json.loads(subprocess.run(["gh", "api", f"search/issues?q=repo:{slug}+type:issue+state:open"],
                                           capture_output=True, text=True, check=True).stdout)
        pulls = json.loads(subprocess.run(["gh", "api", f"search/issues?q=repo:{slug}+type:pr+state:open"],
                                          capture_output=True, text=True, check=True).stdout)
        status[name] = {
            "repository": grammar["repository"], "commit": grammar["commit"], "version": grammar["version"],
            "license": grammar["license"], "crate": grammar["crate"], "archived": repo["archived"],
            "stars": repo["stargazers_count"], "lastPush": repo["pushed_at"],
            "lastRelease": {"tag": releases[0]["tag_name"], "date": releases[0]["published_at"]} if releases else None,
            "openIssues": issues["total_count"], "openPullRequests": pulls["total_count"],
        }
    return status


def main() -> None:
    arguments = argparse.ArgumentParser()
    arguments.add_argument("--out", required=True)
    arguments.add_argument("--mysql-port", type=int)
    arguments.add_argument("--jobs", type=int, default=10)
    arguments.add_argument("--corpus", action="append")
    options = arguments.parse_args()
    common.check_pins()
    names = options.corpus or list(common.CORPORA)
    jobs = [(name, path, dialect) for name in names for path, dialect in common.corpus_units(name)]
    print(f"{len(jobs)} files", file=sys.stderr)
    with concurrent.futures.ProcessPoolExecutor(options.jobs) as pool:
        rows = list(pool.map(measure_file, jobs, chunksize=4))
        print("grammars measured", file=sys.stderr)
        fluff = {(cell["corpus"], cell["path"]): cell for cell in pool.map(sqlfluff_file, jobs, chunksize=1)}
        print("sqlfluff measured", file=sys.stderr)
    mysql = mysql_reference(options.mysql_port, jobs) if options.mysql_port else None
    report = {
        "format": "rerail.language-conformance/v1",
        "pack": "sql",
        "tier": "t1-grammar",
        "date": datetime.date.today().isoformat(),
        "ticket": "NER-1698",
        "kernel": {"path": "none", "version": "pre-engine: no SQL family in the kernel yet; candidate grammars "
                   "measured through py-tree-sitter 0.26.0 (tree-sitter CLI 0.27.0 builds, ABI 15)"},
        "corpora": [{"name": name, "repository": common.CORPORA[name]["repository"],
                     "commit": common.CORPORA[name]["commit"], "tag": common.CORPORA[name]["tag"],
                     "units": [{"glob": glob, "dialect": dialect} for glob, dialect in common.CORPORA[name]["units"]],
                     "manifestSha256": manifest_sha(rows, name)} for name in names],
        "tools": [{"name": name, "version": version} for name, version in common.TOOLS.items()]
        + [{"name": f"tree-sitter-grammar:{name}", "version": f"{grammar['version']} ({grammar['commit']})"}
           for name, grammar in common.GRAMMARS.items()],
        "benchmark": "SQL candidate-grammar selection (pre-engine T1): grammar vs reference parser per dialect",
        "generator": "rerail-fixtures/sql/t1/measure_grammars.py",
        "method": __doc__.strip(),
        "maintenance": maintenance(),
        "results": summarize(rows, fluff, mysql),
    }
    pathlib.Path(options.out).write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n")
    detail = pathlib.Path(options.out).with_suffix(".files.json")
    detail.write_text(json.dumps({"files": rows, "sqlfluff": list(fluff.values()), "mysql": mysql}, indent=None,
                                 ensure_ascii=False))
    print(f"wrote {options.out} and {detail}", file=sys.stderr)


if __name__ == "__main__":
    main()
