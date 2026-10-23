#!/usr/bin/env python3
"""One-off (kept for the record): the two dump-built rules (`state-flow-project-1`,
`quality-conditional-1`, S3's w3 narrowed texts in `original/`) rewritten over one gate-free node
fact and one selection fact per rule.

- `py_nodes` (per content, gate-free): every node the dumps can select (`$NODE` over every kind but
  the six leaf kinds, valid syntax), its `PARENT`, and per field of the union the two rules project
  the first child under that field (start, end, kind; source text for the six fields a rule reads as
  text). Text and line as the dumps project them. It is the union of the 25 / 20 dump chunks with
  the gate removed and the field projections widened to the union.
- `sel_sfp` (per content, rule-specific): the module of a content where state-flow-project-1's gate
  holds. Every conjunct of that gate is `matches U or inside U` with every alternative of `U` a
  `module`, or an option/filePath test, so the gate holds at a node iff it holds at the module: the
  selection is the whole content or nothing.
- `sel_qc` (per content, rule-specific): the nodes where `quality-conditional-1-selection`'s gate
  holds, with `ANCESTOR` (nearest proper ancestor where the selection holds). Not closed under
  descendants, so the rule recomputes `ordinal`, `edges`, `ownership` over its own subset (its stages).

Each rule's `syntax_rows` is the dump union recovered from the facts: one `SELECT` per projection
group of the original chunks (fields a chunk did not project are `NULL`; qc's metadata chunk keeps
its thin text `''`). Every other stage is the rule's own text over its own subset (the S4 views are
not declared facts in this pack; see the report). Writes `pack/facts.nodes.json` and
`pack/rules/{state-flow-project-1,quality-conditional-1}.json`; `link.py` binds them."""
import copy
import json
import re
from pathlib import Path

HERE = Path(__file__).parent
ST = json.loads((HERE / "original/data-layer-serverplan-state-tripwires.json").read_text())
QU = json.loads((HERE / "original/data-layer-serverplan-quality.json").read_text())
FIELDS = ('name', 'left', 'right', 'value', 'type', 'function', 'arguments', 'object', 'attribute', 'body', 'parameters',
          'return_type', 'condition', 'subject', 'consequence', 'alias', 'module_name')
LEAF_KINDS = ['comment', 'string_start', 'string_content', 'string_end', 'escape_sequence', 'line_continuation']
LEAVES = {"not": {"any": [{"kind": k} for k in LEAF_KINDS]}}
NODE = {"pattern": "$NODE"}
PARENT = {"any": [{"inside": {"pattern": "$PARENT", "stopBy": "neighbor"}}, {"pattern": "$_"}]}


def field_capture(f):
    return {"any": [{"has": {"pattern": "$" + f.upper(), "stopBy": "neighbor", "field": f}}, {"pattern": "$_"}]}


def clause(doc, cid):
    return next(c for c in doc["clauses"] if c["id"] == cid)


def chunks(rel):
    """Per dump input `syntax_k`, in order: (own predicate, projection)."""
    out = []
    for k in range(len([n for n in rel["inputs"] if n.startswith("syntax_")])):
        ev = rel["inputs"][f"syntax_{k}"]["evidence"]
        rule = ev["query"]["ast"]["rule"]["all"]
        assert rule[1] == LEAVES and ev["query"]["ast"]["syntax"] == "valid" and ev["as"] == "row", k
        out.append((rule[0], rule[2], ev["project"]))
    return out


def kinds_of(pred):
    """The kinds a chunk's own predicate selects: a kind list, or None for the complement chunk."""
    if "not" in pred:
        return None
    s = json.dumps(pred)
    return sorted(set(re.findall(r'"kind": "([a-z_]+)"', s)))


def columns(rel):
    """The dump union's column order (the first `syntax_rows` SELECT list)."""
    first = rel["stages"][0]["query"].split(" FROM ")[0][len("SELECT "):]
    return [c.split(" AS ")[-1] for c in first.split(",")]


