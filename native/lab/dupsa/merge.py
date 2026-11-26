"""The union of two `dupsa` outputs, post-processed.

usage: python3 merge.py A.tsv B.tsv [--names lines,nodes] [--data json,md] [--out merged.tsv] [--classes classes.tsv]

0. Data files: in a file of a `--data` extension, only A's (the lines') blocks count. A data file's nodes are its
   schema: two records of one shape match whatever their values.
1. Self-overlap: a block's other places that lie inside its own span (same file) are dropped, and a block left with
   none is dropped: a structure repeated every few lines matches itself shifted, which is not a copy.
2. Union: per file, blocks of either output that overlap are one finding, spanning both, with the other places of
   both and which outputs found it.
3. Classes: findings that name each other (an other place lies in a finding) are one clone class.

--out: path, first line, last line, lines, found by (A, B or A+B), class, other places.
--classes: class, copies, lines (the largest copy), found by, the copies (path:first-last).
"""
import sys
from collections import defaultdict


def load(p, tag):
    for line in open(p):
        path, first, last, _units, others = (line.rstrip("\n").split("\t") + [""])[:5]
        yield path, int(first), int(last), tag, [o for o in others.split(", ") if o]


def place(o):
    p, l = o.rsplit(":", 1)
    return p, int(l)


def main():
    args = sys.argv[1:]
    opt = lambda k: args[args.index(k) + 1] if k in args else None
    names = (opt("--names") or "A,B").split(",")
    stats = defaultdict(lambda: [0, 0])

    data = tuple("." + x for x in (opt("--data") or "json,md").split(",") if x)
    # 1. Self-overlap.
    by = defaultdict(list)
    for i, p in enumerate(args[:2]):
        for path, s, e, tag, others in load(p, names[i]):
            if i == 1 and path.endswith(data):
                stats[f"{tag}: dropped, a data file"][0] += 1
                stats[f"{tag}: dropped, a data file"][1] += e - s + 1
                continue
            kept = [o for o in others if not (place(o)[0] == path and s <= place(o)[1] <= e)]
            if not kept:
                stats[f"{tag}: dropped, matches only itself"][0] += 1
                stats[f"{tag}: dropped, matches only itself"][1] += e - s + 1
                continue
            by[path].append((s, e, tag, kept))

    # 2. Union per file.
    findings = []  # [path, s, e, tags, others]
    for path in sorted(by):
        cur = None
        for s, e, tag, others in sorted(by[path]):
            if cur and s <= cur[2]:
                cur[2] = max(cur[2], e)
                cur[3].add(tag)
                cur[4].update(others)
            else:
                if cur:
                    findings.append(cur)
                cur = [path, s, e, {tag}, set(others)]
        findings.append(cur)
    # An other place inside the finding itself (two blocks merged into one) is not another copy.
    for f in findings:
        f[4] = {o for o in f[4] if not (place(o)[0] == f[0] and f[1] <= place(o)[1] <= f[2])}

    # 3. Classes: union-find over findings, linked by the other places that fall in a finding.
    index = defaultdict(list)
    for k, f in enumerate(findings):
        index[f[0]].append(k)
    parent = list(range(len(findings)))

    def root(k):
        while parent[k] != k:
            parent[k] = parent[parent[k]]
            k = parent[k]
        return k

    for k, f in enumerate(findings):
        for o in f[4]:
            p, l = place(o)
            for j in index[p]:
                if findings[j][1] <= l <= findings[j][2]:
                    parent[root(k)] = root(j)
    classes = defaultdict(list)
    for k in range(len(findings)):
        classes[root(k)].append(k)
    order = sorted(classes.values(), key=lambda ks: -max(findings[k][2] - findings[k][1] + 1 for k in ks))
    class_of = {k: c + 1 for c, ks in enumerate(order) for k in ks}

    found = lambda tags: "+".join(n for n in names if n in tags)
    out = open(opt("--out"), "w") if opt("--out") else sys.stdout
    for k, (path, s, e, tags, others) in enumerate(findings):
        stats[f"finding, found by {found(tags)}"][0] += 1
        stats[f"finding, found by {found(tags)}"][1] += e - s + 1
        print(f"{path}\t{s}\t{e}\t{e - s + 1}\t{found(tags)}\t{class_of[k]}\t{', '.join(sorted(others))}", file=out)
    if opt("--classes"):
        with open(opt("--classes"), "w") as c:
            for n, ks in enumerate(order):
                tags = set().union(*(findings[k][3] for k in ks))
                big = max(findings[k][2] - findings[k][1] + 1 for k in ks)
                copies = ", ".join(f"{findings[k][0]}:{findings[k][1]}-{findings[k][2]}" for k in sorted(ks, key=lambda k: findings[k][:2]))
                print(f"{n + 1}\t{len(ks)}\t{big}\t{found(tags)}\t{copies}", file=c)
    for k in sorted(stats):
        print(f"{k}: {stats[k][0]} ({stats[k][1]} lines)", file=sys.stderr)
    singles = sum(1 for ks in order if len(ks) == 1)
    print(f"{len(findings)} findings in {len(order)} classes ({singles} of one finding: its copies were not reported)", file=sys.stderr)


main()
