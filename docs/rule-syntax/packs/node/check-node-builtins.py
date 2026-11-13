"""Cross-check n-no-unsupported-features-node-builtins.facts.rr against the policy JSON it was generated from, by a
round trip. Run: python3 check-node-builtins.py   (exit status 1 on any difference)

It rebuilds eslint-plugin-n's trace map from the table (modules, globals, import.meta; every API with its how and
versions), runs the JSON's own generator on it (scripts/generate/packs/node_policies.py, unsupported_builtins_doc and
compact_doc), and compares the result with the shipped JSON, compact utilities inlined: every value utility, every
clause's alternatives (as a multiset), and each clause's version gate, languageOptions and message. So a wrong path,
a wrong how (call for new), a wrong version or a missing row each changes some clause. It never parses the clauses'
alternatives itself, so it is independent of gen-node-builtins.py's reading of them. (After a reviewer's roundtrip.py.)
It also checks the rows values.rr reads: node_module, node_global and api_known.

With --plugin DIR (eslint-plugin-n 18.4.1 unpacked; or $ESLINT_PLUGIN_N) it checks the experimental columns, which
the JSON does not have, against the plugin's own trace map: every row's experimental versions, and its supported
ones, equal the plugin's, and the table has exactly the APIs the plugin traces. The plugin's files are read by
gen-node-builtins.py's PluginData; that reader is the one the supported comparison also exercises, against data the
round trip has tied to the JSON.
"""
import collections, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
sys.path.insert(0, os.path.join(REPO, "scripts", "generate", "packs"))
sys.dont_write_bytecode = True
import node_policies as NP  # noqa: E402
import importlib.util  # noqa: E402

FACTS = os.path.join(HERE, "n-no-unsupported-features-node-builtins.facts.rr")
JSON = os.path.join(REPO, "packs", "node", "rules", "n-no-unsupported-features-node-builtins.json")
FLAG = {"read": "$READ", "call": "$CALL", "new": "$CONSTRUCT"}


def read_facts():
    since, back, exp, specs, globals_, known = {}, collections.defaultdict(list), set(), {}, set(), set()
    trial = collections.defaultdict(list)
    for name, args in re.findall(r"^(api_since|api_backport|api_never|api_experimental_since|api_experimental_backport"
                                 r"|node_module|node_global|api_known)"
                                 r"\(([^)]*)\)\.", open(FACTS).read().replace(". api_", ".\napi_"), re.M):
        a = json.loads("[" + args + "]")
        if name == "api_since":
            assert (a[0], a[1]) not in since, a
            since[(a[0], a[1])] = a[2]
        elif name == "api_backport":
            back[(a[0], a[1])].append(a[2])
        elif name == "api_never":
            exp.add((a[0], a[1]))
        elif name == "api_experimental_since":
            assert (a[0], a[1]) not in trial, a
            trial[(a[0], a[1])].insert(0, a[2])
        elif name == "api_experimental_backport":
            assert (a[0], a[1]) in trial, a
            trial[(a[0], a[1])].append(a[2])
        elif name == "node_module":
            specs[a[0]] = a[1]
        elif name == "node_global":
            globals_.add(a[0])
        else:
            known.add(a[0])
    return since, back, exp, specs, globals_, known, dict(trial)


def trace_map(since, back, exp, specs):
    """The table as the generator's input: {modules, globals, meta}, each a trie with $READ/$CALL/$CONSTRUCT infos."""
    def info(k):
        # The JSON was lowered with allowExperimental off, so no experimental versions reach it (or the table).
        return {"supported": ([since[k]] + back[k]) if k in since else [], "experimental": []}

    def put(trie, path, flag, inf):
        for p in path:
            trie = trie.setdefault(p, {})
        assert flag not in trie, path
        trie[flag] = inf

    roots, globals_, meta = {r: {} for r in specs.values()}, {}, {}
    for k in sorted(set(since) | exp):
        api, how = k
        if how == "load":
            assert api in specs, api
        elif "#" in api:
            r, rest = api.split("#", 1)
            put(roots.setdefault(r, {}), rest.split("."), FLAG[how], info(k))
        elif api.startswith("import.meta."):
            put(meta, api[len("import.meta."):].split("."), FLAG[how], info(k))
        else:
            put(globals_, api.split("."), FLAG[how], info(k))
    modules = {}
    for s, r in sorted(specs.items()):
        m = json.loads(json.dumps(roots[r]))
        if (s, "load") in since or (s, "load") in exp:
            m["$READ"] = info((s, "load"))
        modules[s] = m
    return {"modules": modules, "globals": globals_, "meta": meta}


def inline(t, utils):
    if isinstance(t, dict):
        if set(t) == {"matches"} and re.fullmatch(r"c\d+", t["matches"]):
            return inline(utils[t["matches"]], utils)
        out = {}
        for k, v in t.items():
            if k == "matches" and isinstance(v, str) and re.fullmatch(r"c\d+", v):
                out.update(inline(utils[v], utils))
                continue
            out[k] = inline(v, utils)
        return out
    if isinstance(t, list):
        return [inline(x, utils) for x in t]
    if isinstance(t, str):   # alternation order inside a name regex is not meaningful
        m = re.fullmatch(r"\^\(\?:(.*)\)\$", t)
        if m and "(" not in m.group(1):
            return "^(?:" + "|".join(sorted(m.group(1).split("|"))) + ")$"
    return t


