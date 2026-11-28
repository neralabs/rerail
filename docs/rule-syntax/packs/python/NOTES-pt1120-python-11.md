# pt1120-python-11: PERF403, RSE102, SIM102, SLF001

Checked with ptcheck on `tbelanger/pack-translations-1120` plus this batch's files (engine `rerail-engine-1120`),
every sample key against Ruff 0.17.6 (`ruff check --select <CODE> --no-cache`) and Ruff's source at that tag.
PERF403, RSE102 and SLF001 are `same` now; SIM102 differs at one key, where Ruff agrees with the translation. Each
counterexample below is a `.test` in its rule (`test-rr`: 4 passed; the rules as they were fail all four). The rules
stay per file: with `RERAIL_STRICT_CROSSINGS`, `RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS` the four run. No
library file was added: no concept here is shared by two of the rules.

## PERF403 (`same`; was 2 original-only, 1 translation-only)

Ruff's `is_dict` is `check_type::<DictChecker>`, which reads more bindings than misc2-perf's `perf_dict_site`
(assignments and parameters). The rule's `dict_site` adds the two the fixtures hit:

- A `with` item's target: `with dict() as res: for k, v in items: res[k] = v` is PERF403
  (`PERF403-parenthesized-elements-and-nonlocal.py:8`); also `with {} as d`, `with dict(a=1) as d`. A tuple target
  (`with dict() as (d, z)`) is not: Ruff's `match_value` pairs a tuple target only with a tuple value.
- A def's return annotation: `def made() -> dict: …` then `made[k] = v` in a loop is PERF403 (`…:10`); so is
  `-> Dict[str, int]`. A string annotation `-> "dict"` is not.

`nonlocal_after`: Ruff's `nonlocal x` adds a read of the enclosing scope's last binding of x, so `for a, b in items:
acc[a] = b` followed by `def g(): nonlocal b` is not PERF403 (`…:15`; the translation reported it). Only a
`nonlocal` after the loop counts (Ruff compares the read's offset with the loop's end), and only when the loop
target's binding is the scope's last one written there: with `b = 1` after the loop, the `nonlocal` reads that, and
the loop is reported. `global x` adds no read (checked: a module-level loop with `def g(): global b` after it is
PERF403).

**Library change proposed:** both belong in misc2-perf's `perf_typed_site`, for PERF401 too (Ruff's `is_list` is the
same `check_type`).

## RSE102 (`same`; was 3 original-only, 3 translation-only)

Ruff decides with `lookup_attribute`, at the raise, on the callee's dotted name: the first name's binding, then for
each further name the class body's binding of it (`scopes[class].get`, the last one). The translation resolved bare
names only, and through `global`/`nonlocal`.

- `raise A.B.C.Foo()` with `Foo` a def in class C is no RSE102; `raise (A.B).Make()` neither (parentheses are not in
  Ruff's tree). The translation reported both (`RSE102-class-walk-global-nonlocal.py:16, 18`).
- `raise A.B.C.bar()` with `bar` a class is RSE102 whatever its case (`…:17`, missed).
- A name declared `global` or `nonlocal` in the raise's scope is bound there to the declaration (`BindingKind::Global`
  / `Nonlocal`), neither a def nor a class, so the name heuristic decides: `global lowc; raise lowc()` is not RSE102
  though `lowc` is a class (`…:32`), `global Upfn; raise Upfn()` is though `Upfn` is a def (`…:33`), and so is
  `nonlocal Fn; raise Fn()` (`…:42`).

`callee_kind`/`callee_class`/`class_member` in the rule do this. Not followed: Ruff's lookup at a module-level raise
sees only the bindings before it (`raise Foo()` then `class Foo`), the translation every binding in the scope; no
corpus key differs.

## SIM102 (`different`, 1 original-only; the translation is right)

`django/urls/resolvers.py:341` (samples-python): an `if` with two `elif`s, the first `elif self._is_endpoint:` holding
a lone `if`, the last `elif path.startswith(…)` a `return`. Ruff looks at the last branch only, so no SIM102 there
(`ruff check --select SIM102`: all checks passed); the original reports the first `elif`. The translation follows
Ruff (`branch`: the `if` itself without `elif`/`else`, else its last `elif`). Rule unchanged.

## SLF001 (`same`; was 4 translation-only)

- Ruff's `is_dunder` is `__` prefix and suffix, `is_sunder` `_` prefix and suffix and neither `__`: `x._`, `x.__` and
  `x.___` are not SLF001. The translation's globs (`__*__`, `_*_`) needed two characters between, and reported
  `builtins._(…)` (`flake8_gettext/INT001.py:51`, `INT002.py:47`, `INT003.py:47`). `slf_magic` uses prefix/suffix.
- `check_type` reads a def binding's return annotation: `def make() -> Outer: …`, then `make._b` inside class
  `Outer`, is no SLF001 (`SLF001-current-class-and-dunder-scopes.py:9`), nor with `-> Self`, `-> Annotated[Outer, …]`
  or `-> Outer[int]`; `-> "Outer"` is. `slf_returns_own`, with `slf_ret_head` walking the annotation as
  misc2-self's `slf_ann_head` does.
- Ruff's current scope for a parameter's default is the scope around the def or lambda: in `def __eq__(self, o):
  f = lambda a=o._c: a._d`, `o._c` is not reported (it runs in `__eq__`), `a._d` is; `def __eq__(self, o, d=o._p)`
  reports `o._p` (it runs in the class body). The translation used the innermost scope (`slf_runs_in` now). Line keys
  hid this in the corpora (the same line held a finding either way).

**Library change proposed:** misc2-self's `slf_ann` should seed def return annotations (`field(f, "return_type",
t)`), and `slf_same_instance` take a def binding through it; `slf_ret_head` then goes.
