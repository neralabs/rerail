"""Prototype-method features: es-x's definePrototypeMethodHandler reports a MemberExpression `obj.name` (or
`obj[name]`, name a constant string as getPropertyName folds it) wherever name is one of a class's method names and
es-x's object type checker (lib/util/define-prototype-method-handler/object-type-checker.js, aggressive off) types obj
as that class.

The checker is ported as one utility per type name, `pt:<Type>`: the node is an expression getType types so. Each
clause carries the type utilities its classes reach (a String's reaches them all, through `+`'s known-type test); ids
are content-unique, so es_syntax_doc shares them through one define.

The name maps are es-x 7.8.0's (eslint-plugin-n 18.4.1's pin), copied from each rule's definePrototypeMethodHandler
call: the generator is given no path to the installed plugin when it imports this module.
"""
import node_policies

LAST = node_policies.LAST

# class → method names, per es-x rule.
TYPED_ARRAYS = ("Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array",
                "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array")


def array_and_typed(*names):
    return {"Array": names, **{t: names for t in TYPED_ARRAYS}}


METHODS = {
    "no-array-prototype-copywithin": {"Array": ("copyWithin",)},
    "no-array-prototype-entries": {"Array": ("entries",)},
    "no-array-prototype-every": {"Array": ("every",)},
    "no-array-prototype-fill": {"Array": ("fill",)},
    "no-array-prototype-filter": {"Array": ("filter",)},
    "no-array-prototype-find": {"Array": ("find",)},
    "no-array-prototype-findindex": {"Array": ("findIndex",)},
    "no-array-prototype-findlast-findlastindex": array_and_typed("findLast", "findLastIndex"),
    "no-array-prototype-flat": {"Array": ("flat", "flatMap")},
    "no-array-prototype-foreach": {"Array": ("forEach",)},
    "no-array-prototype-includes": array_and_typed("includes"),
    "no-array-prototype-indexof": {"Array": ("indexOf",)},
    "no-array-prototype-keys": {"Array": ("keys",)},
    "no-array-prototype-lastindexof": {"Array": ("lastIndexOf",)},
    "no-array-prototype-map": {"Array": ("map",)},
    "no-array-prototype-reduce": {"Array": ("reduce",)},
    "no-array-prototype-reduceright": {"Array": ("reduceRight",)},
    "no-array-prototype-some": {"Array": ("some",)},
    "no-array-prototype-toreversed": array_and_typed("toReversed"),
    "no-array-prototype-tosorted": array_and_typed("toSorted"),
    "no-array-prototype-tospliced": {"Array": ("toSpliced",)},
    "no-array-prototype-values": {"Array": ("values",)},
    "no-array-prototype-with": array_and_typed("with"),
    "no-array-string-prototype-at": {"Array": ("at",), "String": ("at",), **{t: ("at",) for t in TYPED_ARRAYS}},
    "no-date-prototype-getyear-setyear": {"Date": ("getYear", "setYear")},
    "no-date-prototype-togmtstring": {"Date": ("toGMTString",)},
    "no-function-prototype-bind": {"Function": ("bind",)},
    "no-intl-datetimeformat-prototype-formatrange": {"Intl.DateTimeFormat": ("formatRange",)},
    "no-intl-datetimeformat-prototype-formattoparts": {"Intl.DateTimeFormat": ("formatToParts",)},
    "no-intl-numberformat-prototype-formatrange": {"Intl.NumberFormat": ("formatRange",)},
    "no-intl-numberformat-prototype-formatrangetoparts": {"Intl.NumberFormat": ("formatRangeToParts",)},
    "no-intl-numberformat-prototype-formattoparts": {"Intl.NumberFormat": ("formatToParts",)},
    "no-intl-pluralrules-prototype-selectrange": {"Intl.PluralRules": ("selectRange",)},
    "no-promise-prototype-finally": {"Promise": ("finally",)},
    "no-regexp-prototype-compile": {"RegExp": ("compile",)},
    "no-regexp-prototype-flags": {"RegExp": ("flags",)},
    "no-resizable-and-growable-arraybuffers": {"ArrayBuffer": ("maxByteLength", "resizable", "resize"),
                                               "SharedArrayBuffer": ("grow", "growable", "maxByteLength")},
    "no-string-create-html-methods": {"String": ("anchor", "big", "blink", "bold", "fixed", "fontcolor", "fontsize", "italics",
                                                 "link", "small", "strike", "sub", "sup")},
    "no-string-prototype-codepointat": {"String": ("codePointAt",)},
    "no-string-prototype-endswith": {"String": ("endsWith",)},
    "no-string-prototype-includes": {"String": ("includes",)},
    "no-string-prototype-iswellformed-towellformed": {"String": ("isWellFormed", "toWellFormed")},
    "no-string-prototype-matchall": {"String": ("matchAll",)},
    "no-string-prototype-normalize": {"String": ("normalize",)},
    "no-string-prototype-padstart-padend": {"String": ("padEnd", "padStart")},
    "no-string-prototype-repeat": {"String": ("repeat",)},
    "no-string-prototype-replaceall": {"String": ("replaceAll",)},
    "no-string-prototype-startswith": {"String": ("startsWith",)},
    "no-string-prototype-substr": {"String": ("substr",)},
    "no-string-prototype-trim": {"String": ("trim",)},
    "no-string-prototype-trimleft-trimright": {"String": ("trimLeft", "trimRight")},
    "no-string-prototype-trimstart-trimend": {"String": ("trimEnd", "trimStart")},
    "no-symbol-prototype-description": {"Symbol": ("description",)},
}

