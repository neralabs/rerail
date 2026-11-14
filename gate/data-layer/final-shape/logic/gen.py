"""Writes ../data-layer-serverplan-quality.rr from template.rr.

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
    "ANCHOR050": ("spq-runtime-state-annotations", "SPQ-050", input_matcher("spq-runtime-state-annotations", "annotated")),
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


# spq-runtime-state-annotations: its files (not tests, naming a runtime-state type), and its inputs'
# matchers without their file gate (the anchor's gate admits the files).
SERVICES = "AsyncCapabilityTokenService|BackendExecutor|CapabilityTokenService|CapabilityTokenStore|Connection|Cursor|Hydrator|RuntimeBundle|SyncDatabaseConnection|TokenStore|Transaction"
RUNTIME_FILES = [{"filePath": {"notRegex": "(?:^|/)tests(?:/|$)"}}, {"inside": {"all": [{"kind": "module"}, {"regex": "Callable|" + SERVICES}], "stopBy": "end"}}]
IN_ANNOTATION = {"inside": {"kind": "assignment", "stopBy": "end", "field": "type"}}


def runtime(*conditions):
    return ast({"all": list(conditions) + RUNTIME_FILES})


def runtime_input(name):
    return ast(CLAUSES["spq-runtime-state-annotations"]["relational"]["inputs"][name]["evidence"]["query"]["ast"]["rule"])


HELPERS = {
    "CONSTANT": module_constant(VECTOR_QUERY),
    "IDCONSTANT": module_constant(VECTOR_QUERY, "^[rRuU]*['\"](?:branch_id|revision_id)['\"]$"),
    # Scopes, and the names bound in them.
    "FN": runtime_input("fn_scopes"),
    "CLS": runtime_input("class_scopes"),
    "BOUND": runtime_input("bound_names"),
    "PLAIN_PARAM": runtime_input("plain_params"),
    "TYPED_PARAM": runtime_input("typed_params"),
    "GLOBAL": runtime_input("global_names"),
    "NONLOCAL": runtime_input("nonlocal_names"),
    # Imports: statements, and their aliased entries.
    "FROM": runtime({"kind": "import_from_statement"}),
    "IMPORT": runtime({"kind": "import_statement"}),
    "ALIASED": runtime({"kind": "aliased_import"}),
    # An annotation's atoms: a name not in an attribute, or an outermost dotted attribute; and the
    # first identifier of an attribute chain (an atom's head shares its start).
    "NAME_ATOM": runtime_input("name_atoms"),
    "MEMBER_ATOM": runtime({"kind": "attribute"}, {"not": {"inside": {"kind": "attribute", "stopBy": "neighbor"}}},
                           {"regex": "^[A-Za-z_]\\w*(?:\\s*\\.\\s*[A-Za-z_]\\w*)*$"}, IN_ANNOTATION),
    "HEAD": runtime({"kind": "identifier"}, {"inside": {"kind": "attribute", "stopBy": "neighbor", "field": "object"}}, IN_ANNOTATION),
    "GENERIC": runtime({"kind": "generic_type"}, IN_ANNOTATION),
    # Identifiers whose text is Callable, or names a runtime-state type: an origin's pieces.
    "CALLABLE_IDENT": runtime({"kind": "identifier"}, {"regex": "^Callable$"}),
    "SERVICE_IDENT": runtime({"kind": "identifier"}, {"regex": SERVICES}),
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
text = (HERE / "template.rr").read_text()
for name, value in values.items():
    text = text.replace("{{" + name + "}}", json.dumps(value))
assert "{{" not in text, "a placeholder the generator does not fill"
(HERE.parent / "data-layer-serverplan-quality.rr").write_text(text)
