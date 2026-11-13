"""Static approximation of `pytest --collect-only` over conform/**/test_*.py.

Usage: python3 -I collect_est.py ROOT [--level N] [--json OUT]
Levels (each adds to the previous):
  0  the rule's current lower bound: top-level test* defs + test* methods of top-level Test* classes, names deduped
  1  pytest's name patterns (test_*, Test*), nested classes, inherited methods (same file and cross-file),
     __init__ / __test__ = False, imported test names
  2  @pytest.mark.parametrize with literal argvalues (list/tuple/set/dict literal, len), stacked = product,
     class-level and module pytestmark marks
  3  argvalues resolved through names: module constants, imports across files, simple expressions
     (concatenation, starred, comprehension over a known-length iterable, range, list()/tuple()/sorted())
  4  fixtures with params= (direct and transitive through fixture arguments, autouse, usefixtures),
     overridden by direct parametrization
"""
import ast
import json
import os
import sys
from collections import Counter, defaultdict

ROOT = sys.argv[1]
LEVEL = 6
OUT = None
args = sys.argv[2:]
while args:
    a = args.pop(0)
    if a == "--level":
        LEVEL = int(args.pop(0))
    elif a == "--json":
        OUT = args.pop(0)

stats = Counter()
UNRES = []
TEST_OFF = {}
UNK = None  # unknown length


# ---------------------------------------------------------------- modules ------------------------------------------
class Mod:
    def __init__(self, path, rel, tree):
        self.path, self.rel, self.tree = path, rel, tree
        self.defs = {}  # top-level name -> node (FunctionDef / ClassDef / value expr) or ("import", mod, name)
        for st in tree.body:
            self._bind(st)

    def _bind(self, st):
        if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            self.defs[st.name] = st
        elif isinstance(st, ast.Assign):
            for t in st.targets:
                if isinstance(t, ast.Name):
                    self.defs[t.id] = ("value", st.value)
        elif isinstance(st, ast.AnnAssign) and isinstance(st.target, ast.Name) and st.value is not None:
            self.defs[st.target.id] = ("value", st.value)
        elif isinstance(st, ast.ImportFrom):
            base = resolve_from(self.rel, st.module, st.level)
            for al in st.names:
                if al.name != "*":
                    self.defs[al.asname or al.name] = ("import", base, al.name)
        elif isinstance(st, ast.Import):
            for al in st.names:
                if al.asname:
                    self.defs[al.asname] = ("module", al.name)
                else:
                    self.defs[al.name.split(".")[0]] = ("module", al.name.split(".")[0])
        elif isinstance(st, ast.If):
            # `if TYPE_CHECKING:` and version checks: take both branches, later wins
            for s in st.body + st.orelse:
                self._bind(s)
        elif isinstance(st, ast.Try):
            for s in st.body:
                self._bind(s)


def resolve_from(rel, module, level):
    if level == 0:
        return module or ""
    parts = rel[:-3].split("/")
    pkg = parts[:-1]
    if level > 1:
        pkg = pkg[: len(pkg) - (level - 1)]
    return ".".join(pkg + ([module] if module else []))


MODS = {}


def load_module(dotted):
    if dotted in MODS:
        return MODS[dotted]
    MODS[dotted] = None
    base = os.path.join(ROOT, *dotted.split("."))
    for cand in (base + ".py", os.path.join(base, "__init__.py")):
        if os.path.isfile(cand):
            m = parse(cand)
            MODS[dotted] = m
            return m
    return None


def parse(path):
    rel = os.path.relpath(path, ROOT)
    try:
        src = open(path, encoding="utf-8").read()
        tree = ast.parse(src)
    except (SyntaxError, UnicodeDecodeError):
        stats["unparsable"] += 1
        return None
    return Mod(path, rel, tree)


def lookup(mod, name, depth=0):
    """Resolve a top-level name of mod to (mod, node-or-value)."""
    if mod is None or depth > 12:
        return None
    d = mod.defs.get(name)
    if d is None:
        return None
    if isinstance(d, tuple):
        if d[0] == "import":
            tgt = load_module(d[1])
            if tgt is not None and d[2] in tgt.defs:
                return lookup(tgt, d[2], depth + 1)
            sub = load_module(d[1] + "." + d[2])
            if sub is not None:
                return (sub, ("modobj", sub))
            return None
        if d[0] == "module":
            m = load_module(d[1])
            return (m, ("modobj", m)) if m else None
        return (mod, d)
    return (mod, d)