# ---- the object type checker ---------------------------------------------------------------------------------------

# WELLKNOWN_GLOBALS: a global function's call or construction (or tag) is its return type; Intl's constructors too.
GLOBAL_FUNCTIONS = ("String", "Number", "Boolean", "Symbol", "BigInt", "Object", "Function", "Array", "RegExp", "Date", "Promise",
                    *TYPED_ARRAYS, "ArrayBuffer", "SharedArrayBuffer")
INTL = ("Collator", "DateTimeFormat", "ListFormat", "NumberFormat", "PluralRules", "RelativeTimeFormat", "Segmenter")
# Every type getType names.
TYPES = (*GLOBAL_FUNCTIONS, *(f"Intl.{name}" for name in INTL), "null", "undefined")


def t(type_name):
    return "pt:" + type_name


def kinds(*names):
    return [{"kind": name} for name in names]


def op(*ops):
    return {"has": {"field": "operator", "regex": node_policies.alt_rx(ops)}}


def left(rule):
    return {"has": {"field": "left", **rule}}


def right(rule):
    return {"has": {"field": "right", **rule}}


def is_(type_name):
    return {"matches": t(type_name)}


def not_(rule):
    return {"not": rule}


def global_ident(*names):
    """An unshadowed reference to a declared global (eslint-scope: a variable of the global scope with no definition)."""
    return {"kind": "identifier", "regex": node_policies.alt_rx(names), "resolves": {"global": list(names), "directives": "eslint"}}


def paren(util):
    return {"kind": "parenthesized_expression", "has": {"matches": util}}


KNOWN = {"matches": "pt:known"}
LOGICAL = ("&&", "||", "??")
ARITHMETIC = ("-", "*", "/", "%", "^", "**", "&", "|")
SHIFTS = ("<<", ">>", ">>>")
COMPARISONS = ("==", "!=", "===", "!==", "<", "<=", ">", ">=", "in", "instanceof")


def binary(ops, *rules):
    """A BinaryExpression or LogicalExpression with one of `ops`, or an AssignmentExpression with `op=`."""
    return {"any": [{"kind": "binary_expression", **op(*ops)}, {"kind": "augmented_assignment_expression", **op(*(o + "=" for o in ops))}],
            **({"all": list(rules)} if rules else {})}


def unary(ops, *rules):
    return {"kind": "unary_expression", **op(*ops), **({"all": list(rules)} if rules else {})}


def arg(rule):
    return {"has": {"field": "argument", **rule}}


def plus(*rules):
    return binary(("+",), *rules)


S_L, S_R = left(is_("String")), right(is_("String"))
B_L, B_R = left(is_("BigInt")), right(is_("BigInt"))
N_L, N_R = left(is_("Number")), right(is_("Number"))
NULLISH_R = {"any": [right(is_("null")), right(is_("undefined"))]}


