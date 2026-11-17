"""Layout-invariance corpus (0083 §0, step 0): move top-level definitions into new modules.

move.py SRC DST ORACLE.json N  -> DST is SRC with N definitions moved; DST/../move-map.tsv maps
every (new path, new line) of a touched file to (old path, old line).

A definition `f` of `pkg/a.py` moves to `pkg/_moved_<k>.py`, whose header is a copy of a.py's imports and
`from pkg.a import <names f reads that a.py defines earlier>`; a.py keeps `from pkg._moved_<k> import f` where f was.
Importers of `a.f` are unchanged (the re-export). Candidates are the definitions the oracle's calls target most.
"""
import ast, builtins, collections, json, os, shutil, sys

src, dst, oracle, n = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
shutil.rmtree(dst, ignore_errors=True)
shutil.copytree(src, dst)
hits = collections.Counter()
for corpus in json.load(open(oracle))["corpora"]:
    for records in corpus["records"].values():
        for r in records:
            for t in r[3]:
                if t[0] in ("def", "class"):
                    hits[(t[1], t[2])] += 1


def dotted(path):
    return os.path.splitext(path)[0].replace("/", ".")


def names_read(node):
    return {x.id for x in ast.walk(node) if isinstance(x, ast.Name)}


chosen = []  # (path, node)
for (path, line), _ in hits.most_common():
    if len(chosen) >= n or any(p == path for p, _, _ in chosen) or path.endswith("__init__.py") or not os.path.exists(os.path.join(src, path)):
        continue
    text = open(os.path.join(src, path)).read()
    tree = ast.parse(text)
    for i, node in enumerate(tree.body):
        if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)) or node.lineno != line:
            continue
        if any(isinstance(x, (ast.Global, ast.Nonlocal)) for x in ast.walk(node)):
            break
        earlier, later = set(), set()
        for j, other in enumerate(tree.body):
            bound = set()
            if isinstance(other, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                bound = {other.name}
            elif isinstance(other, (ast.Assign, ast.AnnAssign)):
                bound = {t.id for t in ast.walk(other) if isinstance(t, ast.Name) and isinstance(t.ctx, ast.Store)}
            (earlier if j < i else later if j > i else set()).update(bound)
        reads = names_read(node) - {node.name}
        if reads & later - earlier:
            break  # it reads a name a.py binds after it: the copy's import would run too early
        chosen.append((path, node, sorted(reads & earlier)))
        break

mapping = []
for k, (path, node, reads) in enumerate(chosen):
    full = os.path.join(src, path)
    lines = open(full).read().split("\n")
    tree = ast.parse("\n".join(lines))
    start = min([node.lineno] + [d.lineno for d in getattr(node, "decorator_list", [])])
    end = node.end_lineno
    module = os.path.splitext(os.path.basename(path))[0]
    imports, future = [], []
    for other in tree.body:
        if isinstance(other, (ast.Import, ast.ImportFrom)):
            block = lines[other.lineno - 1 : other.end_lineno]
            (future if isinstance(other, ast.ImportFrom) and other.module == "__future__" else imports).extend(block)
    header = future + imports + ([f"from {dotted(path)} import {', '.join(reads)}"] if reads else []) + [""]
    moved_name = f"_moved_{k}.py"
    moved_path = os.path.join(os.path.dirname(path), moved_name)
    body = lines[start - 1 : end]
    open(os.path.join(dst, moved_path), "w").write("\n".join(header + body) + "\n")
    for d, line in enumerate(range(start, end + 1)):
        mapping.append((moved_path, len(header) + 1 + d, path, line))
    stub = [f"from {dotted(moved_path)} import {node.name}"]
    new = lines[: start - 1] + stub + lines[end:]
    open(os.path.join(dst, path), "w").write("\n".join(new))
    for line in range(1, start):
        mapping.append((path, line, path, line))
    for line in range(end + 1, len(lines) + 1):
        mapping.append((path, line - (end - start + 1) + len(stub), path, line))
    print(f"moved {path}:{start}-{end} {node.name} -> {moved_path} (reads {reads})")

with open(os.path.join(os.path.dirname(dst.rstrip("/")), "move-map.tsv"), "w") as out:
    for a, b, c, d in mapping:
        out.write(f"{a}\t{b}\t{c}\t{d}\n")
