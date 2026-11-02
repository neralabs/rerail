"""ES2015-ES2025 syntax features: operators, literals, modules, rest/spread, templates, trailing commas, RegExp flags."""
import node_policies
from node_policies import NOT_COMMENT

# `:function` in esquery, plus a method (whose value is a FunctionExpression).
FUNCTIONS = [{"kind": k} for k in ("function_declaration", "function_expression", "generator_function",
                                   "generator_function_declaration", "arrow_function", "method_definition")]
# The node a variable scope (eslint-scope's variableScope) starts at: a function or a class static block.
VARIABLE_SCOPES = {"any": FUNCTIONS + [{"kind": "class_static_block"}]}
# A function's own parameter list.
FUNCTION_PARAMS = {"kind": "formal_parameters", "inside": {"any": FUNCTIONS}}
# A function's parameter: a child of its parameter list, or (TypeScript) the pattern of a required/optional parameter.
FUNCTION_PARAM = {"any": [{"inside": FUNCTION_PARAMS}, {"inside": {"field": "pattern", "not": {"kind": "comment"}, "inside": FUNCTION_PARAMS}}]}
# A getter or setter: its `get`/`set` keyword (an object Property whose `method` is false).
ACCESSOR = {"has": {"regex": "^(?:get|set)$", "not": {"any": [{"kind": "property_identifier"}, {"kind": "comment"}]}}}
# The next token after a node (comments skipped).
def before_token(rx):
    return {"precedes": {"stopBy": {"not": {"kind": "comment"}}, "regex": rx}}


# An array/object literal or pattern ending in a comma. A TypeScript pattern's `?` and type annotation (siblings after
# it in tree-sitter) are part of its ESTree range, so its last token is no comma.
TRAILING_COMMA = {"has": {"regex": "^,$", **before_token("^[\\]}]$")},
                  "not": {"any": [{"kind": k} for k in ("array_pattern", "object_pattern")], "precedes": {"stopBy": "neighbor", "regex": "^[:?]"}}}

# The String of the second argument of a RegExp() / new RegExp() of the global RegExp (getStringIfConstant, as
# getStaticValue folds it) containing the flag: a constant string, a scalar whose String does ("null", "undefined",
# "true", "false", "Infinity"), a regex literal, or an array literal of constants (joined, null and undefined empty).
SCALAR_TEXT = (({"eq": None}, "null"), ({"type": "undefined"}, "undefined"), ({"eq": True}, "true"), ({"eq": False}, "false"),
               ({"gt": 1.7976931348623157e308}, "Infinity"), ({"lt": -1.7976931348623157e308}, "-Infinity"))
_REGEXP_UTILS, _REGEXP_REPORT = node_policies.tracker({}, {"RegExp": {"$CALL": True, "$CONSTRUCT": True}})
# ReferenceTracker skips a global that is written anywhere (isModifiedGlobal).
REGEXP_WRITTEN = {"kind": "identifier", "regex": "^RegExp$", "resolves": {"global": ["RegExp"]}, "any": [
    {"inside": {"kind": "assignment_expression", "field": "left"}}, {"inside": {"kind": "augmented_assignment_expression", "field": "left"}},
    {"inside": {"kind": "update_expression"}}, {"inside": {"kind": "for_in_statement", "field": "left", "not": {"has": {"field": "kind", "regex": "."}}}},
    {"inside": {"kind": "array_pattern"}}, {"inside": {"kind": "rest_pattern"}},
    {"inside": {"kind": "pair_pattern", "field": "value"}}, {"inside": {"kind": "assignment_pattern", "field": "left"}}]}


