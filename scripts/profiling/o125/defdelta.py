"""O126: the base delta per definition over a repository's history (dev tool).

For each non-merge commit, over its changed .py files:
- defs_changed: innermost definitions (module body, class, function) whose own content changed, added or removed.
  A definition's own content excludes its nested definitions, which are their own units.
- rows_definition: AST nodes in those definitions: the base rows a (definition, index) id scheme rewrites.
- rows_preorder: AST nodes at or after the first changed line: the rows a (document, preorder) scheme renumbers
  (an upper bound when the edit keeps the node count; exact when it inserts or deletes nodes).
- rows_file: all AST nodes of the changed files: what a per-file memo re-derives.
Commits with more than CAP changed .py files are printed with their file count only.
"""
import ast, hashlib, subprocess, sys

repo, n, CAP = sys.argv[1], int(sys.argv[2]), 40
# Optional: a file of paths (only those count) and a file of commits (in place of the last n).
only = set(open(sys.argv[3]).read().split()) if len(sys.argv) > 3 else None
given = open(sys.argv[4]).read().split() if len(sys.argv) > 4 else None
DEF = (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)
cat = subprocess.Popen(["git", "-C", repo, "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)


def blob(spec):
    cat.stdin.write((spec + "\n").encode()); cat.stdin.flush()
    head = cat.stdout.readline().split()
    if len(head) < 3 or head[1] == b"missing":
        return ""
    data = cat.stdout.read(int(head[2])); cat.stdout.read(1)
    return data.decode(errors="replace")


def units(src):
    """{unit name: (digest of own content, own node count)}, total nodes, tree; None if it does not parse."""
    try:
        tree = ast.parse(src)
    except Exception:
        return None
    out, seen = {}, {}

    def visit(node, q):
        own, parts = 1, [type(node).__name__ + getattr(node, "name", "")]
        stack = list(ast.iter_child_nodes(node))[::-1]
        while stack:
            ch = stack.pop()
            if isinstance(ch, DEF):
                nm = f"{q}.{ch.name}" if q else ch.name
                seen[nm] = seen.get(nm, 0) + 1
                if seen[nm] > 1:
                    nm = f"{nm}#{seen[nm]}"
                parts.append("D:" + nm)
                visit(ch, nm)
            else:
                own += 1
                parts.append(type(ch).__name__ + repr([v for _, v in ast.iter_fields(ch) if not isinstance(v, (ast.AST, list))]))
                stack.extend(list(ast.iter_child_nodes(ch))[::-1])
        out[q or "<module>"] = (hashlib.sha1("\n".join(parts).encode()).hexdigest(), own)

    visit(tree, "")
    return out, sum(1 for _ in ast.walk(tree)), tree


def after_first_change(a, b, tree):
    al, bl = a.splitlines(), b.splitlines()
    i = 0
    while i < min(len(al), len(bl)) and al[i] == bl[i]:
        i += 1
    return sum(1 for nd in ast.walk(tree) if getattr(nd, "lineno", 0) >= i + 1)


log = given or subprocess.run(["git", "-C", repo, "log", "--no-merges", f"-{n}", "--format=%H"], capture_output=True).stdout.decode().split()
print("commit\tpy_files\tdefs_changed\tdefs_in_files\trows_definition\trows_preorder\trows_file", flush=True)
for c in log:
    files = [f for f in subprocess.run(["git", "-C", repo, "diff-tree", "--no-commit-id", "--name-only", "-r", c],
                                       capture_output=True).stdout.decode().split() if f.endswith(".py") and (only is None or f in only)]
    if not files:
        continue
    if len(files) > CAP:
        print(f"{c[:10]}\t{len(files)}\t-\t-\t-\t-\t-", flush=True)
        continue
    dc = dt = rd = rp = rf = ok = 0
    for f in files:
        a, b = blob(f"{c}^:{f}"), blob(f"{c}:{f}")
        ua = units(a) if a else ({}, 0, None)
        ub = units(b) if b else ({}, 0, None)
        if ua is None or ub is None:
            continue
        (da, ta, _), (db, tb, tree) = ua, ub
        ok += 1
        changed = [k for k in set(da) | set(db) if da.get(k, ("",))[0] != db.get(k, ("",))[0]]
        dc += len(changed)
        dt += len(set(da) | set(db))
        rd += sum(max(da.get(k, ("", 0))[1], db.get(k, ("", 0))[1]) for k in changed)
        rp += after_first_change(a, b, tree) if tree is not None else ta
        rf += max(ta, tb)
    if ok:
        print(f"{c[:10]}\t{ok}\t{dc}\t{dt}\t{rd}\t{rp}\t{rf}", flush=True)
