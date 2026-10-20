#!/usr/bin/env python3
"""Clause-coverage cover for the 12 data-layer policies (data only; no engine runs).

Every clause needs a cover file where native reports a finding for it (hit) and a cover file in
its scope where native reports nothing for it although the clause runs there (clean): the
RequiredText gate (re-derived from the policy as native's `required_text.rs` does) holds on it.
Cost of a file = its native all-rules time in profile.json (bytes where absent). Weighted greedy
set cover, then reverse redundancy elimination.

Writes $RERAIL_BENCH_HOME/distill/cover.json (RERAIL_BENCH_HOME defaults to ~/rerail-bench). Reads
the corpus and policies `just rerail-bench-setup` lays out there, plus `data-layer/out/baseline/`
(`just rerail-data-layer whole-tree`) and `data-layer/out/profile/` (`just rerail-data-layer
profile`). Needs the `lits` helper beside this script, built with
`cargo build --release --manifest-path benchmark/data-layer/setup/lits/Cargo.toml`.
"""
import ast
import collections
import json
import os
import re
import subprocess
import sys

BENCH = os.path.expanduser(os.environ.get("RERAIL_BENCH_HOME") or "~/rerail-bench")
CORPUS = f"{BENCH}/nera-data-layer-fce4dc72"
POLICIES = f"{BENCH}/data-layer/policies"
LINES = f"{BENCH}/data-layer/out/baseline/whole-tree-1.lines.txt"
PROFILE = f"{BENCH}/data-layer/out/profile/profile.json"
HELPER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "lits/target/release/lits")
# COVER_LEAN=1: drop the literal preference for clean files (gate and relatedness only), to
# measure what the near-miss preference costs.
LEAN = os.environ.get("COVER_LEAN") == "1"
OUT = os.environ.get("COVER_OUT") or (f"{BENCH}/distill/cover-lean.json" if LEAN else f"{BENCH}/distill/cover.json")
# COVER_SQL_COST: a JSON {path: sql_ms} of measured SQL costs; selection weighs a file by the
# larger of its native ms and its SQL ms / COVER_SQL_DIVISOR, so SQL-expensive files are avoided.
SQL_COST = json.load(open(os.environ["COVER_SQL_COST"])) if os.environ.get("COVER_SQL_COST") else {}
SQL_DIVISOR = float(os.environ.get("COVER_SQL_DIVISOR", "1"))

MAX_ALTERNATIVES = 16
MAX_CLAUSES = 8
SEMANTIC_THREE_VALUED = {"value", "arguments", "parse", "formatArgs", "calls", "binds"}
PLAIN_COMPARE_TEXT = {None, "source", "unquoted", "tokens", "node", "index"}
RELATIONS = ("has", "inside", "precedes", "follows")
RELATION_KEYS = ("stopBy", "field")

# ---------------------------------------------------------------- inputs
profile = json.load(open(PROFILE))
passes = {p["id"]: p["files"] for p in profile["passes"]}
files = sorted(passes["all"])
index = {p: i for i, p in enumerate(files)}
ms = [passes["all"][p]["ms"] for p in files]
size = [os.path.getsize(f"{CORPUS}/{p}") for p in files]

pols = {}
for name in sorted(os.listdir(POLICIES)):
    path = f"{POLICIES}/{name}/policy.json"
    if os.path.exists(path):
        pols[name] = json.load(open(path))
policy_by_id = {p["id"]: name for name, p in pols.items()}

findings = collections.defaultdict(collections.Counter)  # (policy, clause) -> path -> n
failures = collections.defaultdict(set)
for line in open(LINES):
    row = line.rstrip("\n").split("\t")
    key = (policy_by_id[row[1]], row[2])
    if row[0] == "violation":
        findings[key][row[4]] += 1
    elif row[0] == "failure":
        failures[key].add(row[4])

# ---------------------------------------------------------------- clause model
def util_tables(policy):
    shared = {}
    for c in policy["clauses"]:
        if "define" in c:
            shared.update(c["define"].get("utils", {}))
    return shared


