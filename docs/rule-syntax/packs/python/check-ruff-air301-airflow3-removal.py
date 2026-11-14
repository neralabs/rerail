"""Checks that ruff-air301-airflow3-removal.names.rr round-trips: per name clause, the table's old names are the
clause's `old-name` list (with its prefix flag), its messages are the clause's message with `$N` and `$L` filled in, and the guard's module and name are the ones the clause's `new-import` util matches."""
import importlib.util, json, re
spec = importlib.util.spec_from_file_location("gen", __file__.replace("check-", "gen-"))
gen = importlib.util.module_from_spec(spec); spec.loader.exec_module(gen)
policy = json.load(open(gen.JSON))
got = [tuple(json.loads("[" + r + "]")) for r in re.findall(r"^air301_name\((.*)\)\.$", open(gen.OUT).read(), re.M)]
bad = 0; i = 0
for c in gen.clauses(policy):
    ut = c["forbid"]["ast"]["utils"]; res = ut["old-name"]["resolves"]
    for q in res["import"]:
        old, how, m, x, msg = got[i]; i += 1
        want = c["violation"]["message"].replace("$N", q).replace("$L", q.rsplit(".", 1)[1])
        ok = old == q and how == ("prefix" if res.get("prefix") else "exact") and msg == want
        if "guarded" in ut:
            ok = ok and re.escape(m) in json.dumps(ut["new-import"]).replace("\\\\", "\\") and x in json.dumps(ut["new-import"])
        else:
            ok = ok and m == x == ""
        bad += not ok
ok = bad == 0 and i == len(got)
print(f"round trip: {len(got)} names, {'0 differences' if ok else f'{bad} DIFFERENT'}")
raise SystemExit(0 if ok else 1)
