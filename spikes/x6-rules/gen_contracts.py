#!/usr/bin/env python3
"""One-off (kept for the record): rewrite `required-contracts` (S3's w3 narrowed text of
`data-layer-serverplan-target-architecture`, in `original/`) against general per-content facts.

The original's ten matcher inputs each fuse a general syntactic relation with the rule's own
selection (module level, top-level class bodies, `@dataclass(frozen=True, slots=True)`). Here the
relations are declared once, gate-free, in `pack/facts.contracts.json`; the selections are SQL in
the rule's stages. Every capture is a relation search on the root chain binding one metavariable
(the pattern front end's lowerable subset). Grammar facts relied on (tree-sitter-python, valid
trees): a definition statement's parent is `module`, `block` or `decorated_definition`, and a
`decorated_definition`'s / `expression_statement`'s parent is `module` or `block` (so the nearest
`block`/`module` ancestor is the statement's container); a class's `argument_list` child is its
`superclasses` field; annotations are `type` nodes under `assignment.type`,
`typed_(default_)parameter.type` and `function_definition.return_type`.
Writes `pack/facts.contracts.json` and `pack/rules/required-contracts.json`; `link.py` binds them."""
import json
from pathlib import Path

HERE = Path(__file__).parent
DOC = json.loads((HERE / "original/data-layer-serverplan-target-architecture.json").read_text())
ORIG = next(c for c in DOC["clauses"] if c["id"] == "required-contracts")
REL = ORIG["relational"]


def k(kind):
    return {"kind": kind}


def any_(*kinds):
    return {"any": [k(x) for x in kinds]}


def opt(rel):
    """An optional capture: `any[REL{… $X …}, $_]`."""
    return {"any": [rel, {"pattern": "$_"}]}


def has(field, rule, stop="neighbor"):
    return {"has": {**rule, "stopBy": stop, **({"field": field} if field else {})}}


def inside(rule, stop="neighbor", field=None):
    return {"inside": {**rule, "stopBy": stop, **({"field": field} if field else {})}}


def cap(name, *rules):
    return {"all": [*rules, {"pattern": "$" + name}]} if rules else {"pattern": "$" + name}


def fact(rule, project):
    q = {"ast": {"language": "python", "syntax": "valid", "rule": {"all": rule}}}
    return {"over": "content", "define": {"match": {"as": "row", "query": q, "project": project}}}


# A referenced name: an identifier that is neither an attribute's member nor a keyword's name
# (the original inputs' token filter, verbatim).
NAME_TOKEN = [{"all": [k("identifier"),
                       {"not": inside(k("attribute"), field="attribute")},
                       {"not": inside(k("keyword_argument"), field="name")}]},
              {"pattern": "$TOKEN"}]
DEFS = ("class_definition", "function_definition")
CONTAINER = opt(inside(cap("CONTAINER", any_("block", "module")), stop="end"))

