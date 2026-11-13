#!/usr/bin/env python3
"""Generate isolated SQL experiments; never edit the shipped policy."""
import argparse
import copy
import json
from pathlib import Path
import re
import sqlite3


def sql_strings(value):
    if isinstance(value, str):
        yield value
    else:
        for child in value.get("cases", {}).values():
            yield from sql_strings(child)
        if "default" in value:
            yield from sql_strings(value["default"])


def map_sql(value, change):
    if isinstance(value, str):
        return change(value)
    return {key: ({k: map_sql(v, change) for k, v in child.items()} if key == "cases"
                  else map_sql(child, change) if key == "default" else copy.deepcopy(child)) for key, child in value.items()}


def wrap(query, ctes):
    return "WITH " + ",".join(ctes) + " SELECT * FROM (" + query + ")"


def closure_variant(plan, variant):
    stages = plan.get("stages", [])
    position = next((i for i, stage in enumerate(stages) if stage["name"] == "descendants" or
                     (variant == "union-all" and stage["name"] == "descendant_pairs")), None)
    if position is None:
        return
    stage = stages[position]
    if variant == "union-all":
        assert " UNION SELECT w." in stage["query"]
        stage["query"] = stage["query"].replace(" UNION SELECT w.", " UNION ALL SELECT w.")
        return
    if variant == "roots":
        # Conservative candidate roots from every consumer family, including
        # non-function roots. This changes recursive seeds, not traversal edges.
        fields = ["name", "left", "right", "value", "type", "function", "arguments", "object", "attribute", "body", "parameters", "return_type", "condition", "subject", "consequence", "alias", "module_name"]
        projected = {column for value in plan["inputs"].values() for column in value["evidence"]["project"]}
        fields = [f for f in fields if {f + "_kind", f + "_start", f + "_end"} <= projected]
        seeds = ["SELECT __path,id FROM nodes WHERE kind IN ('function_definition','class_definition','for_statement','for_in_clause','assignment','delete_statement','block','if_statement','elif_clause','match_statement')"]
        # loop_names traverses the comprehension enclosing a for_in_clause.
        # through_conditions traverses each unfielded comparison operand.
        seeds += ["SELECT __path,parent FROM nodes WHERE kind='for_in_clause' AND parent IS NOT NULL",
                  "SELECT child.__path,child.id FROM nodes child JOIN nodes cmp ON cmp.__path=child.__path AND cmp.id=child.parent WHERE cmp.kind='comparison_operator'"]
        seeds += [f"SELECT __path,{f}_kind||':'||{f}_start||':'||{f}_end FROM nodes WHERE {f}_start IS NOT NULL" for f in fields]
        if any(s["name"] == "arguments" for s in stages):
            seeds.append("SELECT __path,id FROM arguments")
        # The argument relation is an earlier producer in this policy.
        stage["query"] = stage["query"].replace("SELECT __path,id,id FROM nodes", "SELECT __path,id,id FROM (" + " UNION ".join(seeds) + ")")
        return
    compact = variant in ["compact-keys", "compact-no-path"]
    no_path = variant in ["no-path", "compact-no-path"]
    extra = []
    if compact:
        extra += [{"name": "node_keys", "query": "SELECT key,row_number() OVER(ORDER BY key) AS compact FROM (SELECT id AS key FROM nodes UNION SELECT src FROM edges UNION SELECT dst FROM edges)", "indexes": [["key"], ["compact"]]},
                  {"name": "compact_edges", "query": "SELECT a.compact AS src,b.compact AS dst FROM edges e JOIN node_keys a ON a.key=e.src JOIN node_keys b ON b.key=e.dst", "indexes": [["src", "dst"]]}]
    path_columns = "" if no_path else "__path,"
    path_seed = "" if no_path else "(SELECT __path FROM nodes LIMIT 1),"
    seed = "SELECT " + path_seed + ("compact,compact FROM node_keys WHERE EXISTS(SELECT 1 FROM nodes WHERE id=key)" if compact else "id,id FROM nodes")
    # Preserve a previously restricted seed set when experiments are composed.
    if "SELECT __path,id,id FROM nodes UNION SELECT" not in stage["query"]:
        root_query = stage["query"].split(" AS (", 1)[1].split(" UNION SELECT w.", 1)[0]
        seed = "SELECT " + path_seed + ("k.compact,k.compact FROM (" + root_query + ") s JOIN node_keys k ON k.key=s.id" if compact else "s.id,s.id FROM (" + root_query + ") s")
    edge = "compact_edges" if compact else "edges"
    step = f"SELECT {'' if no_path else 'w.__path,'}w.root,e.dst FROM walk w JOIN {edge} e ON e.src=w.node"
    query = f"WITH RECURSIVE walk({path_columns}root,node) AS ({seed} UNION {step}) SELECT * FROM walk"
    pairs = {"name": "descendant_pairs", "query": query,
             "indexes": [[*(["__path"] if not no_path else []), "root", "node"], [*(["__path"] if not no_path else []), "node", "root"]]}
    path = "(SELECT __path FROM nodes LIMIT 1)" if no_path else "p.__path"
    if compact:
        view = f"descendants AS NOT MATERIALIZED (SELECT {path} AS __path,a.key AS root,b.key AS node FROM descendant_pairs p JOIN node_keys a ON a.compact=p.root JOIN node_keys b ON b.compact=p.node)"
    else:
        view = f"descendants AS NOT MATERIALIZED (SELECT {path} AS __path,p.root,p.node FROM descendant_pairs p)"
    stages[position:position + 1] = [*extra, pairs]
    for consumer in stages[position + len(extra) + 1:]:
        consumer["query"] = map_sql(consumer["query"], lambda q: wrap(q, [view]) if re.search(r"\bdescendants\b", q) else q)
    plan["query"] = map_sql(plan["query"], lambda q: wrap(q, [view]) if re.search(r"\bdescendants\b", q) else q)


