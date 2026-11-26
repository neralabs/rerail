"""compare.py MAP ORIG_ROOT MOVED_ROOT ORIG_CSV_DIR MOVED_CSV_DIR REL...: a relation's rows that differ once each
node (`path:slot.index`) is its line in its layout (its start, from `answer_at`), and the moved layout's lines are mapped back (t22's compare.py)."""
import bisect, sys

m = {}
for row in open(sys.argv[1]):
    a, b, c, d = row.rstrip("\n").split("\t")
    m[(a, int(b))] = (c, int(d))
orig_root, moved_root, orig_dir, moved_dir = sys.argv[2:6]
starts = {}


def line_of(root, path, byte):
    key = (root, path)
    if key not in starts:
        try:
            text = open(f"{root}/{path}", "rb").read()
        except OSError:
            text = b""
        starts[key] = [0] + [i + 1 for i, c in enumerate(text) if c == 10]
    return bisect.bisect_right(starts[key], byte)


spans = {}


def place(root, value, back):
    path, sep, rest = value.rpartition(":")
    if not sep or (root, value) not in spans:
        return value
    line = line_of(root, path, spans[(root, value)])
    if back:
        path, line = m.get((path, line), (path, line))
    return f"{path}:{line}"


def rows(root, folder, rel, back):
    if (root, "spans") not in spans:
        spans[(root, "spans")] = 1
        for l in open(f"{folder}/answer_at.csv"):
            if l.strip():
                node, start = l.rstrip("\n").split("\t")
                spans[(root, node)] = int(start)
    out = set()
    for l in open(f"{folder}/{rel}.csv"):
        if l.strip():
            out.add("\t".join(place(root, v, back) for v in l.rstrip("\n").split("\t")))
    return out


for rel in sys.argv[6:]:
    a, b = rows(orig_root, orig_dir, rel, False), rows(moved_root, moved_dir, rel, True)
    print(f"{rel}: original {len(a)}, moved {len(b)}; only original {len(a - b)}, only moved {len(b - a)}")
    for x in sorted(a - b)[:5]:
        print("  -", x)
    for x in sorted(b - a)[:5]:
        print("  +", x)
