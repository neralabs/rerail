"""Static analysis of the data-layer rule pack for the fusion / performance model.

Reads rerail.policy/v1 rule documents, resolves `$option` with the documents' default option values,
finds every AST query (forbid/budget/define/relational inputs/evidence), and extracts per query:
root (anchor) kinds, relation sites (axis, stopBy, field, inner digest, depth), strata, boolean sites,
utility references (with recursion), regex/pattern counts, unsupported (non-structural) keys, and
the rule's candidate scope. Also lowers each query to the bench IR (JSON op lists) for calibration.

Usage: python3 -I analyze.py RULES_DIR OUT_DIR
"""
import hashlib
import json
import os
import re
import sys
from collections import Counter, defaultdict

RELATIONS = ("has", "inside", "precedes", "follows")
STRUCTURAL = {"kind", "pattern", "regex", "all", "any", "not", "matches", "nthChild", "field", "stopBy",
              "has", "inside", "precedes", "follows"}
# Keys that are Rerail semantic extensions: they need facts / text / evidence beyond pure structure.
SEMANTIC = {"compare", "count", "resolves", "arguments", "binds", "parse", "formatArgs", "layout", "host",
            "embedded", "calls", "filePath", "transform", "value"}


def digest(x):
    return hashlib.sha1(json.dumps(x, sort_keys=True).encode()).hexdigest()[:12]


def resolve_options(x, opts):
    """Replace {"$option": name, "cases": {...}} by the case for the option's value."""
    if isinstance(x, dict):
        if "$option" in x and "cases" in x:
            v = opts.get(x["$option"], x.get("default"))
            key = json.dumps(v) if not isinstance(v, str) else v
            key = {"true": "true", "false": "false"}.get(str(key).lower(), str(key))
            cases = x["cases"]
            if key in cases:
                return resolve_options(cases[key], opts)
            if "default" in x and not isinstance(x["default"], bool) and str(x["default"]) in cases:
                return resolve_options(cases[str(x["default"])], opts)
            return resolve_options(next(iter(cases.values())), opts)
        return {k: resolve_options(v, opts) for k, v in x.items()}
    if isinstance(x, list):
        return [resolve_options(v, opts) for v in x]
    return x


def clause_active(clause_body, opts):
    """`when: {option: {name: value}}` gates a clause/query on option values."""
    when = clause_body.get("when") if isinstance(clause_body, dict) else None
    if not isinstance(when, dict):
        return True
    for name, want in (when.get("option") or {}).items():
        if opts.get(name) != want:
            return False
    return True


def glob_re(g):
    out = ""
    i = 0
    while i < len(g):
        if g.startswith("**/", i):
            out += "(?:.*/)?"
            i += 3
        elif g.startswith("**", i):
            out += ".*"
            i += 2
        elif g[i] == "*":
            out += "[^/]*"
            i += 1
        elif g[i] == "?":
            out += "[^/]"
            i += 1
        else:
            out += re.escape(g[i])
            i += 1
    return re.compile("^" + out + "$")


class Query:
    def __init__(self, rule_id, clause_id, role, q, utils, scope_inc, scope_exc, path_filter):
        self.rule_id, self.clause_id, self.role = rule_id, clause_id, role
        self.language = q.get("language")
        self.rule = q.get("rule")
        self.utils = dict(utils)
        self.utils.update(q.get("utils") or {})
        self.constraints = q.get("constraints") or {}
        self.scope_inc, self.scope_exc, self.path_filter = scope_inc, scope_exc, path_filter
        self.syntax = q.get("syntax")