def leaves(query, domain):
    """(role, kind, body, domain) leaves of a query; role 'report' is the leaf whose witnesses
    are the clause's findings."""
    if "select_when" in query:
        sw = query["select_when"]
        out = [("exists", *leaf[1:]) for leaf in leaves(sw["exists"], domain)]
        out += leaves(sw["select"], domain)
        return out
    if "evidence" in query:
        ev = query["evidence"]
        paths = ev.get("paths")
        dom = None if paths is None else {
            "include": paths.get("include", ["**"]), "exclude": paths.get("exclude", [])}
        return leaves(ev["query"], dom)
    if "ast" in query:
        return [("report", "ast", query["ast"], domain)]
    if "regex" in query:
        return [("report", "regex", query["regex"], domain)]
    raise ValueError(query)


clauses = []
for pname, policy in pols.items():
    shared = util_tables(policy)
    for c in policy["clauses"]:
        if "define" in c:
            continue
        if "forbid" in c:
            kind = next(k for k in ("ast", "regex", "select_when") if k in c["forbid"])
            ls = leaves(c["forbid"], None)
        else:
            kind = "budget"
            ls = leaves(c["budget"]["query"], None)
        clauses.append({"policy": pname, "id": c["id"], "kind": kind, "leaves": ls,
                        "shared": shared, "budget": c.get("budget")})

# ---------------------------------------------------------------- RequiredText (native port)
regex_patterns = set()
def collect_regexes(node):
    if isinstance(node, dict):
        for k, v in node.items():
            if k == "regex" and isinstance(v, str):
                regex_patterns.add(v)
            else:
                collect_regexes(v)
    elif isinstance(node, list):
        for v in node:
            collect_regexes(v)

for c in clauses:
    for role, kind, body, _ in c["leaves"]:
        if kind == "regex":
            regex_patterns.add(body["pattern"])
        else:
            collect_regexes(body)
    collect_regexes(c["shared"])

regex_list = sorted(regex_patterns)
path_regexes = set()
for c in clauses:
    for role, kind, body, _ in c["leaves"]:
        if kind == "ast":
            cond = body.get("file", {}).get("path", {})
            for k in ("regex", "notRegex"):
                if k in cond:
                    path_regexes.add(cond[k])
path_regex_list = sorted(path_regexes)
globs = []
for c in clauses:
    for leaf in c["leaves"]:
        if leaf[3] is not None and leaf[3] not in globs:
            globs.append(leaf[3])
content_regex_list = sorted({body["pattern"] for c in clauses for role, kind, body, _ in c["leaves"]
                             if role == "exists" and kind == "regex"})


def helper(payload):
    payload.setdefault("extract", [])
    payload.setdefault("path_regexes", [])
    payload.setdefault("globs", [])
    payload.setdefault("literals", [])
    payload["paths"] = files
    payload["root"] = CORPUS
    out = subprocess.run([HELPER], input=json.dumps(payload), capture_output=True, text=True,
                         check=True)
    return json.loads(out.stdout)


h = helper({"extract": regex_list, "path_regexes": path_regex_list, "globs": globs,
            "content_regexes": content_regex_list})
regex_lits = {}
for p, lits in zip(regex_list, h["extract"]):
    if isinstance(lits, dict):
        sys.exit(f"regex does not parse: {p[:200]}")
    regex_lits[p] = None if lits is None else frozenset(lits)
path_match = {p: set(m) for p, m in zip(path_regex_list, h["path_match"])}
glob_match = [set(m) for m in h["glob_match"]]
content_match = {p: set(m) for p, m in zip(content_regex_list, h["content_match"])}

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*|[0-9][0-9_.]*")
def pattern_leaves(pattern):
    # Every pattern in these policies is a bare metavariable or a dotted name: named leaves are
    # the identifiers (and numbers) outside metavariables.
    text = re.sub(r"\$+[A-Z_][A-Z0-9_]*", " ", pattern)
    return [frozenset([t]) for t in IDENT.findall(text)]


