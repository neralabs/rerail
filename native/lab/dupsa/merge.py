"""The union of two `dupsa` outputs: per file, blocks of either that overlap are one finding, spanning both, with
the other places of both and which outputs found it.

usage: python3 merge.py A.tsv B.tsv [--names lines,nodes] [--out merged.tsv]
Output: path, first line, last line, lines, found by (A, B or A+B), other places.
"""
import sys
from collections import defaultdict


def load(p, tag):
    for line in open(p):
        path, first, last, _units, others = (line.rstrip("\n").split("\t") + [""])[:5]
        yield path, int(first), int(last), tag, [o for o in others.split(", ") if o]


def main():
    args = sys.argv[1:]
    names = ["A", "B"]
    out = sys.stdout
    if "--names" in args:
        names = args[args.index("--names") + 1].split(",")
    if "--out" in args:
        out = open(args[args.index("--out") + 1], "w")
    by = defaultdict(list)
    for i, p in enumerate(args[:2]):
        for path, s, e, tag, others in load(p, names[i]):
            by[path].append((s, e, tag, others))
    counts = defaultdict(int)
    lines = defaultdict(int)
    for path in sorted(by):
        cur = None
        for s, e, tag, others in sorted(by[path]):
            if cur and s <= cur[1]:
                cur[1] = max(cur[1], e)
                cur[2].add(tag)
                cur[3].update(others)
            else:
                if cur:
                    emit(out, path, cur, names, counts, lines)
                cur = [s, e, {tag}, set(others)]
        emit(out, path, cur, names, counts, lines)
    for k in sorted(counts):
        print(f"{k}: {counts[k]} findings, {lines[k]} lines", file=sys.stderr)
    print(f"all: {sum(counts.values())} findings, {sum(lines.values())} lines", file=sys.stderr)


def emit(out, path, cur, names, counts, lines):
    s, e, tags, others = cur
    found = "+".join(n for n in names if n in tags)
    counts[found] += 1
    lines[found] += e - s + 1
    print(f"{path}\t{s}\t{e}\t{e - s + 1}\t{found}\t{', '.join(sorted(others))}", file=out)


main()