class Analyzer:
    def __init__(self, q: Query):
        self.q = q
        self.sites = []  # (axis, stopBy, field, inner_digest, depth)
        self.bools = Counter()
        self.atoms = Counter()
        self.semantic = Counter()
        self.utils_used = set()
        self.recursive_utils = set()
        self.max_depth = 0
        self.patterns = []
        self.regexes = []
        self.unbounded = False

    def walk(self, r, depth, stack):
        if not isinstance(r, dict):
            return
        self.max_depth = max(self.max_depth, depth)
        for k, v in r.items():
            if k == "kind":
                self.atoms["kind"] += 1
            elif k == "pattern":
                self.atoms["pattern"] += 1
                self.patterns.append(v if isinstance(v, str) else v.get("context", ""))
            elif k == "regex":
                self.atoms["regex"] += 1
                self.regexes.append(v)
            elif k == "nthChild":
                self.atoms["nthChild"] += 1
            elif k in ("all", "any"):
                if isinstance(v, list):
                    if len(v) > 1:
                        self.bools[k] += 1
                    for x in v:
                        self.walk(x, depth, stack)
            elif k == "not":
                self.bools["not"] += 1
                self.walk(v, depth, stack)
            elif k in RELATIONS:
                inner = v
                stop = inner.get("stopBy", "neighbor") if isinstance(inner, dict) else "neighbor"
                stop_kind = stop if isinstance(stop, str) else "rule"
                field = inner.get("field") if isinstance(inner, dict) else None
                body = {kk: vv for kk, vv in inner.items() if kk not in ("stopBy", "field")} if isinstance(inner, dict) else inner
                self.sites.append((k, stop_kind, field, digest(body), depth + 1))
                if stop_kind == "rule":
                    self.walk(stop, depth + 1, stack)
                self.walk(body, depth + 1, stack)
            elif k == "matches":
                name = v if isinstance(v, str) else next(iter(v))
                self.utils_used.add(name)
                if name in stack:
                    self.recursive_utils.add(name)
                    self.unbounded = True
                    continue
                u = self.q.utils.get(name)
                if u is not None:
                    self.walk(u, depth, stack | {name})
            elif k in SEMANTIC:
                self.semantic[k] += 1
                # descend into nested AstRules of semantic keys (e.g. host/embedded/calls inner rules)
                if isinstance(v, dict):
                    self.walk(v, depth, stack)

    def root_kinds(self, r, stack=frozenset()):
        """Set of kinds a match root can have; None = unconstrained (any node)."""
        if not isinstance(r, dict):
            return None
        sets = []
        if "kind" in r:
            sets.append({r["kind"]})
        if "pattern" in r:
            p = r["pattern"]
            sets.append({("pattern", p if isinstance(p, str) else json.dumps(p))})
        if "all" in r and isinstance(r["all"], list):
            for x in r["all"]:
                s = self.root_kinds(x, stack)
                if s is not None:
                    sets.append(s)
        if "any" in r and isinstance(r["any"], list):
            u = set()
            for x in r["any"]:
                s = self.root_kinds(x, stack)
                if s is None:
                    u = None
                    break
                u |= s
            if u is not None:
                sets.append(u)
        if "matches" in r:
            name = r["matches"] if isinstance(r["matches"], str) else next(iter(r["matches"]))
            if name not in stack and name in self.q.utils:
                s = self.root_kinds(self.q.utils[name], stack | {name})
                if s is not None:
                    sets.append(s)
        if not sets:
            return None
        # intersection over conjuncts; patterns are resolved later to kinds, so keep the smallest
        # concrete set (a pattern conjunct with a kind conjunct: the kind wins).
        concrete = [s for s in sets if all(isinstance(x, str) for x in s)]
        if concrete:
            out = set(concrete[0])
            for s in concrete[1:]:
                out &= s
            return out
        return min(sets, key=len)


def find_queries(rule):
    """Yield (clause_id, role, query_dict, clause_body) for every AST query in the rule."""
    opts = {k: v.get("default") for k, v in (rule.get("options") or {}).items()}
    for c in rule["clauses"]:
        if "define" in c:
            for k, v in (c["define"].get("options") or {}).items():
                opts.setdefault(k, v.get("default"))
    out = []

    def rec(x, cid, role, body):
        if isinstance(x, dict):
            if "language" in x and "rule" in x:
                out.append((cid, role, x, body))
                return
            for k, v in x.items():
                rec(v, cid, role if role != "?" else k, x if isinstance(x, dict) else body)
        elif isinstance(x, list):
            for v in x:
                rec(v, cid, role, body)

    for c in rule["clauses"]:
        cid = c.get("id")
        kind = next((k for k in ("forbid", "budget", "define", "relational", "require_each", "select_when") if k in c), "?")
        if kind == "define":
            continue  # utilities only; their AST rules are analysed where used
        rec(c[kind], cid, kind, c[kind])
    return out, opts


