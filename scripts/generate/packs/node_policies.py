#!/usr/bin/env python3
"""Writes the Node pack's generated Rerail policies.

Run offline by a maintainer, never in CI:

    python3 scripts/generate/packs/node_policies.py [--tools <dir>] [--root <dir>]

--tools defaults to the nearest `.rerail/tools/node` above the repository (eslint-plugin-n 18.4.1 and globals 17.13.0
installed there; the script refuses other versions). --root (default: the repository) is where the files are
written, for comparing a regeneration against the committed files.

Data read from the pinned tools (through `node`):
- eslint-plugin-n 18.4.1 `lib/unsupported-features/node-builtins.js` NodeBuiltinModules (builtin module names);
- eslint-plugin-n 18.4.1 `lib/rules/no-deprecated-api.js` rawModules and globals (the deprecation trace map);
- globals 17.13.0 `node` (Node's globals).

Writes (every other policy of the pack is hand-written: packs/node/rules/n-no-process-exit.json):
- packs/node/rules/<id>.json: every ported rule but n-no-process-exit;
- packs/node/candidates/rules/<id>.json: n-no-deprecated-api, n-hashbang;
- packs/node/candidates/probes/rules/<id>.json: the seven probes.
The pack manifests, recordings and the probes' hand-labelled expectations are not written here.

Each port's violations name its rule under suppress.eslint and, for a rule oxlint 1.88.0 implements (OXLINT_NODE_RULES),
under suppress.oxlint. suppress.biome belongs to scripts/generate/packs/biome-suppression.mjs: this script carries each
violation's suppress.biome over from the committed document, and biome-suppression.mjs, run after it, updates it.
A document keeps its committed revision when nothing else in it changes; a shipped one (packs/node/rules) that changes
bumps it, as biome-suppression.mjs does. A document with no committed version starts at revision 1.

The utilities emulate @eslint-community/eslint-utils' ReferenceTracker (legacy mode) and ESTree shapes over
tree-sitter's: see each policy's description. Some helpers exist in two spellings (e.g. a string constant matched by
its string_fragment or by its value; member property tested before or after the object) because the committed
policies were authored with both; regeneration reproduces them byte for byte.
"""
import argparse
import json
import os
import re
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
REPOSITORY = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
# The pack's directory, relative to the repository.
PACK = os.path.join("packs", "node")
PINS = {"eslint-plugin-n": "18.4.1", "globals": "17.13.0"}
# The rules of oxlint 1.88.0's `node` plugin (`oxlint --rules --format json`, scope `node`; oxc tag oxlint_v1.88.0,
# `rules::node::*` in crates/oxc_linter/src/generated/rules_enum.rs). The plugin ports eslint-plugin-node, of which
# eslint-plugin-n is the fork, so a port of `n/<rule>` for one of these names carries suppress.oxlint `<rule>` (as
# set-up's `oxlintRuleOfPort` maps it).
OXLINT_NODE_RULES = {"callback-return", "exports-style", "global-require", "handle-callback-err", "no-exports-assign",
                     "no-mixed-requires", "no-new-require", "no-path-concat", "no-process-env", "no-sync", "no-top-level-await"}

INCLUDE = ["**/*.ts", "**/*.mts", "**/*.cts", "**/*.tsx", "**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"]


# ---- pinned data -------------------------------------------------------------------------------------------------

def nearest_tools():
    directory = REPOSITORY
    while True:
        candidate = os.path.join(directory, ".rerail", "tools", "node")
        if os.path.isdir(candidate):
            return candidate
        if os.path.dirname(directory) == directory:
            raise SystemExit(f"no .rerail/tools/node above {REPOSITORY}")
        directory = os.path.dirname(directory)


