"""Checks that ruff-air302-airflow3-moved-to-provider.moved.rr round-trips: regrouping the table by message gives
each clause's `old` list and message, in order."""
import importlib.util, json, re
spec = importlib.util.spec_from_file_location("gen", __file__.replace("check-", "gen-"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)
policy = json.load(open(gen.JSON))
want = [(c["forbid"]["ast"]["utils"]["old"]["resolves"]["import"], c["violation"]["message"])
        for c in policy["clauses"] if "forbid" in c]
got = []
for old, p, v, m, n in re.findall(r'air302_moved\("([^"]+)", "([^"]+)", "([^"]+)", "([^"]+)", "([^"]+)"\)\.', open(gen.OUT).read()):
    # The message names the old name's own last part as `$ORIG`.
    shown = "$ORIG" if any(old in w and "$ORIG" in msg for w, msg in want) else n
    msg = f"Install `apache-airflow-providers-{p}>={v}` and import `{shown}` from `{m}` instead."
    if got and got[-1][1] == msg and old not in got[-1][0] and any(got[-1][0][0] in w and old in w for w, _ in want):
        got[-1][0].append(old)
    else:
        got.append(([old], msg))
ok = got == want
print(f"round trip: {sum(len(o) for o, _ in got)} names in {len(got)} clauses, {'0 differences' if ok else 'DIFFERENT'}")
raise SystemExit(0 if ok else 1)