def selectivity(clause):
    return min((len(l) for l in clause), default=0)


class Ctx:
    def __init__(self, local, shared, relaxed=False):
        self.local, self.shared = local or {}, shared
        self.visiting, self.read = [], {}
        self.rv_seen = set()
        self.relaxed = relaxed
        self.cyclic = recursive_utils(self) if relaxed else set()

    def util(self, name):
        u = self.local.get(name, self.shared.get(name))
        if u is None:
            raise KeyError(name)
        return u["rule"] if isinstance(u, dict) and "params" in u and "rule" in u else u


def util_name(v):
    return v if isinstance(v, str) else next(iter(v))


def util_refs(rule, out):
    if isinstance(rule, dict):
        for k, v in rule.items():
            if k == "matches":
                out.add(util_name(v))
            else:
                util_refs(v, out)
    elif isinstance(rule, list):
        for v in rule:
            util_refs(v, out)
    return out


_recursive_cache = {}
def recursive_utils(ctx):
    """Utilities that reach themselves through `matches` references."""
    key = (id(ctx.local), id(ctx.shared))
    if key not in _recursive_cache:
        names = set(ctx.local) | set(ctx.shared)
        graph = {n: util_refs(ctx.util(n), set()) & names for n in names}
        cyclic = set()
        for start in names:
            stack, seen = list(graph[start]), set()
            while stack:
                n = stack.pop()
                if n == start:
                    cyclic.add(start)
                    break
                if n not in seen:
                    seen.add(n)
                    stack.extend(graph[n])
        _recursive_cache[key] = cyclic
    return _recursive_cache[key]


def needs(rule, ctx, reach="file"):
    out = []
    if not isinstance(rule, dict):
        return out
    for k, v in rule.items():
        if k == "regex" and isinstance(v, str):
            lits = regex_lits[v]
            if lits:
                out.append(lits)
        elif k == "pattern" and isinstance(v, str):
            out += pattern_leaves(v)
        elif k == "all":
            for item in v:
                out += needs(item, ctx, reach)
        elif k == "any":
            alts = set()
            ok = True
            for item in v:
                n = needs(item, ctx, reach)
                if not n:
                    ok = False
                    break
                if ctx.relaxed:
                    # native's pick among equally selective clauses follows compiled term order,
                    # which this port does not reproduce: take every tied clause (weaker)
                    top = max(selectivity(x) for x in n)
                    best = frozenset().union(*(x for x in n if selectivity(x) == top))
                else:
                    best = max(reversed(n), key=selectivity)  # Rust max_by_key: last maximum
                alts |= best
                if len(alts) > MAX_ALTERNATIVES:
                    ok = False
                    break
            if ok and alts:
                out.append(frozenset(alts))
        elif k == "has":
            out += needs({a: b for a, b in v.items() if a not in RELATION_KEYS}, ctx, reach)
        elif k in ("inside", "precedes", "follows") and reach == "file":
            out += needs({a: b for a, b in v.items() if a not in RELATION_KEYS}, ctx, reach)
        elif k == "not":
            if isinstance(v, dict) and list(v) == ["not"]:
                out += needs(v["not"], ctx, reach)
        elif k == "matches":
            name = util_name(v)
            if ctx.relaxed and name in ctx.cyclic:
                continue  # a recursive utility's weakening depends on visit order: need nothing
            if name in ctx.read:
                out += ctx.read[name]
            elif name not in ctx.visiting:
                ctx.visiting.append(name)
                n = needs(ctx.util(name), ctx, reach)
                ctx.visiting.pop()
                ctx.read[name] = n
                out += n
    return out