def leaves(type_name):
    """What getType types `type_name` beyond the structure every type shares (`type_util`)."""
    out = []
    if type_name in GLOBAL_FUNCTIONS or type_name.startswith("Intl."):
        out.append({"any": [{"kind": "call_expression", "has": {"field": "function", "matches": f"pt:callee:{type_name}"}},
                            {"kind": "new_expression", "has": {"field": "constructor", "matches": f"pt:callee:{type_name}"}}]})
    if type_name == "Array":
        out += kinds("array")
    elif type_name == "Object":
        out += [{"kind": "object"}, global_ident("Intl")]
    elif type_name == "Function":
        out += kinds("function_expression", "arrow_function", "generator_function", "class")
        out.append(global_ident(*GLOBAL_FUNCTIONS))
        # A function declaration's (or named function expression's) name, the variable's only definition.
        out.append({"kind": "identifier", "resolves": {"binding": {"only": True, "site": {"any": kinds(
            "function_declaration", "generator_function_declaration", "function_expression", "generator_function")}}}})
    elif type_name == "RegExp":
        out += kinds("regex")
    elif type_name == "String":
        out += kinds("string", "template_string")
        out.append(unary(("typeof",)))
        # getPlusOperatorType: a String operand; else, the right one known, neither a BigInt, the right no Number, and not
        # a Number plus null or undefined.
        out.append(plus({"any": [S_L, S_R, {"all": [right(KNOWN), not_(B_L), not_(B_R), not_(N_R), not_({"all": [N_L, NULLISH_R]})]}]}))
    elif type_name == "Number":
        out += [{"kind": "number", "not": {"regex": "n$"}}, global_ident("NaN", "Infinity"), {"kind": "update_expression"}, unary(("+",))]
        out.append(unary(("-", "~"), arg(KNOWN), not_(arg(is_("BigInt")))))
        out.append(binary(ARITHMETIC, {"any": [left(KNOWN), right(KNOWN)]}, not_(B_L), not_(B_R)))
        out.append(binary(SHIFTS))
        out.append(plus(not_(S_L), not_(S_R), not_(B_L), not_(B_R), {"any": [N_R, {"all": [N_L, NULLISH_R]}]}))
    elif type_name == "BigInt":
        out.append({"kind": "number", "regex": "n$"})
        out.append(unary(("-", "~"), arg(is_("BigInt"))))
        out.append(binary(ARITHMETIC, {"any": [B_L, B_R]}))
        out.append(plus(not_(S_L), not_(S_R), {"any": [B_L, B_R]}))
    elif type_name == "Boolean":
        out += kinds("true", "false")
        out.append(binary(COMPARISONS))
        out.append(unary(("!", "delete")))
    elif type_name == "null":
        out += kinds("null")
    elif type_name == "undefined":
        out += [{"kind": "undefined", "resolves": {"global": ["undefined"], "directives": "eslint"}}, unary(("void",))]
    return out


def type_util(type_name):
    """getType(node) === type_name, through what passes a type along: parentheses, a sequence's last expression, `=`,
    a logical operator or conditional whose operands agree, and a variable typed by its declarator's initial value
    (`pt:typed-variable`)."""
    me = is_(type_name)
    return {"any": [
        *leaves(type_name),
        paren(t(type_name)),
        {"kind": "sequence_expression", "has": {"nthChild": LAST, **me}},
        {"kind": "assignment_expression", **right(me)},
        binary(LOGICAL, left(me), right(me)),
        {"kind": "ternary_expression", "all": [{"has": {"field": "consequence", **me}}, {"has": {"field": "alternative", **me}}]},
        {"kind": "identifier", "matches": "pt:typed-variable", "resolves": {"binding": {"site": {"kind": "variable_declarator", "has": {"field": "value", **me}}}}},
    ]}