def main():
    rules_dir, out_dir = sys.argv[1], sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    rows = []
    rule_rows = []
    full = []
    text_regex = []
    for fn in sorted(os.listdir(rules_dir)):
        if not fn.endswith(".json"):
            continue
        raw = json.load(open(os.path.join(rules_dir, fn)))
        queries, opts = find_queries(raw)
        rule = resolve_options(raw, opts)
        queries, _ = find_queries(rule)
        scope = rule.get("scope") or {}
        inc = [glob_re(g) for g in scope.get("include", ["**"])]
        exc = [glob_re(g) for g in scope.get("exclude", [])]
        utils = {}
        for c in rule["clauses"]:
            if "define" in c:
                utils.update(c["define"].get("utils") or {})
        ck = Counter(next((k for k in ("forbid", "budget", "define", "relational", "require_each", "select_when") if k in c), "?") for c in rule["clauses"])
        active_q = 0
        for cid, role, q, body in queries:
            active = clause_active(body, opts) and clause_active(q, opts)
            pf = (q.get("file") or {}).get("path") or {}
            query = Query(rule["id"], cid, role, q, utils, scope.get("include", ["**"]), scope.get("exclude", []), pf)
            a = Analyzer(query)
            a.walk(q["rule"], 0, frozenset())
            for name, u in (q.get("constraints") or {}).items():
                a.walk(u, 0, frozenset())
            rk = a.root_kinds(q["rule"])
            full.append({"rule": rule["id"], "clause": cid, "role": role, "active": active, "query": q,
                         "utils": {**utils, **(q.get("utils") or {})},
                         "include": scope.get("include", ["**"]), "exclude": scope.get("exclude", []),
                         "notRegex": pf.get("notRegex"), "pathRegex": pf.get("regex")})
            active_q += active
            rows.append({
                "rule": rule["id"], "clause": cid, "role": role, "active": active, "language": q.get("language"),
                "root_kinds": sorted(map(str, rk)) if rk is not None else None,
                "sites": a.sites, "max_depth": a.max_depth, "bools": dict(a.bools), "atoms": dict(a.atoms),
                "semantic": dict(a.semantic), "utils_used": sorted(a.utils_used),
                "recursive_utils": sorted(a.recursive_utils), "patterns": a.patterns,
                "n_regex": len(a.regexes), "rule_digest": digest(q["rule"]),
                "path_filter": pf,
            })
        for c in rule["clauses"]:
            f = c.get("forbid")
            if isinstance(f, dict) and "regex" in f:
                text_regex.append({"rule": rule["id"], "clause": c.get("id"), "active": clause_active(f, opts),
                                   "pattern": f["regex"].get("pattern"), "lines": "lines" in f["regex"],
                                   "include": scope.get("include", ["**"]), "exclude": scope.get("exclude", [])})
        rule_rows.append({"rule": rule["id"], "clause_kinds": dict(ck), "queries": len(queries), "active_queries": active_q,
                          "include": scope.get("include", ["**"]), "exclude": scope.get("exclude", []),
                          "bytes": os.path.getsize(os.path.join(rules_dir, fn)),
                          "file_regex_clauses": sum(1 for c in rule["clauses"] if isinstance(c.get("forbid"), dict) and "regex" in c["forbid"])})
    json.dump(rows, open(os.path.join(out_dir, "queries.json"), "w"), indent=0)
    json.dump(full, open(os.path.join(out_dir, "queries_full.json"), "w"))
    json.dump(text_regex, open(os.path.join(out_dir, "text_regex.json"), "w"), indent=0)
    json.dump(rule_rows, open(os.path.join(out_dir, "rules.json"), "w"), indent=1)
    pats = sorted({p for r in rows for p in r["patterns"] if p})
    with open(os.path.join(out_dir, "patterns.txt"), "w") as f:
        for p in pats:
            f.write(json.dumps(p) + "\n")
    print(f"rules {len(rule_rows)}, queries {len(rows)} (active {sum(r['active'] for r in rows)}), distinct patterns {len(pats)}")


if __name__ == "__main__":
    main()
