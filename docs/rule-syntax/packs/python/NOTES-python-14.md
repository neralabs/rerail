# python-14: flake8-comprehensions C400 to C406, C408

Ruff checks a string type definition (an annotation, `cast("…", x)`'s type, a TypeVar bound, a string inside one) as
code and reports what it finds at the string. Every difference of these rules from their originals was there: the
translations found nothing in a parsed string (C400 to C406), and C408 took every parsed call for one without
arguments. The rules now share `lib/python/python-14.rerail` (new), which reaches the parsed trees through
python-01's `typedef_node` and says each shape without the relations that miss those trees.

Ground truth: `ruff check --select C4 --no-cache` (Ruff on the fleet box), on the corpus probes and on
`t1.py` below.

## Per rule

All eight follow Ruff, which the originals agree with on every corpus file.

- **C400, C401, C403**: `def f(list: int) -> "list(x for x in xs)"` is reported (Ruff: C400 at 8:22 of
  `rerail/probes/C400-string-annotations.py`): a parameter's and the return annotation are evaluated where the def
  runs, so the parameter `list` does not shadow the builtin there. `d: "list(x for x in xs)"` in the body is not
  reported (the parameter shadows it), nor `e: "…"` in a class body after `list = tuple`. python/upgrade's
  `builtin_at` looks a signature's names up from inside the function (it misses `def f(list: int) -> list(x for x in
  xs): …` in a plain file too, which Ruff reports: t1.py below), and has no lookup at all for a name parsed from a
  string outside an annotation (`cast`). python-14's `c14_name_at` asks comprehensions' `c4_builtin_here` at the def
  for a signature's call and at the file's string otherwise.
- **C402**: `dict` by name, as before; the class-body `dict = list` case of the probe is reported, as Ruff does.
- **C404, C406**: two-element tuples by `c14_two` (child numbers, comments aside), not counts. `y:
  Literal["dict([(1, 2)])"]` is not reported: a literal's string is a value (python-01's `type_ctx` stops there).
- **C405**: `x: "List['set([1])']"`: the string inside a parsed string is parsed too (Ruff 3:11).
- **C408**: `y = cast("1 not in list()", 1)` is reported (Ruff 11:20 of
  `rerail/probes/RUF060-fstring-literal-parts-and-annotations.py`); `a: "list(x for x in xs)"` no longer is. In
  a parsed string the callee is matched by name only: `"builtins.list()"` is not resolved (no corpus case).
- **`global x`** (C408's `rerail/probes/C408-builtins-import-order.py:3`, a translation-only key until the last
  round): Ruff's `resolve_builtin_symbol` takes a name its function declares `global` for the function's binding, not
  the builtin, until it has visited an import of `builtins`; after that it resolves the name to the builtin.
  python/upgrade's `builtin_at` takes it for the builtin always. Ruff visits module-level statements first, then
  function bodies by depth, each depth in source order, so (Ruff 0.17.6 on the box, the translation agreeing):
  `def a(): global dict; return dict()` followed by `def b(): import builtins` is not reported, nor with the import in
  a function nested in `a`; with `import builtins` or `from builtins import len` at module level, before or after
  `a`, it is. The original's `seenBefore` reads as source order; if it is, the original misses the module-level
  import after `a` and reports the nested one (not run: neither is in a corpus). C400 to C406 use
  `has_builtin_binding`, which never takes a global-declared name for the builtin: `import builtins` / `def a(y):
  global list; return list(x for x in y)` is not C400 (Ruff agrees). `c14_global_ref`, `c14_seen` model this.

`t1.py` (Ruff's findings in the comments; the translations report the same):

```python
from typing import cast
xs = range(3)
def f(list: int) -> list(x for x in xs):                              # C400
    return list
def g(set: int, a: "set(x for x in xs)") -> "set([x for x in xs])":   # C401, C403
    return set
y = cast("list()", 1)                                                 # C408
z = cast("dict(a=1)", 1)                                              # C408
def h(dict: int):
    w = cast("dict()", 1)                                             # not reported: the parameter
    return w
```

## Engine gaps (repros on the box, `rerail-engine run-policies`)

Both make library relations silently miss the nodes of a parsed string; python-14 works around them.

1. A body joining `kind(c, K)`, `field(c, F, x)` and `kind(x, K2)` gives no row for a node of a parsed string, while
   each pair of those atoms does. `typedef_node(c), kind(c, "call"), field(c, "function", f), kind(f, "identifier")`
   has no row for `a: "list(x)"`; without `kind(c, "call")`, or through a relation `fn(c, f) :- typedef_node(c),
   field(c, "function", f)` and then `fn(c, f), kind(f, "identifier")`, it has. python/ruff's `call_item` and
   comprehensions' `c4_call`, `c4_named` and `c4_only` are such bodies, so every rule built on them misses parsed
   strings.
2. A count over an input relation counts no node of a parsed string: `n = count : { child(al, _, _) }` is 0 for a
   parsed call's argument list, while a count over a derived relation of the same nodes (`c14_item`) is right.
   python-01 notes this for counts in general; python/ruff's `call_nargs`, `call_nkwargs`, comprehensions'
   `c4_pair` and `c4_simple` count children.

## Library changes proposed

- Fix (1) and (2) in the engine; then python-14's `c14_only`, `c14_nopos`, `c14_nokw`, `c14_two` fold back into
  python/ruff (`call_only`, `call_nargs`, `call_nkwargs`) and comprehensions (`c4_only`, `c4_pair`), and
  `c14_call` (a file's call or a `typedef_node` call) is what the C4 rules start from.
- python/names (`wstart`) or python/upgrade (`builtin_at`): look a parameter's annotation and the return annotation
  up from the scope around the def, and give a name parsed from any `typedef_node` string a start (today only
  `parsed_in`'s annotation strings have one). `c14_name_at` does this for the C4 collection names only.