def node_json(tools, cwd, script, *args):
    result = subprocess.run(["node", "--input-type=module", "-e", script, *args], cwd=cwd, capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def load_data(tools):
    for name, version in PINS.items():
        found = json.load(open(os.path.join(tools, "node_modules", name, "package.json")))["version"]
        if found != version:
            raise SystemExit(f"{name} {found} under {tools}; this generator pins {version}")
    plugin = os.path.join(tools, "node_modules", "eslint-plugin-n")
    builtins = node_json(tools, plugin, 'import { NodeBuiltinModules } from "./lib/unsupported-features/node-builtins.js"; console.log(JSON.stringify(Object.keys(NodeBuiltinModules)))')
    globals_node = node_json(tools, tools, 'import { createRequire } from "node:module"; const require = createRequire(process.cwd() + "/package.json"); console.log(JSON.stringify(require("globals").node))')
    utils_module = os.path.join(tools, "node_modules", "@eslint-community", "eslint-utils", "index.mjs")
    trace = node_json(tools, plugin, r'''
import fs from "node:fs";
import { pathToFileURL } from "node:url";
const U = pathToFileURL(process.argv[1]).href;
let src = fs.readFileSync("lib/rules/no-deprecated-api.js", "utf8");
const base = pathToFileURL(process.cwd() + "/lib/rules/").href;
src = src.replace(/from "(\.\.?\/[^"]+)"/g, (m, p) => `from "${new URL(p, base).href}"`).replace(/from "@eslint-community\/eslint-utils"/, `from "${U}"`);
src = src.replace(/export default \{[\s\S]*$/, "export { rawModules, globals }");
const { rawModules, globals } = await import("data:text/javascript," + encodeURIComponent(src));
const { READ, CALL, CONSTRUCT } = await import(U);
function conv(o) { const r = {}; for (const [k, v] of Object.entries(o)) r[k] = conv(v); if (o[READ]) r.$READ = 1; if (o[CALL]) r.$CALL = 1; if (o[CONSTRUCT]) r.$CONSTRUCT = 1; return r; }
console.log(JSON.stringify({ modules: conv(rawModules), globals: conv(globals) }));
''', utils_module)
    return builtins, globals_node, trace


# ---- documents ---------------------------------------------------------------------------------------------------

def suppress(upstream):
    rules = {"eslint": {"rules": [upstream]}}
    if upstream.startswith("n/") and upstream[2:] in OXLINT_NODE_RULES:
        rules["oxlint"] = {"rules": [upstream[2:]]}
    return rules


def policy(rid, upstream, description, checks):
    doc = {"format": "rerail.policy/v1", "id": rid, "revision": 1, "mode": "steer", "description": description,
           "scope": {"include": INCLUDE, "exclude": []}, "clauses": []}
    for check in checks:
        doc["clauses"].append({"id": check["id"], "forbid": {"ast": {"language": "typescript", **check["ast"]}},
                               "violation": {"anchor": "witness.span", "code": upstream, "message": check["message"],
                                             "suppress": suppress(upstream)}})
    return doc


def probe(pid, description, checks):
    doc = policy(pid, pid, description, checks)
    for check in doc["clauses"]:
        check["violation"].pop("suppress", None)
    return doc


def hoist(doc):
    """Clauses that all carry the same utils share them through one define."""
    forbid = [c for c in doc["clauses"] if "forbid" in c]
    shared = [c["forbid"]["ast"].get("utils") for c in forbid]
    if len(forbid) < 2 or any(u is None for u in shared) or any(json.dumps(u, sort_keys=True) != json.dumps(shared[0], sort_keys=True) for u in shared):
        return doc
    for check in forbid:
        del check["forbid"]["ast"]["utils"]
    doc["clauses"] = [{"id": "shared", "define": {"language": "typescript", "utils": shared[0]}}] + forbid
    return doc


# ---- shared rule fragments -----------------------------------------------------------------------------------------

NOT_TAGGED = {"not": {"has": {"field": "arguments", "kind": "template_string"}}}
NOT_COMMENT = {"not": {"kind": "comment"}}
FIRST = {"position": 1, "ofRule": NOT_COMMENT}
LAST = {"position": 1, "reverse": True, "ofRule": NOT_COMMENT}
SECOND_LAST = {"position": 2, "reverse": True, "ofRule": NOT_COMMENT}
CJS = {"sourceType": "commonjs"}
NODE_LO = {"sourceType": "commonjs", "globals": {"process": "readonly"}}


def esc(text):
    return re.escape(text)


def alt_rx(names):
    return "^(?:" + "|".join(esc(n) for n in names) + ")$"


def op(rx):
    return {"has": {"field": "operator", "regex": rx}}


def str_fragment_const(rx):
    """A string or substitution-free template whose string_fragment matches."""
    return {"any": [{"kind": "string", "has": {"kind": "string_fragment", "regex": rx}},
                    {"kind": "template_string", "not": {"has": {"kind": "template_substitution"}}, "has": {"kind": "string_fragment", "regex": rx}}]}


def str_const(rx):
    """A string or substitution-free template whose decoded value matches (getStringIfConstant)."""
    return {"any": [{"kind": "string"}, {"kind": "template_string", "not": {"has": {"kind": "template_substitution"}}}], "value": {"regex": rx}}


def first_arg(rx):
    return {"has": {"field": "arguments", "kind": "arguments", "has": {"nthChild": FIRST, **str_const(rx)}}}


def key_named(rx):
    return {"any": [{"kind": "property_identifier", "regex": rx}, str_const(rx), {"kind": "computed_property_name", "has": str_const(rx)}]}


def ident(name):
    return {"kind": "identifier", "regex": f"^{name}$", "resolves": {"global": [name]}}


def member_object_first(obj_util, rx):
    return {"any": [{"kind": "member_expression", "all": [{"has": {"field": "object", "matches": obj_util}}, {"has": {"field": "property", "kind": "property_identifier", "regex": rx}}]},
                    {"kind": "subscript_expression", "all": [{"has": {"field": "object", "matches": obj_util}}, {"has": {"field": "index", **str_const(rx)}}]}]}


def member_property_first(obj_util, rx):
    return {"any": [{"kind": "member_expression", "all": [{"has": {"field": "property", "kind": "property_identifier", "regex": rx}}, {"has": {"field": "object", "matches": obj_util}}]},
                    {"kind": "subscript_expression", "all": [{"has": {"field": "index", **str_const(rx)}}, {"has": {"field": "object", "matches": obj_util}}]}]}


def pattern_from(value_util):
    """An object pattern destructuring `value_util` (declarator, assignment or default)."""
    return {"kind": "object_pattern", "any": [
        {"inside": {"kind": "variable_declarator", "field": "name", "has": {"field": "value", "matches": value_util}}},
        {"inside": {"kind": "assignment_expression", "field": "left", "has": {"field": "right", "matches": value_util}}},
        {"inside": {"kind": "assignment_pattern", "field": "left", "has": {"field": "right", "matches": value_util}}}]}


# ---- module references (ReferenceTracker, legacy mode) ---------------------------------------------------------------

def module_utils(rx):
    """module-call: require(M) with the global require or process.getBuiltinModule(M) with the global process;
    module-value: that call or a constant alias, an ESM default/namespace import (and its .default)."""
    return {
        "require-callee": {"any": [{"kind": "identifier", "regex": "^require$", "resolves": {"global": ["require"]}},
                                   {"kind": "parenthesized_expression", "has": {"matches": "require-callee"}}]},
        "builtin-callee": {"any": [{"kind": "member_expression", "all": [
            {"has": {"field": "object", "kind": "identifier", "regex": "^process$", "resolves": {"global": ["process"]}}},
            {"has": {"field": "property", "kind": "property_identifier", "regex": "^getBuiltinModule$"}}]},
            {"kind": "parenthesized_expression", "has": {"matches": "builtin-callee"}}]},
        "module-call": {"kind": "call_expression", **NOT_TAGGED, "all": [first_arg(rx), {"has": {"field": "function", "any": [{"matches": "require-callee"}, {"matches": "builtin-callee"}]}}]},
        "module-value": {"any": [{"matches": "module-call"},
                                 {"kind": "parenthesized_expression", "has": {"matches": "module-value"}},
                                 {"kind": "assignment_expression", "has": {"field": "right", "matches": "module-value"}},
                                 {"kind": "identifier", "resolves": {"binding": {"destructuredValue": False, "value": {"matches": "module-value"}}}},
                                 {"kind": "identifier", "resolves": {"binding": {"occurrence": {"inside": {"kind": "assignment_expression", "field": "left", "has": {"field": "right", "matches": "module-value"}}}}}},
                                 {"matches": "esm-ref"},
                                 {"kind": "member_expression", "all": [{"has": {"field": "object", "matches": "esm-ref"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^default$"}}]}]},
        "esm-source": {"kind": "string", "has": {"kind": "string_fragment", "regex": rx}},
        "esm-whole-module": {"any": [{"kind": "namespace_import"}, {"kind": "import_clause"}, {"kind": "identifier", "inside": {"kind": "import_clause"}}],
                             "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", "matches": "esm-source"}}},
        "esm-ref": {"kind": "identifier", "resolves": {"binding": {"site": {"matches": "esm-whole-module"}}}},
    }


def prefer_global_checks(rx, prop, message):
    utils = module_utils(rx)
    if prop is None:
        return [{"id": "module-call", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"matches": "module-call"}}, "message": message},
                {"id": "esm-declaration", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"any": [{"kind": "import_statement"}, {"kind": "export_statement"}], "has": {"field": "source", "matches": "esm-source"}}}, "message": message}]
    prx = f"^{prop}$"
    pattern = {"kind": "object_pattern", "any": [
        {"inside": {"kind": "variable_declarator", "field": "name", "has": {"field": "value", "matches": "module-value"}}},
        {"inside": {"kind": "assignment_expression", "field": "left", "has": {"field": "right", "matches": "module-value"}}},
        {"inside": {"kind": "assignment_pattern", "field": "left", "has": {"field": "right", "matches": "module-value"}}}]}
    return [
        {"id": "member", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"any": [
            {"kind": "member_expression", "all": [{"has": {"field": "object", "matches": "module-value"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": prx}}]},
            {"kind": "subscript_expression", "all": [{"has": {"field": "object", "matches": "module-value"}}, {"has": {"field": "index", **str_const(prx)}}]}]}}, "message": message},
        {"id": "destructured", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"inside": pattern, "any": [
            {"kind": "shorthand_property_identifier_pattern", "regex": prx},
            {"kind": "pair_pattern", "has": {"field": "key", "any": [{"kind": "property_identifier", "regex": prx}, str_const(prx), {"kind": "computed_property_name", "has": str_const(prx)}]}},
            {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": prx}}]}}, "message": message},
        {"id": "esm-specifier", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"any": [
            {"kind": "import_specifier", "has": {"field": "name", "regex": prx}, "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", "matches": "esm-source"}}},
            {"kind": "export_specifier", "has": {"field": "name", "regex": prx}, "inside": {"stopBy": "end", "kind": "export_statement", "has": {"field": "source", "matches": "esm-source"}}}]}}, "message": message},
        {"id": "esm-export-all", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"kind": "export_statement", "has": {"field": "source", "matches": "esm-source"}, "not": {"has": {"kind": "export_clause"}}}}, "message": message},
    ]


