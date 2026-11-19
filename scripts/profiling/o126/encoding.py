"""O126 / 0094 §2: node ids `document | slot | index` over a repository's history (dev tool).

Granules, as 0094 §2 sets them:
- the innermost function or method (its own nodes: a nested definition is its own granule);
- a class header (the class's own nodes, its methods excluded);
- each module-level statement: an import keyed by the names it binds, any other by its digest and `#k`.

A granule's symbol is its qualified name (`#k` for a redefinition). Its slot comes from a hash of the symbol, modulo
2^SLOT_BITS. Clashes are settled in two ways:
- `hashed`: by symbol order (0094 §2), so an insert moves only granules that clash with it;
- `insertion`: in source order, as the `RERAIL_NODE_IDS=definition` flag probes today.

For each non-merge commit, over its changed .py files, the rows renumbered are:
- `preorder`: nodes at or after the first changed line (the `(document, preorder)` scheme);
- `hashed` and `insertion`: nodes of granules whose content changed, were added or removed, or whose slot moved.

Python's `ast` nodes stand for tree-sitter's: the counts are relative, not the engine's rows.
Also printed, over the files at HEAD: granules per file, the largest granule, and clashes per slot width.

usage: encoding.py <repo> <commits> [slot bits, default 14] [commits to skip, default 0]
ENCODING_NO_HEAD=1 leaves out the field widths over HEAD. ENCODING_MODULE_ONE=1 makes the module's own statements one
granule, as the flag does today.
"""
import ast, collections, hashlib, os, subprocess, sys

