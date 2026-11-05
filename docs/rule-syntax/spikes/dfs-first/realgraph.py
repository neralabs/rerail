"""LEG-083's call graph from the real data-layer repository, for the dfs_first spike.

    python3 -I realgraph.py REPO COMMIT OUT [leg|all]

Reads every .py file at COMMIT through `git cat-file` (the checkout is not touched), parses it with
ast, and rebuilds what scripts/lint_legacy_execution_surfaces.py builds for LEG-083
(FileCollector._enter_function, on_Call, _resolved_function_calls): per file, functions keyed by
symbol ("Class.method"), their eager .resolve()/.resolve_async() lines, and their calls resolved by
name to a unique function in the same file. Nothing from the repository is imported or run; the
script's constants are read with ast.literal_eval.

It then runs a copy of the script's own search, _transitive_eager_resolve (recursive, one memo per
file, roots in sorted(symbol) order), and three repaired versions of it (VARIANTS), and writes the
graph and every variant's answers for the Rust spike to compare (`native real OUT`).

The script counts any `.resolve()`/`.resolve_async()` as an eager lazy terminal. Each call is also
classified (eager_kind): with arguments, on a pathlib path (PathScope), or of the lazy terminals'
shape. The corrected test keeps the last kind only.

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
        self.eager, self.calls, self.kinds = [], [], []


PATH_CLASSES = {"Path", "PurePath", "PosixPath", "WindowsPath", "PurePosixPath", "PureWindowsPath"}
# Methods and attributes of a path that give a path again.
PATH_METHODS = {"absolute", "expanduser", "joinpath", "relative_to", "resolve", "with_name", "with_stem",
                "with_suffix", "readlink", "cwd", "home"}
PATH_ATTRS = {"parent", "parents"}


def path_names(tree):
    """The names `from pathlib import ...` binds to a path class, and the names bound to the module."""
    classes, modules = set(), set()
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module == "pathlib":
            for a in node.names:
                if a.name in PATH_CLASSES:
                    classes.add(a.asname or a.name)
        elif isinstance(node, ast.Import):
            for a in node.names:
                if a.name == "pathlib":
                    modules.add(a.asname or "pathlib")
    return classes, modules


class PathScope:
    """Which names hold a pathlib path in one scope, by approximate, flow-insensitive inference.

    A name is a path if some binding of it in the scope is: a parameter whose annotation mentions a path
    class; an annotated assignment whose annotation does; or an assignment from a path expression.
    A path expression is a path class called, a path's `.parent`/`.parents[i]`, a path method called on a
    path, `path / x`, `a if c else b` with a path on either side, or a name that is a path here or in an enclosing scope (LEGB, as python/names.rr's
    ref_site: may-precise, no flow pruning).
    """

    def __init__(self, node, outer, classes, modules):
        self.outer, self.classes, self.modules, self.names = outer, classes, modules, set()
        body = []
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            a = node.args
            for arg in [*a.posonlyargs, *a.args, *a.kwonlyargs, a.vararg, a.kwarg]:
                if arg is not None and arg.annotation is not None and self.mentions_path(arg.annotation):
                    self.names.add(arg.arg)
            body = node.body
        else:
            body = node.body
        assigns = []
        for st in own_nodes(body):
            if isinstance(st, ast.Assign):
                for t in st.targets:
                    if isinstance(t, ast.Name):
                        assigns.append((t.id, st.value, None))
            elif isinstance(st, ast.AnnAssign) and isinstance(st.target, ast.Name):
                assigns.append((st.target.id, st.value, st.annotation))
            elif isinstance(st, ast.NamedExpr):
                assigns.append((st.target.id, st.value, None))
        changed = True
        while changed:  # a = Path(x); b = a / "c": iterate to a fixpoint
            changed = False
            for name, value, ann in assigns:
                if name in self.names:
                    continue
                if (ann is not None and self.mentions_path(ann)) or (value is not None and self.is_path(value)):
                    self.names.add(name)
                    changed = True

    def is_class(self, e):
        if isinstance(e, ast.Name):
            return e.id in self.classes
        return isinstance(e, ast.Attribute) and e.attr in PATH_CLASSES and isinstance(e.value, ast.Name) \
            and e.value.id in self.modules

    def mentions_path(self, ann):
        if isinstance(ann, ast.Constant) and isinstance(ann.value, str):
            try:
                ann = ast.parse(ann.value, mode="eval").body
            except SyntaxError:
                return False
        return any(self.is_class(n) for n in ast.walk(ann))

    def has(self, name):
        s = self
        while s is not None:
            if name in s.names:
                return True
            s = s.outer
        return False

    def is_path(self, e):
        if isinstance(e, ast.Name):
            return self.has(e.id)
        if isinstance(e, ast.Call):
            f = e.func
            if self.is_class(f):
                return True
            return isinstance(f, ast.Attribute) and f.attr in PATH_METHODS and (self.is_path(f.value) or self.is_class(f.value))
        if isinstance(e, ast.Attribute):
            return e.attr in PATH_ATTRS and self.is_path(e.value)
        if isinstance(e, ast.Subscript):
            return isinstance(e.value, ast.Attribute) and e.value.attr == "parents" and self.is_path(e.value.value)
        if isinstance(e, ast.BinOp) and isinstance(e.op, ast.Div):
            return self.is_path(e.left)
        if isinstance(e, ast.IfExp):
            return self.is_path(e.body) or self.is_path(e.orelse)
        return False


def own_nodes(body):
    """Every node under these statements that belongs to this scope: not inside a nested def, class or lambda."""
    stack = list(reversed(body))
    while stack:
        n = stack.pop()
        yield n
        for c in reversed(list(ast.iter_child_nodes(n))):
            if not isinstance(c, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda)):
                stack.append(c)


def eager_kind(call, scope):
    """LEG-083's eager test is the method name alone. The lazy terminals are LazyResult/LazySeq
    `resolve(self)` and `resolve_async(self)`: no arguments, and the receiver is not a pathlib path."""
    if call.args or call.keywords:
        return "args"
    if scope is not None and scope.is_path(call.func.value):
        return "path"
    return "lazy"


def collect(tree, legacy_decorators):
    """FileCollector's function facts: pre-order, ast.iter_child_nodes order, with exit hooks."""
    functions, fstack, cstack = {}, [], []
    classes, modules = path_names(tree)
    scopes = [PathScope(tree, None, classes, modules)]

    def visit(node):
        if isinstance(node, ast.ClassDef):
            cstack.append(node.name)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            legacy = any(decorator_name(d) in legacy_decorators for d in node.decorator_list)
            symbol = ".".join([*cstack, node.name]) if cstack else node.name
            functions[symbol] = Facts(symbol, legacy)
            fstack.append(symbol)
            scopes.append(PathScope(node, scopes[-1], classes, modules))
        elif isinstance(node, ast.Call) and fstack:
            func = node.func
            f = functions[fstack[-1]]
            if isinstance(func, ast.Attribute):
                if func.attr in {"resolve", "resolve_async"}:
                    f.eager.append((node.lineno, func.attr))
                    f.kinds.append((node.lineno, eager_kind(node, scopes[-1])))
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
            scopes.pop()

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