def narrow_selectors(plan):
    removed = 0
    for name, value in plan["inputs"].items():
        if not name.startswith("syntax_"):
            continue
        evidence = value["evidence"]
        used = set()
        for source in evidence["project"].values():
            used.update(re.findall(r"capture\.([A-Z_]+)", json.dumps(source)))
        def prune(rule):
            nonlocal removed
            if isinstance(rule, dict):
                if isinstance(rule.get("all"), list):
                    parts = []
                    for item in rule["all"]:
                        branches = item.get("any", []) if isinstance(item, dict) else []
                        if len(branches) == 2 and branches[1] == {"pattern": "$_"} and "has" in branches[0]:
                            capture = re.fullmatch(r"\$([A-Z_]+)", str(branches[0]["has"].get("pattern", "")))
                            if capture and capture[1] not in used:
                                removed += 1
                                continue
                        prune(item); parts.append(item)
                    rule["all"] = parts
                else:
                    for child in rule.values():
                        prune(child)
            elif isinstance(rule, list):
                for child in rule:
                    prune(child)
        prune(evidence["query"]["ast"]["rule"])
    return removed


def scope_stages(plan, reference):
    """Keep secondary consumers' shared prefix; gate primary-only producers in SQL."""
    secondary = set()
    groups = [c for c in reference["clauses"] if c["id"].startswith("quality-contracts-")]
    primary = [c for c in groups if "_check_compile_direct_io" in json.dumps(c["relational"])]
    if len(primary) != 1 or len(groups) != 3:
        raise ValueError("Expected one primary/fallback group and two secondary reference groups")
    for clause in groups:
        if clause is not primary[0]:
            secondary.update(stage["name"] for stage in clause["relational"]["stages"])
    stages = plan["stages"]
    files = next(stage for stage in stages if stage["name"] == "files")
    stages.remove(files)
    stages.insert(next(i for i, stage in enumerate(stages) if stage["name"] == "nodes") + 1, files)
    changed = []
    for stage in stages:
        name = stage["name"]
        if name in secondary or name.startswith(("syntax_rows_", "diagnostics_")):
            continue
        original = stage["query"]
        gated = map_sql(original, lambda query: "SELECT * FROM (" + query + ") WHERE EXISTS(SELECT 1 FROM files WHERE primary_file OR fallback_file)")
        stage["query"] = {"$option":"entryPoint", "cases":{"all":{
            "$option":"oracleAnalyzer", "cases":{"false":gated}, "default":copy.deepcopy(original)}},
            "default":original}
        changed.append(name)
    return changed


