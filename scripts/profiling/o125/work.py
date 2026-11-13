"""O125: a run's native join work (RERAIL_PROFILE native.tsv) in the cross part, by kind of rule (dev tool)."""
import re, sys
def kinds(path):
    out = {}
    for line in open(path).read().splitlines()[1:]:
        span, work, _, plan = line.split("\t", 3)
        if not span.startswith("cross"):
            continue
        m = re.match(r"(?:seed|join) (\w+)\(([^)]*)\)[^:]*: (.*)", plan)
        if not m:
            k = "other"
        else:
            head, body = m[1], m[3]
            base = re.sub(r"__(del|ins|minus|plus)$", "", head)
            if head.endswith("__del"): k = "over-delete"
            elif head.endswith("__ins"): k = "insert"
            elif head.endswith("__minus") or head.endswith("__plus"): k = "deltas (r__minus, r__plus)"
            elif f"{head}__old(" in body and f"!{head}__del(" in body: k = "survivor copy"
            elif body.strip().startswith(f"{head}__ins("): k = "union of inserts"
            elif f"{head}__del(" in body: k = "re-derive"
            else: k = "rules as given (strata derived again)"
        out[k] = out.get(k, 0) + int(work)
    return out
for p in sys.argv[1:]:
    o = kinds(p); t = sum(o.values())
    print(p.rsplit("/", 1)[-1], f"cross native work {t:,}")
    for k, v in sorted(o.items(), key=lambda x: -x[1]):
        print(f"  {k}: {v:,} ({100*v/t:.0f}%)")
