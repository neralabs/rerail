"""Checks ruff-air312-airflow3-suggested-to-move-to-provider.moved.rr against its two sources: its old names are the
JSON's `deprecated` list, in order, and each (old, module, name) row is an arm of Ruff 0.16.9's
suggested_to_move_to_provider_in_3 (the commit the JSON was written from), read from the Rust source."""
import json, os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__))
JSON = os.path.join(HERE, "..", "..", "..", "..", "packs", "python", "rules", "ruff-air312-airflow3-suggested-to-move-to-provider.json")
OUT = os.path.join(HERE, "ruff-air312-airflow3-suggested-to-move-to-provider.moved.rr")
RUST = os.environ.get("RUFF_SRC", "/Users/jeremy/orca/workspaces/rerail/wolfeel/.rerail/port-readiness/upstream/ruff/"
                      "0be08a206f9c3180afd3e93bcc792ed5cb1f4db1/crates/ruff_linter") + \
    "/src/rules/airflow/rules/suggested_to_move_to_provider_in_3.rs"
got = re.findall(r'air312_moved\("([^"]+)", "([^"]+)", "([^"]+)"\)\.', open(OUT).read())
olds = json.load(open(JSON))["clauses"][0]["forbid"]["ast"]["utils"]["deprecated"]["resolves"]["import"]
src = open(RUST).read()
src = src[src.index("let replacement = match"):]
ruff = set()
for m in re.finditer(r'\[([^\]]*)\]\s*=>\s*\{?\s*ProviderReplacement::\w+\s*\{(.*?)\}', src, re.S):
    segs = [s.strip() for s in m.group(1).split(",") if s.strip()]
    module = re.search(r'module:\s*"([^"]+)"', m.group(2)).group(1)
    named = re.search(r'name:\s*"([^"]+)"', m.group(2))
    last = segs[-1]
    alts = re.findall(r'"([^"]+)"', last)
    head = ".".join(s.strip('"') for s in segs[:-1])
    for x in alts:
        ruff.add((head + "." + x, module, named.group(1) if named else x))
ok = [o for o, _, _ in got] == olds and set(got) == ruff
print(f"round trip: {len(got)} names, {len(ruff)} Ruff arms, {'0 differences' if ok else 'DIFFERENT'}")
if not ok:
    print(sorted(set(got) ^ ruff), file=sys.stderr)
raise SystemExit(0 if ok else 1)
