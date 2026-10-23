#!/usr/bin/env python3
"""One-off (kept for the record): split the batch-5 symbol-table rules (`unreachable-internal-symbols`,
`exported-families`, S3's w3 narrowed texts in `original/`) into the pack's declared-facts layer and
two short rules. Per-content facts are the shared matcher inputs (`define.match`), renamed `py_*`;
membership facts are the shared stages (`define.sql`), renamed `proj_*`; stages that only copied an
input are dropped and their readers read the fact. Writes `pack/facts.symbols.json` and
`pack/rules/{unreachable-internal-symbols,exported-families}.json`; `link.py` binds them."""
import json
import re
from pathlib import Path

HERE = Path(__file__).parent
DC = json.loads((HERE / "original/data-layer-internal-dead-code.json").read_text())
EF = json.loads((HERE / "original/data-layer-public-api-triples.json").read_text())

INPUTS = {  # original input -> per-content fact
    "raw_functions": "py_functions", "raw_classes": "py_classes", "raw_decorators": "py_decorators",
    "raw_parameters": "py_parameters", "raw_parameter_annotations": "py_parameter_annotations",
    "raw_return_annotations": "py_return_annotations", "raw_imports": "py_imports", "raw_bases": "py_bases",
    "raw_all": "py_all", "raw_all_names": "py_all_names", "raw_modules": "py_modules",
}
SHARED = ["inventory", "declarations", "parameters", "annotation_names", "imports", "resolved_names", "base_edges",
          "hierarchy", "public_methods", "explicit_exports", "export_names", "exported_roots"]
IDENTITY = {"decorators": "py_decorators", "all_names": "py_all_names", "bases": "py_bases", "function_headers": "py_functions"}
TABLES = {**INPUTS, **{s: "proj_" + s for s in SHARED}, **IDENTITY}


def rename(sql):
    if isinstance(sql, dict):  # {"$option": .., "cases": {..}}
        return {**sql, "cases": {k: rename(v) for k, v in sql["cases"].items()}}
    # table positions only: after FROM / JOIN (CTE names and columns never follow them here)
    return re.sub(r"\b(FROM|JOIN)(\s+)([a-z_]+)\b", lambda m: m[1] + m[2] + TABLES.get(m[3], m[3]), sql)


def clause(doc, cid):
    return next(c for c in doc["clauses"] if c["id"] == cid)


dc, ef = clause(DC, "unreachable-internal-symbols")["relational"], clause(EF, "exported-families")["relational"]
facts = {}
for raw, fact in INPUTS.items():
    ev = (dc["inputs"].get(raw) or ef["inputs"][raw])["evidence"]
    if raw in ef["inputs"]:
        assert ef["inputs"][raw]["evidence"] == ev, raw
    facts[fact] = {"over": "content", "define": {"match": ev}}
stages = {s["name"]: s for s in dc["stages"]}
for s in SHARED:
    assert json.dumps(stages[s], sort_keys=True) == json.dumps(next(x for x in ef["stages"] if x["name"] == s), sort_keys=True), s
    facts["proj_" + s] = {"over": "membership", "define": {"sql": rename(stages[s]["query"])}, "indexes": stages[s].get("indexes", [])}
define = clause(EF, "shared")["define"]
assert all(clause(DC, "shared")["define"]["utils"][k] == define["utils"][k] for k in clause(DC, "shared")["define"]["utils"])
pack = {"scope": EF["scope"], "define": define, "facts": facts}
out = HERE / "pack"
(out / "rules").mkdir(parents=True, exist_ok=True)
(out / "facts.symbols.json").write_text(json.dumps(pack, indent=1) + "\n")


def rule(doc, cid, rel, private_inputs):
    own = [s for s in rel["stages"] if s["name"] not in SHARED and s["name"] not in IDENTITY]
    r = {"id": cid, "violation": clause(doc, cid)["violation"],
         "inputs": {k: rel["inputs"][k] for k in private_inputs},
         "stages": [{**s, "query": rename(s["query"])} for s in own],
         **{k: (rename(v) if k == "query" else v) for k, v in rel.items() if k not in ("inputs", "stages")}}
    (out / "rules" / f"{cid}.json").write_text(json.dumps(r, indent=1) + "\n")


rule(EF, "exported-families", ef, ["raw_execution_calls"])
rule(DC, "unreachable-internal-symbols", dc, ["raw_module_names", "raw_dynamic_imports"])
r = json.loads((out / "rules/exported-families.json").read_text())
r["stages"] = [s for s in r["stages"] if s["name"] != "execution_calls"]
for s in r["stages"]:
    s["query"] = re.sub(r"\bFROM execution_calls\b", "FROM raw_execution_calls", s["query"]) if isinstance(s["query"], str) else s["query"]
(out / "rules/exported-families.json").write_text(json.dumps(r, indent=1) + "\n")