def narrow_projections(plan):
    import sqlglot
    from sqlglot.optimizer.qualify import qualify
    from sqlglot.optimizer.pushdown_projections import pushdown_projections
    db = sqlite3.connect(":memory:", cached_statements=0)
    for name, arity in [("rx_matches", 2), ("rx_extract", 3), ("rx_replace", 3)]:
        db.create_function(name, arity, lambda *args: None)
    schema, columns = {}, {}
    for name, value in plan["inputs"].items():
        cols = ["__path", "__start", "__end", *value["evidence"]["project"]]
        db.execute(f'CREATE TABLE "{name}" ({",".join(chr(34)+c+chr(34) for c in cols)})')
        columns[name] = cols; schema[name] = {c: "UNKNOWN" for c in cols}
    for stage in plan.get("stages", []):
        query = next(sql_strings(stage["query"]))
        db.execute(f'CREATE TEMP TABLE "{stage["name"]}" AS SELECT * FROM ({query}) LIMIT 0')
        cols = [row[1] for row in db.execute(f'PRAGMA table_info("{stage["name"]}")')]
        columns[stage["name"]] = cols; schema[stage["name"]] = {c: "UNKNOWN" for c in cols}
    reads = set()
    def authorizer(action, table, column, *_):
        if action == sqlite3.SQLITE_READ and table in schema:
            reads.add((table, column))
        return sqlite3.SQLITE_OK
    def collect(query):
        db.set_authorizer(authorizer)
        try: db.execute("SELECT * FROM (" + query + ") LIMIT 0")
        finally: db.set_authorizer(None)
    for q in sql_strings(plan["query"]): collect(q)
    changes = []
    skipped = set()
    for stage in reversed(plan.get("stages", [])):
        name = stage["name"]
        if name.startswith("syntax_rows_"):
            for query in sql_strings(stage["query"]): collect(query)
            continue
        wanted = {c for t,c in reads if t == name}
        wanted.update(c for index in stage.get("indexes", []) for c in index)
        if not wanted or "" in wanted:
            wanted = set(columns[name])
        keep = [c for c in columns[name] if c in wanted]
        def project(query):
            if len(keep) == len(columns[name]):
                collect(query); return query
            select = ','.join('"'+c+'"' for c in keep)
            wrapped = f'SELECT {select} FROM ({query}) AS _spike_projection'
            expression = qualify(sqlglot.parse_one(wrapped, read="sqlite"), dialect="sqlite", schema=schema, quote_identifiers=False, identify=False)
            optimized = pushdown_projections(expression).sql(dialect="sqlite")
            if len(optimized.encode()) > 16_384:
                skipped.add(name)
                collect(query)
                return query
            collect(optimized)
            return optimized
        stage["query"] = map_sql(stage["query"], project)
        if len(keep) < len(columns[name]): changes.append({"stage":name,"before":len(columns[name]),"after":len(columns[name]) if name in skipped else len(keep),"sqlSizeFallback":name in skipped})
    db.close()
    return changes


def variant(document, name, reference=None):
    output = copy.deepcopy(document); details = []
    for clause in output["clauses"]:
        plan = clause.get("relational")
        if not plan or plan.get("partitionBy") != "file" or not clause["id"].startswith("quality-contracts-"):
            continue
        if name in ["union-all", "roots", "compact-keys", "no-path", "compact-no-path"]: closure_variant(plan, name)
        elif name == "narrow-selectors": details.append({"clause":clause["id"],"removedOptionalFields":narrow_selectors(plan)})
        elif name == "narrow-projections": details.append({"clause":clause["id"],"projections":narrow_projections(plan)})
        elif name == "scope-stages":
            if reference is None: raise ValueError("scope-stages requires --scope-reference")
            details.append({"clause":clause["id"],"gatedPrimaryStages":scope_stages(plan,reference)})
        else: raise ValueError(name)
    return output, details


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--policy", type=Path, required=True); p.add_argument("--output", type=Path, required=True)
    p.add_argument("--scope-reference", type=Path, help="Original three-scope policy for the scope-stages experiment")
    p.add_argument("--variant", action="append", choices=["union-all","roots","no-path","compact-keys","compact-no-path","narrow-selectors","narrow-projections","scope-stages"], required=True)
    a=p.parse_args(); result=json.loads(a.policy.read_text()); details=[]
    for name in a.variant:
        result, changes=variant(result,name,json.loads(a.scope_reference.read_text()) if a.scope_reference else None); details.append({"variant":name,"details":changes})
    a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+"\n")
    a.output.with_suffix(".experiment.json").write_text(json.dumps({"variants":a.variant,"details":details},indent=2)+"\n")