FACTS = {
    # Every class and function definition: its name, body, nearest enclosing definition (NULL at
    # top level), the block/module holding its statement, and its decorated_definition wrapper.
    "py_definitions": fact(
        [any_(*DEFS), {"pattern": "$DEF"}, has("name", cap("NAME")),
         opt(has("body", cap("BODY"))),
         opt(inside(cap("OUTER", any_(*DEFS)), stop="end")),
         CONTAINER,
         opt(inside(cap("DECORATED", k("decorated_definition"))))],
        {"kind": "capture.DEF.kind", "name": "capture.NAME", "body": "capture.BODY.startByte",
         "outer": "capture.OUTER.startByte", "container": "capture.CONTAINER.startByte",
         "container_kind": "capture.CONTAINER.kind", "decorated": "capture.DECORATED.startByte"}),
    # Every decorator: the decorated_definition it belongs to and its call expression, if any.
    "py_decorations": fact(
        [k("decorator"), inside(cap("DECORATED", k("decorated_definition"))),
         opt(has(None, cap("CALL", k("call"))))],
        {"decorated": "capture.DECORATED.startByte", "call": "capture.CALL.startByte"}),
    # Every call's callee when it is a name or an attribute: its text, kind and member name.
    "py_callees": fact(
        [any_("identifier", "attribute"), {"pattern": "$FN"}, inside(cap("CALL", k("call")), field="function"),
         opt(has("attribute", cap("ATTR")))],
        {"call": "capture.CALL.startByte", "kind": "capture.FN.kind", "name": "capture.FN",
         "attribute": "capture.ATTR"}),
    # Every keyword argument: its name, value text and the call whose arguments hold it.
    "py_keyword_arguments": fact(
        [k("keyword_argument"), has("name", cap("KEY")), has("value", cap("VALUE")),
         opt(inside(cap("CALL", k("call")), stop="end", field="arguments"))],
        {"name": "capture.KEY", "value": "capture.VALUE", "call": "capture.CALL.startByte"}),
    # Every assignment statement or expression: its identifier target, annotation, parent kind and
    # the block/module holding it.
    "py_assignments": fact(
        [any_("assignment", "augmented_assignment"), {"pattern": "$ASSIGN"},
         opt(has("left", cap("NAME", k("identifier")))), opt(has("type", cap("TYPE"))),
         opt(inside(cap("PARENT"))), CONTAINER],
        {"kind": "capture.ASSIGN.kind", "name": "capture.NAME", "type": "capture.TYPE",
         "parent_kind": "capture.PARENT.kind", "container": "capture.CONTAINER.startByte",
         "container_kind": "capture.CONTAINER.kind"}),
    # Every typed parameter: its function (nearest definition) and splat pattern kind, if any.
    "py_typed_parameters": fact(
        [any_("typed_parameter", "typed_default_parameter"),
         opt(inside(cap("FN", k("function_definition")), stop="end")),
         opt(has(None, cap("SPLAT", any_("list_splat_pattern", "dictionary_splat_pattern"))))],
        {"function": "capture.FN.startByte", "splat": "capture.SPLAT.kind"}),
    # Every referenced name inside an annotation (`type` node): the annotated assignment or typed
    # parameter, or the function whose return type holds it.
    "py_annotation_tokens": fact(
        [*NAME_TOKEN, inside(k("type"), stop="end"),
         opt(inside(cap("ANNOTATED", any_("assignment", "typed_parameter", "typed_default_parameter")), stop="end", field="type")),
         opt(inside(cap("RETURNS", k("function_definition")), stop="end", field="return_type"))],
        {"name": "capture.TOKEN", "annotated": "capture.ANNOTATED.startByte",
         "annotated_kind": "capture.ANNOTATED.kind", "returns": "capture.RETURNS.startByte"}),
    # Every referenced name inside a class's superclass list, with its nearest keyword argument.
    "py_base_tokens": fact(
        [*NAME_TOKEN, inside(cap("CLASS", k("class_definition")), stop="end", field="superclasses"),
         opt(inside(cap("KW", k("keyword_argument")), stop="end"))],
        {"name": "capture.TOKEN", "class": "capture.CLASS.startByte", "keyword": "capture.KW.startByte"}),
    # Every referenced name on the right of an assignment statement (parent expression_statement).
    "py_assigned_tokens": fact(
        [*NAME_TOKEN, inside({"all": [any_("assignment", "augmented_assignment"), {"pattern": "$ASSIGN"},
                                      inside(k("expression_statement"))]}, stop="end", field="right")],
        {"name": "capture.TOKEN", "assignment": "capture.ASSIGN.startByte"}),
}

