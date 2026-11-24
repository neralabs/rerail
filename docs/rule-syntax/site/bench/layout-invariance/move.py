"""Layout-invariance corpus (0083 §0, step 0; t22's move.py): move top-level definitions into new modules.

move.py SRC DST N  -> DST is SRC with N definitions moved; DST/../move-map.tsv maps every (new path, new line) of a
touched file to (old path, old line).

As t22's, but the candidates are the top-level definitions other files import by name most (`from m import f`), in
place of the Jedi recording's call targets: one a file, never an `__init__.py`.
"""
import ast, builtins, collections, json, os, shutil, sys

src, dst, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
shutil.rmtree(dst, ignore_errors=True)
shutil.copytree(src, dst)
files = {}
for base, _, names in os.walk(src):
    for name in names:
        if name.endswith(".py"):
            full = os.path.join(base, name)
            files[os.path.relpath(full, src)] = full
by_module = {os.path.splitext(p)[0].replace("/", "."): p for p in files}
hits = collections.Counter()
for p, full in files.items():
    try:
        tree = ast.parse(open(full).read())
    except SyntaxError:
        continue
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module in by_module and by_module[node.module] != p:
            target = by_module[node.module]
            defs = {d.name: d.lineno for d in ast.parse(open(files[target]).read()).body if isinstance(d, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))}
            for alias in node.names:
                if alias.name in defs:
                    hits[(target, defs[alias.name])] += 1

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
