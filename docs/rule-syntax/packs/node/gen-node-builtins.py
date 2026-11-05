"""Generate n-no-unsupported-features-node-builtins.facts.rr from the policy JSON. Run: python3 gen-node-builtins.py

The JSON (packs/node/rules/n-no-unsupported-features-node-builtins.json) is eslint-plugin-n 18.4.1's trace map for
n/no-unsupported-features/node-builtins, lowered by scripts/generate/packs/node_policies.py (unsupported_builtins_doc)
to one clause per supported-version list. This script inverts that lowering: it walks each clause's alternatives,
recognises the reference form each one is (member, destructured key, import or export specifier, module load, global,
call, new), and recovers which API it reports, how, and the clause's versions. Then it writes one row per API.

API names, as lib/tsx/values.rr spells them:
  "fs"  "node:fs"           a module, loaded by that specifier (how "load"): require, getBuiltinModule, import, export
  "fs#promises.readFile"    a member of a module: module, `#`, then the dotted member path (as names.rr's "m#name")
  "fetch"  "crypto.subtle"  a global and its members
  "import.meta.dirname"     a member of import.meta
A module that Node accepts with and without node: has one member trie, under its bare name; a module only one
spelling names (node:test, node:sqlite, node:sea, node:_http_agent, ...) keeps that spelling.
"""
import json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
JSON = os.path.normpath(os.path.join(HERE, "..", "..", "..", "..", "packs", "node", "rules",
                                     "n-no-unsupported-features-node-builtins.json"))
COMPACT = re.compile(r"^c\d+$")   # utilities compact_doc hoisted out (shared sub-terms), inlined here


def names_of(rx):
    """'^(?:a|b\\.c)$' or '^a$' -> ['a', 'b.c'] (alt_rx / esc of the generator, undone)."""
    m = re.fullmatch(r"\^\(\?:(.*)\)\$", rx) or re.fullmatch(r"\^(.*)\$", rx)
    assert m, rx
    return [re.sub(r"\\(.)", r"\1", p) for p in m.group(1).split("|")]


def spec_names(rx):
    """'^(?:node:)?fs$' -> ['fs', 'node:fs']; '^node:test$' -> ['node:test']."""
    m = re.fullmatch(r"\^\(\?:node:\)\?(.*)\$", rx)
    if m:
        b = re.sub(r"\\(.)", r"\1", m.group(1))
        return [b, "node:" + b]
    return names_of(rx)


