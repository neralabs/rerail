"""LEG-083's call graph from the real data-layer repository, for the dfs_first spike.

    python3 -I realgraph.py REPO COMMIT OUT [leg|all]

Reads every .py file at COMMIT through `git cat-file` (the checkout is not touched), parses it with
ast, and rebuilds what scripts/lint_legacy_execution_surfaces.py builds for LEG-083
(FileCollector._enter_function, on_Call, _resolved_function_calls): per file, functions keyed by
symbol ("Class.method"), their eager .resolve()/.resolve_async() lines, and their calls resolved by
name to a unique function in the same file. Nothing from the repository is imported or run; the
script's constants are read with ast.literal_eval.

It then runs a copy of the script's own search, _transitive_eager_resolve (recursive, one memo per
file, roots in sorted(symbol) order), and writes the graph and the script's answers for the Rust
spike to compare (`native real OUT`).

Scope `leg` is LEG-083's: _is_internal_execution_path files only. `all` takes every .py file, for a
bigger graph of the same shape.
"""
import ast
import subprocess
import sys

sys.setrecursionlimit(100000)


def constants(src):
    """The frozensets the script defines, read without running it."""
    want = {"LEGACY_WRAPPER_DECORATORS", "DIRECT_LEGACY_SURFACE_DECORATORS", "ADMIN_EAGER_TERMINAL_ALLOWLIST"}
    out = {}
    for node in ast.parse(src).body:
        target, value = None, None
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            target, value = node.targets[0].id, node.value
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            target, value = node.target.id, node.value
        if target in want and isinstance(value, ast.Call):
            out[target] = frozenset(ast.literal_eval(value.args[0]))
    return out


def is_test_path(p):
    return "tests" in p.split("/") or p.startswith("tests/")


def is_internal_execution_path(p):
    if is_test_path(p) or p.startswith("scripts/"):
        return False
    if p in {"l2050_transaction/legacy_wrappers.py", "l2050_transaction/execution_boundary.py"}:
        return False
    root = p.split("/", 1)[0]
    return root in ("impl", "data_layer") or (len(root) >= 2 and root[0] == "l" and root[1].isdigit())


def decorator_name(node):
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return node.attr
    if isinstance(node, ast.Call):
        return decorator_name(node.func)
    return None


class Facts:
    def __init__(self, symbol, legacy):
        self.symbol, self.legacy = symbol, legacy
        self.eager, self.calls = [], []


def collect(tree, legacy_decorators):
    """FileCollector's function facts: pre-order, ast.iter_child_nodes order, with exit hooks."""
    functions, fstack, cstack = {}, [], []

    def visit(node):
        if isinstance(node, ast.ClassDef):
            cstack.append(node.name)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            legacy = any(decorator_name(d) in legacy_decorators for d in node.decorator_list)
            symbol = ".".join([*cstack, node.name]) if cstack else node.name
            functions[symbol] = Facts(symbol, legacy)
            fstack.append(symbol)
        elif isinstance(node, ast.Call) and fstack:
            func = node.func
            f = functions[fstack[-1]]
            if isinstance(func, ast.Attribute):
                if func.attr in {"resolve", "resolve_async"}:
                    f.eager.append((node.lineno, func.attr))
                else:
                    f.calls.append((node.lineno, func.attr))
                    if cstack and isinstance(func.value, ast.Name) and func.value.id in {"self", "cls"}:
                        f.calls.append((node.lineno, ".".join([*cstack, func.attr])))
            elif isinstance(func, ast.Name):
                f.calls.append((node.lineno, func.id))
        for child in ast.iter_child_nodes(node):
            visit(child)
        if isinstance(node, ast.ClassDef):
            cstack.pop()
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            fstack.pop()

    visit(tree)
    return functions


def resolved_calls(functions):
    by_name = {}
    for symbol, f in functions.items():
        by_name.setdefault(symbol.rsplit(".", 1)[-1], set()).add(symbol)
        by_name.setdefault(symbol, set()).add(symbol)
    out = {}
    for symbol, f in functions.items():
        calls = []
        for line, cand in f.calls:
            t = sorted(by_name.get(cand, set()))
            if len(t) == 1 and t[0] != symbol:
                calls.append((line, t[0]))
        out[symbol] = calls
    return out


