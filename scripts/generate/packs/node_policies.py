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
- packs/node/rules/<id>.json: the ports in SHIPPED, the ones that pass the pack's conformance check on eslint-plugin-n's
  tests and on the Node test set (TEST_SETS in scripts/s3/pack-fixtures.mjs); each one's candidates/rules copy removed;
- packs/node/candidates/rules/<id>.json: every other port. A candidate that passes joins SHIPPED, and the other way;
- packs/node/candidates/probes/rules/<id>.json: the seven probes.
The pack manifests, recordings and the probes' hand-labelled expectations are not written here.

Each port's violations name its rule under suppress.eslint and, for a rule oxlint 1.88.0 implements (OXLINT_NODE_RULES),
under suppress.oxlint. suppress.biome belongs to scripts/generate/packs/biome-suppression.mjs: this script carries each
violation's suppress.biome over from the committed document, and biome-suppression.mjs, run after it, updates it.
A document keeps its committed revision when nothing else in it changes; a shipped one (packs/node/rules) that changes
bumps it, as biome-suppression.mjs does. A document with no committed version starts at revision 1.

The utilities emulate @eslint-community/eslint-utils' ReferenceTracker (legacy mode) and ESTree shapes over
tree-sitter's. Each port's description (DESCRIPTIONS) is the steer an agent reads, written by hand. Some helpers exist in two spellings (e.g. a string constant matched by
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
# The ports that pass the pack's conformance check on eslint-plugin-n's tests and on the Node test set, written to
# packs/node/rules/ (n-no-process-exit, hand-written, ships too); every other port is written to
# packs/node/candidates/rules/. Each is removed from the other directory, so promoting a port moves its document.
SHIPPED = {"n-exports-style", "n-global-require", "n-handle-callback-err", "n-no-callback-literal", "n-no-mixed-requires",
           "n-no-new-require", "n-no-process-env", "n-no-sync", "n-prefer-global-console", "n-prefer-global-text-decoder",
           "n-prefer-global-text-encoder", "n-prefer-global-url", "n-prefer-global-url-search-params",
           "n-prefer-node-protocol", "n-prefer-promises-dns", "n-prefer-promises-fs"}

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
                              "violation": {"anchor": check.get("anchor", "witness.span"), "code": upstream, "message": check["message"],
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
# No sourceType: each file keeps its own module system (the engine reads it from the extension and package.json).
FILE_LO = {}
NODE_LO = {"globals": {"process": "readonly"}}
# Each port's description: the steer an agent reads.
DESCRIPTIONS = {
    "n-callback-return": "Return right after calling a callback named callback, cb or next; otherwise the function keeps running and the callback can fire twice.",
    "n-exports-style": "Use module.exports instead of the bare exports shortcut in CommonJS files, for one consistent export style.",
    "n-global-require": "Keep require() calls at the top level of a module, not inside functions or blocks; nested requires hide dependencies and load synchronously mid-run.",
    "n-handle-callback-err": "Handle the err parameter of error-first callbacks instead of leaving it unused; ignoring it silently swallows failures.",
    "n-hashbang": "Start the files that package.json lists as bin executables with a node shebang line, and leave the shebang off every other file.",
    "n-no-callback-literal": "Pass an Error or null, not a string, number, array or object literal, as the first argument of error-first callbacks named callback or cb.",
    "n-no-deprecated-api": "Avoid deprecated Node.js APIs, such as new Buffer() or the domain module; they may be removed and usually have safer replacements.",
    "n-no-exports-assign": "Don't assign to exports in CommonJS files; it breaks the link to module.exports, so assign to module.exports instead.",
    "n-no-mixed-requires": "Keep require() declarations separate from other variable declarations instead of mixing them in one statement.",
    "n-no-new-require": "Don't use new directly on a require() call; require the module into a variable first, then construct it, for readability.",
    "n-no-path-concat": "Use path.join() or path.resolve() instead of concatenating __dirname, __filename or import.meta paths with strings; hand-built paths break across platforms.",
    "n-no-process-env": "Avoid reading process.env directly; pass configuration in explicitly so dependencies on the environment stay visible and testable.",
    "n-no-sync": "Avoid calling methods whose name ends in Sync, such as readFileSync; they block the Node.js event loop.",
    "n-prefer-global-buffer": "Use the global Buffer instead of importing or requiring it from the buffer module; it is always available in Node.js.",
    "n-prefer-global-console": "Use the global console instead of importing or requiring the console module; it is always available in Node.js.",
    "n-prefer-global-process": "Use the global process instead of importing or requiring the process module; it is always available in Node.js.",
    "n-prefer-global-text-decoder": "Use the global TextDecoder instead of importing or requiring it from the util module; it is always available in Node.js.",
    "n-prefer-global-text-encoder": "Use the global TextEncoder instead of importing or requiring it from the util module; it is always available in Node.js.",
    "n-prefer-global-url": "Use the global URL instead of importing or requiring it from the url module; it is always available in Node.js.",
    "n-prefer-global-url-search-params": "Use the global URLSearchParams instead of importing or requiring it from the url module; it is always available in Node.js.",
    "n-prefer-node-protocol": "Load Node.js built-in modules with the node: prefix, such as node:fs, when the project's Node versions support it; it marks them as built-ins npm packages can't shadow.",
    "n-prefer-promises-dns": "Use the promise-based dns API instead of the callback-style dns functions and Resolver, for cleaner async code.",
    "n-prefer-promises-fs": "Use the promise-based fs API instead of callback-style fs functions such as readFile and writeFile, for cleaner async code.",
}


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

# An aliasing branch below reads every occurrence of a binding, once per reference it tests: quadratic in a file that
# references an imported or const function thousands of times. No running program reassigns an import or a const, so
# for the n/prefer-promises ports, which read every call, those bindings skip the branch.
NOT_REASSIGNABLE = [{"not": {"resolves": {"binding": {"form": "import"}}}},
                    {"not": {"resolves": {"binding": {"site": {"kind": "variable_declarator", "has": {"field": "name", "kind": "identifier"},
                                                               "inside": {"kind": "lexical_declaration", "regex": "^const\\b"}}}}}}]


def occurrence_branch(occurrence, guarded):
    """An identifier whose binding has an `occurrence`; `guarded` skips bindings no running program reassigns."""
    return {"kind": "identifier", **({"all": NOT_REASSIGNABLE} if guarded else {}), "resolves": {"binding": {"occurrence": occurrence}}}


def module_utils(rx, guarded=False):
    """module-call: require(M) with the global require or process.getBuiltinModule(M) with the global process;
    module-value: that call or a constant alias, an ESM default/namespace import (and its .default). `guarded`: see
    `NOT_REASSIGNABLE`."""
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
                                 occurrence_branch({"inside": {"kind": "assignment_expression", "field": "left", "has": {"field": "right", "matches": "module-value"}}}, guarded),
                                 {"matches": "esm-ref"},
                                 {"kind": "member_expression", "all": [{"has": {"field": "object", "matches": "esm-ref"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^default$"}}]}]},
        "esm-source": {"kind": "string", "has": {"kind": "string_fragment", "regex": rx}},
        "esm-whole-module": {"any": [{"kind": "namespace_import"}, {"kind": "import_clause"}, {"kind": "identifier", "inside": {"kind": "import_clause"}}],
                             "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", "matches": "esm-source"}}},
        "esm-ref": {"kind": "identifier", "resolves": {"binding": {"site": {"matches": "esm-whole-module"}}}},
    }


def prefer_global_checks(rx, prop, mod):
    utils = module_utils(rx)
    if prop is None:
        return [{"id": "module-call", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"matches": "module-call"}}, "message": f"Use the global {mod} instead of requiring the {mod} module."},
                {"id": "esm-declaration", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"any": [{"kind": "import_statement"}, {"kind": "export_statement"}], "has": {"field": "source", "matches": "esm-source"}}}, "message": f"Use the global {mod} instead of importing or re-exporting the {mod} module."}]
    prx = f"^{prop}$"
    pattern = {"kind": "object_pattern", "any": [
        {"inside": {"kind": "variable_declarator", "field": "name", "has": {"field": "value", "matches": "module-value"}}},
        {"inside": {"kind": "assignment_expression", "field": "left", "has": {"field": "right", "matches": "module-value"}}},
        {"inside": {"kind": "assignment_pattern", "field": "left", "has": {"field": "right", "matches": "module-value"}}}]}
    return [
        {"id": "member", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"any": [
            {"kind": "member_expression", "all": [{"has": {"field": "object", "matches": "module-value"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": prx}}]},
            {"kind": "subscript_expression", "all": [{"has": {"field": "object", "matches": "module-value"}}, {"has": {"field": "index", **str_const(prx)}}]}]}}, "message": f"Use the global {prop} instead of reading {prop} from the {mod} module."},
        {"id": "destructured", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"inside": pattern, "any": [
            {"kind": "shorthand_property_identifier_pattern", "regex": prx},
            {"kind": "pair_pattern", "has": {"field": "key", "any": [{"kind": "property_identifier", "regex": prx}, str_const(prx), {"kind": "computed_property_name", "has": str_const(prx)}]}},
            {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": prx}}]}}, "message": f"Drop {prop} from this destructuring and use the global {prop} instead."},
        {"id": "esm-specifier", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"any": [
            {"kind": "import_specifier", "has": {"field": "name", "regex": prx}, "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", "matches": "esm-source"}}},
            {"kind": "export_specifier", "has": {"field": "name", "regex": prx}, "inside": {"stopBy": "end", "kind": "export_statement", "has": {"field": "source", "matches": "esm-source"}}}]}}, "message": f"Remove {prop} from this import or export and use the global {prop} instead."},
        {"id": "esm-export-all", "ast": {"languageOptions": NODE_LO, "utils": utils, "rule": {"kind": "export_statement", "has": {"field": "source", "matches": "esm-source"}, "not": {"has": {"kind": "export_clause"}}}}, "message": f"Re-export only the names you need from the {mod} module; {prop} is already a global."},
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
        docs.append(hoist(policy(f"n-prefer-global-{name}", f"n/prefer-global/{name}", DESCRIPTIONS[f"n-prefer-global-{name}"], prefer_global_checks(rx, prop, mod))))
    return docs


def fn_utils(rx_mod, rx_fn, guarded=False):
    """fn-value: a value naming member rx_fn of the module: a member, a constant alias, a destructured property or an
    import specifier. `guarded`: see `NOT_REASSIGNABLE`."""
    utils = module_utils(rx_mod, guarded)
    utils["fn-value"] = {"any": [member_object_first("module-value", rx_fn),
                                 {"kind": "parenthesized_expression", "has": {"matches": "fn-value"}},
                                 {"kind": "identifier", "resolves": {"binding": {"destructuredValue": False, "value": {"matches": "fn-value"}}}},
                                 occurrence_branch({"any": [
                                     {"kind": "shorthand_property_identifier_pattern", "regex": rx_fn, "inside": pattern_from("module-value")},
                                     {"kind": "identifier", "inside": {"kind": "pair_pattern", "field": "value", "has": {"field": "key", **key_named(rx_fn)}, "inside": pattern_from("module-value")}},
                                     {"kind": "identifier", "inside": {"kind": "object_assignment_pattern", "has": {"field": "left", "kind": "shorthand_property_identifier_pattern", "regex": rx_fn}, "inside": pattern_from("module-value")}}]}, guarded),
                                 {"kind": "identifier", "resolves": {"binding": {"site": {"kind": "import_specifier", "has": {"field": "name", "regex": rx_fn},
                                                                                          "inside": {"stopBy": "end", "kind": "import_statement", "has": {"field": "source", "matches": "esm-source"}}}}}}]}
    return utils


# Upstream also reports `watch`; Rerail does not (declared in packs/node/divergences.json): fs.promises.watch
# returns an async iterator of change events, not an FSWatcher, so it is not a drop-in replacement.
FS_FNS = "access copyFile open rename truncate rmdir mkdir readdir readlink symlink lstat stat link unlink chmod lchmod lchown chown utimes realpath mkdtemp writeFile appendFile readFile cp glob lutimes opendir rm statfs".split()
DNS_CALL = "lookup lookupService getServers resolve resolve4 resolve6 resolveAny resolveCname resolveMx resolveNaptr resolveNs resolvePtr resolveSoa resolveSrv resolveTxt reverse setServers".split()


def plain_alt(names):
    return "^(?:" + "|".join(names) + ")$"


def promises_utils(mod, rx_fn):
    """`fn_utils` for an n/prefer-promises port, guarded, with file-names-module: a file holding a string naming the
    module, which every call the port reports needs (a needle, `mod` or an escape, anchors the walk)."""
    return {"file-names-module": {"kind": "program", "has": {
        "stopBy": "end", "any": [{"kind": "string"}, {"kind": "template_string", "not": {"has": {"kind": "template_substitution"}}}],
        "regex": f"{mod}|\\\\", "value": {"regex": f"^(?:node:)?{mod}$"}}},
        **fn_utils(f"^(?:node:)?{mod}$", rx_fn, guarded=True)}


IN_FILE_NAMING_MODULE = {"inside": {"stopBy": "end", "matches": "file-names-module"}}


def prefer_promises_docs():
    docs = []
    for mod, calls, constructs in (("fs", FS_FNS, []), ("dns", DNS_CALL, ["Resolver"])):
        checks = [{"id": "call", "ast": {"languageOptions": NODE_LO, "utils": promises_utils(mod, plain_alt(calls)),
                                         "rule": {"kind": "call_expression", **IN_FILE_NAMING_MODULE, **NOT_TAGGED, "has": {"field": "function", "matches": "fn-value"}}}, "message": f"Use the {mod}.promises version of this function and await it instead of passing a callback."}]
        if constructs:
            checks.append({"id": "construct", "ast": {"languageOptions": NODE_LO, "utils": promises_utils(mod, plain_alt(constructs)),
                                                      "rule": {"kind": "new_expression", **IN_FILE_NAMING_MODULE, "has": {"field": "constructor", "matches": "fn-value"}}}, "message": f"Construct {mod}.promises.Resolver instead of the callback-style {mod}.Resolver."})
        docs.append(policy(f"n-prefer-promises-{mod}", f"n/prefer-promises/{mod}", DESCRIPTIONS[f"n-prefer-promises-{mod}"], checks))
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
    # __dirname/__filename are CommonJS wrapper bindings: declared only where the file is CommonJS.
    commonjs = {"when": {"sourceType": "commonjs"}, "languageOptions": {"globals": {"process": "readonly", "__dirname": "readonly", "__filename": "readonly"}}}
    other = {"when": {"sourceType": ["script", "module"]}, "languageOptions": NODE_LO}
    checks = []
    for suffix, options in (("", other), ("-commonjs", commonjs)):
        checks += [
            {"id": "binary" + suffix, "ast": {**options, "utils": utils, "rule": {"kind": "binary_expression", "all": [op("^\\+$"), {"has": {"field": "left", "matches": "path-source"}}, {"has": {"field": "right", "matches": "first-slash"}}]}}, "message": "Build this path with path.join() or path.resolve() instead of concatenating strings."},
            {"id": "template" + suffix, "ast": {**options, "utils": utils, "rule": {"kind": "template_string", "has": {"kind": "template_substitution", "has": {"matches": "path-source"},
                                                                                                                       "precedes": {"any": [slash_text, {"kind": "template_substitution", "has": {"matches": "first-slash"}}]}}}}, "message": "Build this path with path.join() or path.resolve() instead of a template string."}]
    return hoist(policy("n-no-path-concat", "n/no-path-concat", DESCRIPTIONS["n-no-path-concat"], checks))


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
    # isEnablingThisRule: the configured Node range must be a subset of supportedRangeForEsm, and for require also of
    # supportedRangeForCjs, which lies inside it (so the require gate is that range alone); getBuiltinModule is ungated.
    esm = {"package": {"nodeVersion": {"subsetOf": "^12.20.0 || >=14.13.1"}}}
    cjs = {"package": {"nodeVersion": {"subsetOf": "^14.18.0 || >=16.0.0"}}}
    checks = [
        {"id": "require", "ast": {"file": cjs, "rule": {**builtin, "nthChild": FIRST, "inside": {"kind": "arguments", "not": {"has": {"nthChild": {"position": 2, "ofRule": NOT_COMMENT}}}, "inside": {"field": "arguments", **require_call}}}}, "message": message},
        {"id": "get-builtin-module", "ast": {"rule": {**builtin, **first_argument_of(get_builtin_module)}}, "message": message},
        {"id": "import-export-source", "ast": {"file": esm, "rule": {**builtin, "inside": {"field": "source", "any": [{"kind": "import_statement"}, {"kind": "export_statement"}]}}}, "message": message},
        {"id": "import-expression", "ast": {"file": esm, "rule": {**builtin, **first_argument_of(import_call)}}, "message": message}]
    return policy("n-prefer-node-protocol", "n/prefer-node-protocol", DESCRIPTIONS["n-prefer-node-protocol"], checks)


# ---- hand-shaped syntax rules ----------------------------------------------------------------------------------------

def syntax_docs():
    docs = []
    docs.append(policy("n-no-new-require", "n/no-new-require", DESCRIPTIONS["n-no-new-require"], [
        {"id": "upstream", "ast": {"rule": {"kind": "new_expression", "has": {"field": "constructor", "kind": "identifier", "regex": "^require$"}}}, "message": "Require the module into a variable first, then call new on that variable."}]))
    docs.append(policy("n-no-process-env", "n/no-process-env", DESCRIPTIONS["n-no-process-env"], [
        {"id": "upstream", "ast": {"rule": {"any": [
            {"kind": "member_expression", "all": [{"has": {"field": "object", "kind": "identifier", "regex": "^process$"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^env$"}}]},
            {"kind": "subscript_expression", "all": [{"has": {"field": "object", "kind": "identifier", "regex": "^process$"}}, {"has": {"field": "index", "kind": "string", "has": {"kind": "string_fragment", "regex": "^env$"}}}]}]}}, "message": "Pass this value in as configuration instead of reading process.env here."}]))

    accept = ["assignment_expression", "augmented_assignment_expression", "variable_declarator", "member_expression", "subscript_expression", "expression_statement", "call_expression",
              "ternary_expression", "program", "lexical_declaration", "variable_declaration", "arguments", "parenthesized_expression"]
    chain = {"any": [{"has": {"kind": "optional_chain"}},
                     {"inside": {"stopBy": "end", "any": [{"kind": "call_expression"}, {"kind": "member_expression"}, {"kind": "subscript_expression"}],
                                 "has": {"stopBy": {"any": [{"kind": "arguments"}, {"kind": "parenthesized_expression"}, {"kind": "template_string"}]}, "kind": "optional_chain"}}}]}
    docs.append(policy("n-global-require", "n/global-require", DESCRIPTIONS["n-global-require"], [
        {"id": "upstream", "ast": {"languageOptions": FILE_LO, "rule": {"kind": "call_expression",
                                                                                         "has": {"field": "function", "kind": "identifier", "regex": "^require$", "resolves": {"global": ["require"], "declared": False}},
                                                                                         "any": [{"inside": {"stopBy": "end", "not": {"any": [{"kind": k} for k in accept]}}}, chain]}}, "message": "Move this require() to the top level of the module."}]))

    module_exports = {"kind": "member_expression", "all": [{"has": {**ident("module"), "field": "object"}}, {"has": {"field": "property", "kind": "property_identifier", "regex": "^exports$"}}]}
    docs.append(policy("n-no-exports-assign", "n/no-exports-assign", DESCRIPTIONS["n-no-exports-assign"], [
        {"id": "upstream", "ast": {"languageOptions": FILE_LO, "rule": {"any": [{"kind": "assignment_expression"}, {"kind": "augmented_assignment_expression"}],
                                                                    "has": {**ident("exports"), "field": "left"},
                                                                    "not": {"any": [
                                                                        {"inside": {"kind": "assignment_expression", "all": [{"has": {"field": "left", **module_exports}}]}, "nthChild": {"position": 1, "reverse": True}},
                                                                        {"has": {"field": "right", "kind": "assignment_expression", "has": {"field": "left", **module_exports}}}]}}},
         "message": "Assign to module.exports instead of exports, which only rebinds a local variable."}]))

    sync_id = {"any": [{"kind": "identifier"}, {"kind": "property_identifier"}], "regex": "Sync$"}
    callee_member = {"kind": "member_expression", "inside": {"kind": "call_expression", "field": "function", **NOT_TAGGED}}
    call_child = {"kind": "call_expression", **NOT_TAGGED, "any": [{"has": {"field": "function", **sync_id}}, {"has": {"field": "arguments", "kind": "arguments", "has": sync_id}}]}
    docs.append(policy("n-no-sync", "n/no-sync", DESCRIPTIONS["n-no-sync"], [
        {"id": "call-child", "ast": {"rule": call_child}, "message": "Replace this Sync call with its async or promise-based version, such as fs.promises.readFile() for readFileSync()."},
        {"id": "callee-member-descendant", "ast": {"rule": {"has": sync_id, "any": [callee_member, {"inside": {"stopBy": "end", **callee_member}}], "not": {"any": [{"kind": "arguments"}, call_child]}}}, "message": "Replace this Sync call with its async or promise-based version, such as fs.promises.readFile() for readFileSync()."}]))

    require_init = {"require-init": {"not": {"has": {"kind": "optional_chain"}}, "any": [
        {"kind": "call_expression", **NOT_TAGGED, "has": {"field": "function", "kind": "identifier", "regex": "^require$"}},
        {"any": [{"kind": "member_expression"}, {"kind": "subscript_expression"}], "has": {"field": "object", "matches": "require-init"}},
        {"kind": "parenthesized_expression", "has": {"matches": "require-init"}}]}}
    docs.append(policy("n-no-mixed-requires", "n/no-mixed-requires", DESCRIPTIONS["n-no-mixed-requires"], [
        {"id": "upstream", "ast": {"utils": require_init, "rule": {"any": [{"kind": "variable_declaration"}, {"kind": "lexical_declaration"}], "all": [
            {"has": {"kind": "variable_declarator", "has": {"field": "value", "matches": "require-init"}}},
            {"has": {"kind": "variable_declarator", "not": {"has": {"field": "value", "matches": "require-init"}}}}]}}, "message": "Do not mix 'require' and other declarations."}]))

    docs.append(policy("n-exports-style", "n/exports-style", DESCRIPTIONS["n-exports-style"], [
        {"id": "upstream", "ast": {"languageOptions": FILE_LO, "rule": {"any": [{"kind": "identifier"}, {"kind": "shorthand_property_identifier"}], "regex": "^exports$", "resolves": {"global": ["exports"]}}}, "message": "Replace this bare exports with module.exports."}]))

    err_id = {"kind": "identifier", "regex": "^err$", "binds": {"form": "parameter", "referenced": False}}
    first_err = {"any": [err_id,
                         {"any": [{"kind": "required_parameter"}, {"kind": "optional_parameter"}], "has": {"field": "pattern", "any": [err_id, {"kind": "rest_pattern", "has": err_id}]}},
                         {"kind": "assignment_pattern", "has": {"field": "left", **err_id}},
                         {"kind": "rest_pattern", "has": err_id}]}
    fn_kinds = ["function_declaration", "function_expression", "generator_function_declaration", "generator_function", "arrow_function"]
    docs.append(policy("n-handle-callback-err", "n/handle-callback-err", DESCRIPTIONS["n-handle-callback-err"], [
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
    docs.append(policy("n-no-callback-literal", "n/no-callback-literal", DESCRIPTIONS["n-no-callback-literal"], [
        {"id": "upstream", "ast": {"utils": {"cannot-be-error": cannot_be_error}, "rule": {"kind": "call_expression", **NOT_TAGGED, "all": [
            {"has": {"field": "function", "kind": "identifier", "regex": "^(?:callback|cb)$"}},
            {"has": {"field": "arguments", "kind": "arguments", "has": {"nthChild": FIRST, "matches": "cannot-be-error"}}}]}}, "message": "Pass an Error or null as this callback's first argument, unless its API expects a different first argument."}]))

    stop = {"any": [{"kind": "statement_block"}, {"kind": "return_statement"}, {"kind": "arrow_function"}]}
    any_func = {"any": [{"kind": k} for k in ["function_declaration", "function_expression", "generator_function_declaration", "generator_function", "arrow_function", "method_definition"]]}

    def statement_of_call(extra):
        return {"any": [{"inside": {"kind": "expression_statement", **extra}},
                        {"inside": {"kind": "binary_expression", "field": "right", "inside": {"kind": "expression_statement", **extra}}}]}

    ok_last = statement_of_call({"nthChild": LAST, "inside": {"kind": "statement_block", "inside": {**any_func, "field": "body"}}})
    ok_before_return = statement_of_call({"nthChild": SECOND_LAST, "inside": {"kind": "statement_block", "has": {"kind": "return_statement", "nthChild": LAST}}})
    docs.append(policy("n-callback-return", "n/callback-return", DESCRIPTIONS["n-callback-return"], [
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


# n-no-deprecated-api: the message of the clause reporting each root of the trace map (a module, or `global <name>`).
DEPRECATED_API_MESSAGES = {
    "_linklist": "Replace the removed internal _linklist module with your own linked list or an npm package.",
    "_stream_wrap": "Replace the internal _stream_wrap module with the public stream or net APIs.",
    "async_hooks": "Replace async_hooks currentId or triggerId with executionAsyncId() or triggerAsyncId().",
    "buffer": "Replace this Buffer constructor or SlowBuffer with Buffer.from(), Buffer.alloc() or Buffer.allocUnsafeSlow().",
    "constants": "Replace the constants module with the module's own constants, such as os.constants or fs.constants.",
    "crypto": "Replace this deprecated crypto API with a supported one, such as createCipheriv() or randomBytes().",
    "domain": "Replace the domain module with try/catch, promise rejection handling or AsyncLocalStorage.",
    "events": "Call emitter.listenerCount(name) instead of the static listenerCount(emitter, name).",
    "freelist": "Replace the removed freelist module with your own object pool or an npm package.",
    "fs": "Replace this deprecated fs API with a supported one, such as fs.access() or fs.stat() for fs.exists().",
    "http": "Replace http.createClient() with http.request() or http.get().",
    "module": "Replace this deprecated module API with a supported one, such as createRequire() for createRequireFromPath().",
    "net": "Remove this call to the internal net._setSimultaneousAccepts(); Node.js no longer supports it.",
    "os": "Replace os.getNetworkInterfaces() or os.tmpDir() with os.networkInterfaces() or os.tmpdir().",
    "path": "Replace path._makeLong() with path.toNamespacedPath().",
    "process": "Replace this deprecated process API with a supported one, such as the assert module for process.assert().",
    "punycode": "Replace the built-in punycode module with the punycode npm package or url.domainToASCII().",
    "readline": "Replace this internal readline helper with a public API, such as util.stripVTControlCharacters().",
    "repl": "Replace this deprecated repl API with a supported one, such as module.builtinModules for repl.builtinModules.",
    "safe-buffer": "Replace this safe-buffer constructor or SlowBuffer with the built-in Buffer.from() or Buffer.alloc().",
    "sys": "Require util instead of sys, a deprecated alias for it.",
    "timers": "Replace timers.enroll() or timers.unenroll() with setTimeout() or clearTimeout().",
    "tls": "Replace this deprecated tls API with a supported one, such as tls.TLSSocket for createSecurePair().",
    "tty": "Replace tty.setRawMode() with setRawMode() on the TTY stream, such as process.stdin.setRawMode().",
    "url": "Replace url.parse() or url.resolve() with the WHATWG URL API, such as new URL(input, base).",
    "util": "Replace this deprecated util function with its modern equivalent, such as Array.isArray() for util.isArray().",
    "vm": "Replace vm.runInDebugContext() with the inspector module.",
    "zlib": "Construct this zlib class with new, or use its factory such as zlib.createGzip(), instead of calling it bare.",
    "global Buffer": "Replace this Buffer constructor call with Buffer.from(), Buffer.alloc() or Buffer.allocUnsafe().",
    "global COUNTER_NET_SERVER_CONNECTION": "Remove COUNTER_NET_SERVER_CONNECTION, a global Node.js no longer provides; use diagnostics_channel for these metrics.",
    "global COUNTER_NET_SERVER_CONNECTION_CLOSE": "Remove COUNTER_NET_SERVER_CONNECTION_CLOSE, a global Node.js no longer provides; use diagnostics_channel for these metrics.",
    "global COUNTER_HTTP_SERVER_REQUEST": "Remove COUNTER_HTTP_SERVER_REQUEST, a global Node.js no longer provides; use diagnostics_channel for these metrics.",
    "global COUNTER_HTTP_SERVER_RESPONSE": "Remove COUNTER_HTTP_SERVER_RESPONSE, a global Node.js no longer provides; use diagnostics_channel for these metrics.",
    "global COUNTER_HTTP_CLIENT_REQUEST": "Remove COUNTER_HTTP_CLIENT_REQUEST, a global Node.js no longer provides; use diagnostics_channel for these metrics.",
    "global COUNTER_HTTP_CLIENT_RESPONSE": "Remove COUNTER_HTTP_CLIENT_RESPONSE, a global Node.js no longer provides; use diagnostics_channel for these metrics.",
    "global GLOBAL": "Replace GLOBAL with globalThis.",
    "global Intl": "Replace Intl.v8BreakIterator with Intl.Segmenter.",
    "global require": "Remove this use of require.extensions; precompile the files or use a module loader hook instead.",
    "global root": "Replace root with globalThis.",
    "global process": "Replace this deprecated process API with a supported one, such as the assert module for process.assert().",
}


def no_deprecated_api_doc(trace):
    utils, report = tracker(trace["modules"], trace["globals"])
    # `global` is Node's (globals.node) in both module systems; require only CommonJS's, as the file decides.
    lo = {"globals": {"Buffer": "readonly", "global": "readonly", "process": "readonly"}}
    checks = [{"id": "trace-map", "define": {"language": "typescript", "utils": utils}}]
    for root, alts in report.items():
        checks.append({"id": re.sub(r"[^a-z0-9]+", "-", root.lower()).strip("-"),
                       "forbid": {"ast": {"language": "typescript", "languageOptions": lo, "rule": {"any": alts}}},
                       "violation": {"anchor": "witness.span", "code": "n/no-deprecated-api", "message": DEPRECATED_API_MESSAGES[root],
                                    "suppress": suppress("n/no-deprecated-api")}})
    return {"format": "rerail.policy/v1", "id": "n-no-deprecated-api", "revision": 1, "mode": "steer", "description": DESCRIPTIONS["n-no-deprecated-api"],
            "scope": {"include": INCLUDE, "exclude": []}, "clauses": checks}


def node_shebang_rx():
    """The text upstream's getShebangInfo reads as a shebang and isNodeShebang (default executableMap) accepts:
    `#!/usr/bin/env`, then what its loop strips (ENV_FLAGS `-(-.*?\\b|[ivS]+|[Pu](\\s+|=)\\S+)(?=\\s|$)`, ENV_VARS
    `\\w+=("…"|\\w+)`, each after JavaScript whitespace), then `node` before a space or the trimmed end, then the line
    break. Without lookaround: a flag also takes the whitespace its lookahead requires, and an unquoted variable is
    followed by whitespace or a flag, as the greedy strip leaves it."""
    line = "\\r\\n\\x{2028}\\x{2029}"
    blank = "\\t\\x0B\\x0C \\x{A0}\\x{1680}\\x{2000}-\\x{200A}\\x{202F}\\x{205F}\\x{3000}\\x{FEFF}"
    ws, nonspace, word = f"[{blank}]", f"[^{blank}{line}]", "[A-Za-z0-9_]"
    flag = f"(?:--(?:[^A-Za-z0-9_{line}]|{word}+[^A-Za-z0-9_{blank}{line}])*{word}+|-[ivS]+|-[Pu](?:{ws}+|=){nonspace}+){ws}"
    quoted = f"{word}+=\"(?:[^\"\\\\{line}]|\\\\[^{line}])*\""
    unquoted = f"{word}+={word}+"
    token = f"(?:{quoted}|(?:{unquoted})?{flag}|{unquoted}{ws})"
    return f"\\A#!/usr/bin/env(?:{ws}*{token})*{ws}*node(?: [^{line}]*|{ws}*)\\r?\\n"


def hashbang_doc():
    # getShebangInfo: `#!` and at least one character at the start of the text, ended by a line break.
    shebang = {"kind": "program", "regex": "\\A#![^\\r\\n\\x{2028}\\x{2029}]+\\r?\\n"}
    return policy("n-hashbang", "n/hashbang", DESCRIPTIONS["n-hashbang"], [
        {"id": "needs-no-shebang", "ast": {"file": {"package": {"bin": False}}, "rule": {"kind": "hash_bang_line", "inside": shebang}}, "message": "Remove the shebang line; package.json does not list this file as a bin executable."},
        {"id": "needs-shebang", "anchor": "file.start", "ast": {"file": {"package": {"bin": True}}, "rule": {"kind": "program", "not": {"regex": node_shebang_rx()}}}, "message": "Add #!/usr/bin/env node as the first line; package.json lists this file as a bin executable."}])


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

def settle(relative, doc, shipped, other=None):
    """Carries suppress.biome over from the committed document (under `relative`, else under `other` for a port that
    moves) and sets the revision: the committed one when nothing else changed, one more for a shipped document that
    changed."""
    paths = [os.path.join(REPOSITORY, directory, f"{doc['id']}.json") for directory in (relative, other) if directory]
    path = next((path for path in paths if os.path.exists(path)), None)
    if path is None:
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


def write(root, relative, doc, compact=False, shipped=False, other=None):
    settle(relative, doc, shipped, other)
    path = os.path.join(root, relative, f"{doc['id']}.json")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    text = json.dumps(doc, separators=(",", ":")) if compact else json.dumps(doc, indent=2)
    with open(path, "w") as handle:
        handle.write(text + "\n")
    if other and os.path.exists(os.path.join(root, other, f"{doc['id']}.json")):
        os.remove(os.path.join(root, other, f"{doc['id']}.json"))


def write_port(root, doc, compact=False):
    """Writes a port to packs/node/rules when SHIPPED lists it, else to packs/node/candidates/rules."""
    rules, candidates = os.path.join(PACK, "rules"), os.path.join(PACK, "candidates", "rules")
    if doc["id"] in SHIPPED:
        write(root, rules, doc, compact, shipped=True, other=candidates)
    else:
        write(root, candidates, doc, compact, other=rules)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--tools")
    parser.add_argument("--root", default=REPOSITORY)
    args = parser.parse_args()
    builtins, globals_node, trace = load_data(os.path.abspath(args.tools or nearest_tools()))
    for doc in syntax_docs() + prefer_global_docs() + prefer_promises_docs() + [no_path_concat_doc(), prefer_node_protocol_doc(builtins)]:
        write_port(args.root, doc)
    write_port(args.root, no_deprecated_api_doc(trace), compact=True)
    write_port(args.root, hashbang_doc())
    for doc in probe_docs(builtins, globals_node):
        write(args.root, os.path.join(PACK, "candidates", "probes", "rules"), doc)


if __name__ == "__main__":
    main()