class Doc:
    def __init__(self, path=JSON):
        self.doc = json.load(open(path))
        self.utils = self.doc["clauses"][0]["define"]["utils"]
        self.clauses = self.doc["clauses"][1:]

    # ---- walking a term ---------------------------------------------------------------------------------------
    def leaves(self, t):
        """The alternatives of t: pure `any` nodes and compact utilities are flattened."""
        if isinstance(t, dict) and set(t) == {"any"}:
            return [x for a in t["any"] for x in self.leaves(a)]
        if isinstance(t, dict) and set(t) == {"matches"} and COMPACT.match(t["matches"]):
            return self.leaves(self.utils[t["matches"]])
        return [t]

    def scan(self, t, out=None):
        """Facts about a term, looking through compact utilities but not into named ones: the kinds, the named
        utilities it refers to, the regexes and the global names it tests."""
        out = out or {"kinds": set(), "refs": set(), "rx": set(), "globals": set(), "no_clause": False}
        if isinstance(t, dict):
            for k, v in t.items():
                if k == "kind":
                    out["kinds"].add(v)
                elif k == "matches":
                    if COMPACT.match(v):
                        self.scan(self.utils[v], out)
                    else:
                        out["refs"].add(v)
                elif k == "regex" and isinstance(v, str):
                    out["rx"].add(v)
                elif k == "global" and isinstance(v, list):
                    out["globals"].update(v)
                elif k == "not" and json.dumps(v).find('"export_clause"') >= 0:
                    out["no_clause"] = True
                else:
                    self.scan(v, out)
        elif isinstance(t, list):
            for x in t:
                self.scan(x, out)
        return out

    def top_kind(self, t):
        while isinstance(t, dict) and "kind" not in t and set(t) == {"matches"} and COMPACT.match(t["matches"]):
            t = self.utils[t["matches"]]
        if isinstance(t, dict) and "kind" in t:
            return t["kind"]
        if isinstance(t, dict) and "any" in t and "kind" not in t:   # (import | export) statement
            ks = {a.get("kind") for a in t["any"] if isinstance(a, dict)}
            if ks == {"import_statement", "export_statement"}:
                return "import_or_export"
        return None

    # ---- what a clause reports --------------------------------------------------------------------------------
    @staticmethod
    def path_of(util):
        """'v:fs.promises' -> 'fs.promises'; 'v:g.process' -> 'g.process'; 'global-object' -> 'g' (globals)."""
        if util == "global-object":
            return "g"
        assert util.startswith("v:"), util
        return util[2:]

    def member_names(self, util):
        """The member names a call:/construct: utility tests (its member_property_first alternative)."""
        for a in self.leaves(self.utils[util]):
            s = self.scan(a)
            if self.top_kind(a) in ("member_expression", "subscript_expression"):
                (obj,) = [r for r in s["refs"] if r.startswith("v:")]
                (rx,) = s["rx"] - {"^default$"}
                return self.path_of(obj), names_of(rx)
        raise AssertionError(util)

    def items(self, clause):
        """(api, how, form) for each alternative of the clause; form says which reference form it was."""
        rule = clause["forbid"]["ast"]["rule"]
        for a in self.leaves(rule):
            s, k = self.scan(a), self.top_kind(a)
            refs = s["refs"]
            if k in ("member_expression", "subscript_expression"):
                (obj,) = [r for r in refs if r.startswith("v:") or r == "global-object"]
                (rx,) = s["rx"]
                for x in names_of(rx):
                    yield self.path_of(obj) + "." + x, "read", "member"
            elif k in ("shorthand_property_identifier_pattern", "pair_pattern", "object_assignment_pattern") \
                    or (k is None and any(r.startswith("pat:") for r in refs)):
                (pat,) = [r for r in refs if r.startswith("pat:")]
                p = pat[4:]
                (rx,) = {r for r in s["rx"]}
                for x in names_of(rx):
                    yield p + "." + x, "read", "pattern"
            elif k in ("import_specifier", "export_specifier"):
                (src,) = [r for r in refs if r.endswith(":esm-source")]
                (rx,) = s["rx"]
                for x in names_of(rx):
                    yield src[2:-len(":esm-source")] + "." + x, "read", k
            elif k == "export_statement" and s["no_clause"]:
                (src,) = [r for r in refs if r.endswith(":esm-source")]
                yield src[2:-len(":esm-source")], "read", "export-all"
            elif isinstance(a, dict) and set(a) == {"matches"} and re.match(r"^[rm]:.*:call$", a["matches"]):
                u = a["matches"]
                (rx,) = self.scan(self.utils[u]["all"][0])["rx"]
                for m in spec_names(rx):
                    yield m, "read", "load"
            elif k == "import_or_export":
                if refs:
                    (src,) = [r for r in refs if r.endswith(":esm-source")]
                    (rx,) = self.scan(self.utils[src])["rx"]
                else:
                    (rx,) = s["rx"]
                for m in spec_names(rx):
                    yield m, "read", "load"
            elif k == "identifier" and s["globals"]:
                (g,) = s["globals"]
                yield "g." + g, "read", "global"
            elif k in ("call_expression", "new_expression"):
                how = "call" if k == "call_expression" else "new"
                (u,) = [r for r in refs if r.startswith(("call:", "construct:", "v:"))]
                if u.startswith("v:"):
                    yield self.path_of(u), how, "global"
                else:
                    p, names = self.member_names(u)
                    for x in names:
                        yield p + "." + x, how, "member"
            else:
                raise AssertionError(("unrecognised alternative", clause["id"], json.dumps(a)[:300]))

    @staticmethod
    def versions(clause):
        """The clause's supported versions, latest first: '^a || ^b || >=a' -> ['a', 'b']; [] for `unsupported`."""
        rng = clause["forbid"]["ast"].get("file", {}).get("package", {}).get("nodeVersion", {}).get("notSubsetOf")
        if rng is None:
            return []
        parts = rng.split(" || ")
        assert parts[-1] == ">=" + parts[0][1:], rng
        return [p[1:] for p in parts[:-1]]

    def node_globals(self):
        """The global roots the JSON traces (a v:g.<name> utility each), and globalThis."""
        return sorted({k[4:] for k in self.utils if k.startswith("v:g.") and "." not in k[4:]} | {"globalThis"})

    def module_specs(self):
        """Every module specifier the JSON accepts, with its member root: 'node:fs' -> 'fs', 'node:test' -> itself."""
        out = {}
        for u, body in self.utils.items():
            m = re.fullmatch(r"m:(.*):esm-source", u)
            if m:
                (rx,) = self.scan(body)["rx"]
                for s in spec_names(rx):
                    out[s] = m.group(1)
        return out


