"""The findings explorer's data: every method's blocks on both scopes, post-processed and grouped, with their code.

usage: python3 explorer_data.py <corpus root> <findings dir> <out.json> [--jeff native=<stdout.json> --jeff stp=<stdout.json>]
                                [--timings <timings.tsv>]

<findings dir>: run_all.sh's output, `<scope>-<method>.tsv`. Post-processing is merge.py's, for all methods:
- a block whose other places all lie inside it matches only itself (a structure repeated every few lines): noise;
- in a data file (json, md) the structure-based methods' blocks are noise (a record's nodes are its schema);
- per file, the kept blocks of every method that overlap are one finding; findings naming each other, one class.
"""
import json
import sys
from collections import defaultdict

METHODS = [
    ("lines", "Lines", "counted lines"),
    ("tlines-text", "Tape tokens", "counted lines"),
    ("tlines-shape", "Tape shapes", "counted lines"),
    ("nodes-row", "Node rows", "nodes"),
    ("nodes-local", "Node symbols", "nodes"),
]
STRUCTURAL = {"tlines-shape", "nodes-row", "nodes-local"}
DATA = (".json", ".md")
SCOPES = [("native", "native/src (Rust)"), ("stp", "src, test, packs")]
CONTEXT = 3


def place(o):
    p, l = o.rsplit(":", 1)
    return p, int(l)


