"""compare.py MAP ORIGINAL.tsv MOVED.tsv: the answers that differ once the moved layout's places are mapped back."""
import sys

m = {}
for row in open(sys.argv[1]):
    a, b, c, d = row.rstrip("\n").split("\t")
    m[(a, b)] = (c, d)


def back(place):
    if place in ("-", "<library>"):
        return place
    parts = place.split(":")
    path, line, rest = parts[0], parts[1], parts[2:]
    p, l = m.get((path, line), (path, line))
    return ":".join([p, l] + rest)


orig = {l.rstrip("\n") for l in open(sys.argv[2]) if l.strip()}
moved = set()
for l in open(sys.argv[3]):
    if l.strip():
        s, k, t = l.rstrip("\n").split("\t")
        moved.add(f"{back(s)}\t{k}\t{back(t)}")
lost, gained = sorted(orig - moved), sorted(moved - orig)
print(f"answers: original {len(orig)}, moved {len(moved)}; only original {len(lost)}, only moved {len(gained)}")
for x in lost[:40]:
    print("-", x)
for x in gained[:40]:
    print("+", x)