# Stage columns in the original's order (an input's columns: `__path,__start,__end`, then its
# projection keys sorted), so `SELECT *` readers and row comparisons see identical tuples.
TOP_CLASS = "c.kind='class_definition' AND c.outer IS NULL"
STAGES = [
    # The decorated_definitions carrying `@dataclass(…frozen=True…slots=True…)` (or `@x.dataclass(…)`).
    ("frozen_decorated",
     "SELECT DISTINCT d.__path,d.decorated FROM py_decorations d JOIN py_callees f ON f.__path=d.__path AND f.call=d.call\n"
     " JOIN py_keyword_arguments a ON a.__path=d.__path AND a.call=d.call JOIN py_keyword_arguments b ON b.__path=d.__path AND b.call=d.call\n"
     " WHERE (f.kind='identifier' AND f.name='dataclass' OR f.kind='attribute' AND f.attribute='dataclass')\n"
     " AND a.name='frozen' AND a.value='True' AND b.name='slots' AND b.value='True'"),
    ("classes",
     "SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,name ORDER BY __start DESC) AS selected FROM (\n"
     " SELECT c.__path,c.__start,c.__end,CASE WHEN z.decorated IS NULL THEN NULL ELSE c.__start END AS frozen,c.name FROM py_definitions c\n"
     f" LEFT JOIN frozen_decorated z ON z.__path=c.__path AND z.decorated=c.decorated WHERE {TOP_CLASS})) WHERE selected=1"),
    ("symbols",
     "SELECT __path,name FROM classes UNION SELECT __path,name FROM py_definitions WHERE kind='function_definition' AND container_kind='module'\n"
     " UNION SELECT __path,name FROM py_assignments WHERE parent_kind='expression_statement' AND container_kind='module' AND name IS NOT NULL"),
    ("members",
     "SELECT a.__path,a.__start,a.__end,a.name,c.__start AS owner,a.type FROM py_assignments a JOIN py_definitions c ON c.__path=a.__path AND c.body=a.container\n"
     f" WHERE a.kind='assignment' AND a.name IS NOT NULL AND a.parent_kind='expression_statement' AND a.container_kind='block' AND {TOP_CLASS}"),
    ("method_defs",
     "SELECT f.__path,f.__start,f.__end,f.name,c.__start AS owner FROM py_definitions f JOIN py_definitions c ON c.__path=f.__path AND c.body=f.container\n"
     f" WHERE f.kind='function_definition' AND f.container_kind='block' AND {TOP_CLASS}"),
    ("methods",
     "SELECT * FROM (SELECT *,row_number() OVER(PARTITION BY __path,owner,name ORDER BY __start DESC) AS selected FROM method_defs) WHERE selected=1"),
    ("annotations",
     "SELECT t.__path,t.__start,t.__end,m.name AS field,t.name,m.owner FROM py_annotation_tokens t JOIN members m ON m.__path=t.__path AND m.__start=t.annotated\n"
     " WHERE t.annotated_kind='assignment'"),
    ("returns",
     "SELECT t.__path,t.__start,t.__end,m.owner AS class,m.name AS method,t.name,m.__start AS owner FROM py_annotation_tokens t\n"
     " JOIN method_defs m ON m.__path=t.__path AND m.__start=t.returns"),
    ("parameters",
     "SELECT t.__path,t.__start,t.__end,m.owner AS class,m.name AS method,t.name,m.__start AS owner FROM py_annotation_tokens t\n"
     " JOIN py_typed_parameters p ON p.__path=t.__path AND p.__start=t.annotated JOIN method_defs m ON m.__path=p.__path AND m.__start=p.function\n"
     " WHERE t.annotated_kind IN ('typed_parameter','typed_default_parameter') AND p.splat IS NULL"),
    ("bases",
     "SELECT t.__path,t.__start,t.__end,t.name,t.class AS owner FROM py_base_tokens t JOIN py_definitions c ON c.__path=t.__path AND c.__start=t.class\n"
     f" WHERE t.keyword IS NULL AND {TOP_CLASS}"),
    ("union_names",
     "SELECT t.__path,t.__start,t.__end,a.name AS alias,t.name FROM py_assigned_tokens t JOIN py_assignments a ON a.__path=t.__path AND a.__start=t.assignment\n"
     " WHERE a.container_kind='module' AND a.name IS NOT NULL"),
]
orig_stages = {s["name"]: s for s in REL["stages"]}
stages = []
for name, sql in STAGES:
    s = {"name": name, "query": sql}
    if name in orig_stages and orig_stages[name].get("indexes"):
        s["indexes"] = orig_stages[name]["indexes"]
    stages.append(s)
stages += [orig_stages[n] for n in ("required_symbols", "required_fields", "required_immutable", "required_enums", "required_unions")]
assert {s["name"] for s in REL["stages"]} <= {s["name"] for s in stages}

shared = next(c for c in DOC["clauses"] if c["id"] == "shared")["define"]
rule = {"id": "required-contracts", "alias": "rc", "violation": ORIG["violation"], "scope": DOC["scope"],
        "options": {"requiredShape": shared["options"]["requiredShape"]},
        "inputs": {}, "stages": stages,
        **{key: v for key, v in REL.items() if key not in ("inputs", "stages")},
        "findings": {"detail": "issue || ':' || requirement || ':' || symbol"}}
out = HERE / "pack"
(out / "rules").mkdir(parents=True, exist_ok=True)
(out / "facts.contracts.json").write_text(json.dumps({"facts": FACTS}, indent=1) + "\n")
(out / "rules/required-contracts.json").write_text(json.dumps(rule, indent=1) + "\n")