def reads_values(rule, ctx):
    if isinstance(rule, list):
        return any(reads_values(r, ctx) for r in rule)
    if not isinstance(rule, dict):
        return False
    for k, v in rule.items():
        if k in SEMANTIC_THREE_VALUED:
            return True
        if k == "compare":
            if isinstance(v, dict) and (v.get("text") not in PLAIN_COMPARE_TEXT or "transform" in v):
                return True
        elif k == "resolves":
            b = v.get("binding") if isinstance(v, dict) else None
            if isinstance(b, dict) and any(reads_values(b.get(s), ctx)
                                           for s in ("site", "value", "annotation", "occurrence")):
                return True
        elif k == "matches":
            name = util_name(v)
            if name not in ctx.rv_seen:
                ctx.rv_seen.add(name)
                if reads_values(ctx.util(name), ctx):
                    return True
        elif k in ("regex", "pattern", "kind", "field", "layout"):
            continue
        elif reads_values(v, ctx):
            return True
    return False


def calls_terms(rule, ctx, seen, out):
    """The `calls` terms a rule reaches (directly or through utilities)."""
    if isinstance(rule, list):
        for r in rule:
            calls_terms(r, ctx, seen, out)
    elif isinstance(rule, dict):
        for k, v in rule.items():
            if k == "calls":
                if v not in out:
                    out.append(v)
            elif k == "matches":
                name = util_name(v)
                if name not in seen:
                    seen.add(name)
                    calls_terms(ctx.util(name), ctx, seen, out)
            else:
                calls_terms(v, ctx, seen, out)
    return out


def native_cnf(cnf, relaxed=False):
    ordered = sorted(cnf, key=lambda c: -selectivity(c))  # stable, like sort_by_key(Reverse)
    out = []
    for c in ordered:
        if not out or out[-1] != c:
            out.append(c)
    if relaxed and len(out) > MAX_CLAUSES and selectivity(out[MAX_CLAUSES]) == selectivity(
            out[MAX_CLAUSES - 1]):
        # which tied clauses survive the cut follows compiled term order: keep none of them
        cut = selectivity(out[MAX_CLAUSES - 1])
        return [c for c in out if selectivity(c) > cut]
    return out[:MAX_CLAUSES]


for c in clauses:
    report = [l for l in c["leaves"] if l[0] == "report"]
    assert len(report) == 1, c["id"]
    role, kind, body, domain = report[0]
    c["report_kind"] = kind
    c["crossfile"] = []
    if c["kind"] == "select_when":
        c["crossfile"].append("select_when exists is a global boolean over its scope")
    if c["kind"] == "budget":
        c["crossfile"].append(f"budget {json.dumps({k: v for k, v in c['budget'].items() if k != 'query'})}")
    if kind == "ast":
        ctx = Ctx(body.get("utils"), c["shared"])
        full = needs(body["rule"], ctx)
        three = reads_values(body["rule"], Ctx(body.get("utils"), c["shared"]))
        c["three_valued"] = three
        c["full"] = list(dict.fromkeys(full))
        c["gate"] = None if three else (native_cnf(full) or None)
        # weaker than native's gate whatever the compiled term order: a file failing it never
        # runs the clause
        c["gate_relaxed"] = None if three else (
            native_cnf(needs(body["rule"], Ctx(body.get("utils"), c["shared"], relaxed=True)),
                       relaxed=True) or None)
        terms = calls_terms(body["rule"], Ctx(body.get("utils"), c["shared"]), set(), [])
        if terms:
            c["crossfile"].append("calls: workspace call graph")
            # what a definition the call reaches must contain (seed RequiredText), per term
            c["seeds"] = [native_cnf(needs({k: v for k, v in t.items() if k not in ("through", "chain")},
                                           Ctx(body.get("utils"), c["shared"])))
                          for t in terms]
    else:
        lits = regex_lits[body["pattern"]]
        c["three_valued"] = False
        c["full"] = [lits] if lits else []
        c["gate"] = None  # Query::Regex has no RequiredText gate
        c["gate_relaxed"] = None
    # scope: code-rule scope (profile pass) ∩ language ∩ file condition ∩ evidence domain
    scope = {index[p] for p in passes[f"rule:{c['policy']}"]}
    if kind == "ast":
        scope = {i for i in scope if files[i].endswith(".py")}
        cond = body.get("file", {}).get("path", {})
        if "regex" in cond:
            scope &= path_match[cond["regex"]]
        if "notRegex" in cond:
            scope -= path_match[cond["notRegex"]]
    if domain is not None:
        scope &= glob_match[globs.index(domain)]
    c["scope"] = scope
    c["exists"] = None
    for r, k, b, d in c["leaves"]:
        if r == "exists":
            ex_scope = {index[p] for p in passes[f"rule:{c['policy']}"]}
            if d is not None:
                ex_scope &= glob_match[globs.index(d)]
            c["exists"] = sorted(content_match[b["pattern"]] & ex_scope) if k == "regex" else "ast"