def regexp_flag(flag):
    scalars = [{"value": {**test, "follow": True}} for test, text in SCALAR_TEXT if flag in text]
    # Array.prototype.join reads null and undefined as empty strings.
    in_array = [{"value": {"regex": flag, "follow": True}}] + [
        {"value": {**test, "follow": True}} for test, text in SCALAR_TEXT if flag in text and text not in ("null", "undefined")]
    array = {"kind": "array", "not": {"has": {"not": {"kind": "comment"}, "value": {"constant": False, "follow": True}}}, "has": {"any": in_array}}
    flags = {"any": [{"value": {"regex": flag, "follow": True}}, {"kind": "regex", "regex": flag}, array] + scalars}
    call = {"any": _REGEXP_REPORT["global RegExp"],
            "has": {"field": "arguments", "kind": "arguments", "has": {"nthChild": {"position": 2, "ofRule": NOT_COMMENT}, **flags}},
            "not": {"inside": {"stopBy": "end", "kind": "program", "has": {"stopBy": "end", **REGEXP_WRITTEN}}}}
    return {"rule": {"any": [{"kind": "regex", "has": {"field": "flags", "regex": flag}}, call]}, "utils": _REGEXP_UTILS}


# A binding of $N in a declarator's or for-in/of head's target: the name, or a name a pattern binds.
BINDS_N = {"any": [{"kind": "identifier", "pattern": "$N"},
                   {"any": [{"kind": "object_pattern"}, {"kind": "array_pattern"}], "has": {"stopBy": "end", "any": [
                       {"kind": "shorthand_property_identifier_pattern", "pattern": "$N"},
                       {"kind": "identifier", "pattern": "$N", "any": [
                           {"inside": {"kind": "array_pattern"}}, {"inside": {"kind": "rest_pattern"}},
                           {"inside": {"kind": "pair_pattern", "field": "value"}}, {"inside": {"kind": "assignment_pattern", "field": "left"}}]}]}}]}
# A catch clause of parameter $N in the same variable scope.
CATCH_OF_N = {"stopBy": VARIABLE_SCOPES, "kind": "catch_clause", "has": {"field": "parameter", "kind": "identifier", "pattern": "$N"}}

LOGICAL_ASSIGNMENT = {"kind": "augmented_assignment_expression", "has": {"field": "operator", "regex": "^(?:\\|\\||&&|\\?\\?)=$"}}
# A parenthesized left operand of a logical assignment (its outermost parentheses).
PAREN_LEFT = {"kind": "parenthesized_expression", "inside": {**LOGICAL_ASSIGNMENT, "field": "left"}}

# A TypeScript file (@typescript-eslint/parser).
TS_PATH = "\\.[cm]?tsx?$"

# Template elements whose cooked value is null to es-x (see no-malformed-template-literals).
OCTAL_ESCAPE_TAGGED = {"inside": {"kind": "call_expression", "field": "arguments"}, "has": {
    "kind": "escape_sequence", "any": [{"regex": "^\\\\(?:[1-9]|0[0-9])"},
                                       {"regex": "^\\\\0$", "precedes": {"stopBy": "neighbor", "kind": "string_fragment", "regex": "^[0-9]"}}]}}
TEXT_PART = {"any": [{"kind": "string_fragment"}, {"kind": "escape_sequence"}]}
NULL_ELEMENT = [{"has": {"kind": "string_fragment", "regex": "^null$",
                         "not": {"any": [{"follows": {"stopBy": "neighbor", **TEXT_PART}}, {"precedes": {"stopBy": "neighbor", **TEXT_PART}}]}}},
                {"not": {"has": {"kind": "template_substitution"}}, "value": {"eq": "null"}}]

TAGGED = {"kind": "call_expression", "has": {"field": "arguments", "kind": "template_string"}}
UNTAGGED = {"kind": "template_string", "not": {"inside": {"kind": "call_expression", "field": "arguments"}}}
# tree-sitter's TypeScript reading of f<T>`x`: (f < T) > `x`.
TS_TYPE_TAGGED = {"kind": "binary_expression", "all": [
    {"has": {"field": "operator", "regex": "^>$"}}, {"has": {"field": "right", "kind": "template_string"}},
    {"has": {"field": "left", "kind": "binary_expression", "has": {"field": "operator", "regex": "^<$"}}}]}
