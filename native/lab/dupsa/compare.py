"""Compare two `dupsa` outputs: which blocks of each overlap a block of the other in the same file.

usage: python3 compare.py A.tsv B.tsv [--show N]
"""
import sys
from collections import defaultdict


def load(p):
    out = []
    for line in open(p):
        path, first, last, units, others = (line.rstrip("\n").split("\t") + [""])[:5]
        out.append((path, int(first), int(last), int(units), others))
    return out


def lines_covered(rows):
    return sum(r[2] - r[1] + 1 for r in rows)


def main():
    a, b = load(sys.argv[1]), load(sys.argv[2])
    show = int(sys.argv[sys.argv.index("--show") + 1]) if "--show" in sys.argv else 5
    by = defaultdict(list)
    for r in b:
        by[r[0]].append(r)
    hit = lambda r, idx: any(s[1] <= r[2] and r[1] <= s[2] for s in idx[r[0]])
    by_a = defaultdict(list)
    for r in a:
        by_a[r[0]].append(r)
    a_only = [r for r in a if not hit(r, by)]
    b_only = [r for r in b if not hit(r, by_a)]
    print(f"A {sys.argv[1]}: {len(a)} blocks, {lines_covered(a)} lines; {len(a) - len(a_only)} overlap B, {len(a_only)} A only")
    print(f"B {sys.argv[2]}: {len(b)} blocks, {lines_covered(b)} lines; {len(b) - len(b_only)} overlap A, {len(b_only)} B only")
    for name, rows in (("A only", a_only), ("B only", b_only)):
        rows = sorted(rows, key=lambda r: -(r[2] - r[1]))
        print(f"-- {name}, largest {show}:")
        for r in rows[:show]:
            print(f"   {r[0]}:{r[1]}-{r[2]} ({r[2] - r[1] + 1} lines) = {r[4][:150]}")


main()