# ---------------------------------------------------------------- literal presence
all_lits = sorted({l for c in clauses
                   for cnf in [c["full"], c["gate_relaxed"] or []] + c.get("seeds", [])
                   for cl in cnf for l in cl})
h = helper({"literals": all_lits})
presence = {l: set(m) for l, m in zip(all_lits, h["presence"])}


def holds(cnf, i):
    return all(any(i in presence[l] for l in cl) for cl in cnf)


# ---------------------------------------------------------------- relatedness (dirs, imports)
def module_of(path):
    parts = path[:-3].split("/")
    return parts[:-1] if parts[-1] == "__init__" else parts


def file_imports(path):
    """Imported module names (absolute) of a Python file; `from m import n` yields m and m.n."""
    text = open(f"{CORPUS}/{path}", encoding="utf-8", errors="replace").read()
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return set()
    pkg = path[:-3].split("/")[:-1]
    mods = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            mods.update(a.name for a in node.names)
        elif isinstance(node, ast.ImportFrom):
            base = node.module.split(".") if node.module else []
            if node.level:
                base = pkg[: len(pkg) - (node.level - 1)] + base
            name = ".".join(base)
            if name:
                mods.add(name)
            mods.update(f"{name}.{a.name}" if name else a.name for a in node.names if a.name != "*")
    return mods


imports = [file_imports(p) if p.endswith(".py") else set() for p in files]
module_file = {".".join(module_of(p)): i for i, p in enumerate(files) if p.endswith(".py")}
# deps[i]: corpus files of the modules file i imports (one level; `from m import n` resolves to
# m's file and to n's file where n is a submodule).
deps = [{module_file[m] for m in mods if m in module_file} - {i} for i, mods in enumerate(imports)]
df = collections.Counter(m for mods in imports for m in mods)
DISTINCT = 0.02 * len(files)
distinct = [{m for m in mods if df[m] <= DISTINCT} for mods in imports]
dirname = [os.path.dirname(p) for p in files]

# ---------------------------------------------------------------- elements and eligibility
# An element is satisfied by any of its options; an option is a set of files that must all be in
# the cover. `calls` clauses reach definitions through the workspace call graph, so their hit
# option is the finding file plus the corpus files it imports (one level).
elements = []  # (clause index, 'hit'|'clean'|'exists')
options = []  # (element id, frozenset of files)


def add_element(ci, role, opts):
    e = len(elements)
    elements.append((ci, role))
    for o in opts:
        options.append((e, frozenset(o)))


