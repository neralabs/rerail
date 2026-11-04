"""Writes ../data-layer-serverplan-quality.rdl from template.rdl.

The template is the authored program with each long matcher left as `{{NAME}}`; this fills them in as
JSON. The anchors' matchers come from the relational clauses' inputs in the PIAT port
(../data-layer-serverplan-quality.json), so they stay those clauses' node sets; the helpers are written
here, each with a comment on what it matches.

    python3 gate/data-layer/final-shape/logic/gen.py
"""

import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = json.loads((HERE.parent / "data-layer-serverplan-quality.json").read_text())
CLAUSES = {clause["id"]: clause for clause in SOURCE["clauses"]}
DEFINE = next(clause for clause in SOURCE["clauses"] if "define" in clause)


def ast(rule, file=None):
    matcher = {"language": "python", "syntax": "valid", "rule": rule}
    if file is not None:
        matcher["file"] = file
    return matcher


def input_matcher(clause, name):
    """The matcher of input `name` of relational clause `clause`, with its file gate."""
    query = CLAUSES[clause]["relational"]["inputs"][name]["evidence"]["query"]["ast"]
    return ast(query["rule"], query.get("file"))


def admission(clause):
    """The file gate of the clause's admission input (its `partitionBy: file admission`)."""
    return CLAUSES[clause]["relational"]["inputs"]["admission"]["evidence"]["query"]["ast"]["file"]


# ---- the anchors: (clause, code, matcher) ----
ANCHORS = {
    "ANCHOR042": ("spq-vector-filter-constants", "SPQ-042", input_matcher("spq-vector-filter-constants", "filter_names")),
}

# ---- the helpers ----
VECTOR_QUERY = {"filePath": {"regex": "^(?:impl/common/l2070_server_plan/steps/intent_vector_query\\.py)$"}}
# A string literal with no interpolation (the SQL's module_strings input).
PLAIN_STRING = {
    "any": [
        {"all": [{"kind": "string"}, {"regex": "(?i)^[ru]*['\"]"}, {"not": {"inside": {"kind": "concatenated_string", "stopBy": "neighbor"}}}]},
        {
            "all": [
                {"kind": "concatenated_string"},
                {"not": {"has": {"kind": "interpolation", "stopBy": "end"}}},
                {"not": {"has": {"all": [{"kind": "string"}, {"regex": "(?i)^(?:fr|rf|f)['\"]"}], "stopBy": "end"}}},
            ]
        },
    ]
}


def module_constant(path, value=None):
    """The name of a module-level `NAME = <plain string>` (no annotation) in the file, its string
    matching `value` where given."""
    right = {"all": [PLAIN_STRING] + ([{"regex": value}] if value else []), "stopBy": "neighbor", "field": "right"}
    module_level = {"inside": {"all": [{"kind": "expression_statement"}, {"inside": {"kind": "module", "stopBy": "neighbor"}}], "stopBy": "neighbor"}}
    assignment = {"all": [{"kind": "assignment"}, {"not": {"has": {"pattern": "$_", "stopBy": "neighbor", "field": "type"}}}, {"has": right}, module_level]}
    return ast({"all": [{"kind": "identifier"}, path, {"inside": dict(assignment, stopBy="neighbor", field="left")}]})


HELPERS = {
    "CONSTANT": module_constant(VECTOR_QUERY),
    "IDCONSTANT": module_constant(VECTOR_QUERY, "^[rRuU]*['\"](?:branch_id|revision_id)['\"]$"),
}

# ---- the policy: the define's options, and one forbid clause per anchor ----
POLICY = {
    "format": "rerail.policy/v1",
    "id": "data-layer-serverplan-quality-logic",
    "revision": 1,
    "mode": "block",
    "description": "serverplan_quality's relational clauses in Rerail logic: anchor clauses here, violations in the rules.",
    "scope": SOURCE["scope"],
    "clauses": [{"id": DEFINE["id"], "define": {"options": DEFINE["define"]["options"], "language": DEFINE["define"].get("language", "python")}}]
    + [
        {"id": clause, "forbid": {"ast": matcher}, "violation": {"code": code, "message": "Architecture contract violation", "anchor": "witness.span"}}
        for clause, code, matcher in ANCHORS.values()
    ],
}

values = {"POLICY": POLICY, **{name: matcher for name, (_, _, matcher) in ANCHORS.items()}, **HELPERS}
text = (HERE / "template.rdl").read_text()
for name, value in values.items():
    text = text.replace("{{" + name + "}}", json.dumps(value))
assert "{{" not in text, "a placeholder the generator does not fill"
(HERE.parent / "data-layer-serverplan-quality.rdl").write_text(text)