PREFER_GLOBAL = {
    "buffer": ("^(?:node:)?buffer$", "Buffer", "buffer"),
    "console": ("^(?:node:)?console$", None, "console"),
    "process": ("^(?:node:)?process$", None, "process"),
    "url": ("^(?:node:)?url$", "URL", "url"),
    "url-search-params": ("^(?:node:)?url$", "URLSearchParams", "url"),
    "text-decoder": ("^(?:node:)?util$", "TextDecoder", "util"),
    "text-encoder": ("^(?:node:)?util$", "TextEncoder", "util"),
}


def prefer_global_docs():
    docs = []
    for name, (rx, prop, mod) in PREFER_GLOBAL.items():
        if prop:
            message = f"Unexpected use of 'require(\"{mod}\").{prop}'. Use the global variable '{prop}' instead."
            what = f"`{prop}` read from the {mod} module"
        else:
            message = f"Unexpected use of 'require(\"{mod}\")'. Use the global variable '{mod}' instead."
            what = f"the {mod} module itself"
        description = (f"eslint-plugin-n 18.4.1 n/prefer-global/{name} (default \"always\"): {what} as ReferenceTracker (legacy mode) finds it — "
                       f"`require('{mod}')`/`require('node:{mod}')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, "
                       + ("member reads, object-pattern properties, import/export specifiers and `export *`" if prop else "and every ESM import/export declaration from it") + ".")
        docs.append(hoist(policy(f"n-prefer-global-{name}", f"n/prefer-global/{name}", description, prefer_global_checks(rx, prop, message))))
    return docs