for ci, c in enumerate(clauses):
    key = (c["policy"], c["id"])
    hits = {index[p] for p in findings.get(key, {})}
    c["hits"] = hits
    c["failed"] = {index[p] for p in failures.get(key, set())}
    c["n_findings"] = sum(findings.get(key, {}).values())
    c["calls"] = any(x.startswith("calls") for x in c["crossfile"])
    # sanity: the native gate must hold on every finding file
    c["gate_violations"] = sorted(files[i] for i in hits if c["gate"] and not holds(c["gate"], i))
    if hits and c["calls"]:
        # a reached definition holds its seed's literals: keep the imported files that do
        c["seed_files"] = {i for i in range(len(files))
                           if any(holds(cnf, i) for cnf in c["seeds"])}
    if hits:
        add_element(ci, "hit", [{i} for i in hits])
    clean = c["scope"] - hits - c["failed"]
    gated = {i for i in clean if c["gate"] is None or holds(c["gate"], i)}
    literal = {i for i in gated if holds(c["full"], i)} if c["full"] else set()
    if hits:
        dirs = {dirname[i] for i in hits}
        mods = set().union(*(distinct[i] for i in hits))
        related = {i for i in gated if dirname[i] in dirs or distinct[i] & mods}
        tiers = [("gate+related+literals", related & literal), ("gate+related", related),
                 ("gate+literals", literal), ("gate", gated)]
    else:
        tiers = [("gate+literals", literal), ("gate", gated)]
    if not c["full"] or LEAN:
        tiers = [t for t in tiers if "literals" not in t[0]]
    # last resort: files only the order-independent (weaker) gate admits; they run the clause if
    # native's compiled term order breaks selectivity ties as this tier assumes
    relaxed = {i for i in clean if c["gate_relaxed"] is None or holds(c["gate_relaxed"], i)}
    tiers.append(("gate-relaxed (tie order unverified)", relaxed))
    c["clean_relaxed"] = len(relaxed)
    c["clean_tier"], c["clean_pool"] = next(((n, s) for n, s in tiers if s), (None, set()))
    c["clean_scope"] = len(clean)
    c["clean_gated"] = len(gated)
    if c["clean_pool"]:
        add_element(ci, "clean", [{i} for i in c["clean_pool"]])
    if c["kind"] == "select_when":
        if c["exists"] == "ast":
            c["exists_status"] = "unknown: ast exists leaf (needs an engine run)"
        elif c["exists"]:
            c["exists_status"] = f"exists regex holds on {len(c['exists'])} in-scope files"
            if hits or c["clean_pool"]:
                add_element(ci, "exists", [{i} for i in c["exists"]])
        else:
            c["exists_status"] = ("exists regex holds on no in-scope file: the select leaf never "
                                  "runs in this corpus; clean coverage exercises the exists leaf only")
        if hits:
            assert isinstance(c["exists"], list) and c["exists"], c["id"]

failing_files = {index[p] for paths in failures.values() for p in paths}
weight = [ms[i] if ms[i] > 0 else size[i] / 1e5 for i in range(len(files))]
# Native budget failures are not emulated by the SQL engine: avoid their files where possible.
pick_weight = [max(w, SQL_COST.get(files[i], 0) / SQL_DIVISOR) * (10 if i in failing_files else 1)
               for i, w in enumerate(weight)]

# ---------------------------------------------------------------- weighted greedy + pruning
by_file = collections.defaultdict(list)  # file -> option indices touching it
for k, (e, o) in enumerate(options):
    for f in o:
        by_file[f].append(k)
candidates = sorted({o for _, o in options}, key=sorted)
uncovered = set(range(len(elements)))
chosen = set()
while uncovered:
    best, best_key, best_gain = None, None, None
    for cand in candidates:
        added = cand - chosen
        if not added:
            continue
        after = chosen | cand
        gain = {options[k][0] for f in added for k in by_file[f]
                if options[k][0] in uncovered and options[k][1] <= after}
        if not gain:
            continue
        cost = sum(pick_weight[f] for f in added)
        key = (len(gain) / cost, -cost)
        if best_key is None or key > best_key:
            best, best_key, best_gain = cand, key, gain
    chosen |= best
    uncovered -= best_gain


def satisfied(files_in):
    return {e for e, o in options if o <= files_in}


assert len(satisfied(chosen)) == len(elements)
for f in sorted(chosen, key=lambda f: -pick_weight[f]):
    if len(satisfied(chosen - {f})) == len(elements):
        chosen.discard(f)
chosen_set = chosen

