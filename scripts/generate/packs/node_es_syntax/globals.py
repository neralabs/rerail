"""ES built-in globals: the es-x rules that report references to ECMAScript globals or their static members.

Each runs ReferenceTracker.iterateGlobalReferences over a trace map (read here from the installed eslint-plugin-es-x:
each rule is run with iterateGlobalReferences stubbed to record the map), which node_policies.tracker ports. On top of
it, what iterateGlobalReferences does for globals that the tracker leaves out:
- a global that the file writes anywhere (`Map = x`, `Map++`, `[Map] = x`, `for (Map in o)`) is not traced at all
  (isModifiedGlobal), and neither are global objects that are written;
- the global objects are es-x's default globalObjectNames (global, globalThis, self, window; each traced only where it
  is declared: global by globals.node, globalThis by ecmaVersion, self and window by a /* global */ comment), and they
  are destructured and aliased like any other value (`const { Map } = globalThis`);
- values pass through `a || b`, `a && b`, `a ?? b`, a conditional's branches, a sequence's last expression and the
  TypeScript wrappers (isPassThrough), and through every assignment (`a ||= b`) and every reference of a variable
  declared with them, reassigned or not;
- `{ Map }` in an object literal is a reference, and so is a TypeScript type reference (`Promise<void>`, `let m: Map`),
  as typescript-eslint's scope manager reads it; a member chain in a type query (`typeof Array.from`) or a JSX name is
  not followed.
The tracker mode (strict, es-x's default) only decides ES module imports, which a global trace does not read.
"""
import os

import node_policies
from node_policies import FIRST, LAST, esc, key_named, pattern_from, uid

IDS = """no-array-from no-array-isarray no-array-of no-atomics-waitasync no-atomics no-bigint no-date-now no-error-cause
no-escape-unescape no-global-this no-intl-displaynames no-intl-getcanonicallocales no-intl-listformat no-intl-locale
no-intl-pluralrules no-intl-relativetimeformat no-intl-segmenter no-intl-supportedvaluesof no-json no-map no-math-acosh
no-math-asinh no-math-atanh no-math-cbrt no-math-clz32 no-math-cosh no-math-expm1 no-math-fround no-math-hypot
no-math-imul no-math-log10 no-math-log1p no-math-log2 no-math-sign no-math-sinh no-math-tanh no-math-trunc
no-number-epsilon no-number-isfinite no-number-isinteger no-number-isnan no-number-issafeinteger
no-number-maxsafeinteger no-number-minsafeinteger no-number-parsefloat no-number-parseint no-object-assign
no-object-create no-object-defineproperties no-object-defineproperty no-object-entries no-object-freeze
no-object-fromentries no-object-getownpropertydescriptor no-object-getownpropertydescriptors
no-object-getownpropertynames no-object-getownpropertysymbols no-object-getprototypeof no-object-hasown no-object-is
no-object-isextensible no-object-isfrozen no-object-issealed no-object-keys no-object-map-groupby
no-object-preventextensions no-object-seal no-object-setprototypeof no-object-values no-promise-all-settled
no-promise-any no-promise-withresolvers no-promise no-proxy no-reflect no-set no-shared-array-buffer
no-string-fromcodepoint no-string-raw no-subclassing-builtins no-symbol no-typed-arrays no-weak-map no-weak-set
no-weakrefs""".split()