# Its reading of new F<T>`x` (ESTree: new (F<T>`x`), the tagged template at F): (new F<T>) applied to `x`.
TS_NEW_TAGGED_CALL = {"kind": "call_expression", "has": {"field": "arguments", "kind": "template_string"},
                      "all": [{"has": {"field": "function", "kind": "new_expression", "not": {"has": {"field": "arguments", "not": {"kind": "comment"}}}}}]}
TS_TAGGED = {"any": [{**TAGGED, "not": TS_NEW_TAGGED_CALL},
                     {"not": {"kind": "comment"}, "inside": {"kind": "new_expression", "field": "constructor", "inside": {**TS_NEW_TAGGED_CALL, "field": "function"}}},
                     TS_TYPE_TAGGED]}
# The template of f<A, B>`x`, read as the sequence (f < A), (B > `x`): its tagged template is left unreported (LIMITS).
TS_SEQUENCE_TAGGED_TEMPLATE = {"inside": {"kind": "binary_expression", "field": "right", "has": {"field": "operator", "regex": "^>$"}, "inside": {
    "kind": "sequence_expression", "has": {"kind": "binary_expression", "has": {"field": "operator", "regex": "^<$"}}}}}

IN_FUNCTION = {"inside": {"stopBy": "end", "any": FUNCTIONS}}
FOR_AWAIT = {"kind": "for_in_statement", "has": {"regex": "^await$", "not": {"kind": "identifier"}}}
TOP_LEVEL_AWAIT = {"any": [{"kind": "await_expression"}, FOR_AWAIT,
                           {"kind": "identifier", "regex": "^await$", "not": {"inside": {"any": [{"kind": "import_specifier"}, {"kind": "export_specifier"}]}}}],
                   "not": IN_FUNCTION}