# A target the reference writes: an assignment's, update's or for-in/of's, through destructuring patterns.
WRITE_TARGET = {"any": [
    {"inside": {"any": kinds("assignment_expression", "augmented_assignment_expression", "for_in_statement"), "field": "left"}},
    {"inside": {"kind": "update_expression"}},
    {"inside": {"any": kinds("array_pattern", "object_pattern", "rest_pattern", "parenthesized_expression"), "matches": "pt:write-target"}},
    {"inside": {"any": kinds("assignment_pattern", "object_assignment_pattern"), "field": "left", "matches": "pt:write-target"}},
    {"inside": {"kind": "pair_pattern", "field": "value", "matches": "pt:write-target"}},
]}
# getIdentifierType: a variable whose one definition is a declarator (its whole initial value types every name it
# declares, a destructured one too), const or never written but by that declarator.
TYPED_VARIABLE = {"resolves": {"binding": {"only": True}}, "any": [
    {"resolves": {"binding": {"site": {"kind": "variable_declarator", "inside": {"kind": "lexical_declaration", "regex": "^const\\b"}}}}},
    {"not": {"resolves": {"binding": {"occurrence": {"matches": "pt:write-target"}}}}},
]}


def callee_util(type_name):
    if type_name.startswith("Intl."):
        member = {"kind": "member_expression", "all": [
            {"has": {"field": "property", "kind": "property_identifier", "regex": f"^{type_name[5:]}$"}},
            {"has": {"field": "object", "matches": "pt:intl"}}]}
        return {"any": [member, paren(f"pt:callee:{type_name}")]}
    return {"any": [global_ident(type_name), paren(f"pt:callee:{type_name}")]}


def all_type_utils():
    utils = {t(name): type_util(name) for name in TYPES}
    utils["pt:known"] = {"any": [is_(name) for name in TYPES]}
    utils["pt:intl"] = {"any": [global_ident("Intl"), paren("pt:intl")]}
    utils["pt:typed-variable"] = TYPED_VARIABLE
    utils["pt:write-target"] = WRITE_TARGET
    for name in TYPES:
        if name in GLOBAL_FUNCTIONS or name.startswith("Intl."):
            utils[f"pt:callee:{name}"] = callee_util(name)
    return utils


ALL_UTILS = all_type_utils()


def reached(roots):
    """The utilities `roots` reach through `matches`."""
    seen, stack = set(), list(roots)
    while stack:
        name = stack.pop()
        if name in seen:
            continue
        seen.add(name)

        def walk(node):
            if isinstance(node, dict):
                for key, value in node.items():
                    if key == "matches" and isinstance(value, str):
                        stack.append(value)
                    else:
                        walk(value)
            elif isinstance(node, list):
                for item in node:
                    walk(item)
        walk(ALL_UTILS[name])
    return {name: ALL_UTILS[name] for name in ALL_UTILS if name in seen}


TOKEN_OR_COMMENT = {"any": [{"regex": "^[\\[\\],]$"}, {"kind": "comment"}]}


def key_util(util, names):
    """A computed key getPropertyName reads as one of `names`: getStringIfConstant's static value, a string or an array
    of exactly one element (no hole before it) that stringifies to one."""
    return {"any": [
        {"value": {"regex": node_policies.alt_rx(names), "follow": True}},
        {"kind": "array", "count": {"has": {"not": TOKEN_OR_COMMENT}, "min": 1, "max": 1},
         "has": {"matches": util, "all": [{"not": TOKEN_OR_COMMENT}, {"not": {"follows": {"stopBy": "end", "regex": "^,$"}}}]}},
    ]}


def feature(name_map):
    """`obj.name` where obj is of a class with that method; a computed key goes through `pt:key:<names>`."""
    utils = reached([t(cls) for cls in name_map])
    by_names = {}
    for cls, names in name_map.items():
        by_names.setdefault(names, []).append(cls)
    alternatives = []
    for names, classes in by_names.items():
        key = "pt:key:" + "|".join(names)
        utils[key] = key_util(key, names)
        alternatives.append({"any": [
            {"kind": "member_expression", "has": {"field": "property", "kind": "property_identifier", "regex": node_policies.alt_rx(names)}},
            {"kind": "subscript_expression", "has": {"field": "index", "matches": key}}],
            "has": {"field": "object", "any": [is_(cls) for cls in classes]}})
    return {"rule": {"any": alternatives}, "utils": utils}


FEATURES = {rule_id: feature(name_map) for rule_id, name_map in METHODS.items()}


