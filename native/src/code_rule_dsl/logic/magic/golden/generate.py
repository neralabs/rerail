# Regenerates the goldens from T's rdl.py: python3 generate.py <dir with rdl.py and rdl/> <program.rdl> demand|adorn|gate <out>
# (gate: run with RDL_GATE=1 RDL_NATIVE=program).
import sys, json, os
front, prog_name, mode, out = sys.argv[1:5]
sys.path.insert(0, front)
import rdl
src = open(os.path.join(front, "rdl", prog_name)).read()
p = rdl.prune(rdl.parse(src))
if mode == "demand":
    p = rdl.prune(rdl.magic_sets(p))
else:
    p = rdl.prune(rdl.magic_adorn(p))
lines = []
for r, cols in sorted(p.decls.items()):
    if r.startswith(("magic_", "done_")):
        lines.append("decl " + r + "(" + ", ".join(f"{c}: {t}" for c, t in cols) + ")")
for h, b in p.rules:
    lines.append("rule " + repr(h) + " :- " + ", ".join(map(repr, b)))
for name, g in sorted(getattr(p, "gates", {}).items()):
    lines.append("gate " + name + " " + json.dumps(g, sort_keys=True))
rep = getattr(p, "magic_report", None)
if rep is not None:
    lines.append("report " + json.dumps(rep, sort_keys=True))
lines.append("dropped " + json.dumps(getattr(p, "demand_dropped", [])))
lines.append("extras " + json.dumps(getattr(p, "magic_extras", None)))
lines.append("demand " + json.dumps(p.demand))
open(out, "w").write("\n".join(lines) + "\n")