def fn_utils(rx_mod, rx_fn):
    """fn-value: a value naming member rx_fn of the module: a member, a constant alias, a destructured property or an
    import specifier."""
    utils = module_utils(rx_mod)
    utils["fn-value"] = {"any": [member_object_first("module-value", rx_fn),
                                 {"kind": "parenthesized_expression", "has": {"matches": "fn-value"}},
                                 {"kind": "identifier", "resolves": {"binding": {"destructuredValue": False, "value": {"matches": "fn-value"}}}},
                                 {"kind": "identifier", "resolves": {"binding": {"occurrence": {"any": [
                                     {"kind": "shorthand_property_identifier_pattern", "regex": rx_fn, "inside": pattern_from("module-value")},
                                     {"kind": "identifier", "inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(rx_fn)}, "inside": pattern_from("module-value")}},
                                     {"kind": "identifier", "inside": {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": rx_fn}, "inside": pattern_from("module-value")}}]}}}},
                                 {"kind": "identifier", "resolves": {"binding": {"site": {"kind": "import_specifier", "has": {"field": "name", "regex": rx_fn},
                                                                                          "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", "matches": "esm-source"}}}}}}]}
    return utils


# Upstream also reports `watch`; Rerail does not (NER-1834, declared in packs/node/divergences.json): fs.promises.watch
# returns an async iterator of change events, not an FSWatcher, so it is not a drop-in replacement.
FS_FNS = "access copyFile open rename truncate rmdir mkdir readdir readlink symlink lstat stat link unlink chmod lchmod lchown chown utimes realpath mkdtemp writeFile appendFile readFile cp glob lutimes opendir rm statfs".split()
DNS_CALL = "lookup lookupService getServers resolve resolve4 resolve6 resolveAny resolveCname resolveMx resolveNaptr resolveNs resolvePtr resolveSoa resolveSrv resolveTxt reverse setServers".split()


def plain_alt(names):
    return "^(?:" + "|".join(names) + ")$"


def prefer_promises_docs():
    docs = []
    for mod, calls, constructs in (("fs", FS_FNS, []), ("dns", DNS_CALL, ["Resolver"])):
        checks = [{"id": "call", "ast": {"languageOptions": NODE_LO, "utils": fn_utils(f"^(?:node:)?{mod}$", plain_alt(calls)),
                                         "rule": {"kind": "call_expression", **NOT_TAGGED, "has": {"field": "function", "matches": "fn-value"}}}, "message": f"Use '{mod}.promises.<name>()' instead."}]
        if constructs:
            checks.append({"id": "construct", "ast": {"languageOptions": NODE_LO, "utils": fn_utils(f"^(?:node:)?{mod}$", plain_alt(constructs)),
                                                      "rule": {"kind": "new_expression", "has": {"field": "constructor", "matches": "fn-value"}}}, "message": f"Use 'new {mod}.promises.<name>()' instead."})
        description = (f"eslint-plugin-n 18.4.1 n/prefer-promises/{mod}: calls of the {mod} module's callback API ({', '.join(calls)}"
                       + (f"; construction of {', '.join(constructs)}" if constructs else "")
                       + f") as ReferenceTracker (legacy mode) finds them: members of `require('{mod}')`/`require('node:{mod}')` (global require), `process.getBuiltinModule(…)` (global process), ESM default/namespace imports and constant aliases, destructured properties and import specifiers.")
        if mod == "fs":
            description += (" Divergence (NER-1834): `watch` is not reported, because `fs.promises.watch` returns an async iterator of change"
                            " events rather than an FSWatcher and is not a drop-in replacement.")
        docs.append(policy(f"n-prefer-promises-{mod}", f"n/prefer-promises/{mod}", description, checks))
    return docs


def no_path_concat_doc():
    slash_text = {"any": [{"kind": "string_fragment", "regex": "^/"}, {"kind": "escape_sequence", "regex": "^\\\\/$"}]}
    utils = fn_utils("^(?:node:)?path$", "^sep$")
    utils["path-source"] = {"any": [
        {"kind": "identifier", "regex": "^(?:__dirname|__filename)$", "resolves": {"global": ["__dirname", "__filename"]}},
        {"kind": "member_expression", "all": [{"has": {"field": "object", "kind": "meta_property"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^(?:dirname|filename|url)$"}}]},
        {"kind": "subscript_expression", "all": [{"has": {"field": "object", "kind": "meta_property"}}, {"has": {"field": "index", **str_const("^(?:dirname|filename|url)$")}}]}]}
    utils["first-slash"] = {"any": [
        {"any": [{"kind": "string"}, {"kind": "template_string", "not": {"has": {"kind": "template_substitution"}}}], "value": {"startsWith": "/"}},
        {"any": [{"kind": "identifier"}, {"kind": "member_expression"}], "value": {"startsWith": "/", "follow": True}},
        {"matches": "fn-value"},
        {"kind": "assignment_expression", "has": {"field": "right", "matches": "first-slash"}},
        {"kind": "binary_expression", "has": {"field": "left", "matches": "first-slash"}},
        {"kind": "binary_expression", "all": [op("^(?:\\|\\||&&|\\?\\?)$"), {"has": {"field": "right", "matches": "first-slash"}}]},
        {"kind": "ternary_expression", "any": [{"has": {"field": "consequence", "matches": "first-slash"}}, {"has": {"field": "alternative", "matches": "first-slash"}}]},
        {"kind": "parenthesized_expression", "has": {"matches": "first-slash"}},
        {"kind": "template_string", "has": {"nthChild": 1, "any": [slash_text, {"kind": "template_substitution", "has": {"matches": "first-slash"}}]}}]}
    lo = {"sourceType": "commonjs", "globals": {"process": "readonly", "__dirname": "readonly", "__filename": "readonly"}}
    message = "Use path.join() or path.resolve() instead of string concatenation."
    checks = [
        {"id": "binary", "ast": {"languageOptions": lo, "utils": utils, "rule": {"kind": "binary_expression", "all": [op("^\\+$"), {"has": {"field": "left", "matches": "path-source"}}, {"has": {"field": "right", "matches": "first-slash"}}]}}, "message": message},
        {"id": "template", "ast": {"languageOptions": lo, "utils": utils, "rule": {"kind": "template_string", "has": {"kind": "template_substitution", "has": {"matches": "path-source"},
                                                                                                                     "precedes": {"any": [slash_text, {"kind": "template_substitution", "has": {"matches": "first-slash"}}]}}}}, "message": message}]
    description = "eslint-plugin-n 18.4.1 n/no-path-concat: `__dirname`/`__filename` (Node's CommonJS-scope globals) or `import.meta.dirname|filename|url` as the left of `+` or a template substitution, when the next part's first character is `/` (path.sep on the recorder's POSIX host): a string starting with /, a constant (followed binding), `path.sep` of the path module (ReferenceTracker reach), or recursively the left of a binary, either side of ||/&&/??, either branch of ?:, an assignment's right side, or a template's first part."
    return hoist(policy("n-no-path-concat", "n/no-path-concat", description, checks))


def prefer_node_protocol_doc(builtins):
    plain = [k for k in builtins if not k.startswith("node:") and "node:" + k in builtins]
    builtin = {"kind": "string", "value": {"regex": "^(?:" + "|".join(re.escape(b).replace("\\/", "/") for b in plain) + ")$"}}
    require_call = {"kind": "call_expression", "not": {"has": {"kind": "optional_chain"}}, "has": {"field": "function", "kind": "identifier", "regex": "^require$"}}
    get_builtin_module = {"kind": "call_expression", "has": {"field": "function", "any": [
        {"kind": "member_expression", "all": [{"has": {"field": "property", "kind": "property_identifier", "regex": "^getBuiltinModule$"}},
                                              {"has": {"field": "object", "any": [{"kind": "identifier", "regex": "^process$"},
                                                                                  {"kind": "member_expression", "all": [{"has": {"field": "object", "kind": "identifier", "regex": "^globalThis$"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^process$"}}]},
                                                                                  {"kind": "subscript_expression", "all": [{"has": {"field": "object", "kind": "identifier", "regex": "^globalThis$"}}, {"has": {"field": "index", **str_fragment_const("^process$")}}]}]}}]},
        {"kind": "subscript_expression", "all": [{"has": {"field": "index", **str_fragment_const("^getBuiltinModule$")}}, {"has": {"field": "object", "kind": "identifier", "regex": "^process$"}}]}]}}
    import_call = {"kind": "call_expression", "has": {"field": "function", "kind": "import"}}

    def first_argument_of(call):
        return {"nthChild": FIRST, "inside": {"kind": "arguments", "inside": {"field": "arguments", **call}}}

    message = "Prefer `node:<module>` over `<module>`."
    checks = [
        {"id": "require", "ast": {"rule": {**builtin, "nthChild": FIRST, "inside": {"kind": "arguments", "not": {"has": {"nthChild": {"position": 2, "ofRule": NOT_COMMENT}}}, "inside": {"field": "arguments", **require_call}}}}, "message": message},
        {"id": "get-builtin-module", "ast": {"rule": {**builtin, **first_argument_of(get_builtin_module)}}, "message": message},
        {"id": "import-export-source", "ast": {"rule": {**builtin, "inside": {"field": "source", "any": [{"kind": "import_statement"}, {"kind": "export_statement"}]}}}, "message": message},
        {"id": "import-expression", "ast": {"rule": {**builtin, **first_argument_of(import_call)}}, "message": message}]
    description = "eslint-plugin-n 18.4.1 n/prefer-node-protocol (default options; every pinned source's configured Node version supports `node:`): a string literal whose value names a module of the plugin's NodeBuiltinModules table (lib/unsupported-features/node-builtins.js) that the table also lists as `node:<name>`, as the only argument of a non-optional `require(…)`, the first of `process.getBuiltinModule(…)`/`globalThis.process.getBuiltinModule(…)`, the source of an import/export declaration or of `import(…)`. Upstream reads names only, no scope."
    return policy("n-prefer-node-protocol", "n/prefer-node-protocol", description, checks)


# ---- hand-shaped syntax rules ----------------------------------------------------------------------------------------

def syntax_docs():
    docs = []
    docs.append(policy("n-no-new-require", "n/no-new-require", "eslint-plugin-n 18.4.1 n/no-new-require: `new` whose callee is the identifier `require` (upstream reads the name only, no scope).", [
        {"id": "upstream", "ast": {"rule": {"kind": "new_expression", "has": {"field": "constructor", "kind": "identifier", "regex": "^require$"}}}, "message": "Unexpected use of new with require."}]))
    docs.append(policy("n-no-process-env", "n/no-process-env", "eslint-plugin-n 18.4.1 n/no-process-env: `process.env` / `process['env']` member reads on an identifier named process (upstream reads the name only, no scope); default options allow no variable.", [
        {"id": "upstream", "ast": {"rule": {"any": [
            {"kind": "member_expression", "all": [{"has": {"field": "object", "kind": "identifier", "regex": "^process$"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^env$"}}]},
            {"kind": "subscript_expression", "all": [{"has": {"field": "object", "kind": "identifier", "regex": "^process$"}}, {"has": {"field": "index", "kind": "string", "has": {"kind": "string_fragment", "regex": "^env$"}}}]}]}}, "message": "Unexpected use of process.env."}]))

    accept = ["assignment_expression", "augmented_assignment_expression", "variable_declarator", "member_expression", "subscript_expression", "expression_statement", "call_expression",
              "ternary_expression", "program", "lexical_declaration", "variable_declaration", "arguments", "parenthesized_expression"]
    chain = {"any": [{"has": {"kind": "optional_chain"}},
                     {"inside": {"stopBy": "end", "any": [{"kind": "call_expression"}, {"kind": "member_expression"}, {"kind": "subscript_expression"}],
                                 "has": {"stopBy": {"any": [{"kind": "arguments"}, {"kind": "parenthesized_expression"}, {"kind": "template_string"}]}, "kind": "optional_chain"}}}]}
    docs.append(policy("n-global-require", "n/global-require", "eslint-plugin-n 18.4.1 n/global-require: a call of an unshadowed `require` with an ancestor outside upstream's ACCEPTABLE_PARENTS (AssignmentExpression, VariableDeclarator, MemberExpression, ExpressionStatement, CallExpression, ConditionalExpression, Program, VariableDeclaration; tree-sitter's arguments and parenthesized_expression have no ESTree node), or inside an optional chain (ESTree's ChainExpression, which tree-sitter has no node for).", [
        {"id": "upstream", "ast": {"languageOptions": {"sourceType": "commonjs"}, "rule": {"kind": "call_expression",
                                                                                         "has": {"field": "function", "kind": "identifier", "regex": "^require$", "resolves": {"global": ["require"], "declared": False}},
                                                                                         "any": [{"inside": {"stopBy": "end", "not": {"any": [{"kind": k} for k in accept]}}}, chain]}}, "message": "Unexpected require()."}]))

    module_exports = {"kind": "member_expression", "all": [{"has": {**ident("module"), "field": "object"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^exports$"}}]}
    docs.append(policy("n-no-exports-assign", "n/no-exports-assign", "eslint-plugin-n 18.4.1 n/no-exports-assign: an assignment to the global `exports` (CommonJS scope), except `module.exports = exports = …` and `exports = module.exports = …`.", [
        {"id": "upstream", "ast": {"languageOptions": CJS, "rule": {"any": [{"kind": "assignment_expression"}, {"kind": "augmented_assignment_expression"}],
                                                                    "has": {**ident("exports"), "field": "left"},
                                                                    "not": {"any": [
                                                                        {"inside": {"kind": "assignment_expression", "all": [{"has": {"field": "left", **module_exports}}]}, "nthChild": {"position": 1, "reverse": True}},
                                                                        {"has": {"field": "right", "kind": "assignment_expression", "has": {"field": "left", **module_exports}}}]}}},
         "message": "Unexpected assignment to 'exports' variable. Use 'module.exports' instead."}]))

    sync_id = {"any": [{"kind": "identifier"}, {"kind": "property_identifier"}], "regex": "Sync$"}
    callee_member = {"kind": "member_expression", "inside": {"kind": "call_expression", "field": "function", **NOT_TAGGED}}
    call_child = {"kind": "call_expression", **NOT_TAGGED, "any": [{"has": {"field": "function", **sync_id}}, {"has": {"field": "arguments", "kind": "arguments", "has": sync_id}}]}
    docs.append(policy("n-no-sync", "n/no-sync", "eslint-plugin-n 18.4.1 n/no-sync (default options): upstream's selectors `CallExpression > MemberExpression.callee Identifier[name=/Sync$/]` and `CallExpression > Identifier[name=/Sync$/]`, reported at the identifier's ESTree parent: a call with a /Sync$/ identifier as callee or argument, or a node inside (or being) a call's member callee with a /Sync$/ identifier child (tree-sitter's arguments node is no ESTree parent: its identifiers belong to the call).", [
        {"id": "call-child", "ast": {"rule": call_child}, "message": "Unexpected sync method."},
        {"id": "callee-member-descendant", "ast": {"rule": {"has": sync_id, "any": [callee_member, {"inside": {"stopBy": "end", **callee_member}}], "not": {"any": [{"kind": "arguments"}, call_child]}}}, "message": "Unexpected sync method."}]))

    require_init = {"require-init": {"not": {"has": {"kind": "optional_chain"}}, "any": [
        {"kind": "call_expression", **NOT_TAGGED, "has": {"field": "function", "kind": "identifier", "regex": "^require$"}},
        {"any": [{"kind": "member_expression"}, {"kind": "subscript_expression"}], "has": {"field": "object", "matches": "require-init"}},
        {"kind": "parenthesized_expression", "has": {"matches": "require-init"}}]}}
    docs.append(policy("n-no-mixed-requires", "n/no-mixed-requires", "eslint-plugin-n 18.4.1 n/no-mixed-requires (default options, grouping off): a variable declaration with a declarator initialised by `require(…)` (or a member chain on one; the name only, no scope) beside one uninitialised or initialised otherwise. A chain with `?.` is ESTree's ChainExpression, so not a require declarator.", [
        {"id": "upstream", "ast": {"utils": require_init, "rule": {"any": [{"kind": "variable_declaration"}, {"kind": "lexical_declaration"}], "all": [
            {"has": {"kind": "variable_declarator", "has": {"field": "value", "matches": "require-init"}}},
            {"has": {"kind": "variable_declarator", "not": {"has": {"field": "value", "matches": "require-init"}}}}]}}, "message": "Do not mix 'require' and other declarations."}]))

    docs.append(policy("n-exports-style", "n/exports-style", "eslint-plugin-n 18.4.1 n/exports-style (default \"module.exports\", no batch assign): every reference to the global `exports` (Node's CommonJS-scope binding; a local `exports` shadows it).", [
        {"id": "upstream", "ast": {"languageOptions": CJS, "rule": {"any": [{"kind": "identifier"}, {"kind": "shorthand_property_identifier"}], "regex": "^exports$", "resolves": {"global": ["exports"]}}}, "message": "Unexpected access to 'exports'. Use 'module.exports' instead."}]))

    err_id = {"kind": "identifier", "regex": "^err$", "binds": {"form": "parameter", "referenced": False}}
    first_err = {"any": [err_id,
                         {"any": [{"kind": "required_parameter"}, {"kind": "optional_parameter"}], "has": {"field": "pattern", "any": [err_id, {"kind": "rest_pattern", "has": err_id}]}},
                         {"kind": "assignment_pattern", "has": {"field": "left", **err_id}},
                         {"kind": "rest_pattern", "has": err_id}]}
    fn_kinds = ["function_declaration", "function_expression", "generator_function_declaration", "generator_function", "arrow_function"]
    docs.append(policy("n-handle-callback-err", "n/handle-callback-err", "eslint-plugin-n 18.4.1 n/handle-callback-err (default \"err\"): a function whose first parameter variable is named `err` and has no reference (reads or writes); a method is reported at its parameter list, where its ESTree FunctionExpression starts.", [
        {"id": "function", "ast": {"utils": {"first-err": first_err}, "rule": {"any": [
            {"any": [{"kind": k} for k in fn_kinds], "has": {"field": "parameters", "has": {"nthChild": FIRST, "matches": "first-err"}}},
            {"kind": "arrow_function", "has": {"field": "parameter", **err_id}},
            {"kind": "formal_parameters", "inside": {"kind": "method_definition", "field": "parameters"}, "has": {"nthChild": FIRST, "matches": "first-err"}}]}}, "message": "Expected error to be handled."}]))

    cannot_be_error = {"any": [
        {"any": [{"kind": "string"}, {"kind": "number"}, {"kind": "true"}, {"kind": "false"}, {"kind": "regex"}, {"kind": "array"}, {"kind": "object"}, {"kind": "template_string"}]},
        {"kind": "parenthesized_expression", "has": {"nthChild": FIRST, "matches": "cannot-be-error"}},
        {"kind": "assignment_expression", "has": {"field": "right", "matches": "cannot-be-error"}},
        {"kind": "sequence_expression", "has": {"nthChild": LAST, "matches": "cannot-be-error"}},
        {"kind": "binary_expression", "all": [op("^(?:\\|\\||&&|\\?\\?)$"), {"has": {"field": "left", "matches": "cannot-be-error"}}, {"has": {"field": "right", "matches": "cannot-be-error"}}]},
        {"kind": "ternary_expression", "all": [{"has": {"field": "consequence", "matches": "cannot-be-error"}}, {"has": {"field": "alternative", "matches": "cannot-be-error"}}]}]}
    docs.append(policy("n-no-callback-literal", "n/no-callback-literal", "eslint-plugin-n 18.4.1 n/no-callback-literal: a call of an identifier named `callback` or `cb` (name only, no scope) whose first argument cannot be an error per upstream's couldBeError: a non-null literal, array, object or template literal, or an assignment/sequence/logical/conditional whose possible results all are. The message is softened (NER-1834): a callback named `cb` or `callback` need not be error-first (ws verifyClient, ssh2 authHandler), so it tells the agent to check before passing an error or null first.", [
        {"id": "upstream", "ast": {"utils": {"cannot-be-error": cannot_be_error}, "rule": {"kind": "call_expression", **NOT_TAGGED, "all": [
            {"has": {"field": "function", "kind": "identifier", "regex": "^(?:callback|cb)$"}},
            {"has": {"field": "arguments", "kind": "arguments", "has": {"nthChild": FIRST, "matches": "cannot-be-error"}}}]}}, "message": "Unexpected literal in error position of callback. This rule goes by the callback's name only: if the callback is error-first, pass an Error or null as its first argument; if its API defines a different first argument (ws verifyClient's cb(result, code), ssh2 authHandler's callback(nextMethod)), leave the call as it is."}]))

    stop = {"any": [{"kind": "statement_block"}, {"kind": "return_statement"}, {"kind": "arrow_function"}]}
    any_func = {"any": [{"kind": k} for k in ["function_declaration", "function_expression", "generator_function_declaration", "generator_function", "arrow_function", "method_definition"]]}

    def statement_of_call(extra):
        return {"any": [{"inside": {"kind": "expression_statement", **extra}},
                        {"inside": {"kind": "binary_expression", "field": "right", "inside": {"kind": "expression_statement", **extra}}}]}

    ok_last = statement_of_call({"nthChild": LAST, "inside": {"kind": "statement_block", "inside": {**any_func, "field": "body"}}})
    ok_before_return = statement_of_call({"nthChild": SECOND_LAST, "inside": {"kind": "statement_block", "has": {"kind": "return_statement", "nthChild": LAST}}})
    docs.append(policy("n-callback-return", "n/callback-return", "eslint-plugin-n 18.4.1 n/callback-return (default callbacks callback, cb, next): a call of one of those names inside a function, unless its nearest block/return/arrow ancestor is a return or an arrow function, it is the last statement of a function body (or the right side of a binary/logical expression statement there), or the statement just before a block's final return.", [
        {"id": "upstream", "ast": {"rule": {"kind": "call_expression", **NOT_TAGGED, "all": [
            {"has": {"field": "function", "kind": "identifier", "regex": "^(?:callback|cb|next)$"}},
            {"not": {"inside": {"stopBy": stop, "any": [{"kind": "return_statement"}, {"kind": "arrow_function"}]}}},
            {"not": ok_last}, {"not": ok_before_return},
            {"inside": {"stopBy": "end", **any_func}}]}}, "message": "Expected return with your callback function."}]))
    return docs


# ---- candidates ------------------------------------------------------------------------------------------------------

def uid(path):
    return "v:" + ".".join(path)


def tracker(modules, globals_):
    """ReferenceTracker (legacy mode) over a trace map: per trie node a value utility v:<path>, and the report rules
    per root (READ at members, destructured properties, specifiers; CALL/CONSTRUCT at calls and `new`)."""
    utils = {
        "require-callee": {"any": [{"kind": "identifier", "regex": "^require$", "resolves": {"global": ["require"]}},
                                   {"kind": "parenthesized_expression", "has": {"matches": "require-callee"}}]},
        "builtin-callee": {"any": [{"kind": "member_expression", "all": [
            {"has": {"field": "property", "kind": "property_identifier", "regex": "^getBuiltinModule$"}},
            {"has": {"field": "object", "matches": "v:g.process"}}]},
            {"kind": "parenthesized_expression", "has": {"matches": "builtin-callee"}}]},
        "global-object": {"any": [{"kind": "identifier", "regex": "^(?:global|globalThis)$", "resolves": {"global": ["global", "globalThis"]}},
                                  {"kind": "parenthesized_expression", "has": {"matches": "global-object"}},
                                  {"kind": "identifier", "resolves": {"binding": {"destructuredValue": False, "value": {"matches": "global-object"}}}}]},
    }
    report = {}
    current = [None]

    def out():
        return report.setdefault(current[0], [])

    def aliases(u):
        return [{"kind": "parenthesized_expression", "has": {"matches": u}},
                {"kind": "assignment_expression", "has": {"field": "right", "matches": u}},
                {"kind": "identifier", "resolves": {"binding": {"destructuredValue": False, "value": {"matches": u}}}},
                {"kind": "identifier", "resolves": {"binding": {"occurrence": {"any": [
                    {"inside": {"kind": "assignment_expression", "field": "left", "has": {"field": "right", "matches": u}}},
                    {"inside": {"kind": "assignment_pattern", "field": "left", "has": {"field": "right", "matches": u}}}]}}}}]

    def pattern_util(path, parent_path, key):
        pid = "pat:" + ".".join(path)
        alts = pattern_from(uid(path))["any"]
        if parent_path is not None:
            alts = alts + [{"inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(f"^{esc(key)}$")},
                                       "inside": {"matches": "pat:" + ".".join(parent_path)}}}]
        utils[pid] = {"kind": "object_pattern", "any": alts}
        return pid

    def destructured(pid, krx):
        return {"any": [{"kind": "shorthand_property_identifier_pattern", "regex": krx},
                        {"kind": "pair_pattern", "has": {"field": "key", **key_named(krx)}},
                        {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": krx}}],
                "inside": {"matches": pid}}

    def walk(path, node, module, pid):
        u = uid(path)
        kids = [k for k in node if not k.startswith("$")]
        reads = [k for k in kids if node[k].get("$READ")]
        if reads:
            rrx = alt_rx(reads)
            out().extend([member_property_first(u, rrx), destructured(pid, rrx)])
            if module is not None and len(path) == 1:
                src = {"matches": f"m:{module}:esm-source"}
                out().extend([{"kind": "import_specifier", "has": {"field": "name", "regex": rrx}, "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", **src}}},
                              {"kind": "export_specifier", "has": {"field": "name", "regex": rrx}, "inside": {"stopBy": "end", "kind": "export_statement", "has": {"field": "source", **src}}},
                              {"kind": "export_statement", "has": {"field": "source", **src}, "not": {"has": {"kind": "export_clause"}}}])

        def value_alts(cu, krx):
            alts = [member_property_first(u, krx)] + aliases(cu) + [
                {"kind": "identifier", "resolves": {"binding": {"occurrence": {"any": [
                    {"kind": "shorthand_property_identifier_pattern", "regex": krx, "inside": {"matches": pid}},
                    {"kind": "identifier", "inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(krx)}, "inside": {"matches": pid}}},
                    {"kind": "identifier", "inside": {"kind": "assignment_pattern", "field": "left", "inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(krx)}, "inside": {"matches": pid}}}},
                    {"kind": "identifier", "inside": {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": krx}, "inside": {"matches": pid}}}]}}}}]
            if module is not None and len(path) == 1:
                src = {"matches": f"m:{module}:esm-source"}
                alts.append({"kind": "identifier", "resolves": {"binding": {"site": {"kind": "import_specifier", "has": {"field": "name", "regex": krx},
                                                                                     "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", **src}}}}}})
            return alts

        # One utility per parent for the called and constructed members: a callee is resolved once, not per name.
        for flag, kind, field in (("$CALL", "call_expression", "function"), ("$CONSTRUCT", "new_expression", "constructor")):
            names = [k for k in kids if node[k].get(flag)]
            if names:
                fu = f"{flag[1:].lower()}:{'.'.join(path)}"
                utils[fu] = {"any": value_alts(fu, alt_rx(names))}
                rule = {"kind": kind, "has": {"field": field, "matches": fu}}
                if kind == "call_expression":
                    rule = {**rule, **NOT_TAGGED}
                out().append(rule)
        for k in kids:
            child = node[k]
            cp = path + [k]
            cu = uid(cp)
            if any(not x.startswith("$") for x in child):
                utils[cu] = {"any": value_alts(cu, f"^{esc(k)}$")}
            cpid = pattern_util(cp, path, k) if any(not x.startswith("$") for x in child) else None
            walk(cp, child, module, cpid)

    for m, node in modules.items():
        current[0] = m
        rx = f"^(?:node:)?{esc(m)}$"
        utils[f"m:{m}:call"] = {"kind": "call_expression", **NOT_TAGGED, "all": [first_arg(rx), {"has": {"field": "function", "any": [{"matches": "require-callee"}, {"matches": "builtin-callee"}]}}]}
        utils[f"m:{m}:esm-source"] = {"kind": "string", "value": {"regex": rx}}
        utils[f"m:{m}:esm-whole"] = {"any": [{"kind": "namespace_import"}, {"kind": "import_clause"}, {"kind": "identifier", "inside": {"kind": "import_clause"}}],
                                     "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", "matches": f"m:{m}:esm-source"}}}
        esm_ref = {"kind": "identifier", "resolves": {"binding": {"site": {"matches": f"m:{m}:esm-whole"}}}}
        utils[uid([m])] = {"any": [{"matches": f"m:{m}:call"}, esm_ref,
                                   {"kind": "member_expression", "all": [{"has": {"field": "property", "kind": "property_identifier", "regex": "^default$"}}, {"has": {"field": "object", **esm_ref}}]}] + aliases(uid([m]))}
        if node.get("$READ"):
            out().extend([{"matches": f"m:{m}:call"}, {"any": [{"kind": "import_statement"}, {"kind": "export_statement"}], "has": {"field": "source", "matches": f"m:{m}:esm-source"}}])
        walk([m], node, m, pattern_util([m], None, None))
    for g, node in globals_.items():
        current[0] = "global " + g
        gu = uid(["g", g])
        grx = f"^{esc(g)}$"
        gid = {"kind": "identifier", "regex": grx, "resolves": {"global": [g], "directives": "eslint"}}
        utils[gu] = {"any": [gid, member_property_first("global-object", grx)] + aliases(gu)}
        if node.get("$READ"):
            out().extend([gid, member_property_first("global-object", grx)])
        if node.get("$CALL"):
            out().append({"kind": "call_expression", **NOT_TAGGED, "has": {"field": "function", "matches": gu}})
        if node.get("$CONSTRUCT"):
            out().append({"kind": "new_expression", "has": {"field": "constructor", "matches": gu}})
        walk(["g", g], node, None, pattern_util(["g", g], None, None))
    return utils, report


def no_deprecated_api_doc(trace):
    utils, report = tracker(trace["modules"], trace["globals"])
    lo = {"sourceType": "commonjs", "globals": {"Buffer": "readonly", "process": "readonly"}}
    checks = [{"id": "trace-map", "define": {"language": "typescript", "utils": utils}}]
    for root, alts in report.items():
        checks.append({"id": re.sub(r"[^a-z0-9]+", "-", root.lower()).strip("-"),
                       "forbid": {"ast": {"language": "typescript", "languageOptions": lo, "rule": {"any": alts}}},
                       "violation": {"anchor": "witness.span", "code": "n/no-deprecated-api", "message": f"Deprecated Node.js API ({root}).",
                                     "suppress": suppress("n/no-deprecated-api")}})
    description = "eslint-plugin-n 18.4.1 n/no-deprecated-api (default options; the configured version only words the message): every read, call or construction its deprecation trace map marks (generated from lib/rules/no-deprecated-api.js rawModules and globals), as ReferenceTracker (legacy mode) reaches it: require/process.getBuiltinModule/ESM imports of the modules (with and without `node:`), global variables (declared by globals.node, the ES builtins or /* global */ comments) and members of global/globalThis, through constant aliases, (nested) destructuring, defaults and member chains."
    return {"format": "rerail.policy/v1", "id": "n-no-deprecated-api", "revision": 1, "mode": "steer", "description": description,
            "scope": {"include": INCLUDE, "exclude": []}, "clauses": checks}


def hashbang_doc():
    return policy("n-hashbang", "n/hashbang", "Candidate for eslint-plugin-n 18.4.1 n/hashbang: reports every shebang line (\"This file needs no shebang.\"). Upstream decides per file whether a shebang is required from the nearest package.json's `bin` (convertPath, npmignore and additionalExecutables included) and also reports a missing or non-node shebang, BOM and CRLF in bin files; Rerail cannot read package.json, so bin files are FPs (shebang present) and FNs (shebang missing).", [
        {"id": "needs-no-shebang", "ast": {"rule": {"kind": "hash_bang_line"}}, "message": "This file needs no shebang."}])


# ---- probes ----------------------------------------------------------------------------------------------------------

def probe_docs(builtins, globals_node):
    cjs_names = ["require", "module", "exports", "__dirname", "__filename"]
    not_esm = {"path": {"notRegex": "\\.m[jt]s$"}}
    docs = []
    docs.append(probe("probe-cjs-scope-bindings", "Node fact: in a CommonJS module, require, module, exports, __dirname and __filename are bindings of the module wrapper (sourceType commonjs declares require/module/exports; __dirname/__filename are declared as Node globals). A reference resolves to them unless a parameter or block binding shadows it.", [
        {"id": "reference", "ast": {"languageOptions": {"sourceType": "commonjs", "globals": {"__dirname": "readonly", "__filename": "readonly"}},
                                    "rule": {"any": [{"kind": "identifier"}, {"kind": "shorthand_property_identifier"}], "regex": "^(?:" + "|".join(cjs_names) + ")$", "resolves": {"global": cjs_names}}, "file": not_esm}, "message": "CommonJS-scope binding"}]))
    docs.append(probe("probe-cjs-wrapper-scope", "Node fact: a CommonJS module body runs inside a function wrapper, so a top-level `var process` is module-local and shadows the global process (sourceType commonjs).", [
        {"id": "reference", "ast": {"languageOptions": {"sourceType": "commonjs", "globals": {"process": "readonly"}}, "rule": {"kind": "identifier", "regex": "^process$", "resolves": {"global": ["process"]}}, "file": not_esm}, "message": "global process"}]))
    script = probe("probe-script-global-scope", "Contrast to probe-cjs-wrapper-scope: in a classic script a top-level `var process` is the global variable itself (scriptDeclarations), so its reads are global reads.", [
        {"id": "reference", "ast": {"languageOptions": {"sourceType": "script", "globals": {"process": "readonly"}}, "rule": {"kind": "identifier", "regex": "^process$", "resolves": {"global": ["process"], "scriptDeclarations": True}}}, "message": "global process"}])
    script["scope"]["include"] = ["wrapper/**"]
    docs.append(script)
    node_globals = ["process", "Buffer", "global", "setImmediate", "clearImmediate", "queueMicrotask", "structuredClone"]
    docs.append(probe("probe-node-globals", "Node fact: process, Buffer, global, setImmediate, clearImmediate, queueMicrotask and structuredClone are Node globals (globals 17.13.0 `node`); declared through languageOptions.globals, a reference resolves to them unless shadowed.", [
        {"id": "reference", "ast": {"languageOptions": {"sourceType": "commonjs", "globals": {g: "readonly" for g in node_globals}}, "rule": {"kind": "identifier", "regex": "^(?:" + "|".join(node_globals) + ")$", "resolves": {"global": node_globals}}}, "message": "Node global"}]))
    docs.append(probe("probe-node-globals-undeclared", "The same names without Node's globals (plain ESLint module defaults): every unshadowed use is unresolved, so a pack must contribute them as data.", [
        {"id": "reference", "ast": {"languageOptions": {"sourceType": "module"}, "rule": {"kind": "identifier", "regex": "^(?:" + "|".join(node_globals) + ")$", "not": {"inside": {"kind": "member_expression", "field": "property"}}, "resolves": {"unresolved": True}}}, "message": "undeclared without Node globals"}]))
    nb_rx = "^(?:" + "|".join(re.escape(b).replace("\\/", "/").replace("\\:", ":") for b in builtins) + ")$"
    docs.append(probe("probe-builtin-module-specifier", "Node fact: builtin module names (eslint-plugin-n 18.4.1 NodeBuiltinModules: bare names, `node:` names and node:-only modules such as node:test and node:sqlite) as specifiers of require, process.getBuiltinModule, import/export declarations and import(); a value test, so template literals without substitutions count.", [
        {"id": "require", "ast": {"rule": {"any": [{"kind": "string"}, {"kind": "template_string", "not": {"has": {"kind": "template_substitution"}}}], "value": {"regex": nb_rx}, "nthChild": FIRST,
                                           "inside": {"kind": "arguments", "inside": {"field": "arguments", "kind": "call_expression", "has": {"field": "function", "any": [
                                               {"kind": "identifier", "regex": "^require$"}, {"kind": "import"}, {"kind": "member_expression", "has": {"field": "property", "regex": "^getBuiltinModule$"}}]}}}}}, "message": "builtin module"},
        {"id": "declaration", "ast": {"rule": {"kind": "string", "value": {"regex": nb_rx}, "inside": {"field": "source", "any": [{"kind": "import_statement"}, {"kind": "export_statement"}]}}}, "message": "builtin module"}]))
    esm_globals = {**{g: "readonly" for g in globals_node}, **{g: "off" for g in cjs_names}}
    docs.append(probe("probe-esm-require-unbound", "Node fact: an ES module (selected here by extension, .mjs/.mts) has no require, module, exports, __dirname or __filename (eslint-plugin-n's recommended-module turns them off over globals.node). Selection by the nearest package.json `type` is not expressible today.", [
        {"id": "reference", "ast": {"file": {"path": {"regex": "\\.m[jt]s$"}}, "languageOptions": {"sourceType": "module", "globals": esm_globals},
                                    "rule": {"kind": "identifier", "regex": "^(?:" + "|".join(cjs_names) + ")$", "resolves": {"unresolved": True}}}, "message": "not defined in an ES module"}]))
    return docs


# ---- main ------------------------------------------------------------------------------------------------------------

def settle(relative, doc, shipped):
    """Carries suppress.biome over from the committed document and sets the revision: the committed one when nothing
    else changed, one more for a shipped document that changed."""
    path = os.path.join(REPOSITORY, relative, f"{doc['id']}.json")
    if not os.path.exists(path):
        return
    with open(path) as handle:
        committed = json.load(handle)
    biome = {clause["id"]: clause["violation"]["suppress"]["biome"] for clause in committed["clauses"]
             if "biome" in clause.get("violation", {}).get("suppress", {})}
    for clause in doc["clauses"]:
        if clause["id"] in biome and "suppress" in clause["violation"]:
            clause["violation"]["suppress"]["biome"] = biome[clause["id"]]
    doc["revision"] = committed["revision"]
    if shipped and doc != committed:
        doc["revision"] += 1


def write(root, relative, doc, compact=False, shipped=False):
    settle(relative, doc, shipped)
    path = os.path.join(root, relative, f"{doc['id']}.json")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    text = json.dumps(doc, separators=(",", ":")) if compact else json.dumps(doc, indent=2)
    with open(path, "w") as handle:
        handle.write(text + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--tools")
    parser.add_argument("--root", default=REPOSITORY)
    args = parser.parse_args()
    builtins, globals_node, trace = load_data(os.path.abspath(args.tools or nearest_tools()))
    for doc in syntax_docs() + prefer_global_docs() + prefer_promises_docs() + [no_path_concat_doc(), prefer_node_protocol_doc(builtins)]:
        write(args.root, os.path.join(PACK, "rules"), doc, shipped=True)
    write(args.root, os.path.join(PACK, "candidates", "rules"), no_deprecated_api_doc(trace), compact=True)
    write(args.root, os.path.join(PACK, "candidates", "rules"), hashbang_doc())
    for doc in probe_docs(builtins, globals_node):
        write(args.root, os.path.join(PACK, "candidates", "probes", "rules"), doc)


if __name__ == "__main__":
    main()