# Each rule's iterateGlobalReferences calls: the trace map (READ/CALL/CONSTRUCT as $READ/$CALL/$CONSTRUCT) and the
# tracker's options.
TRACE_SCRIPT = r'''
import { createRequire } from "node:module";
import path from "node:path";
const dir = process.argv[1];
const require = createRequire(path.join(dir, "package.json"));
const { READ, CALL, CONSTRUCT, ReferenceTracker } = require("@eslint-community/eslint-utils");
let captured;
ReferenceTracker.prototype.iterateGlobalReferences = function* (map) { captured.push({ map: conv(map), mode: this.mode, names: this.globalObjectNames }); };
function conv(o) { const r = {}; for (const [k, v] of Object.entries(o)) r[k] = conv(v); if (o[READ]) r.$READ = 1; if (o[CALL]) r.$CALL = 1; if (o[CONSTRUCT]) r.$CONSTRUCT = 1; return r; }
const out = {};
for (const id of process.argv.slice(2)) {
  captured = [];
  const sourceCode = { getScope: () => ({}) };
  const handlers = require(path.join(dir, "lib", "rules", id + ".js")).create({ sourceCode, getSourceCode: () => sourceCode, report() {} });
  handlers["Program:exit"]({ type: "Program" });
  out[id] = captured;
}
console.log(JSON.stringify(out));
'''


ESX = os.path.join(node_policies.tools_dir(), "node_modules", "eslint-plugin-es-x")
TRACES = node_policies.node_json(None, ESX, TRACE_SCRIPT, ESX, *IDS)

# Where a reference writes its variable (eslint-scope's isWrite): an assignment's or update's target, a for-in/of
# head without a declaration, a destructuring assignment's targets; parentheses read through. Declarations bind a
# name of their own, which the global check of the write rejects.
WRITE_SLOT = {"any": [
    {"inside": {"kind": "assignment_expression", "field": "left"}},
    {"inside": {"kind": "augmented_assignment_expression", "field": "left"}},
    {"inside": {"kind": "update_expression", "field": "argument"}},
    {"inside": {"kind": "for_in_statement", "field": "left"}},
    {"inside": {"kind": "parenthesized_expression", "matches": "write-slot"}},
    {"inside": {"any": [{"kind": "array_pattern"}, {"kind": "object_pattern"}, {"kind": "rest_pattern"}]}},
    {"inside": {"kind": "pair_pattern", "field": "value"}},
    {"inside": {"kind": "assignment_pattern", "field": "left"}},
    {"inside": {"kind": "object_assignment_pattern", "field": "left"}}]}
PASS_THROUGH_WRAPPERS = ("as_expression", "satisfies_expression", "non_null_expression")


def global_ref(name):
    """A reference to the global `name` (declared, unshadowed), in a file that never writes it; `{ name }` in an
    object literal is one too, and in TypeScript a type reference (`Promise<void>`), which typescript-eslint's scope
    manager makes a read reference of the global variable."""
    return {"any": [{"kind": "identifier"}, {"kind": "shorthand_property_identifier"}, {"kind": "type_identifier"}], "regex": f"^{esc(name)}$",
            "resolves": {"global": [name], "directives": "eslint"}, "not": {"inside": {"stopBy": "end", "kind": "program", "matches": f"modified:{name}"}}}


def pass_through(target):
    """Expressions whose value is `target`'s as-is (isPassThrough), and any assignment's value (`a ||= u`, `a += u`)."""
    return [{"kind": "binary_expression", "has": {"field": "operator", "regex": "^(?:&&|\\|\\||\\?\\?)$"},
             "any": [{"has": {"field": "left", "matches": target}}, {"has": {"field": "right", "matches": target}}]},
            {"kind": "ternary_expression", "any": [{"has": {"field": "consequence", "matches": target}}, {"has": {"field": "alternative", "matches": target}}]},
            {"kind": "sequence_expression", "has": {"nthChild": LAST, "matches": target}},
            {"any": [{"kind": k} for k in PASS_THROUGH_WRAPPERS], "has": {"nthChild": FIRST, "matches": target}},
            {"kind": "type_assertion", "has": {"nthChild": LAST, "matches": target}},
            {"kind": "instantiation_expression", "has": {"field": "function", "matches": target}},
            {"kind": "augmented_assignment_expression", "has": {"field": "right", "matches": target}}]


