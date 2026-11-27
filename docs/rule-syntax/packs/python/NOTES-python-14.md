# python-14: flake8-comprehensions C400 to C406, C408

Ruff checks a string type definition (an annotation, `cast("…", x)`'s type, a TypeVar bound, a string inside one) as
code and reports what it finds at the string. The original port's differences were in these trees: C400 to C406
found nothing there and C408 treated every parsed call as argument-free. Native guest tapes and field-bound
matchers are now fixed in #1185. `lib/python/python-14.rerail` keeps the explicit `typedef_node` domain and
Ruff-specific host lookup, global-declaration order and shape semantics.

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

## Native prerequisites and callee normalization

#1185 retains guest tapes and runs field-bound matchers over them, keyed by document rather than shared parsed
facts. The old missing field joins and argument facts are no longer an engine limitation.

`c14_fn` removes callee parentheses once, before any name or binding lookup. Previously only the file-local
`c14_builtin` path did that, so `cast("(dict)()", value)` was missed while the equivalent bare callee worked.
Ruff 0.17.6 reports the parenthesized call; the guest-isolation regression now checks every expected line, the
Literal exclusion, twin-policy equality and the separate guest-request packs.

The domain and lookup distinctions still matter: signature annotations are evaluated around the def, parsed
type-definition strings need their host as a lookup start, and C408's `global` handling follows Ruff's visit order.
The tuple predicates also retain the C4 rules' specific treatment of starred elements and comments.
