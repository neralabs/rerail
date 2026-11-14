"""Isolated policy experiments. No alternative production execution path."""
import argparse
import copy
import json
from pathlib import Path
import re


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        if isinstance(value.get("template"), str):
            yield value["template"]
        for key, child in value.items():
            if key in ("cases", "default"):
                if key == "cases":
                    for branch in child.values():
                        yield from strings(branch)
                else:
                    yield from strings(child)


def map_sql(value, function):
    if isinstance(value, str):
        return function(value)
    result = copy.deepcopy(value)
    if isinstance(result.get("template"), str):
        result["template"] = function(result["template"])
    if "cases" in result:
        result["cases"] = {key: map_sql(child, function) for key, child in result["cases"].items()}
    if "default" in result:
        result["default"] = map_sql(result["default"], function)
    return result


def wrap(query, name, source):
    prefix = f'{name} AS MATERIALIZED ({source})'
    match = re.match(r"\s*WITH\s+(RECURSIVE\s+)?", query, re.I)
    if match:
        return "WITH RECURSIVE " + prefix + "," + query[match.end():]
    return "WITH RECURSIVE " + prefix + " " + query


def private_stages(plan):
    """Fuse nonrecursive producers with exactly one authored SQL consumer.

    The experiment retains multiply consumed producers and native boundaries.
    Declared indexes on the removed private relation give way to SQLite's CTE
    planning. Corpus parity and measured CPU/memory determine the decision.
    """
    removed = []
    while True:
        stages = plan.get("stages", [])
        changed = False
        for stage in stages:
            source = stage.get("query")
            if not isinstance(source, str) or re.search(r"\bRECURSIVE\b", source, re.I):
                continue
            name = stage["name"]
            pattern = re.compile(r"\b" + re.escape(name) + r"\b")
            consumers = []
            for consumer in [*stages, plan]:
                if consumer is stage:
                    continue
                for key in ["query"]:
                    value = consumer.get(key)
                    if value is not None and any(pattern.search(sql) for sql in strings(value)):
                        consumers.append((consumer, key))
                reach = consumer.get("reachability")
                if reach and any(pattern.search(sql) for value in reach.values() for sql in strings(value)):
                    # A native demand is a retained execution boundary.
                    consumers.append((reach, None))
            if len(consumers) != 1 or consumers[0][1] is None:
                continue
            consumer, key = consumers[0]
            if any("{}" in sql for sql in strings(consumer[key])) and "{}" in source:
                continue
            consumer[key] = map_sql(consumer[key], lambda query: wrap(query, name, source))
            stages.remove(stage)
            removed.append({"name": name, "indexes": stage.get("indexes", [])})
            changed = True
            break
        if not changed:
            return removed


def generate(source, output):
    output.mkdir(parents=True)
    receipts = {}
    for path in sorted(source.glob("*.json")):
        policy = json.loads(path.read_text())
        changed = []
        if policy["id"] == "data-layer-serverplan-state-tripwires":
            for clause in policy["clauses"]:
                plan = clause.get("relational", {})
                if list(plan.get("inputs", {})) == ["events"]:
                    plan["requireNonemptyInput"] = "events"
                if clause["id"] == "state-flow-1":
                    changed = private_stages(plan)
        (output / path.name).write_text(json.dumps(policy, indent=2) + "\n")
        if changed:
            receipts[path.name] = changed
    (output.parent / "policy-rewrite.json").write_text(json.dumps(receipts, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path("packs/data-layer/candidates/rules"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    generate(args.source, args.output)