def transitive_eager_resolve(functions, calls, symbol, memo, visiting, steps):
    """The script's _transitive_eager_resolve, unchanged but for the step counter."""
    steps[0] += 1
    if symbol in memo:
        return memo[symbol]
    if symbol in visiting:
        return None
    facts = functions[symbol]
    if facts.legacy:
        memo[symbol] = None
        return None
    visiting.add(symbol)
    if facts.eager:
        line, method = facts.eager[0]
        visiting.remove(symbol)
        memo[symbol] = (line, symbol, method)
        return memo[symbol]
    for call_line, callee in calls.get(symbol, []):
        child = functions[callee]
        if child.legacy:
            continue
        m = transitive_eager_resolve(functions, calls, callee, memo, visiting, steps)
        if m is not None:
            visiting.remove(symbol)
            memo[symbol] = (call_line, m[1], m[2])
            return memo[symbol]
    visiting.remove(symbol)
    memo[symbol] = None
    return None


def main():
    repo, commit, out_path = sys.argv[1:4]
    scope = sys.argv[4] if len(sys.argv) > 4 else "leg"
    git = ["git", "-C", repo]
    script = subprocess.run(git + ["show", f"{commit}:scripts/lint_legacy_execution_surfaces.py"],
                            capture_output=True, text=True, check=True).stdout
    c = constants(script)
    legacy_decorators = c["LEGACY_WRAPPER_DECORATORS"] | c["DIRECT_LEGACY_SURFACE_DECORATORS"]
    allow = c["ADMIN_EAGER_TERMINAL_ALLOWLIST"]
    paths = [p for p in subprocess.run(git + ["ls-tree", "-r", "--name-only", commit], capture_output=True,
                                       text=True, check=True).stdout.splitlines() if p.endswith(".py")]
    if scope == "leg":
        paths = [p for p in paths if is_internal_execution_path(p)]
    batch = subprocess.Popen(git + ["cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    out = open(out_path, "w")
    ids, nfiles, nfun, nedge, nsink, nfind, steps, bad = {}, 0, 0, 0, 0, 0, [0], 0
    for p in paths:
        batch.stdin.write(f"{commit}:{p}\n".encode())
        batch.stdin.flush()
        header = batch.stdout.readline().split()
        size = int(header[2])
        data = batch.stdout.read(size)
        batch.stdout.read(1)
        try:
            tree = ast.parse(data, filename=p)
        except (SyntaxError, ValueError):
            bad += 1
            continue
        functions = collect(tree, legacy_decorators)
        if not functions:
            continue
        nfiles += 1
        out.write(f"F {p}\n")
        for symbol, f in functions.items():
            ids[(p, symbol)] = len(ids)
            sink = f.eager[0][0] if f.eager else -1
            out.write(f"N {ids[(p, symbol)]} {int(f.legacy)} {sink} {symbol}\n")
            nfun += 1
            nsink += sink >= 0
        calls = resolved_calls(functions)
        for symbol, cs in calls.items():
            for k, (line, callee) in enumerate(cs):
                out.write(f"E {ids[(p, symbol)]} {ids[(p, callee)]} {k} {line}\n")
                nedge += 1
        memo = {}
        for symbol, f in sorted(functions.items()):
            if f.legacy or f"{p}:{symbol}" in allow:
                continue
            m = transitive_eager_resolve(functions, calls, symbol, memo, set(), steps)
            r = ids[(p, symbol)]
            if m is None:
                out.write(f"R {r} -\n")
            else:
                out.write(f"R {r} {m[0]} {ids[(p, m[1])]}\n")
                nfind += 1
    out.close()
    batch.stdin.close()
    print(f"{scope}: {len(paths)} files read, {bad} unparsable, {nfiles} with functions; {nfun} functions, "
          f"{nedge} resolved calls, {nsink} with an eager resolve; script search: {nfind} LEG-083 findings, "
          f"{steps[0]} steps")


if __name__ == "__main__":
    main()