FEATURES = {
    # The token after the left operand of `||=`/`&&=`/`??=`: the operator, or the `)` closing a parenthesized left.
    "no-logical-assignment-operators": {"rule": {"any": [
        {"regex": "^(?:\\|\\||&&|\\?\\?)=$", "inside": {**LOGICAL_ASSIGNMENT, "field": "operator", "not": {"has": {"field": "left", "kind": "parenthesized_expression"}}}},
        {"regex": "^\\)$", "inside": {"kind": "parenthesized_expression", "not": {"has": {"kind": "parenthesized_expression"}}, "any": [
            PAREN_LEFT, {"inside": {"stopBy": {"not": {"kind": "parenthesized_expression"}}, **PAREN_LEFT}}]}}]}},
    # A template with an element whose cooked value is null. esquery compares `[value.cooked=null]` as text, so an
    # element whose cooked value is the string "null" (`null`, `${a}null`) is reported, tagged or not; in JavaScript
    # also a tagged template's octal or \8/\9 escape (TypeScript cooks those; a malformed \x or \u escape is a syntax
    # error to tree-sitter: see LIMITS).
    "no-malformed-template-literals": [
        {"file": {"path": {"notRegex": TS_PATH}}, "rule": {"kind": "template_string", "any": [OCTAL_ESCAPE_TAGGED] + NULL_ELEMENT}},
        {"file": {"path": {"regex": TS_PATH}}, "rule": {"kind": "template_string", "any": NULL_ELEMENT}}],
    # Import and export declarations; not TypeScript's `import x = require()`, `export =` or `export as namespace`. A
    # decorated export's ESTree node starts at `export`.
    "no-modules": {"rule": {"any": [
        {"any": [{"kind": "import_statement"}, {"kind": "export_statement"}],
         "not": {"any": [{"regex": "^import\\s+(?:type\\s+)?[^\\s=,{}*'\"]+\\s*="}, {"regex": "^export\\s*="}, {"regex": "^export\\s+as\\s+namespace\\b"},
                         {"has": {"kind": "decorator"}}]}},
        {"regex": "^export$", "inside": {"kind": "export_statement", "has": {"kind": "decorator"}}}]}},
    "no-new-target": {"rule": {"kind": "meta_property", "regex": "^new"}},
    # The `??` operator token.
    "no-nullish-coalescing-operators": {"rule": {"regex": "^\\?\\?$", "inside": {"kind": "binary_expression", "field": "operator"}}},
    "no-numeric-separators": {"rule": {"kind": "number", "regex": "_"}},
    # `super` whose nearest non-arrow function is an object literal's method (not an accessor).
    "no-object-super-properties": {"rule": {"kind": "super", "inside": {
        "stopBy": {"any": [k for k in FUNCTIONS if k["kind"] != "arrow_function"]},
        "kind": "method_definition", "inside": {"kind": "object"}, "not": ACCESSOR}}},
    "no-octal-numeric-literals": {"rule": {"kind": "number", "regex": "^0[oO][0-7_]*$"}},
    "no-optional-catch-binding": {"rule": {"kind": "catch_clause", "not": {"has": {"field": "parameter", "not": {"kind": "comment"}}}}},
    # Each `?.` token (in TypeScript's tree, an optional call's is a bare token).
    "no-optional-chaining": {"rule": {"any": [{"kind": "optional_chain"}, {"regex": "^\\?\\.$", "inside": {"kind": "call_expression"}}]}},
    "no-private-in": {"rule": {"kind": "private_property_identifier", "inside": {
        "kind": "binary_expression", "field": "left", "has": {"field": "operator", "regex": "^in$"}}}},
    "no-property-shorthands": {"rule": {"any": [{"kind": "shorthand_property_identifier"},
                                                {"kind": "method_definition", "inside": {"kind": "object"}, "not": ACCESSOR}]}},
    "no-regexp-d-flag": regexp_flag("d"),
    "no-regexp-s-flag": regexp_flag("s"),
    "no-regexp-u-flag": regexp_flag("u"),
    "no-regexp-v-flag": regexp_flag("v"),
    "no-regexp-y-flag": regexp_flag("y"),
    "no-rest-parameters": {"rule": {"kind": "rest_pattern", **FUNCTION_PARAM}},
    "no-rest-spread-properties": {"rule": {"any": [{"kind": "rest_pattern", "inside": {"kind": "object_pattern"}},
                                                   {"kind": "spread_element", "inside": {"kind": "object"}}]}},
    # The `var` declarator, in a catch clause's body (same variable scope), binding the clause's parameter name; es-x
    # reports it once per such clause, so a second time (at its name, which starts with it) under two of them.
    "no-shadow-catch-param": {"rule": {"any": [
        {"all": [{"any": [{"kind": "variable_declarator", "inside": {"kind": "variable_declaration"}, "has": {"field": "name", **BINDS_N}},
                          {"inside": {"kind": "for_in_statement", "field": "left", "has": {"field": "kind", "regex": "^var$"}}, **BINDS_N}]},
                 {"inside": CATCH_OF_N}]},
        {"all": [{"inside": {"kind": "variable_declarator", "field": "name", "inside": {"kind": "variable_declaration"}}}, BINDS_N,
                 {"inside": {**CATCH_OF_N, "inside": CATCH_OF_N}}]}]}},
    "no-spread-elements": {"rule": {"kind": "spread_element", "inside": {"any": [{"kind": "array"}, {"kind": "arguments"}]}}},
    # A tagged template (at its tag), or an untagged template literal. tree-sitter's TypeScript reads a tagged template
    # with type arguments, f<T>`x`, as the comparisons (f < T) > `x`.
    "no-template-literals": [
        {"file": {"path": {"notRegex": TS_PATH}}, "rule": {"any": [TAGGED, UNTAGGED]}},
        {"file": {"path": {"regex": TS_PATH}}, "rule": {"any": [TS_TAGGED, {**UNTAGGED, "not": {"any": [
            UNTAGGED["not"], {"inside": {**TS_TYPE_TAGGED, "field": "right"}}, TS_SEQUENCE_TAGGED_TEMPLATE]}}]}}],
    # `await` and `for await` outside every function. A JavaScript script reads `await (x)` as a call, so modules only;
    # tree-sitter reads some of a module's awaits (`await (x)()`, `await [x]`, `` await `t` ``) as an `await` identifier,
    # which a module cannot have: that identifier starts the await. TypeScript reads every .cts as a module, and a .ts
    # file as one where it has an import or export; elsewhere only an unambiguous `await x`.
    "no-top-level-await": [
        {"when": {"sourceType": "module"}, "rule": TOP_LEVEL_AWAIT},
        {"file": {"path": {"regex": "\\.cts$"}}, "rule": TOP_LEVEL_AWAIT},
        {"file": {"path": {"regex": "\\.tsx?$"}}, "when": {"sourceType": "commonjs"}, "rule": {"any": [
            {"all": [TOP_LEVEL_AWAIT, {"inside": {"stopBy": "end", "kind": "program", "has": {"any": [{"kind": "import_statement"}, {"kind": "export_statement"}]}}}]},
            {"any": [{"kind": "await_expression", "regex": "^await\\s+[^\\s(\\[+\\-/`<]"}, FOR_AWAIT], "not": IN_FUNCTION}]}}],
    # The array/object literal or pattern whose last token before the closing bracket is a comma.
    "no-trailing-commas": {"rule": {"any": [{"kind": k} for k in ("array", "array_pattern", "object", "object_pattern")], **TRAILING_COMMA}},
    # The trailing comma token of a function's parameters or a call's / new's arguments (not import()).
    "no-trailing-function-commas": {"rule": {"regex": "^,$", **before_token("^\\)$"), "any": [
        {"inside": FUNCTION_PARAMS},
        {"inside": {"kind": "arguments", "not": {"inside": {"kind": "call_expression", "has": {"field": "function", "kind": "import"}}}}}]}},
    # Each `\u{...}` escape of a string or template; an identifier's only where it starts it (see LIMITS).
    "no-unicode-codepoint-escapes": {"rule": {"any": [
        {"kind": "escape_sequence", "regex": "^\\\\u\\{", "inside": {"any": [{"kind": "string"}, {"kind": "template_string"}]}},
        {"any": [{"kind": k} for k in ("identifier", "property_identifier", "shorthand_property_identifier",
                                         "shorthand_property_identifier_pattern", "statement_identifier")],
         "regex": "^\\\\u\\{"}]}},
}

