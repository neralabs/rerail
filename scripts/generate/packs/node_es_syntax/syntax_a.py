"""es-syntax features no-accessor-properties to no-legacy-object-prototype-accessor-methods (alphabetical), each matching
the nodes its eslint-plugin-es-x rule reports, at the node ESLint reports. Rules hold for both grammars the typescript
language reads with: tree-sitter-javascript (.js/.cjs/.mjs) and tree-sitter-typescript (.ts/.tsx); a relation may only
name kinds both grammars have."""


def kinds(*names):
    return [{"kind": name} for name in names]


def token(rx):
    """An anonymous token (a keyword or operator) whose text matches rx, not an identifier (or a statement of one) spelled the same."""
    return {"regex": rx, "not": {"any": kinds("identifier", "property_identifier", "private_property_identifier", "statement_identifier", "comment", "expression_statement")}}


# The brackets around an expression (parentheses, a computed key's) and comments: what a kindless rule must skip.
NOT_PAREN_TOKEN = {"not": {"any": [{"regex": r"^[()\[\]]$"}, {"kind": "comment"}]}}


def unparenthesized_in(parent):
    """The expression ESTree puts where `parent` holds a possibly parenthesized one: ESTree drops the parentheses, so
    the node reported starts inside them."""
    return {"all": [{"not": {"kind": "parenthesized_expression"}}, NOT_PAREN_TOKEN,
                    {"inside": {"stopBy": {"not": {"kind": "parenthesized_expression"}}, **parent}}]}


# TypeScript's `declare` modifier: ESTree starts a declared class or variable declaration at it, where tree-sitter
# wraps the declaration in an ambient_declaration.
DECLARE = token("^declare$")
DECLARED = {"follows": DECLARE}
# A class field's key: the grammars name the field differently (TypeScript public_field_definition `name`, JavaScript
# field_definition `property`), and a relation may name only kinds both have: a class member with a key and no parameters.
FIELD_MEMBER = {"regex": "^", "inside": {"kind": "class_body"}, "not": {"has": {"field": "parameters", "regex": "^"}}}
# A field es-x skips (`declare`, or in a declared class), or one typescript-estree does not make a PropertyDefinition
# (abstract: TSAbstractPropertyDefinition; accessor: AccessorProperty).
SKIPPED_FIELD = {"any": [{"has": DECLARE}, {"inside": {"kind": "class_body", "inside": DECLARED}}]}
NOT_PROPERTY_DEFINITION = {"any": [SKIPPED_FIELD, {"has": token("^(?:abstract|accessor)$")}]}


def field_key(member):
    return {"any": [{"inside": {"field": "name", **FIELD_MEMBER, **member}}, {"inside": {"field": "property", **FIELD_MEMBER, **member}}]}


FIELD_KEY = {"all": [field_key({"all": [{"not": NOT_PROPERTY_DEFINITION}]})]}
IN_PARAMETERS = {"regex": "^", "inside": {"kind": "formal_parameters"}}
FUNCTIONS = ("function_declaration", "function_expression", "arrow_function", "generator_function", "generator_function_declaration")
ASYNC = {"has": token("^async$")}


def method_function(method):
    """A `method`'s FunctionExpression: ESTree starts it at the parameters' `(`, or at a generic method's type parameters."""
    return {"any": [{"kind": "formal_parameters", "not": {"follows": {"regex": "^<"}}, "inside": {"kind": "method_definition", **method}},
                    {"kind": "type_parameters", "inside": {"kind": "method_definition", **method}}]}


# TypeScript types holding an `import('m')` (a TSImportType, not an ImportExpression): tree-sitter reads it as a call.
TYPE_CONTEXT = {"inside": {"stopBy": "end", "any": [
    *kinds("type_annotation", "type_query", "type_arguments", "type_alias_declaration", "union_type", "intersection_type", "generic_type",
           "lookup_type", "array_type", "parenthesized_type", "tuple_type", "conditional_type", "index_type_query", "constraint", "default_type",
           "implements_clause", "extends_type_clause", "type_predicate", "opting_type_annotation", "omitting_type_annotation",
           "asserts_annotation", "readonly_type", "optional_type", "rest_type", "function_type"),
    {"follows": token("^(?:as|satisfies)$")}]}}