sfp = clause(ST, "state-flow-project-1")["relational"]
qc = clause(QU, "quality-conditional-1")["relational"]
sfp_chunks, qc_chunks = chunks(sfp), chunks(qc)
sfp_cols, qc_cols = columns(sfp), columns(qc)

# --- py_nodes: the union of the field captures and projections both rules use --------------------
used_fields, sourced = [], set()
for _, _, proj in sfp_chunks + qc_chunks:
    for col in proj:
        m = re.match(r"(.+)_(start|end|kind|source)$", col)
        if m and m[1] in FIELDS:
            if m[1] not in used_fields:
                used_fields.append(m[1])
            if m[2] == "source":
                sourced.add(m[1])
used_fields = [f for f in FIELDS if f in used_fields]
py_project = {"kind": "capture.NODE.kind", "text": "capture.NODE", "line": "capture.NODE.startLine",
              "parent_start": "capture.PARENT.startByte", "parent_end": "capture.PARENT.endByte", "parent_kind": "capture.PARENT.kind"}
for f in used_fields:
    F = f.upper()
    py_project.update({f + "_start": f"capture.{F}.startByte", f + "_end": f"capture.{F}.endByte", f + "_kind": f"capture.{F}.kind"})
    if f in sourced:
        py_project[f + "_source"] = f"capture.{F}"
# Every projection a dump makes is one of py_nodes' (same capture, same accessor), or the thin text.
for _, _, proj in sfp_chunks + qc_chunks:
    for col, src in proj.items():
        if col.startswith("ancestor_") or col == "text" and isinstance(src, dict):
            continue
        assert py_project[col] == src, (col, src)
py_nodes = {"as": "row", "query": {"ast": {"language": "python", "syntax": "valid",
            "rule": {"all": [LEAVES, NODE, PARENT, *[field_capture(f) for f in used_fields]]}}}, "project": py_project}

# --- sel_sfp: the gate, at the module ---------------------------------------------------------------
CAPTURE_ITEMS = [NODE, PARENT, *[field_capture(f) for f in FIELDS]]


def strip_captures(rule):
    """The gate of a dump rule: the rule with its capture conjuncts removed (structure kept)."""
    if isinstance(rule, dict):
        out = {}
        for k, v in rule.items():
            if k == "all" and isinstance(v, list) and NODE in v:
                rest = [x for x in v if x not in CAPTURE_ITEMS]
                assert len(rest) + len([x for x in v if x in CAPTURE_ITEMS]) == len(v)
                out[k] = [strip_captures(x) for x in rest]
            else:
                out[k] = strip_captures(v)
        return out
    if isinstance(rule, list):
        return [strip_captures(x) for x in rule]
    return rule


sfp_gates = {json.dumps(strip_captures(g), sort_keys=True) for _, g, _ in sfp_chunks}
assert len(sfp_gates) == 1
sfp_gate = strip_captures(sfp_chunks[0][1])
assert not re.search(r'"\$[A-Z]', json.dumps(sfp_gate))
ST_UTILS = clause(ST, "shared")["define"]["utils"]
for u in re.findall(r'"matches": "([^"]+)"', json.dumps(sfp_gate)):  # each gate util: modules only
    alts = ST_UTILS[u]["any"] if "any" in ST_UTILS[u] else [ST_UTILS[u]]
    assert all(a["all"][0] == {"kind": "module"} for a in alts), u
sel_sfp = {"as": "row", "query": {"ast": {"language": "python", "syntax": "valid", "rule": {"all": [{"kind": "module"}, sfp_gate, NODE]}}},
           "project": {"kind": "capture.NODE.kind"}}