def more_aliases(u, direct):
    """What else carries `u`'s value for ReferenceTracker: `pass_through` (its operands read as `direct`, see
    `traced`), the target of an augmented assignment of it, and every reference of a variable declared with it
    (reassigned or not)."""
    return pass_through(direct) + [
        {"kind": "identifier", "resolves": {"binding": {"occurrence": {"any": [
            {"inside": {"kind": "augmented_assignment_expression", "field": "left", "has": {"field": "right", "matches": u}}},
            {"inside": {"kind": "variable_declarator", "field": "name", "has": {"field": "value", "matches": u}}}]}}}}]


def renamed(value, old, new):
    if isinstance(value, dict):
        return {k: (new if k == "matches" and v == old else renamed(v, old, new)) for k, v in value.items()}
    if isinstance(value, list):
        return [renamed(v, old, new) for v in value]
    return value


def destructured(pid, krx):
    """The property of pattern `pid` taking key `krx` (where ReferenceTracker reports a destructured READ)."""
    return {"any": [{"kind": "shorthand_property_identifier_pattern", "regex": krx},
                    {"kind": "pair_pattern", "has": {"field": "key", **key_named(krx)}},
                    {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": krx}}],
            "inside": {"matches": pid}}


def bound(pid, krx):
    """An identifier of the variable that pattern `pid`'s property `krx` binds."""
    return {"kind": "identifier", "resolves": {"binding": {"occurrence": {"any": [
        {"kind": "shorthand_property_identifier_pattern", "regex": krx, "inside": {"matches": pid}},
        {"kind": "identifier", "inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(krx)}, "inside": {"matches": pid}}},
        {"kind": "identifier", "inside": {"kind": "assignment_pattern", "field": "left", "inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(krx)}, "inside": {"matches": pid}}}},
        {"kind": "identifier", "inside": {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": krx}, "inside": {"matches": pid}}}]}}}}


def traced(trace_map, object_names):
    """node_policies.tracker over a global trace map, with what iterateGlobalReferences adds (see the module doc):
    its utilities and the report rules per root global (`global <name>`)."""
    utils, report = node_policies.tracker({}, trace_map)
    names = list(trace_map) + [n for n in object_names if n not in trace_map]
    # Whether the file writes each global: one verdict at the program, its walk bounded by kind and text.
    for name in names:
        utils[f"modified:{name}"] = {"kind": "program", "has": {"stopBy": "end", "any": [{"kind": "identifier"}, {"kind": "shorthand_property_identifier_pattern"}],
                                                                "regex": f"^{esc(name)}$", "resolves": {"global": [name], "directives": "eslint"}, "matches": "write-slot"}}
    utils["write-slot"] = WRITE_SLOT
    # The global objects, each unless written; aliased and destructured as values.
    objects = utils["global-object"]
    assert objects["any"][0]["kind"] == "identifier", objects
    objects["any"][0] = {"any": [global_ref(n) for n in object_names]}
    objects["any"].extend([{"kind": "assignment_expression", "has": {"field": "right", "matches": "global-object"}},
                           {"kind": "identifier", "resolves": {"binding": {"occurrence": {"any": [
                               {"inside": {"kind": "assignment_expression", "field": "left", "has": {"field": "right", "matches": "global-object"}}},
                               {"inside": {"kind": "assignment_pattern", "field": "left", "has": {"field": "right", "matches": "global-object"}}}]}}}}])
    utils["pat:global-object"] = pattern_from("global-object")
    for name, node in trace_map.items():
        value = utils[uid(["g", name])]["any"]
        assert value[0]["kind"] == "identifier" and value[0]["resolves"]["global"] == [name], value[0]
        ref = global_ref(name)
        value[0].clear()
        value[0].update(ref)
        krx = f"^{esc(name)}$"
        if node.get("$READ"):
            report[f"global {name}"].append(destructured("pat:global-object", krx))
        value.append(bound("pat:global-object", krx))
        utils[f"pat:g.{name}"]["any"].append({"inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(krx)},
                                                         "inside": {"matches": "pat:global-object"}}})
    # A pass-through operand is read without following variables (`direct:<u>`: `u` but for its variable aliases):
    # otherwise `a = a || {}` recurses (variable a's value is `a || {}`, whose operand is variable a), where
    # ReferenceTracker stops at a variable it is already following.
    for u, rule in list(utils.items()):
        if {"kind": "parenthesized_expression", "has": {"matches": u}} in rule.get("any", []):
            direct = f"direct:{u}"
            utils[direct] = {"any": [renamed(alt, u, direct) for alt in rule["any"] if "binding" not in alt.get("resolves", {})] + pass_through(direct)}
            rule["any"].extend(more_aliases(u, direct))
    return utils, report


def scoped(prefix, utils, rule):
    """`utils` and `rule` with each utility renamed `<prefix>/<name>` (each feature's clause has its own)."""
    def walk(value):
        if isinstance(value, dict):
            return {k: (f"{prefix}/{v}" if k == "matches" and isinstance(v, str) and v in utils else walk(v)) for k, v in value.items()}
        if isinstance(value, list):
            return [walk(v) for v in value]
        return value
    return {f"{prefix}/{k}": walk(v) for k, v in utils.items()}, walk(rule)


def trace_of(rule_id):
    (call,) = TRACES[rule_id]
    assert call["names"] == ["global", "globalThis", "self", "window"], call
    return call["map"], call["names"]


def reported(report):
    return [alt for alts in report.values() for alt in alts]


CLASS_KINDS = [{"kind": k} for k in ("class_declaration", "class", "abstract_class_declaration")]
# A class's superClass (`extends X`, parentheses read through): the class_heritage's expression in JavaScript's grammar,
# its extends_clause's value in TypeScript's.
HERITAGE = {"any": [{"inside": {"kind": "class_heritage"}}, {"inside": {"kind": "extends_clause", "field": "value"}},
                    {"inside": {"kind": "parenthesized_expression", "matches": "heritage"}}]}
CAUSE_PROPERTY = {"any": [{"kind": "pair", "has": {"field": "key", **key_named("^cause$")}},
                          {"kind": "shorthand_property_identifier", "regex": "^cause$"},
                          {"kind": "method_definition", "has": {"field": "name", **key_named("^cause$")}}]}

# A member chain the tracker does not follow, being no MemberExpression: one naming a JSX element (under the `name`
# field of a node starting with `<`: the opening, closing or self-closing element) or a TypeScript type query's
# (`typeof Array.from` in a type, a TSQualifiedName; a typeof node that is no unary_expression). JSX kinds and type_query
# are not every grammar's, so they are not named. A closing element's name is no reference at all.
UNTRACED_CHAIN = {"any": [{"inside": {"field": "name", "regex": "^<"}}, {"inside": {"regex": "^typeof\\b", "not": {"kind": "unary_expression"}}},
                          {"inside": {"kind": "member_expression", "field": "object", "matches": "untraced-chain"}}]}
JSX_CLOSING_NAME = {"any": [{"inside": {"field": "name", "regex": "^</"}}, {"inside": {"kind": "member_expression", "field": "object", "matches": "jsx-closing-name"}}]}

def cause_arguments(index):
    """Arguments whose `index`th (0-based) is an object literal with a `cause` property, none spread before it."""
    return {"kind": "arguments", "all": [
        {"has": {"nthChild": {"position": index + 1, "ofRule": FIRST["ofRule"]}, "kind": "object", "has": CAUSE_PROPERTY}},
        *[{"not": {"has": {"nthChild": {"position": p, "ofRule": FIRST["ofRule"]}, "kind": "spread_element"}}} for p in range(1, index + 1)]]}


def error_cause():
    """no-error-cause: `new E(message, { cause })` (AggregateError: the third argument) with E a traced error
    constructor, and in a class whose superClass is a direct reference to one, the first `super(...)` call with that
    option (each reference reports one: the first super call recorded for its class)."""
    trace_map, object_names = trace_of("no-error-cause")
    utils, report = traced(trace_map, object_names)
    rules = []
    for index, names in ((1, [n for n in trace_map if n != "AggregateError"]), (2, ["AggregateError"])):
        utils[f"cause-arguments-{index}"] = cause_arguments(index)
        utils[f"error-read-{index}"] = {"any": [alt for n in names for alt in report[f"global {n}"] if alt.get("kind") != "new_expression"]}
        utils[f"super-class-{index}"] = {"any": [{"matches": f"error-read-{index}"}, {"kind": "parenthesized_expression", "has": {"matches": f"super-class-{index}"}}]}
        utils[f"error-subclass-{index}"] = {"any": CLASS_KINDS, "has": {"kind": "class_heritage", "any": [
            {"has": {"matches": f"super-class-{index}"}}, {"has": {"kind": "extends_clause", "has": {"field": "value", "matches": f"super-class-{index}"}}}]}}
        super_call = f"cause-super-{index}"
        utils[super_call] = {"kind": "call_expression", "all": [{"has": {"field": "function", "kind": "super"}}, {"has": {"field": "arguments", "matches": f"cause-arguments-{index}"}}],
                             "inside": {"stopBy": {"any": CLASS_KINDS}, "any": CLASS_KINDS, "matches": f"error-subclass-{index}"}}
        # An earlier such call of the same class (not of a class nested in it).
        earlier = {"follows": {"stopBy": "end", "any": [{"kind": "call_expression", "matches": super_call},
                                                        {"not": {"any": CLASS_KINDS}, "has": {"stopBy": {"any": CLASS_KINDS}, "kind": "call_expression", "regex": "^super\\b", "matches": super_call}}]}}
        rules.append({"kind": "new_expression", "all": [{"has": {"field": "constructor", "any": [{"matches": uid(["g", n])} for n in names]}},
                                                        {"has": {"field": "arguments", "matches": f"cause-arguments-{index}"}}]})
        rules.append({"kind": "call_expression", "regex": "^super\\b", "matches": super_call, "not": {"any": [earlier, {"inside": {"stopBy": {"kind": "class_body"}, **earlier}}]}})
    return utils, {"any": rules}


def feature(rule_id):
    if rule_id == "no-error-cause":
        utils, rule = error_cause()
    else:
        utils, report = traced(*trace_of(rule_id))
        alts = reported(report)
        if rule_id == "no-bigint":
            # Also every BigInt literal.
            alts.append({"kind": "number", "regex": "n$"})
        if rule_id == "no-subclassing-builtins":
            # Only the references that are a class's superClass.
            utils["heritage"] = HERITAGE
            rule = {"all": [{"any": alts}, {"matches": "heritage"}]}
        else:
            rule = {"any": alts}
    # Member reports in a JSX name (`<Array.from />`) or a type query, which the tracker does not follow.
    utils["untraced-chain"] = UNTRACED_CHAIN
    utils["jsx-closing-name"] = JSX_CLOSING_NAME
    rule = {"all": [rule, {"not": {"any": [{"kind": "member_expression"}, {"kind": "subscript_expression"}], "matches": "untraced-chain"}},
                    {"not": {"matches": "jsx-closing-name"}}]}
    utils, rule = scoped(rule_id.removeprefix("no-"), utils, rule)
    # global is Node's (globals.node); the ECMAScript globals are ecmaVersion latest's.
    return {"rule": rule, "utils": utils, "languageOptions": {"globals": {"global": "readonly"}}}


FEATURES = {rule_id: feature(rule_id) for rule_id in IDS}
# What the engine cannot evaluate (each misses a report, never adds one): a name spelled with Unicode escapes
# (`Arr\u0061y.fr\u006fm`; global resolution and the member rules read the source text), and a computed key that
# getStaticValue folds but is no string literal (`Array["fr" + "om"]`, `const k = "from"; Array[k]`; for
# no-error-cause also `{ ["cau" + "se"]: x }`).
_LIMIT = ("References spelled with Unicode escapes and computed keys that only constant folding names (`\"fr\" + \"om\"`, "
          "a const holding the name) are not followed; ReferenceTracker reports them.")
LIMITS = {rule_id: _LIMIT for rule_id in IDS}