def lookup_expr(mod, e, depth=0):
    """Resolve a Name or dotted Attribute to (mod, def)."""
    if isinstance(e, ast.Name):
        return lookup(mod, e.id, depth)
    if isinstance(e, ast.Attribute):
        r = lookup_expr(mod, e.value, depth)
        if r and isinstance(r[1], tuple) and r[1][0] == "modobj":
            m = r[1][1]
            got = lookup(m, e.attr, depth + 1)
            if got is None:
                sub = load_module(m.rel[:-3].replace("/", ".").removesuffix(".__init__") + "." + e.attr)
                if sub:
                    return (sub, ("modobj", sub))
            return got
        if r and isinstance(r[1], ast.ClassDef):  # Class.CONST
            for st in r[1].body:
                if isinstance(st, ast.Assign) and any(isinstance(t, ast.Name) and t.id == e.attr for t in st.targets):
                    return (r[0], ("value", st.value))
    return None


# ------------------------------------------------------------- lengths ---------------------------------------------
def length(mod, e, depth=0):
    """Number of elements of argvalues expression e, or None."""
    if depth > 20:
        return UNK
    if isinstance(e, (ast.List, ast.Tuple, ast.Set)):
        n = 0
        for x in e.elts:
            if isinstance(x, ast.Starred):
                if LEVEL < 3:
                    return UNK
                k = length(mod, x.value, depth + 1)
                if k is UNK:
                    return UNK
                n += k
            else:
                n += 1
        return n
    if isinstance(e, ast.Dict):
        if any(k is None for k in e.keys):
            return UNK
        return len(e.keys)
    if LEVEL < 3:
        return UNK
    if isinstance(e, (ast.Name, ast.Attribute)):
        r = lookup_expr(mod, e)
        if r and isinstance(r[1], tuple) and r[1][0] == "value":
            return length(r[0], r[1][1], depth + 1)
        if r and isinstance(r[1], ast.ClassDef):  # an Enum: count its members
            n = sum(1 for st in r[1].body if isinstance(st, ast.Assign))
            return n or UNK
        return UNK
    if isinstance(e, ast.BinOp) and isinstance(e.op, ast.Add):
        a, b = length(mod, e.left, depth + 1), length(mod, e.right, depth + 1)
        return UNK if a is UNK or b is UNK else a + b
    if isinstance(e, ast.BinOp) and isinstance(e.op, ast.Mult):
        if isinstance(e.right, ast.Constant) and isinstance(e.right.value, int):
            a = length(mod, e.left, depth + 1)
            return UNK if a is UNK else a * e.right.value
        return UNK
    if isinstance(e, (ast.ListComp, ast.SetComp, ast.GeneratorExp, ast.DictComp)):
        n = 1
        for g in e.generators:
            if g.ifs:
                if LEVEL < 5:
                    return UNK
                stats["filtered_comprehension_upper"] += 1
            k = length(mod, g.iter, depth + 1)
            if k is UNK:
                return UNK
            n *= k
        return n
    if isinstance(e, ast.IfExp) and LEVEL >= 5:
        a, b = length(mod, e.body, depth + 1), length(mod, e.orelse, depth + 1)
        stats["ifexp_max"] += 1
        return UNK if a is UNK and b is UNK else max(x for x in (a, b) if x is not UNK)
    if isinstance(e, ast.Call):
        f = e.func
        fname = f.id if isinstance(f, ast.Name) else (f.attr if isinstance(f, ast.Attribute) else None)
        if fname in ("list", "tuple", "sorted", "set", "frozenset", "reversed", "enumerate") and len(e.args) == 1:
            return length(mod, e.args[0], depth + 1)
        if fname == "range" and all(isinstance(a, ast.Constant) for a in e.args):
            return len(range(*[a.value for a in e.args]))
        if fname == "zip" and e.args:
            ks = [length(mod, a, depth + 1) for a in e.args]
            return UNK if UNK in ks else min(ks)
        if fname == "product" and e.args:
            ks = [length(mod, a, depth + 1) for a in e.args]
            if UNK in ks:
                return UNK
            n = 1
            for k in ks:
                n *= k
            return n
        if fname in ("keys", "values", "items") and isinstance(f, ast.Attribute) and not e.args:
            return length(mod, f.value, depth + 1)
        if isinstance(f, ast.Name):  # a local helper that returns a literal: def cases(): return [...]
            r = lookup(mod, f.id)
            if r and isinstance(r[1], (ast.FunctionDef,)):
                rets = [s for s in ast.walk(r[1]) if isinstance(s, ast.Return) and s.value is not None]
                if len(rets) == 1:
                    return length(r[0], rets[0].value, depth + 1)
        return UNK
    return UNK