def canon(d):
    u = d["clauses"][0]["define"]["utils"]
    utils = {k: json.dumps(inline(v, u), sort_keys=True) for k, v in u.items() if not re.fullmatch(r"c\d+", k)}
    alts, meta = collections.defaultdict(collections.Counter), {}
    for c in d["clauses"][1:]:
        base = re.sub(r"-export-all-\d+$", "-export-all", c["id"])
        rule = inline(c["forbid"]["ast"]["rule"], u)
        for a in (rule["any"] if set(rule) == {"any"} else [rule]):
            alts[base][json.dumps(a, sort_keys=True)] += 1
        meta[base] = (json.dumps(c["forbid"]["ast"].get("file"), sort_keys=True),
                      json.dumps(c["forbid"]["ast"].get("languageOptions"), sort_keys=True), c["violation"]["message"])
    return utils, alts, meta


def main():
    since, back, exp, specs, globals_, known, trial = read_facts()
    bad = []
    if set(since) & exp:
        bad.append(f"both supported and experimental: {sorted(set(since) & exp)[:5]}")
    if not set(back) <= set(since):
        bad.append(f"backports without a since: {sorted(set(back) - set(since))[:5]}")
    maps = trace_map(since, back, exp, specs)
    # node_global: the globals traced (the generator traces a global of globals.node, and globalThis)
    if set(maps["globals"]) - globals_:
        bad.append(f"tabled globals missing from node_global: {sorted(set(maps['globals']) - globals_)[:5]}")
    mine = NP.compact_doc(NP.unsupported_builtins_doc(maps, globals_ - {"globalThis"}, "node-builtins"))
    ref = json.load(open(JSON))
    (u1, c1, m1), (u2, c2, m2) = canon(mine), canon(ref)
    for k in sorted(set(u1) | set(u2)):
        if u1.get(k) != u2.get(k):
            bad.append(f"utility {k}: {(u1.get(k) or '-')[:160]} vs JSON {(u2.get(k) or '-')[:160]}")
    for k in sorted(set(c1) | set(c2)):
        a, b = c1.get(k, collections.Counter()), c2.get(k, collections.Counter())
        if a != b:
            bad.append(f"clause {k}: table-only {[x[:160] for x in a - b][:2]} JSON-only {[x[:160] for x in b - a][:2]}")
        if m1.get(k) != m2.get(k):
            bad.append(f"clause {k}: gate/options/message {m1.get(k)} vs JSON {m2.get(k)}")
    # node_module: every specifier the JSON's source tests accept, with its member root
    want = {}
    for u, body in ref["clauses"][0]["define"]["utils"].items():
        m = re.fullmatch(r"m:(.*):esm-source", u)
        if m:
            rx = body["value"]["regex"]
            b = re.fullmatch(r"\^\(\?:node:\)\?(.*)\$", rx)
            for s in ([b.group(1), "node:" + b.group(1)] if b else [rx[1:-1]]):
                want[s.replace("\\", "")] = m.group(1)
    if want != specs:
        bad.append(f"node_module differs: {sorted(set(want.items()) ^ set(specs.items()))[:6]}")
    # api_known: exactly the API names and their prefixes below the root
    names = {a for (a, h) in set(since) | exp if h != "load"}
    expect = set()
    for a in names:
        root, sep, rest = a.partition("#") if "#" in a else ("", "", a)
        parts = rest.split(".")
        for i in range(2 if rest.startswith("import.meta") else 1, len(parts) + 1):
            expect.add(root + sep + ".".join(parts[:i]))
    if expect != known:
        bad.append(f"api_known differs: {sorted(expect ^ known)[:6]}")

    if not set(trial) <= set(since) | exp:
        bad.append(f"experimental versions for untabled APIs: {sorted(set(trial) - set(since) - exp)[:5]}")
    plugin = None
    arg = sys.argv[sys.argv.index("--plugin") + 1] if "--plugin" in sys.argv else os.environ.get("ESLINT_PLUGIN_N")
    if arg:
        spec = importlib.util.spec_from_file_location("gen", os.path.join(HERE, "gen-node-builtins.py"))
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)
        plugin = gen.PluginData(arg).rows(specs, globals_)
        rows = set(since) | exp
        if set(plugin) != rows:
            bad.append(f"plugin-only {sorted(set(plugin) - rows)[:5]}, table-only {sorted(rows - set(plugin))[:5]}")
        for k in sorted(rows & set(plugin)):
            sup = ([since[k]] + back[k]) if k in since else []
            if sorted(sup) != sorted(plugin[k]["supported"]) or gen.latest_first(sup) != sup:
                bad.append(f"{k}: supported {sup}, plugin {plugin[k]['supported']}")
            e = trial.get(k, [])
            if sorted(e) != sorted(plugin[k]["experimental"]) or gen.latest_first(e) != e:
                bad.append(f"{k}: experimental {e}, plugin {plugin[k]['experimental']}")

    count = collections.Counter(h for (_, h) in set(since) | exp)
    print(f"table: {len(set(since) | exp)} APIs ({dict(count)}), {len(exp)} with no supported release, {sum(1 for v in back.values() if v)} "
          f"with backports; {len(specs)} module specifiers, {len(globals_)} globals, {len(known)} traced names")
    print(f"round trip: utilities {len(u1)}/{len(u2)}, clauses {len(c1)}/{len(c2)}, alternatives "
          f"{sum(sum(c.values()) for c in c1.values())}/{sum(sum(c.values()) for c in c2.values())} (table/JSON)")
    print(f"plugin data: {len(trial)} APIs with experimental versions; " +
          (f"checked against the plugin's {len(plugin)} traced APIs" if plugin is not None
           else "NOT checked against the plugin (pass --plugin DIR)"))
    for b in bad:
        print("MISMATCH", b)
    print("ok" if not bad else f"{len(bad)} mismatches")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