def transitive_eager_resolve(functions, calls, symbol, memo, visiting, steps, eager="eager"):
    """The script's _transitive_eager_resolve, unchanged but for the step counter, and `eager`, which
    names the Facts field holding the eager calls (the script's, or the corrected test's)."""
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
    if getattr(facts, eager):
        line, method = getattr(facts, eager)[0]
        visiting.remove(symbol)
        memo[symbol] = (line, symbol, method)
        return memo[symbol]
    for call_line, callee in calls.get(symbol, []):
        child = functions[callee]
        if child.legacy:
            continue
        m = transitive_eager_resolve(functions, calls, callee, memo, visiting, steps, eager)
        if m is not None:
            visiting.remove(symbol)
            memo[symbol] = (call_line, m[1], m[2])
            return memo[symbol]
    visiting.remove(symbol)
    memo[symbol] = None
    return None


# The four searches the spike checks, as (line tag, memo shared across roots, Facts field of eager calls):
#   R  the script as it is;
#   S  the script with a fresh memo per root (the memo fix: dfs_first);
#   U  the script's memo, with the corrected eager test;
#   T  both fixes: dfs_first over lazy terminals only (the recommended LEG-083).
VARIANTS = (("R", True, "eager"), ("S", False, "eager"), ("U", True, "lazy"), ("T", False, "lazy"))


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
    ids, nfiles, nfun, nedge, bad = {}, 0, 0, 0, 0
    kinds = {"lazy": 0, "path": 0, "args": 0}
    found = {v[0]: 0 for v in VARIANTS}
    steps = {v[0]: [0] for v in VARIANTS}
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
            f.lazy = [e for e, (_, k) in zip(f.eager, f.kinds) if k == "lazy"]
            for _, k in f.kinds:
                kinds[k] += 1
            sink = f.eager[0][0] if f.eager else -1
            out.write(f"N {ids[(p, symbol)]} {int(f.legacy)} {sink} {symbol}\n")
            if f.eager:
                # K: the corrected test's first eager line (-1: none), and each call's kind.
                out.write(f"K {ids[(p, symbol)]} {f.lazy[0][0] if f.lazy else -1} "
                          + " ".join(f"{l}:{k}" for l, k in f.kinds) + "\n")
            nfun += 1
        calls = resolved_calls(functions)
        for symbol, cs in calls.items():
            for k, (line, callee) in enumerate(cs):
                out.write(f"E {ids[(p, symbol)]} {ids[(p, callee)]} {k} {line}\n")
                nedge += 1
        for tag, share, eager in VARIANTS:
            memo = {}
            for symbol, f in sorted(functions.items()):
                if f.legacy or f"{p}:{symbol}" in allow:
                    continue
                m = transitive_eager_resolve(functions, calls, symbol, memo if share else {}, set(), steps[tag], eager)
                r = ids[(p, symbol)]
                if m is None:
                    out.write(f"{tag} {r} -\n")
                else:
                    out.write(f"{tag} {r} {m[0]} {ids[(p, m[1])]}\n")
                    found[tag] += 1
    out.close()
    batch.stdin.close()
    print(f"{scope}: {len(paths)} files read, {bad} unparsable, {nfiles} with functions; {nfun} functions, "
          f"{nedge} resolved calls; .resolve()/.resolve_async() calls: {kinds['lazy']} lazy-terminal shape, "
          f"{kinds['path']} on a pathlib path, {kinds['args']} with arguments")
    names = {"R": "script", "S": "fresh memo", "U": "corrected eager test", "T": "both fixes"}
    for tag, _, _ in VARIANTS:
        print(f"  {tag} {names[tag]:22} {found[tag]:6} LEG-083 findings, {steps[tag][0]} search steps")


if __name__ == "__main__":
    main()