# --- sel_qc: the selection with its nearest selected ancestor --------------------------------------
QU_UTILS = clause(QU, "shared")["define"]["utils"]
qsel = QU_UTILS["quality-conditional-1-selection"]["all"]
assert qsel[3] == NODE and qsel[4] == PARENT and all(x in [field_capture(f) for f in FIELDS] for x in qsel[5:]), "selection util shape"
inside = qsel[2]["any"][0]["inside"]
assert inside["all"][0] == qsel[1] and inside["all"][1] == {"pattern": "$ANCESTOR"} and inside["stopBy"] == qsel[1] and qsel[2]["any"][1] == {"pattern": "$_"}
for _, g, _ in qc_chunks:
    assert g == {"matches": "quality-conditional-1-selection"}
SELECTED = "quality-conditional-1-selected"
sel_qc = {"as": "row", "query": {"ast": {"language": "python", "syntax": "valid", "rule": {"all": [
    LEAVES, qsel[0], {"matches": SELECTED}, NODE,
    {"any": [{"inside": {"all": [{"matches": SELECTED}, {"pattern": "$ANCESTOR"}], "stopBy": {"matches": SELECTED}}}, {"pattern": "$_"}]}]}}},
    "project": {"kind": "capture.NODE.kind", "ancestor_start": "capture.ANCESTOR.startByte",
                "ancestor_end": "capture.ANCESTOR.endByte", "ancestor_kind": "capture.ANCESTOR.kind"}}
for _, _, proj in qc_chunks:
    assert all(proj[c] == sel_qc["project"][c] for c in proj if c.startswith("ancestor_"))
for _, _, proj in sfp_chunks:
    assert all(proj[c] == proj["parent" + c[len("ancestor"):]] for c in proj if c.startswith("ancestor_"))

# --- facts file -------------------------------------------------------------------------------------
st_opts, qu_opts = clause(ST, "shared")["define"]["options"], clause(QU, "shared")["define"]["options"]
assert all(st_opts[k] == v for k, v in qu_opts.items())
gate_options = {k: st_opts[k] for k in ("oracleAnalyzer", "entryPoint", "sourceProvided")}
gate_utils = {u: ST_UTILS[u] for u in sorted(set(re.findall(r'"matches": "([^"]+)"', json.dumps(sfp_gate))))}
gate_utils[SELECTED] = qsel[1]
assert not re.search(r'"\$[A-Z]', json.dumps(qsel[1]))
facts = {
    "py_nodes": {"over": "content", "define": {"match": py_nodes}},
    "sel_sfp": {"over": "content", "define": {"match": sel_sfp}},
    "sel_qc": {"over": "content", "define": {"match": sel_qc}},
}
(HERE / "pack/facts.nodes.json").write_text(json.dumps({"define": {"options": gate_options, "language": "python", "utils": gate_utils}, "facts": facts}, indent=1) + "\n")


