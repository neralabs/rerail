"""Which of LEG-083's findings each of the linter's two entry points sees.

    python3 -I plugin_scope.py SCRIPT GRAPH REPO COMMIT

`lint_legacy_execution_surfaces.py --check` (scan_repo) scans the whole repository. The lint-all
plugin (build_plugin's FileScopeSpec) scans only a fixed list of directories, read here from the
script's constants with ast.literal_eval. SCRIPT is the linter's source at COMMIT; GRAPH is
realgraph.py's output. Prints the findings of the script (R) and of both fixes (T) per entry point.
"""
import ast, sys, json, os, hashlib, subprocess
src, graph, repo, commit = sys.argv[1:]
tree = ast.parse(open(src).read()); vals = {}
for n in tree.body:
    t = n.targets[0].id if isinstance(n, ast.Assign) and isinstance(n.targets[0], ast.Name) else (n.target.id if isinstance(n, ast.AnnAssign) and isinstance(n.target, ast.Name) else None)
    if t in {"RAW_TXN_SCOPE_ALLOWLISTS", "RAW_CURSOR_SCOPE_ALLOWLISTS"}:
        vals[t] = [ast.literal_eval(k) for k in n.value.keys]
    elif t == "SQL_POST_FETCH_FILTER_SCOPES":
        v = n.value; vals[t] = ast.literal_eval(v.args[0] if isinstance(v, ast.Call) else v)
    elif t == "GRAPH_FALLBACK_SCOPE":
        vals[t] = ast.literal_eval(n.value)
dirs = set(vals["RAW_TXN_SCOPE_ALLOWLISTS"]) | set(vals["RAW_CURSOR_SCOPE_ALLOWLISTS"]) | set(vals["SQL_POST_FETCH_FILTER_SCOPES"]) \
    | {vals["GRAPH_FALLBACK_SCOPE"], "l2050_transaction", "l7200_versioned_transactions", "l9900_auto_api"}
print("plugin directories:", sorted(dirs))
inside = lambda p: any(p == d or p.startswith(d.rstrip("/") + "/") for d in dirs)
cur = None; fil = {}; found = []
for line in open(graph):
    w = line.split()
    if w[0] == "F": cur = w[1]
    elif w[0] == "N": fil[w[1]] = cur
    elif w[0] in "RT" and w[2] != "-": found.append((w[0], fil[w[1]]))
for tag, name in (("R", "script"), ("T", "both fixes")):
    allf = [p for t, p in found if t == tag]
    ins = [p for p in allf if inside(p)]
    print(f"{name}: {len(allf)} LEG-083 findings over the whole repo (--check); {len(ins)} inside the plugin's directories (lint-all)", sorted(set(ins)))
paths = [p for p in subprocess.run(["git","-C",repo,"ls-tree","-r","--name-only",commit],capture_output=True,text=True).stdout.splitlines() if p.endswith(".py")]
b = subprocess.Popen(["git","-C",repo,"cat-file","--batch"],stdin=subprocess.PIPE,stdout=subprocess.PIPE); byhash = {}
for p in paths:
    b.stdin.write(f"{commit}:{p}\n".encode()); b.stdin.flush(); h=b.stdout.readline().split(); d=b.stdout.read(int(h[2])); b.stdout.read(1)
    byhash[hashlib.sha256(d).hexdigest()] = p
store = os.path.join(repo, "tmp/lint_fact_store/by_source"); cached = set()
if os.path.isdir(store):
    for f in os.listdir(store):
        d = json.load(open(os.path.join(store, f)))
        if any(k.startswith("legacy_execution") for k in d["facts"]) and d.get("content_hash") in byhash: cached.add(byhash[d["content_hash"]])
tests = lambda p: "tests" in p.split("/")
print(f"cached files with LEG facts: {len(cached)}, inside the plugin's directories: {sum(map(inside, cached))}; "
      f"non-test .py files there at the commit: {sum(1 for p in paths if inside(p) and not tests(p))}")