# ------------------------------------------------------------- decorators ------------------------------------------
def dotted(e):
    if isinstance(e, ast.Name):
        return e.id
    if isinstance(e, ast.Attribute):
        b = dotted(e.value)
        return b + "." + e.attr if b else None
    return None


def marks_of(decos):
    """(kind, call) for pytest.mark.<kind>(...) decorators, bare or called."""
    out = []
    for d in decos:
        call = d if isinstance(d, ast.Call) else None
        name = dotted(d.func if call else d)
        if not name:
            continue
        if ".mark." in "." + name or name.startswith("mark."):
            out.append((name.rsplit(".", 1)[-1], call))
    return out


def module_marks(mod):
    out = []
    for st in mod.tree.body:
        if isinstance(st, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "pytestmark" for t in st.targets):
            v = st.value
            items = v.elts if isinstance(v, (ast.List, ast.Tuple)) else [v]
            out += marks_of(items)
    return out


def class_marks(cls):
    out = marks_of(cls.decorator_list)
    for st in cls.body:
        if isinstance(st, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "pytestmark" for t in st.targets):
            v = st.value
            out += marks_of(v.elts if isinstance(v, (ast.List, ast.Tuple)) else [v])
    return out


def argnames(call):
    a = call.args[0] if call.args else next((k.value for k in call.keywords if k.arg == "argnames"), None)
    if isinstance(a, ast.Constant) and isinstance(a.value, str):
        return [s.strip() for s in a.value.split(",") if s.strip()]
    if isinstance(a, (ast.List, ast.Tuple)):
        return [x.value for x in a.elts if isinstance(x, ast.Constant)]
    return []


def param_factor(mod, call):
    vals = call.args[1] if len(call.args) > 1 else next((k.value for k in call.keywords if k.arg == "argvalues"), None)
    if vals is None:
        return 1, True
    n = length(mod, vals)
    if n is UNK:
        return 1, False
    return max(n, 1), True  # an empty list still yields one (skipped) item


# --------------------------------------------------------------- fixtures ------------------------------------------
FIXTURE_NAMES = ("fixture", "yield_fixture")


def fixture_info(fn):
    for d in fn.decorator_list:
        call = d if isinstance(d, ast.Call) else None
        name = dotted(call.func if call else d) or ""
        if name.rsplit(".", 1)[-1] in FIXTURE_NAMES:
            kw = {k.arg: k.value for k in (call.keywords if call else [])}
            nm = kw.get("name")
            fname = nm.value if isinstance(nm, ast.Constant) else fn.name
            auto = isinstance(kw.get("autouse"), ast.Constant) and kw["autouse"].value is True
            return fname, kw.get("params"), auto
    return None


def fn_args(fn):
    a = fn.args
    names = [x.arg for x in a.posonlyargs + a.args + a.kwonlyargs]
    # defaults are not fixtures
    nd = len(a.defaults)
    if nd:
        names = names[: len(names) - nd - len(a.kwonlyargs)] + [x.arg for x in a.kwonlyargs]
    return [n for n in names if n not in ("self", "cls", "request")]


def fixtures_in(mod, body):
    out = {}
    for st in body:
        if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)):
            fi = fixture_info(st)
            if fi:
                out[fi[0]] = (mod, st, fi[1], fi[2])
    return out


CONFTESTS = {}


def conftest_chain(rel):
    """Fixture scopes from the root conftest down to the file's directory."""
    parts = rel.split("/")[:-1]
    chain = []
    for i in range(len(parts) + 1):
        d = "/".join(parts[:i])
        if d not in CONFTESTS:
            p = os.path.join(ROOT, d, "conftest.py")
            m = parse(p) if os.path.isfile(p) else None
            fx = {}
            if m:
                fx = fixtures_in(m, m.tree.body)
                # fixtures imported into the conftest are registered there too
                for name, dd in m.defs.items():
                    if isinstance(dd, tuple) and dd[0] == "import" and name not in fx:
                        r = lookup(m, name)
                        if r and isinstance(r[1], (ast.FunctionDef, ast.AsyncFunctionDef)):
                            fi = fixture_info(r[1])
                            if fi:
                                fx[name] = (r[0], r[1], fi[1], fi[2])
            CONFTESTS[d] = fx
        chain.append(CONFTESTS[d])
    return chain


