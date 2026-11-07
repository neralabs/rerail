#!/usr/bin/env python3
"""Check lib/python/security-blacklist.rr.

1. Round trip: read the table back, rebuild what each rule's JSON says (its `resolves.import` list; its `import M`,
   `import M as $ALIAS`, `from M import $$$NAMES`, `from P import X`, `from P import X as $ALIAS` patterns) and diff.
2. With --bandit DIR (an installed bandit package directory, e.g. site-packages/bandit): every list equals Bandit's
   own (bandit/blacklists/calls.py and imports.py, read with `ast`, nothing imported), in Bandit's order.

    python3 packs/python/check-bandit-blacklist.py [--bandit DIR]
"""
import ast
import importlib.util
import json
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gen", HERE / "gen-bandit-blacklist.py")
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)


def read_table():
    calls, imports = {}, {}
    for line in gen.OUT.read_text().splitlines():
        m = re.fullmatch(r'blacklist_call\("(B\d+)", "([^"]+)"\)\.', line)
        if m:
            calls.setdefault(m.group(1), []).append(m.group(2))
        m = re.fullmatch(r'blacklist_import\("(B\d+)", (\d+), "([^"]+)"\)\.', line)
        if m:
            imports.setdefault((int(m.group(2)), m.group(1)), []).append(m.group(3))
    return calls, dict((c, ms) for (_, c), ms in sorted(imports.items()))


def import_patterns(m):
    out = [f"import {m}", f"import {m} as $ALIAS", f"from {m} import $$$NAMES"]
    if "." in m:
        p, x = m.rsplit(".", 1)
        out += [f"from {p} import {x}", f"from {p} import {x} as $ALIAS"]
    return out


def round_trip(calls, imports):
    bad = 0
    for code, qs in calls.items():
        doc = json.load(open(gen.rule_file(code)))
        if code in gen.EXTRA:
            ok = code == "B307" and '"regex":"^eval$"' in json.dumps(doc, separators=(",", ":"))
            want = qs if ok else ["<JSON does not spell eval>"]
        else:
            want = gen.call_names(doc)
        if want != qs:
            bad += 1; print(f"{code}: table {qs} != JSON {want}")
    for code, ms in imports.items():
        f = gen.rule_file(code)
        if f is None:
            continue
        doc = json.load(open(f))
        have = [a["pattern"] for c in doc["clauses"] for a in c["forbid"]["ast"]["rule"].get("any", [])]
        want = [p for m in ms for p in import_patterns(m)]
        if have != want:
            bad += 1; print(f"{code}: JSON patterns {have} != rebuilt {want}")
    return bad


def bandit_lists(d):
    out = {}
    for name in ("calls.py", "imports.py"):
        for n in ast.walk(ast.parse((pathlib.Path(d) / "blacklists" / name).read_text())):
            if isinstance(n, ast.Call) and getattr(n.func, "attr", None) == "build_conf_dict":
                code, qs = n.args[1].value, [e.value for e in n.args[3].elts]
                out[code] = list(dict.fromkeys(qs))
    return out


def main():
    calls, imports = read_table()
    if (calls, imports) != gen.table():
        print("the table is stale: rerun gen-bandit-blacklist.py"); sys.exit(1)
    bad = round_trip(calls, imports)
    n = sum(map(len, calls.values())) + sum(map(len, imports.values()))
    print(f"round trip: {len(calls)} call lists, {len(imports)} import lists, {n} rows, {bad} differences")
    if "--bandit" in sys.argv:
        b = bandit_lists(sys.argv[sys.argv.index("--bandit") + 1])
        diff = [c for c, qs in {**calls, **imports}.items() if b.get(c) != qs]
        order = [c for c in b if c.startswith("B4")] == list(imports)
        print(f"bandit: {len(b)} lists; {len(diff)} differ {diff}; import order {'matches' if order else 'differs'}")
        bad += len(diff) + (not order)
    else:
        print("bandit: not checked (pass --bandit DIR)")
    print("ok" if not bad else "FAILED")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