MALFORMED_ESCAPE = "tree-sitter cannot parse a tagged template with a malformed \\x or \\u escape (`tag`\\unicode``): the file is source-incomplete and no clause runs on it"
LIMITS = {
    "no-malformed-template-literals": MALFORMED_ESCAPE + "; octal and \\8/\\9 escapes parse and are reported. An element cooked to the string \"null\" is found where it is plain text or the whole substitution-free template; one spelled with escapes beside substitutions (`${a}nu\\u006cl`) is not.",
    "no-template-literals": MALFORMED_ESCAPE + ". tree-sitter's TypeScript reads f<A, B>`x` as a sequence of comparisons, so that tagged template (at f) is not reported.",
    "no-unicode-codepoint-escapes": "es-x reports each escape at its own offset; an identifier is one tree-sitter leaf, so only an escape that starts it is reported (`a\\u{62}` and the second escape of `\\u{61}\\u{62}` are missed). " + MALFORMED_ESCAPE + ".",
    **{f"no-regexp-{f}-flag": "The value DSL folds no array: an array literal of constants is read element by element, but a name bound to an array (`const f = ['u']`) or a nested array is not." for f in "dsuvy"},
    "no-shadow-catch-param": "es-x reports a declarator once per enclosing catch clause naming it; at most two reports (declarator and name) are made, and a for-in head's binding is reported once.",
}