# `calls` hits may need the definitions their calls reach in other files: the add-on gives, per
# calls clause, the finding file whose import closure (one level, seed-holding files) adds least.
calls_addon = set()
calls_pick = {}
for ci, c in enumerate(clauses):
    if c["hits"] and c["calls"]:
        bundles = [({i} | (deps[i] & c["seed_files"])) for i in sorted(c["hits"])]
        best = min(zip(sorted(c["hits"]), bundles),
                   key=lambda b: sum(pick_weight[f] for f in b[1] - chosen_set - calls_addon))
        calls_pick[ci] = best
        calls_addon |= best[1] - chosen_set

# ---------------------------------------------------------------- report
element_id = {el: e for e, el in enumerate(elements)}
opts_of = collections.defaultdict(list)
for e, o in options:
    opts_of[e].append(o)


def pick(ci, role):
    e = element_id.get((ci, role))
    if e is None:
        return None
    inside = [o for o in opts_of[e] if o <= chosen_set]
    return min(inside, key=lambda o: (sum(weight[f] for f in o), sorted(o)))


why = collections.defaultdict(list)
per_clause = []
for ci, c in enumerate(clauses):
    cid = f"{c['policy']}/{c['id']}"
    hit_opt, clean_opt, ex_opt = pick(ci, "hit"), pick(ci, "clean"), pick(ci, "exists")
    hit = clean = ex = None
    if hit_opt is not None:
        (hit,) = hit_opt
        why[hit].append([cid, "hit"])
    if clean_opt is not None:
        (clean,) = clean_opt
        why[clean].append([cid, "clean"])
    if ex_opt is not None:
        (ex,) = ex_opt
        why[ex].append([cid, "exists"])
    row = {
        "clause": cid, "kind": c["kind"], "findings": c["n_findings"],
        "finding_files": len(c["hits"]),
        "hit": files[hit] if hit is not None else None,
        "clean": files[clean] if clean is not None else None,
        "clean_tier": c["clean_tier"], "clean_candidates": len(c["clean_pool"]),
        "scope_files": len(c["scope"]),
        "three_valued": c["three_valued"],
        "gate": [sorted(cl) for cl in c["gate"]] if c["gate"] else None,
        "gate_relaxed": ([sorted(cl) for cl in c["gate_relaxed"]] if c["gate_relaxed"] else None)
        if c["gate_relaxed"] != c["gate"] else "same",
        "crossfile": c["crossfile"],
        "cover_findings": sum(n for p, n in findings.get((c["policy"], c["id"]), {}).items()
                              if index[p] in chosen_set),
    }
    if ci in calls_pick:
        f, bundle = calls_pick[ci]
        row["calls_closure"] = {
            "hit": files[f], "imports_in_core": sorted(files[i] for i in bundle - {f} if i in chosen_set),
            "addon_files": sorted(files[i] for i in bundle - chosen_set),
            "addon_ms": round(sum(ms[i] for i in bundle - chosen_set), 1)}
    if c.get("seeds"):
        row["calls_seed_gates"] = [[sorted(cl) for cl in cnf] for cnf in c["seeds"]]
    if c["kind"] == "select_when":
        row["exists_file"] = files[ex] if ex is not None else None
        row["exists_status"] = c["exists_status"]
    per_clause.append(row)


def no_clean_reason(c):
    if not c["scope"]:
        return "no file in scope in this corpus"
    if not c["clean_scope"]:
        return "every in-scope file has a finding or failure for the clause"
    if c["clean_relaxed"]:
        raise AssertionError(c["id"])
    if c["hits"]:
        return "RequiredText gate holds only on finding files"
    return "RequiredText gate holds on no in-scope file: the clause never runs in this corpus"


cover_files = sorted(chosen_set, key=lambda i: files[i])
top = sorted(per_clause, key=lambda r: -r["findings"])[:20]
no_findings = [r["clause"] for r in per_clause if r["findings"] == 0]
no_clean = [{"clause": f"{c['policy']}/{c['id']}", "findings": c["n_findings"],
             "scope_files": len(c["scope"]), "clean_in_scope": c["clean_scope"],
             "reason": no_clean_reason(c),
             "gate": [sorted(cl) for cl in c["gate"]] if c["gate"] else None}
            for c in clauses if not c["clean_pool"]]