# --- rules ------------------------------------------------------------------------------------------
def syntax_rows(chunks_, cols, sel_join, ancestor, thin_ok):
    """The dump union over py_nodes ⋉ selection, as one SELECT: each row's chunk (projection group)
    is computed once from its kind (`grp`); a column a chunk did not project is NULL on that
    chunk's rows, the thin text is `''` on the metadata chunk's rows."""
    groups = []  # [projection key, kinds, complement?]
    for pred, _, proj in chunks_:
        key = json.dumps(proj, sort_keys=True)
        ks = kinds_of(pred)
        g = next((g for g in groups if g[0] == key), None)
        if g is None:
            groups.append([key, [], False])
            g = groups[-1]
        if ks is None:
            g[2] = True
        else:
            g[1] += [k for k in ks if k not in g[1]]
    listed = [k for g in groups for k in g[1]]
    assert len(listed) == len(set(listed)), "a kind in two projection groups"
    assert sum(g[2] for g in groups) == 1, "one complement chunk"
    q = lambda ks: ",".join(f"'{k}'" for k in ks)

    # The complement chunk's group (it may list kinds of chunks with the same projection) takes
    # every kind no other group lists (`ELSE`); kinds are listed in one group only.
    grp = "CASE " + " ".join(f"WHEN n.kind IN ({q(g[1])}) THEN {i}" for i, g in enumerate(groups) if not g[2]) + \
          f" ELSE {next(i for i, g in enumerate(groups) if g[2])} END AS grp"
    on = lambda gs: f"x.grp IN ({','.join(str(groups.index(g)) for g in gs)})"
    inner, outer = [], []
    for c in cols:
        if c.startswith("ancestor_"):
            inner.append(f"{ancestor}{c[len('ancestor_'):]} AS {c}")
            outer.append(f"x.{c}")
            continue
        inner.append(f"n.{c}")
        if c in ("__path", "__start", "__end"):
            outer.append(f"x.{c}")
            continue
        projs = [json.loads(g[0]) for g in groups]
        thin = [g for g, p in zip(groups, projs) if c == "text" and isinstance(p.get("text"), dict)]
        full = [g for g, p in zip(groups, projs) if c in p and g not in thin]
        assert full, c
        if thin:
            assert thin_ok and all(json.loads(g[0])["text"] == {"source": "capture.NODE", "regex": "^()", "group": 1} for g in thin)
            assert len(full) + len(thin) == len(groups)
            outer.append(f"CASE WHEN {on(thin)} THEN '' ELSE x.text END AS text")
        elif len(full) == len(groups):
            outer.append(f"x.{c}")
        else:
            outer.append(f"CASE WHEN {on(full)} THEN x.{c} END AS {c}")
    return f"SELECT {','.join(outer)} FROM (SELECT {','.join(inner)},{grp} FROM {sel_join}) x"


UNION2 = "(SELECT * FROM syntax_rows_0 UNION ALL SELECT * FROM syntax_rows_1)"


def rule(doc, cid, alias, rel, rows_sql, private):
    stages = [{"name": "syntax_rows", "query": rows_sql}]
    for s in rel["stages"]:
        if s["name"].startswith("syntax_rows_"):
            continue
        s = copy.deepcopy(s)
        if isinstance(s.get("query"), str):
            assert s["name"] in ("nodes", "node_ordinals") or "syntax_rows" not in s["query"]
            s["query"] = s["query"].replace(UNION2, "syntax_rows")
            assert "syntax_rows_" not in s["query"]
        stages.append(s)
    r = {"id": cid, "alias": alias, "violation": clause(doc, cid)["violation"], "scope": doc["scope"],
         "inputs": {k: rel["inputs"][k] for k in private}, "stages": stages,
         **{k: v for k, v in rel.items() if k not in ("inputs", "stages")},
         "findings": {"detail": "issue || ':' || detector || ':' || identity"}}
    return r


sfp_rows = syntax_rows(sfp_chunks, sfp_cols, "sel_sfp s JOIN py_nodes n ON n.__path=s.__path", "n.parent_", False)
qc_rows = syntax_rows(qc_chunks, qc_cols, "sel_qc s JOIN py_nodes n ON n.__path=s.__path AND n.__start=s.__start AND n.__end=s.__end AND n.kind=s.kind", "s.ancestor_", True)
r_sfp = rule(ST, "state-flow-project-1", "sfp", sfp, sfp_rows, ["raw_strings_0", "raw_strings_1", "raw_strings_2", "project_mode"])
r_sfp["options"] = {"returnKindsHex": st_opts["returnKindsHex"]}
r_qc = rule(QU, "quality-conditional-1", "qc", qc, qc_rows, [])
used = set(re.findall(r'"\$option": "([A-Za-z]+)"', json.dumps([r_sfp["inputs"], r_sfp["stages"], r_sfp["query"], r_qc["stages"], r_qc["query"]])))
assert used <= set(gate_options) | {"returnKindsHex"}, used
(HERE / "pack/rules").mkdir(parents=True, exist_ok=True)
for r in (r_sfp, r_qc):
    (HERE / "pack/rules" / f"{r['id']}.json").write_text(json.dumps(r, indent=1) + "\n")