KEYWORDS = ("abstract boolean break byte case catch char class const continue debugger default delete do double else enum export "
            "extends false final finally float for function goto if implements import in instanceof int interface long native new "
            "null package private protected public return short static super switch synchronized this throw throws transient true "
            "try typeof var void volatile while with").split()


def identifier_rx(name):
    """A regex for an identifier spelled `name`, each character possibly a \\uXXXX or \\u{X} escape (ESTree's name is decoded)."""
    return "".join(f"(?:{c}|\\\\u(?i:00{ord(c):02x}|\\{{0*{ord(c):x}\\}}))" for c in name)


KEYWORD_RX = "^(?:" + "|".join(identifier_rx(k) for k in KEYWORDS) + ")$"
LEGACY_ACCESSORS = ("__defineGetter__", "__defineSetter__", "__lookupGetter__", "__lookupSetter__")
LEGACY_ACCESSOR_RX = "^(?:" + "|".join(LEGACY_ACCESSORS) + ")$"
LEGACY_IDENTIFIER_RX = "^(?:" + "|".join(identifier_rx(name) for name in LEGACY_ACCESSORS) + ")$"
# `let` ending its line, then a name: ESLint (as the spec) reads a let declaration, tree-sitter a `let` expression
# statement (automatic semicolon) and the declarator as the next statement.
LET_BEFORE_LINE_BREAK = {"kind": "expression_statement", "regex": "^let$", "inside": {"any": kinds("program", "statement_block", "switch_case", "switch_default")},
                         "precedes": {"stopBy": {"not": {"kind": "comment"}}, "regex": r"^[\p{ID_Start}$_\\]", "not": {"any": [{"kind": "comment"}, {"regex": r"^(?:in|instanceof)\b"}]}}}

# tree-sitter-javascript reads `await (x).y` as the member `.y` of a call `await(x)`, also where `await` is the operator
# (in an async function or a module's top level; a script's top-level `await` is taken for the operator too): ESTree's
# MemberExpression (or call) over `(x)` starts at the parenthesis.
FUNCTION_LIKE = kinds(*FUNCTIONS, "method_definition")
AWAIT_CALL = {"kind": "call_expression", "has": {"field": "function", "regex": "^await$"},
              "not": {"inside": {"stopBy": {"any": FUNCTION_LIKE}, "any": FUNCTION_LIKE, "not": {"has": token("^async$")}}}}
KEYWORD_MEMBER = {"kind": "member_expression", "has": {"field": "property", "kind": "property_identifier", "regex": KEYWORD_RX}}
AWAIT_CALL_UTILS = {
    # A misparsed await call, or a member, subscript or call whose leftmost operand is one.
    "es-a-await-call-head": {"any": [AWAIT_CALL, {"any": kinds("member_expression", "subscript_expression"), "has": {"field": "object", "matches": "es-a-await-call-head"}},
                                     {"kind": "call_expression", "has": {"field": "function", "matches": "es-a-await-call-head"}}]},
    # A node that is the leftmost operand of a keyword member, through members, subscripts and calls.
    "es-a-keyword-member-above": {"any": [{"inside": {"field": "object", "any": [KEYWORD_MEMBER, {"any": kinds("member_expression", "subscript_expression"), "matches": "es-a-keyword-member-above"}]}},
                                          {"inside": {"field": "function", "kind": "call_expression", "matches": "es-a-keyword-member-above"}}]},
}