gate_bad = {f"{c['policy']}/{c['id']}": c["gate_violations"][:5]
            for c in clauses if c["gate_violations"]}
result = {
    "format": "rerail.distill-cover/v1",
    "inputs": {"corpus": CORPUS, "lines": LINES, "profile": PROFILE, "policies": POLICIES},
    "method": {
        "weight": "profile.json pass 'all' per-file ms (bytes/1e5 where ms is 0); files with a "
                  "native budget failure weigh 10x while choosing (the SQL engine does not "
                  "emulate budget failures)",
        "cover": "weighted greedy set cover (new elements / added ms), then reverse redundancy "
                 "pruning",
        "elements": "per clause: hit (a finding file), clean (an eligible clean file); "
                    "select_when clauses also need a file where their exists regex holds",
        "calls_closure": "clauses using `calls` reach definitions through the workspace call "
                         "graph, which a subset run builds from the subset only: their findings "
                         "may need other files. Not in `files`; `calls_closure_addon` lists, per "
                         "calls clause, the finding file whose one-level import closure (files "
                         "holding the seed's RequiredText; every file where the seed requires "
                         "none) adds least to the cover. Heuristic: chains through files the hit "
                         "does not import are not followed",
        "clean_eligibility": "file in clause scope (code-rule scope from profile pass, .py for ast, "
                             "file.path regex/notRegex, evidence paths), no finding and no failure "
                             "for the clause, native RequiredText gate holds (ported from "
                             "required_text.rs; none for three-valued and regex queries); best "
                             "non-empty tier of: related (same dir as a finding file, or shares a "
                             f"distinctive import, df <= {DISTINCT:.0f} files) and contains every "
                             "extracted literal clause, related, literals, gate",
    },
    "totals": {
        "files": len(cover_files), "bytes": sum(size[i] for i in cover_files),
        "native_ms": round(sum(ms[i] for i in cover_files), 1),
        "corpus_files": len(files), "corpus_bytes": sum(size),
        "corpus_native_ms": round(sum(ms), 1),
        "clauses": len(clauses), "clauses_with_findings": sum(1 for c in clauses if c["hits"]),
        "clauses_hit_covered": sum(1 for r in per_clause if r["hit"]),
        "clauses_clean_covered": sum(1 for r in per_clause if r["clean"]),
        "elements": len(elements),
        "cover_findings": sum(r["cover_findings"] for r in per_clause),
        "corpus_findings": sum(r["findings"] for r in per_clause),
        "cover_files_with_native_failures": sorted(files[i] for i in cover_files
                                                   if i in failing_files),
    },
    "calls_closure_addon": {
        "files": sorted(files[i] for i in calls_addon),
        "count": len(calls_addon), "bytes": sum(size[i] for i in calls_addon),
        "native_ms": round(sum(ms[i] for i in calls_addon), 1),
    },
    "files": [{"path": files[i], "bytes": size[i], "native_ms": round(ms[i], 3),
               "why": why[i]} for i in cover_files],
    "clauses": per_clause,
    "clauses_without_findings": no_findings,
    "clauses_without_clean": no_clean,
    "gate_does_not_hold_on_finding_files": gate_bad,
    "top20_by_findings": [{"clause": r["clause"], "findings": r["findings"],
                           "finding_files": r["finding_files"],
                           "cover_findings": r["cover_findings"]} for r in top],
}
os.makedirs(os.path.dirname(OUT), exist_ok=True)
json.dump(result, open(OUT, "w"), indent=1)
print(json.dumps(result["totals"], indent=1))
print("clean tiers", collections.Counter(r["clean_tier"] for r in per_clause))
print("no clean", collections.Counter(r["reason"] for r in no_clean))
print("gate mismatches", len(gate_bad))