def fixture_factor(mod, scopes, roots, direct):
    """Product of params lengths over the fixture closure of roots, nearest definition first."""
    seen = set()
    factor = 1
    stack = [(n, len(scopes)) for n in roots]
    while stack:
        name, below = stack.pop()
        if name in direct or (name, below) in seen:
            continue
        seen.add((name, below))
        hit = None
        for i in range(below - 1, -1, -1):
            if name in scopes[i]:
                hit = (i, scopes[i][name])
                break
        if hit is None:
            continue
        i, (fmod, fn, params, _auto) = hit
        if params is not None:
            k = length(fmod, params)
            if k is UNK:
                stats["fixture_params_unresolved"] += 1
                UNRES.append(("fixture", fmod.rel, fn.lineno, ast.unparse(params)[:90]))
            else:
                factor *= max(k, 1)
                stats["fixture_params_resolved"] += 1
        for a in fn_args(fn):
            # a fixture requesting its own name reaches the definition above it
            stack.append((a, i if a == name else len(scopes)))
    return factor


# --------------------------------------------------------------- collection ----------------------------------------
def is_test_fn(name):
    return name.startswith("test_") if LEVEL >= 1 else name.startswith("test")


def is_test_cls(name):
    return name.startswith("Test")


def class_mro(mod, cls, depth=0):
    """[(mod, ClassDef)] in a simple linearisation; None entries for bases we cannot see."""
    out = [(mod, cls)]
    if depth > 10:
        return out
    for b in cls.bases:
        r = lookup_expr(mod, b)
        if r and isinstance(r[1], ast.ClassDef):
            for x in class_mro(r[0], r[1], depth + 1):
                if all(x[1] is not y[1] for y in out):
                    out.append(x)
        else:
            out.append((None, dotted(b)))
    return out


def has_init(mro):
    for m, c in mro:
        if m is None:
            continue
        for st in c.body:
            if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)) and st.name in ("__init__", "__new__"):
                return True
    return False


def module_test_off(mod):
    off = set()
    for st in mod.tree.body:
        v = st.value if isinstance(st, ast.Expr) else None
        if isinstance(v, ast.Call) and isinstance(v.func, ast.Name) and v.func.id == "setattr" and len(v.args) == 3:
            a, b, c = v.args
            if isinstance(a, ast.Name) and isinstance(b, ast.Constant) and b.value == "__test__" and isinstance(c, ast.Constant) and c.value is False:
                off.add(a.id)
        if isinstance(st, ast.Assign) and isinstance(st.value, ast.Constant) and st.value.value is False:
            for t in st.targets:
                if isinstance(t, ast.Attribute) and t.attr == "__test__" and isinstance(t.value, ast.Name):
                    off.add(t.value.id)
    return off


def returns_new_function(mod, deco, depth=0):
    """A workspace decorator that returns a nested def (a wrapper), so the decorated class becomes a function."""
    target = deco.func if isinstance(deco, ast.Call) else deco
    r = lookup_expr(mod, target)
    if not r or not isinstance(r[1], (ast.FunctionDef, ast.AsyncFunctionDef)):
        return False
    fn = r[1]
    if isinstance(deco, ast.Call):  # a decorator factory: follow the returned inner def
        inner = {s.name: s for s in fn.body if isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef))}
        rets = [s.value for s in fn.body if isinstance(s, ast.Return) and isinstance(s.value, ast.Name)]
        if not rets or rets[-1].id not in inner:
            return False
        fn = inner[rets[-1].id]
    inner = {s.name for s in fn.body if isinstance(s, (ast.FunctionDef, ast.AsyncFunctionDef))}
    rets = [s.value for s in fn.body if isinstance(s, ast.Return) and isinstance(s.value, ast.Name)]
    return bool(rets) and rets[-1].id in inner


def test_flag(mro):
    for m, c in mro:
        if m is None:
            continue
        if LEVEL >= 6 and c.name in TEST_OFF.get(id(m), ()):
            return False
        for st in c.body:
            if isinstance(st, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "__test__" for t in st.targets):
                return not (isinstance(st.value, ast.Constant) and st.value.value is False)
    return True


