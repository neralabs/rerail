"""Rebuild frozen generated SQL scopes with fewer physical candidate cuts."""
import argparse
import json
from pathlib import Path
import re
import shutil
import sqlite3

TOKEN = re.compile(r"\b[A-Za-z_][A-Za-z_0-9]*\b")
HEADER = re.compile(r"(\w+)\(([^)]*)\)\s+AS\s+(?:NOT\s+)?MATERIALIZED\s*\(", re.I)


def covered_input(capture, output):
    """Match the frozen guard's completed structural-preparation entry point."""
    data = json.loads(capture.read_text())
    source = capture.parent / data['database']
    db = sqlite3.connect(source)
    coverage = db.execute('SELECT f.fid,f.nodes,count(p.pre),max(d.fid IS NOT NULL) FROM file f LEFT JOIN decoded_properties p ON p.fid=f.fid LEFT JOIN decoded_files d ON d.fid=f.fid GROUP BY f.fid').fetchall()
    db.close()
    if not coverage or any(nodes != count or not complete for _, nodes, count, complete in coverage):
        raise ValueError('This entry point requires complete per-file structural coverage')
    skipped = {i for i, stage in enumerate(data['stages']) if stage['name'] in ['decode_all_nodes', 'mark_decoded_files']}
    data['schedule'] = [action for action in data['schedule'] if action.get('Prepare') not in skipped]
    data['entryPoint'] = 'Captured guard after full structural preparation. Omit the covering writes that production skips through its covered argument.'
    output.parent.mkdir(parents=True)
    shutil.copyfile(source, output.parent / data['database'])
    output.write_text(json.dumps(data, indent=2) + '\n')


def split_query(sql):
    """Read generated CTE boundaries, respecting SQL quotes and parentheses."""
    prefix = re.match(r"\s*WITH\s+(?:RECURSIVE\s+)?", sql, re.I)
    if not prefix:
        return [], sql
    offset = prefix.end()
    definitions = []
    while True:
        match = HEADER.match(sql, offset)
        if not match:
            raise ValueError("unexpected generated CTE header: " + sql[offset:offset + 100])
        depth, quote, pos = 1, None, match.end()
        while depth:
            char = sql[pos]
            if quote:
                if char == quote:
                    if pos + 1 < len(sql) and sql[pos + 1] == quote:
                        pos += 1
                    else:
                        quote = None
            elif char in "'\"":
                quote = char
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            pos += 1
        definitions.append(sql[offset:pos])
        offset = pos
        while offset < len(sql) and sql[offset].isspace():
            offset += 1
        if offset >= len(sql) or sql[offset] != ",":
            return definitions, sql[offset:]
        offset += 1
        while sql[offset].isspace():
            offset += 1


def fields(definition):
    match = HEADER.match(definition)
    if not match:
        raise ValueError(definition[:80])
    return match[1], match[2], definition[match.end():-1]