FEATURES = {
    # Property / MethodDefinition of kind get or set (JavaScript's grammar lexes `static get` before a line break as one token).
    "no-accessor-properties": {"rule": {"kind": "method_definition", "has": token(r"^(?:static\s+)?(?:get|set)\s*$")}},
    # The string literal naming an import or export. An export specifier without `as` is its own local and exported
    # name: ESLint visits (and reports) that literal twice, so the specifier, which starts there too, stands for the second.
    "no-arbitrary-module-namespace-names": {"rule": {"any": [
        {"kind": "string", "inside": {"any": kinds("namespace_export", "export_specifier", "import_specifier")}},
        {"kind": "export_specifier", "has": {"field": "name", "kind": "string"}, "not": {"has": {"field": "alias", "regex": "^"}}},
    ]}},
    # Async functions; an async method's FunctionExpression.
    "no-async-functions": {"rule": {"any": [{"any": kinds(*FUNCTIONS), **ASYNC}, method_function(ASYNC)]}},
    # Async generators, and for await.
    "no-async-iteration": {"rule": {"any": [{"any": kinds("generator_function", "generator_function_declaration"), **ASYNC},
                                            method_function({"all": [ASYNC, {"has": token(r"^\*$")}]}),
                                            {"kind": "for_in_statement", "has": token("^await$")}]}},
    # A number (not a BigInt) written 0b.
    "no-binary-numeric-literals": {"rule": {"kind": "number", "regex": "^0[bB]", "not": {"regex": "n$"}}},
    # A function declaration in a block that is no function's body, nor a static block's or (TypeScript) a namespace's:
    # a namespace, like a function, has a `name` beside its body, which no statement has.
    "no-block-scoped-functions": {"rule": {"any": kinds("function_declaration", "generator_function_declaration"), "inside": {
        "kind": "statement_block", "not": {"inside": {"any": [*kinds(*FUNCTIONS, "method_definition", "class_static_block"), {"has": {"field": "name", "regex": "^"}}]}}}}},
    # let and const declarations (a declared one starts at `declare`); a for-in/of head's declaration starts at its keyword.
    "no-block-scoped-variables": {"rule": {"any": [{"kind": "lexical_declaration", "not": DECLARED},
                                                   {"kind": "ambient_declaration", "has": {"kind": "lexical_declaration"}},
                                                   {"regex": "^(?:let|const)$", "inside": {"kind": "for_in_statement", "field": "kind"}},
                                                   LET_BEFORE_LINE_BREAK]}},
    # A field's key (a computed key's expression), a private method's key, and every other private name but a declared field's.
    "no-class-fields": {"rule": {"any": [
        {"kind": "private_property_identifier", "not": field_key({"all": [SKIPPED_FIELD]})},
        {"any": kinds("property_identifier", "string", "number"), **FIELD_KEY},
        unparenthesized_in({"kind": "computed_property_name", **FIELD_KEY}),
    ]}},
    "no-class-static-block": {"rule": {"kind": "class_static_block"}},
    # Class declarations and expressions (a declared one starts at `declare`).
    "no-classes": {"rule": {"any": [{"any": kinds("class_declaration", "class", "abstract_class_declaration"), "not": DECLARED},
                                    {"kind": "ambient_declaration", "has": {"regex": r"^(?:abstract\s+)?class\b"}}]}},
    # A property (object literal or pattern) or method with a computed key; class fields are not reported.
    "no-computed-properties": {"rule": {"any": [{"any": kinds("pair", "pair_pattern"), "has": {"field": "key", "kind": "computed_property_name"}},
                                                {"kind": "method_definition", "has": {"field": "name", "kind": "computed_property_name"}}]}},
    # A parameter with a default (TypeScript: an initialized parameter, but a parameter property's; JavaScript: an
    # assignment pattern).
    "no-default-parameters": {"rule": {"any": [{"kind": "required_parameter", "has": {"field": "value", "regex": "^"},
                                                "not": {"has": token("^(?:public|private|protected|readonly|override)$")}},
                                               {"kind": "assignment_pattern"}], "inside": {"kind": "formal_parameters"}}},
    # A pattern that is a parameter (with a default or rest too), an assignment's target, a declarator's or a for-in/of head.
    # JavaScript's grammar puts a parameter's pattern in formal_parameters (or its assignment_pattern / rest_pattern),
    # TypeScript's in a required_parameter's `pattern`.
    "no-destructuring": {"rule": {"any": kinds("object_pattern", "array_pattern"), "all": [{"any": [
        {"inside": {"kind": "formal_parameters"}},
        {"inside": {"any": kinds("assignment_pattern", "rest_pattern"), "inside": {"kind": "formal_parameters"}}},
        {"inside": {"field": "pattern", **IN_PARAMETERS}},
        {"inside": {"kind": "rest_pattern", "inside": {"field": "pattern", **IN_PARAMETERS}}},
        {"inside": {"kind": "assignment_expression", "field": "left"}},
        {"inside": {"kind": "variable_declarator", "field": "name"}},
        {"inside": {"kind": "for_in_statement", "field": "left"}},
    ]}]}},
    # An import() call; in TypeScript, not one in a type.
    "no-dynamic-import": {"rule": {"kind": "call_expression", "has": {"field": "function", "kind": "import"}, "not": TYPE_CONTEXT}},
    "no-exponential-operators": {"rule": {"any": [{"kind": "binary_expression", "has": {"field": "operator", "regex": r"^\*\*$"}},
                                                  {"kind": "augmented_assignment_expression", "has": {"field": "operator", "regex": r"^\*\*=$"}}]}},
    "no-export-ns-from": {"rule": {"kind": "export_statement", "has": {"kind": "namespace_export"}}},
    "no-for-of-loops": {"rule": {"kind": "for_in_statement", "has": {"field": "operator", "regex": "^of$"}}},
    "no-function-declarations-in-if-statement-clauses-without-block": {"rule": {
        "any": kinds("function_declaration", "generator_function_declaration"), "inside": {"any": kinds("if_statement", "else_clause")}}},
    # Generators; a generator method's FunctionExpression.
    "no-generators": {"rule": {"any": [{"any": kinds("generator_function", "generator_function_declaration")},
                                       method_function({"has": token(r"^\*$")})]}},
    # typescript-estree keeps no Shebang comment, so es-x reports a hashbang in JavaScript only.
    "no-hashbang": {"rule": {"kind": "hash_bang_line"}, "file": {"path": {"notRegex": r"\.[cm]?tsx?$"}}},
    "no-import-meta": {"rule": {"kind": "meta_property", "regex": "^import"}},
    # The initializer of a `for (var x = init in o)` head.
    "no-initializers-in-for-in": {"rule": unparenthesized_in({"kind": "for_in_statement", "field": "value"})},
    # A raw U+2028 / U+2029 in a string literal, reported at the character: reachable where a fragment starts with it.
    "no-json-superset": {"rule": {"kind": "string_fragment", "regex": "^[\u2028\u2029]", "inside": {"kind": "string"}}},
    # A non-computed property (object literal or pattern) or member access whose key is an ES3 reserved word. A member
    # over a misparsed `await (x)` starts, in ESTree, at the parenthesis: the misparsed call's arguments stand for it.
    "no-keyword-properties": {"utils": AWAIT_CALL_UTILS, "rule": {"any": [
        {"any": kinds("pair", "pair_pattern"), "has": {"field": "key", "kind": "property_identifier", "regex": KEYWORD_RX}},
        {"kind": "method_definition", "inside": {"kind": "object"}, "has": {"field": "name", "kind": "property_identifier", "regex": KEYWORD_RX}},
        {"any": kinds("shorthand_property_identifier", "shorthand_property_identifier_pattern"), "regex": KEYWORD_RX},
        {**KEYWORD_MEMBER, "not": {"has": {"field": "object", "matches": "es-a-await-call-head"}}},
        {"kind": "arguments", "inside": {"field": "arguments", **AWAIT_CALL, "matches": "es-a-keyword-member-above"}},
    ]}},
    # The labeled statement around a function declaration.
    "no-labelled-function-declarations": {"rule": {"kind": "labeled_statement", "has": {"field": "body", "kind": "function_declaration"}}},
    # A member's property naming a legacy accessor method (computed: a constant string), or a global reference to one.
    "no-legacy-object-prototype-accessor-methods": {"rule": {"any": [
        {"kind": "property_identifier", "regex": LEGACY_IDENTIFIER_RX, "inside": {"kind": "member_expression", "field": "property"}},
        {"all": [unparenthesized_in({"kind": "subscript_expression", "field": "index"}), {"value": {"regex": LEGACY_ACCESSOR_RX}}]},
        {"any": kinds("identifier", "shorthand_property_identifier"), "regex": LEGACY_IDENTIFIER_RX, "resolves": {"unresolved": True}},
    ]}},
}

LIMITS = {
    "no-json-superset": "es-x reports each raw U+2028/U+2029 in a string literal at the character itself; tree-sitter keeps "
                        "a run of unescaped characters in one string_fragment, so only a character that starts a fragment "
                        "(right after the opening quote or an escape sequence) has a node to report; later ones are missed.",
    "no-initializers-in-for-in": "tree-sitter cannot parse some Annex B initializers ESLint accepts, such as a conditional "
                                 "whose last operand is the head's `in` (`for (var a = b ? c : d in e);`): the file has no tree.",
}