def main():
    args = sys.argv[1:]
    root, fdir, out = args[:3]
    jeff = {}
    timings = []
    note = ""
    i = 3
    while i < len(args):
        if args[i] == "--jeff":
            k, v = args[i + 1].split("=", 1)
            jeff[k] = v
        elif args[i] == "--timings":
            for line in open(args[i + 1]):
                if line.startswith("#note\t"):
                    note = line.rstrip("\n").split("\t", 1)[1]
                elif line.strip() and not line.startswith("#"):
                    timings.append(line.rstrip("\n").split("\t"))
        i += 2

    files = {}

    def text(path):
        if path not in files:
            try:
                files[path] = open(f"{root}/{path}", encoding="utf-8").read().split("\n")
            except (OSError, UnicodeDecodeError):
                files[path] = []
        return files[path]

    data = {"methods": [{"id": m, "name": n, "unit": u, "structural": m in STRUCTURAL} for m, n, u in METHODS],
            "scopes": [], "findings": [], "classes": [], "timings": timings, "timings_note": note}
    fid = 0
    for scope, scope_name in SCOPES:
        blocks = []  # (path, s, e, method, units, others, noise)
        raw = {}
        for m, _, _ in METHODS:
            rows = []
            for line in open(f"{fdir}/{scope}-{m}.tsv"):
                path, s, e, units, others = (line.rstrip("\n").split("\t") + [""])[:5]
                s, e, units = int(s), int(e), int(units)
                others = [o for o in others.split(", ") if o]
                rows.append((path, s, e, units, others))
                noise = None
                if m in STRUCTURAL and path.endswith(DATA):
                    noise = "data file"
                elif not [o for o in others if not (place(o)[0] == path and s <= place(o)[1] <= e)]:
                    noise = "matches only itself"
                blocks.append((path, s, e, m, units, others, noise))
            raw[m] = rows
        # Jeff's engine output, row for row against the lines method.
        jeff_same = None
        if scope in jeff:
            d = json.load(open(jeff[scope]))
            theirs = sorted((x["path"], x["parameters"]["s"], x["parameters"]["z"], x["parameters"]["n"], x["parameters"]["others"]) for x in d["diagnostics"])
            mine = sorted((p, s, e, u, ", ".join(o)) for p, s, e, u, o in raw["lines"])
            jeff_same = {"engine": len(theirs), "sa": len(mine), "identical": len(set(theirs) & set(mine))}

        # Union per file: kept blocks into findings; noise blocks that overlap no kept block into noise findings.
        per = defaultdict(list)
        for b in blocks:
            per[b[0]].append(b)
        found = []
        for path in sorted(per):
            for noise_pass in (False, True):
                bs = sorted((b for b in per[path] if (b[6] is not None) == noise_pass), key=lambda b: (b[1], b[2]))
                if noise_pass:
                    kept = [(f["s"], f["e"]) for f in found if f["path"] == path]
                    bs = [b for b in bs if not any(s <= b[2] and b[1] <= e for s, e in kept)]
                cur = None
                for b in bs:
                    if cur and b[1] <= cur["e"]:
                        cur["e"] = max(cur["e"], b[2])
                    else:
                        cur = {"path": path, "s": b[1], "e": b[2], "by": defaultdict(list), "noise": noise_pass}
                        found.append(cur)
                    cur["by"][b[3]].append({"s": b[1], "e": b[2], "units": b[4], "others": b[5], "noise": b[6]})
        # Classes over the kept findings.
        idx = defaultdict(list)
        for k, f in enumerate(found):
            if not f["noise"]:
                idx[f["path"]].append(k)
        parent = list(range(len(found)))

        def root_of(k):
            while parent[k] != k:
                parent[k] = parent[parent[k]]
                k = parent[k]
            return k

        unreported = defaultdict(set)  # class root -> other places no finding holds
        for k, f in enumerate(found):
            if f["noise"]:
                continue
            for blist in f["by"].values():
                for b in blist:
                    for o in b["others"]:
                        p, l = place(o)
                        if p == f["path"] and f["s"] <= l <= f["e"]:
                            continue
                        hit = [j for j in idx[p] if found[j]["s"] <= l <= found[j]["e"]]
                        for j in hit:
                            parent[root_of(k)] = root_of(j)
                        if not hit:
                            unreported[k].add((p, l, b["e"] - b["s"] + 1))
        groups = defaultdict(list)
        for k, f in enumerate(found):
            groups[root_of(k) if not f["noise"] else ("noise", k)].append(k)

        base = fid
        for k, f in enumerate(found):
            lines = text(f["path"])
            a, z = max(1, f["s"] - CONTEXT), min(len(lines), f["e"] + CONTEXT)
            data["findings"].append({
                "id": base + k, "scope": scope, "path": f["path"], "s": f["s"], "e": f["e"], "noise": f["noise"],
                "by": {m: v for m, v in f["by"].items()}, "from": a, "code": lines[a - 1:z],
            })
        fid = base + len(found)
        for key, ks in groups.items():
            extra = []
            for k in ks:
                for p, l, n in sorted(unreported.get(k, ())):
                    lines = text(p)
                    a, z = max(1, l - CONTEXT), min(len(lines), l + n - 1 + CONTEXT)
                    extra.append({"path": p, "s": l, "e": min(len(lines), l + n - 1), "from": a, "code": lines[a - 1:z]})
            seen = set()
            extra = [x for x in extra if not ((x["path"], x["s"]) in seen or seen.add((x["path"], x["s"])))]
            fs = [data["findings"][base + k] for k in ks]
            methods = sorted({m for f in fs for m in f["by"]}, key=[m for m, _, _ in METHODS].index)
            data["classes"].append({
                "id": len(data["classes"]) + 1, "scope": scope, "findings": [base + k for k in sorted(ks, key=lambda k: (found[k]["path"], found[k]["s"]))],
                "lines": max(f["e"] - f["s"] + 1 for f in fs), "methods": methods, "noise": fs[0]["noise"], "unreported": extra,
            })
        counts = {m: len(raw[m]) for m, _, _ in METHODS}
        data["scopes"].append({"id": scope, "name": scope_name, "blocks": counts, "jeff": jeff_same})
    json.dump(data, open(out, "w"), separators=(",", ":"))
    kept = [c for c in data["classes"] if not c["noise"]]
    print(f"{len(data['findings'])} findings, {len(kept)} classes, {len(data['classes']) - len(kept)} noise groups", file=sys.stderr)


main()