def rewrite(capture, output, private):
    data = json.loads(capture.read_text())
    graph = {fields(sql)[0]: sql for sql in data["graph"]}
    stages = data["stages"]
    uses = {name: 0 for name in graph}
    for definition in graph.values():
        name, _, body = fields(definition)
        for token in TOKEN.findall(body):
            if token in uses and token != name:
                uses[token] += 1
    for query in data["queries"]:
        for token in TOKEN.findall(query):
            if token in uses:
                uses[token] += 1
    empty = set()
    simplified = set()
    binary = re.compile(r"SELECT fid,pre FROM (\w+) (UNION|INTERSECT|EXCEPT) SELECT fid,pre FROM (\w+)$")
    for name, definition in list(graph.items()):
        _, columns, body = fields(definition)
        if columns != "fid,pre":
            continue
        if re.search(r"\bWHERE 0$", body):
            empty.add(name)
        match = binary.fullmatch(body)
        if match:
            left, operation, right = match.groups()
            if (operation == "INTERSECT" and (left in empty or right in empty)) or (operation == "EXCEPT" and left in empty) or (left in empty and right in empty):
                body = "SELECT 0 AS fid,0 AS pre WHERE 0"
                empty.add(name)
                simplified.add(name)
            elif operation == "UNION" and (left in empty or right in empty):
                # UNION removes duplicates even if its nonempty input is a bag.
                body = "SELECT DISTINCT fid,pre FROM " + (right if left in empty else left)
                simplified.add(name)
            elif operation == "EXCEPT" and right in empty:
                body = "SELECT DISTINCT fid,pre FROM " + left
                simplified.add(name)
        graph[name] = f"{name}({columns}) AS MATERIALIZED ({body})"
    owned = {stage["name"]: i for i, stage in enumerate(stages) if stage.get("slot") is not None}
    removed = {name for name in owned if name in simplified or name in empty or (private and uses[name] <= 1)}
    physical = set(owned) - removed
    aliases = {name: f"{name}(fid,pre) AS NOT MATERIALIZED (SELECT fid,pre FROM candidate_keys WHERE producer={stages[owned[name]]['slot']})" for name in physical}

    def render(sql, members=()):
        local, body = split_query(sql)
        definitions = dict(graph)
        definitions.update({fields(cte)[0]: cte for cte in local})
        for name in removed:
            definitions[name] = graph[name]
        definitions.update(aliases)
        for name in members:
            definitions[name] = graph[name].replace(" AS MATERIALIZED ", " AS NOT MATERIALIZED ", 1)
        selected = set()

        def visit(name):
            if name in selected or name not in definitions:
                return
            selected.add(name)
            for dependency in TOKEN.findall(fields(definitions[name])[2]):
                if dependency != name:
                    visit(dependency)
        for token in TOKEN.findall(body):
            visit(token)
        # Original generated order is topological. Local missing-key CTEs follow it.
        ordered = [definition for name, definition in definitions.items() if name in selected]
        result = "WITH RECURSIVE " + ",".join(ordered) + " " + body if ordered else body
        if len(result) > 1_048_576:
            raise ValueError("rewritten SQL exceeds the engine's 1 MiB bound")
        return result

    skipped = set()
    for i, stage in enumerate(stages):
        if not stage["sql"]:
            continue
        if stage.get("slot") is not None:
            members = [member for member in stage["members"] if stages[member]["name"] in physical]
            stage["members"] = members
            if not members:
                skipped.add(i)
                continue
            arms = [f"SELECT {stages[m]['slot']} AS producer,fid,pre FROM {stages[m]['name']}" for m in members]
            body = "INSERT INTO candidate_keys SELECT producer,CAST(fid AS INTEGER),CAST(pre AS INTEGER) FROM (" + " UNION ALL ".join(arms) + ")"
            stage["sql"] = render(body, [stages[m]["name"] for m in members])
        else:
            stage["sql"] = render(stage["sql"])
    data["queries"] = [render(sql) for sql in data["queries"]]
    data["schedule"] = [action for action in data["schedule"] if not ("Prepare" in action and action["Prepare"] in skipped) and not ("Release" in action and stages[action["Release"]]["name"] in removed)]
    data["spike"] = {"privateCuts": private, "removedPhysicalBindings": sorted(removed), "remainingPhysicalBindings": len(physical), "simplifiedSetDefinitions": len(simplified), "scope": "Frozen compiler-generated SQL only. No data-layer detector algorithm or runtime flag."}
    output.parent.mkdir(parents=True)
    database = capture.parent / data["database"]
    shutil.copyfile(database, output.parent / data["database"])
    output.write_text(json.dumps(data, indent=2) + "\n")
    return data["spike"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--private", action="store_true", help="Standalone experiment: also inline single-use cuts")
    args = parser.parse_args()
    print(json.dumps(rewrite(args.capture, args.output, args.private), indent=2))