def canon(p):
    """A JSON trie path to an API name: 'g.crypto.subtle' -> 'crypto.subtle', 'fs.promises.readFile' ->
    'fs#promises.readFile', 'import.meta.dirname' stays. Module keys hold no dot, so the first dot ends the module."""
    if p.startswith("g."):
        return p[2:]
    if p == "import.meta" or p.startswith("import.meta."):
        return p
    m, _, rest = p.partition(".")
    return f"{m}#{rest}" if rest else m


def table(doc):
    """{(api, how): versions} (latest first, [] for no supported release) and the export-all groups
    {(module, versions): count}, for the check. Raises if one API gets two version lists."""
    rows, export_all = {}, {}
    for c in doc.clauses:
        v = doc.versions(c)
        for p, how, form in doc.items(c):
            if form == "export-all":
                key = (canon(p), tuple(v))
                export_all[key] = export_all.get(key, 0) + 1
                continue
            how = "load" if form == "load" else how
            api = p if form == "load" else canon(p)
            if rows.setdefault((api, how), v) != v:
                raise SystemExit(f"{api} {how}: two version lists {rows[(api, how)]} and {v}")
    return rows, export_all


def q(s):
    return json.dumps(s)


def main():
    doc = Doc()
    rows, _ = table(doc)
    specs = doc.module_specs()
    out = [f"""// n-no-unsupported-features-node-builtins.facts.rr: which Node.js release added each built-in API.
// GENERATED by packs/node/gen-node-builtins.py from packs/node/rules/n-no-unsupported-features-node-builtins.json
// (eslint-plugin-n 18.4.1's trace map). Do not edit; rerun the script. {len(rows)} APIs.
//
// api_since(api, how, v)      API api, used as how, exists from Node.js v on: every v' >= v has it.
// api_backport(api, how, v)   it was also backported to v's line: every ^v has it (as `^20.16.0`: >=20.16.0 <21).
// api_experimental(api, how)  no release supports API api used as how: it is experimental wherever it exists, or
//                             that form is not supported at all (`zlib.Gzip()` called without new).
// how: "load" a module specifier loaded (require, process.getBuiltinModule, import, export … from);
//      "read" a member, destructured key, import or export specifier, or global named; "call"; "new".
// API names: lib/tsx/values.rr ("fs", "node:fs", "fs#promises.readFile", "fetch", "crypto.subtle", "import.meta.x").
.decl api_since(api: symbol, how: symbol, v: symbol)
.decl api_backport(api: symbol, how: symbol, v: symbol)
.decl api_experimental(api: symbol, how: symbol)

// node_module(spec, root) rows for lib/tsx/values.rr: the module specifiers Node resolves to a built-in, and the name
// their members are traced under ("node:fs" -> "fs"). A specifier missing here (an npm "test") is not a built-in.
"""]
    for s in sorted(specs):
        out.append(f"node_module({q(s)}, {q(specs[s])}).\n")
    globals_ = doc.node_globals()
    out.append("\n// node_global(g) rows for lib/tsx/values.rr: the globals the trace map traces (globals.node's names it has,\n"
               "// and globalThis, which ECMAScript declares and the JSON's global-object test reads).\n")
    out.append("".join(f"node_global({q(g)}).\n" for g in globals_))
    known = set()
    for (api, how) in rows:
        if how == "load":
            continue
        root, sep, rest = api.partition("#") if "#" in api else ("", "", api)
        parts = rest.split(".")
        start = 2 if rest.startswith("import.meta") else 1
        for i in range(start, len(parts) + 1):
            known.add(root + sep + ".".join(parts[:i]))
    out.append("\n// api_known(p) rows for lib/tsx/values.rr: every API name below and its prefixes, under the root.\n")
    out.append("".join(f"api_known({q(k)}).\n" for k in sorted(known)))
    out.append("\n// The table.\n")
    for (api, how) in sorted(rows):
        v = rows[(api, how)]
        if not v:
            out.append(f"api_experimental({q(api)}, {q(how)}).\n")
            continue
        out.append(f"api_since({q(api)}, {q(how)}, {q(v[0])}).")
        out.append("".join(f" api_backport({q(api)}, {q(how)}, {q(b)})." for b in v[1:]) + "\n")
    path = os.path.join(HERE, "n-no-unsupported-features-node-builtins.facts.rr")
    open(path, "w").write("".join(out))
    print(f"{path}: {len(rows)} APIs, {len(specs)} module specifiers, {len(known)} traced names")


if __name__ == "__main__":
    main()