def count_function(mod, fn, outer_marks, scopes, extra_fixtures):
    marks = marks_of(fn.decorator_list) + outer_marks if LEVEL >= 2 else []
    n = 1
    direct = set()
    for kind, call in marks:
        if kind == "parametrize" and call is not None:
            stats["parametrize"] += 1
            k, ok = param_factor(mod, call)
            if not ok:
                stats["parametrize_unresolved"] += 1
                UNRES.append(("parametrize", mod.rel, call.lineno, ast.unparse(call.args[1] if len(call.args) > 1 else call)[:90]))
            n *= k
            direct.update(argnames(call))
    if LEVEL >= 4:
        uses = []
        for kind, call in marks:
            if kind == "usefixtures" and call is not None:
                uses += [a.value for a in call.args if isinstance(a, ast.Constant)]
        autos = [nm for sc in scopes for nm, v in sc.items() if v[3]]
        n *= fixture_factor(mod, scopes, fn_args(fn) + uses + extra_fixtures + autos, direct)
    return n


def collect_class(mod, cls, outer_marks, scopes, top_only):
    if LEVEL >= 6 and any(returns_new_function(mod, d) for d in cls.decorator_list):
        stats["class_replaced_by_decorator"] += 1
        return 0
    mro = class_mro(mod, cls) if LEVEL >= 1 else [(mod, cls)]
    if LEVEL >= 1 and (has_init(mro) or not test_flag(mro)):
        stats["class_with_init_or_test_false"] += 1
        return 0
    if LEVEL == 0 and has_init([(mod, cls)]):
        return 0
    marks = outer_marks + (class_marks(cls) if LEVEL >= 2 else [])
    cscopes = scopes + [fixtures_in(mod, cls.body)] if LEVEL >= 4 else scopes
    total = 0
    seen = set()
    for m, c in mro:
        if m is None:
            if c and LEVEL >= 1:
                stats["unseen_base"] += 1
            continue
        for st in c.body:
            if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)) and is_test_fn(st.name) and st.name not in seen:
                seen.add(st.name)
                if LEVEL >= 1 and fixture_info(st):
                    continue
                if m is not mod:
                    stats["inherited_method"] += 1
                total += count_function(m, st, marks, cscopes, [])
            elif (
                LEVEL >= 1
                and not top_only
                and isinstance(st, ast.ClassDef)
                and is_test_cls(st.name)
                and st.name not in seen
                and m is mod
            ):
                seen.add(st.name)
                total += collect_class(mod, st, marks, cscopes, False)
    return total


def collect_file(path):
    mod = parse(path)
    if mod is None:
        return 0
    MODS[mod.rel[:-3].replace("/", ".")] = mod
    if LEVEL >= 6:
        TEST_OFF[id(mod)] = module_test_off(mod)
    scopes = conftest_chain(mod.rel) + [fixtures_in(mod, mod.tree.body)] if LEVEL >= 4 else []
    mmarks = module_marks(mod) if LEVEL >= 2 else []
    total = 0
    names = {}
    if LEVEL >= 1:
        for name, d in mod.defs.items():  # module __dict__, last binding wins
            names[name] = d
    else:
        for st in mod.tree.body:
            if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names.setdefault(st.name, st)
    for name, d in names.items():
        tmod, node = mod, d
        if isinstance(d, tuple):
            if d[0] != "import" or not (is_test_fn(name) or is_test_cls(name)):
                continue
            r = lookup(mod, name)
            if not r:
                continue
            tmod, node = r
            stats["imported_test_name"] += 1
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and is_test_fn(name):
            if fixture_info(node):
                continue
            total += count_function(tmod, node, mmarks, scopes, [])
        elif isinstance(node, ast.ClassDef) and is_test_cls(name):
            total += collect_class(tmod, node, mmarks, scopes, LEVEL == 0)
    return total


def main():
    conform = os.path.join(ROOT, "conform")
    files = []
    for d, _, fs in os.walk(conform):
        for f in fs:
            if f.startswith("test_") and f.endswith(".py"):
                files.append(os.path.join(d, f))
    files.sort()
    by_dir = defaultdict(int)
    per_file = {}
    for p in files:
        rel = os.path.relpath(p, ROOT)
        n = collect_file(p)
        parts = rel.split("/")
        by_dir[parts[1] if len(parts) >= 3 else "root"] += n
        per_file[rel] = n
    total = sum(by_dir.values())
    print(json.dumps({"level": LEVEL, "files": len(files), "total": total, "stats": dict(stats)}, sort_keys=True))
    if OUT:
        json.dump({"unresolved": sorted(Counter(UNRES).items(), key=lambda kv: -kv[1]) if False else [list(k) + [v] for k, v in Counter(UNRES).items()], "total": total, "by_directory": dict(by_dir), "per_file": per_file}, open(OUT, "w"), indent=1)


main()