def prefixed(node, ids, prefix):
    """`node` with every `matches` of one of `ids` renamed `prefix` + id."""
    if isinstance(node, dict):
        return {k: (prefix + v if k == "matches" and v in ids else prefixed(v, ids, prefix)) for k, v in node.items()}
    if isinstance(node, list):
        return [prefixed(item, ids, prefix) for item in node]
    return node


def resizable_constructor_arguments():
    """no-resizable-and-growable-arraybuffers also reports the second argument (its ESTree node: inside any
    parentheses) of each `new` ReferenceTracker traces to the global ArrayBuffer or SharedArrayBuffer whose first two
    arguments spread nothing. Beyond node_policies.tracker's map: a name destructured from the global object, and one
    a declarator initializes to the constructor whatever writes it later (ReferenceTracker follows every read)."""
    utils, report = node_policies.tracker({}, {"ArrayBuffer": {"$CONSTRUCT": True}, "SharedArrayBuffer": {"$CONSTRUCT": True}})
    krx = "^(?:ArrayBuffer|SharedArrayBuffer)$"
    constructors = [{"matches": "v:g.ArrayBuffer"}, {"matches": "v:g.SharedArrayBuffer"}]
    utils["global-pattern"] = node_policies.pattern_from("global-object")
    utils["buffer-alias"] = {"any": [
        {"kind": "identifier", "resolves": {"binding": {"occurrence": {"any": [
            {"kind": "shorthand_property_identifier_pattern", "regex": krx, "inside": {"matches": "global-pattern"}},
            {"kind": "shorthand_property_identifier_pattern", "regex": krx, "inside": {"kind": "object_assignment_pattern", "field": "left", "inside": {"matches": "global-pattern"}}},
            {"kind": "identifier", "inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **node_policies.key_named(krx)}, "inside": {"matches": "global-pattern"}}},
            {"kind": "identifier", "inside": {"kind": "assignment_pattern", "field": "left", "inside": {
                "kind": "pair_pattern", "field": "value", "has": {"field": "key", **node_policies.key_named(krx)}, "inside": {"matches": "global-pattern"}}}},
            {"kind": "identifier", "inside": {"kind": "variable_declarator", "field": "name", "has": {"field": "value", "any": constructors}}}]}}}},
        paren("buffer-alias")]}
    constructions = report["global ArrayBuffer"] + report["global SharedArrayBuffer"] + [
        {"kind": "new_expression", "has": {"field": "constructor", "matches": "buffer-alias"}}]
    second = {"nthChild": {"position": 2, "ofRule": node_policies.NOT_COMMENT}, "inside": {
        "kind": "arguments", "not": {"has": {"kind": "spread_element", "nthChild": {"position": "-n+2", "ofRule": node_policies.NOT_COMMENT}}},
        "inside": {"any": constructions}}}
    rule = {"not": {"any": [{"regex": "^[(),]$"}, {"kind": "comment"}, {"kind": "parenthesized_expression"}]},
            "any": [second, {"inside": {"kind": "parenthesized_expression", "stopBy": {"not": {"kind": "parenthesized_expression"}}, **second}}]}
    # Their own ids: another feature's tracker may give the same ids other bodies.
    ids = set(utils)
    return prefixed(rule, ids, "pt:ab:"), {"pt:ab:" + k: prefixed(v, ids, "pt:ab:") for k, v in utils.items()}


_rule, _utils = resizable_constructor_arguments()
FEATURES["no-resizable-and-growable-arraybuffers"]["rule"]["any"].append(_rule)
FEATURES["no-resizable-and-growable-arraybuffers"]["utils"].update(_utils)
# globals.node's `global` (ReferenceTracker's global object), in ES modules too.
FEATURES["no-resizable-and-growable-arraybuffers"]["languageOptions"] = {"globals": {"global": "readonly"}}

TS_LIMIT = ("In TypeScript files es-x types the receiver with the TypeScript checker (object-type-checker-for-ts.js): annotated "
            "and inferred types are out of reach without type information, so TypeScript is read as JavaScript (the literal "
            "shortcuts and the JavaScript checker's inference), which can miss what TypeScript types and report what it "
            "types otherwise (Date() is a string to TypeScript).")
LIMITS = {feature: TS_LIMIT for feature in METHODS}
