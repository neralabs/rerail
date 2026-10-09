"""T3 input for the SQL pack's `names` and `values` capabilities: PostgreSQL 18.6's reading (libpg_query) of every
relation, column reference and literal constant in the pinned PostgreSQL-dialect corpora. Run offline by a maintainer,
never in CI, with the pinned venv (../common.py):

    <checkout>/.rerail/tools/sql/venv/bin/python generate_corpus.py [<corpus> ...]   # default: every corpus with
                                                                                    # PostgreSQL files

Writes the gitignored `<checkout>/.rerail/t3/sql/<corpus>-names-values.json`, the reference side a future
`benchmark_sql_names_corpus` compares the pack against, site by site. Per file (dialect postgres by the corpus's path
globs; psql meta-commands and COPY data blanked in place, see reference.clean_psql) and per statement the server
accepts (each parsed alone, so the regression suite's deliberate errors do not hide the rest), every site is
{kind, line, column, value}: kind `relation` or `column` with the identifier parts the server stores, or `constant`
with the value (reference.pg_names_values). Constants the server builds from a keyword or identifier rather than a
literal token (`SET x = on`) are left out, as in T1. Lines are 1-based and columns count Unicode code points from
1. Rejected statements are listed per file with their line, so a scorer never counts sites inside them.
"""

from __future__ import annotations

import concurrent.futures
import json
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402
import reference  # noqa: E402

OUT = common.CHECKOUT / ".rerail/t3/sql"
LITERAL_TOKEN = re.compile(rb"[-+]?\s*[0-9.]|'|[eEbBxXnN]'|[uU]&'|\$|(?i:true|false|null)\b")
STRING_TOKEN = re.compile(rb"'|[eEnN]'|[uU]&'|\$")


def file_sites(job: tuple[str, str]) -> dict:
    corpus, relative = job
    raw = (common.corpus_dir(corpus) / relative).read_bytes()
    text = reference.clean_psql(raw.decode("utf-8", "replace"))[0]
    data = text.encode("utf-8")
    try:
        statements = reference.postgres_statements(text)
    except reference.pg.ParseError as error:  # the server's scanner rejects a token (`""`, `123abc`, a bad escape)
        return {"path": relative, "scannerError": str(error)[:200], "sites": [], "rejected": []}
    sites, rejected = [], []
    for statement in statements:
        start = len(text[:statement.start].encode("utf-8"))
        if not statement.accepted:
            rejected.append(data.count(b"\n", 0, start) + 1)
            continue
        body = data[start:start + len(statement.text(text).encode("utf-8"))]
        reading = reference.pg_names_values(statement.tree)
        for category, kind in (("relations", "relation"), ("columns", "column"), ("constants", "constant")):
            for item in reading[category]:
                location = item["location"]
                if location < 0:
                    continue
                if kind == "constant" and (
                        item["value"][0] == "unknown" or not LITERAL_TOKEN.match(body, location)
                        or (item["value"][0] == "string" and not STRING_TOKEN.match(body, location))):
                    continue  # a constant the server builds from a keyword or identifier, not a literal
                offset = start + location
                line_start = data.rfind(b"\n", 0, offset) + 1
                sites.append({"kind": kind, "line": data.count(b"\n", 0, offset) + 1,
                              "column": len(data[line_start:offset].decode("utf-8", "replace")) + 1,
                              "value": item["parts"] if kind != "constant" else item["value"]})
    sites.sort(key=lambda site: (site["line"], site["column"], site["kind"]))
    return {"path": relative, "sites": sites, "rejected": rejected}


def main() -> None:
    common.check_pins(need_grammars=False)
    names = sys.argv[1:] or [name for name in common.CORPORA
                             if any(dialect == "postgres" for _, dialect in common.CORPORA[name]["units"])]
    OUT.mkdir(parents=True, exist_ok=True)
    for name in names:
        jobs = [(name, path) for path, dialect in common.corpus_units(name) if dialect == "postgres"]
        with concurrent.futures.ProcessPoolExecutor(8) as pool:
            files = list(pool.map(file_sites, jobs, chunksize=2))
        declared = common.CORPORA[name]
        counts = {kind: sum(1 for row in files for site in row["sites"] if site["kind"] == kind)
                  for kind in ("relation", "column", "constant")}
        report = {"tool": "libpg_query", "version": f"pglast {common.TOOLS['pglast']}, PostgreSQL 18.6",
                  "corpus": {"name": name, "repository": declared["repository"], "commit": declared["commit"]},
                  "generator": "fixtures/sql/t3/generate_corpus.py", "sites": counts, "files": files}
        path = OUT / f"{name}-names-values.json"
        path.write_text(json.dumps(report, ensure_ascii=False))
        print(f"{name}: {len(files)} files, {counts}, rejected statements "
              f"{sum(len(row['rejected']) for row in files)} -> {path}", file=sys.stderr)


if __name__ == "__main__":
    main()