repo, n = sys.argv[1], int(sys.argv[2])
SLOT_BITS = int(sys.argv[3]) if len(sys.argv) > 3 else 14
SKIP = int(sys.argv[4]) if len(sys.argv) > 4 else 0
CAP = 40
DEF = (ast.FunctionDef, ast.AsyncFunctionDef)
cat = subprocess.Popen(["git", "-C", repo, "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)


def blob(spec):
    cat.stdin.write((spec + "\n").encode()); cat.stdin.flush()
    head = cat.stdout.readline().split()
    if len(head) < 3 or head[1] == b"missing":
        return ""
    data = cat.stdout.read(int(head[2])); cat.stdout.read(1)
    return data.decode(errors="replace")


def shape(node):
    return type(node).__name__ + repr([v for _, v in ast.iter_fields(node) if not isinstance(v, (ast.AST, list))])


def granules(src):
    """[(symbol, digest, own nodes, first line)] in source order; None if it does not parse."""
    try:
        tree = ast.parse(src)
    except Exception:
        return None
    out, seen = [], collections.Counter()

    def name(base):
        seen[base] += 1
        return base if seen[base] == 1 else f"{base}#{seen[base]}"

    def own(node, q):
        """The node's own content (nested functions and classes are their own granules)."""
        count, parts, stack = 1, [shape(node)], list(ast.iter_child_nodes(node))[::-1]
        while stack:
            ch = stack.pop()
            if isinstance(ch, DEF + (ast.ClassDef,)):
                parts.append("D:" + ch.name)
                unit(ch, f"{q}.{ch.name}")
                continue
            count += 1
            parts.append(shape(ch))
            stack.extend(list(ast.iter_child_nodes(ch))[::-1])
        return count, hashlib.sha1("\n".join(parts).encode()).hexdigest()

    def unit(node, q):
        sym = name(q)
        count, digest = own(node, sym)
        out.append((sym, digest, count, node.lineno))

    for st in tree.body:
        if isinstance(st, DEF + (ast.ClassDef,)):
            unit(st, st.name)
        else:
            if isinstance(st, (ast.Import, ast.ImportFrom)):
                key = "import:" + ",".join(a.asname or a.name for a in st.names)
            else:
                key = "stmt:" + hashlib.sha1(ast.dump(st, include_attributes=False).encode()).hexdigest()[:12]
            sym = name(key)
            count = sum(1 for _ in ast.walk(st))
            out.append((sym, hashlib.sha1(ast.dump(st, include_attributes=False).encode()).hexdigest(), count, st.lineno))
    if os.environ.get("ENCODING_MODULE_ONE"):
        # The flag's units today: the module's own statements are one granule, the root's.
        top = [g for g in out if g[0].startswith(("import:", "stmt:"))]
        rest = [g for g in out if not g[0].startswith(("import:", "stmt:"))]
        digest = hashlib.sha1("".join(g[1] for g in top).encode()).hexdigest()
        out = [("<module>", digest, sum(g[2] for g in top), 1)] + rest
    return out, sum(1 for _ in ast.walk(tree)), tree


def slot_hash(sym):
    return int.from_bytes(hashlib.blake2b(sym.encode(), digest_size=8).digest(), "little") % (1 << SLOT_BITS)


def assign(gs, order):
    """{symbol: slot}: each symbol at its hash, or the next free slot, in `order` ("hashed": by symbol; "insertion":
    in source order)."""
    syms = [g[0] for g in gs]
    if order == "hashed":
        syms = sorted(syms, key=lambda s: (slot_hash(s), s))
    taken, out = set(), {}
    for s in syms:
        k = slot_hash(s)
        while k in taken:
            k = (k + 1) % (1 << SLOT_BITS)
        taken.add(k); out[s] = k
    return out


def after_first_change(a, b, tree):
    al, bl = a.splitlines(), b.splitlines()
    i = 0
    while i < min(len(al), len(bl)) and al[i] == bl[i]:
        i += 1
    return sum(1 for nd in ast.walk(tree) if getattr(nd, "lineno", 0) >= i + 1)


def renumbered(ga, gb, order):
    da, db = {g[0]: g for g in ga}, {g[0]: g for g in gb}
    sa, sb = assign(ga, order), assign(gb, order)
    rows = moved = 0
    for s in set(da) | set(db):
        size = max(da.get(s, (0, 0, 0))[2], db.get(s, (0, 0, 0))[2])
        if da.get(s, (0, ""))[1] != db.get(s, (0, ""))[1]:
            rows += size
        elif sa.get(s) != sb.get(s):
            rows += size; moved += 1
    return rows, moved


log = subprocess.run(["git", "-C", repo, "log", "--no-merges", f"--skip={SKIP}", f"-{n}", "--format=%H"], capture_output=True).stdout.decode().split()
print(f"# slot bits {SLOT_BITS}")
print("commit\tpy_files\tpreorder\thashed\thashed_moved\tinsertion\tinsertion_moved\tfile_nodes", flush=True)
tot = collections.Counter()
for c in log:
    files = [f for f in subprocess.run(["git", "-C", repo, "diff-tree", "--no-commit-id", "--name-only", "-r", c],
                                       capture_output=True).stdout.decode().split() if f.endswith(".py")]
    if not files or len(files) > CAP:
        continue
    row = collections.Counter()
    for f in files:
        a, b = blob(f"{c}^:{f}"), blob(f"{c}:{f}")
        ua = granules(a) if a else ([], 0, None)
        ub = granules(b) if b else ([], 0, None)
        if ua is None or ub is None:
            continue
        (ga, ta, _), (gb, tb, tree) = ua, ub
        row["files"] += 1
        row["preorder"] += after_first_change(a, b, tree) if tree is not None else ta
        for order in ("hashed", "insertion"):
            r, m = renumbered(ga, gb, order)
            row[order] += r; row[order + "_moved"] += m
        row["file_nodes"] += max(ta, tb)
    if row["files"]:
        tot.update(row); tot["commits"] += 1
        print(f"{c[:10]}\t{row['files']}\t{row['preorder']}\t{row['hashed']}\t{row['hashed_moved']}\t{row['insertion']}\t{row['insertion_moved']}\t{row['file_nodes']}", flush=True)
print(f"# total over {tot['commits']} commits: preorder {tot['preorder']}, hashed {tot['hashed']} "
      f"({tot['hashed_moved']} granules moved by a clash), insertion {tot['insertion']} ({tot['insertion_moved']} moved), "
      f"changed files' nodes {tot['file_nodes']}")

# Field widths, over the files at HEAD.
if os.environ.get("ENCODING_NO_HEAD"):
    sys.exit(0)
head = subprocess.run(["git", "-C", repo, "ls-files", "*.py"], capture_output=True).stdout.decode().split()
per_file, largest, clashes = [], 0, collections.Counter()
for f in head:
    u = granules(blob(f"HEAD:{f}"))
    if not u:
        continue
    gs = u[0]
    per_file.append(len(gs))
    largest = max([largest] + [g[2] for g in gs])
    for bits in (10, 12, 14, 16):
        used = collections.Counter(int.from_bytes(hashlib.blake2b(g[0].encode(), digest_size=8).digest(), "little") % (1 << bits) for g in gs)
        clashes[bits] += sum(v - 1 for v in used.values() if v > 1)
per_file.sort()
q = lambda p: per_file[min(len(per_file) - 1, int(p * len(per_file)))]
print(f"# HEAD: {len(per_file)} files; granules per file p50 {q(.5)} p99 {q(.99)} max {per_file[-1]}; largest granule {largest} nodes")
print("# granules sharing a slot with an earlier one, by slot bits: " + ", ".join(f"{b}: {clashes[b]}" for b in sorted(clashes)))
