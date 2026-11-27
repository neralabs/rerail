# pt1120-python-11 translation review

Ground truth: Ruff 0.17.6 (`ruff check --isolated --select <CODE> --output-format concise --no-cache`; `--preview`
gives the same findings for all four codes here). I read the rules at the 0.17.6 tag
(`perflint/rules/manual_dict_comprehension.rs`, `flake8_raise/rules/unnecessary_paren_on_raise_exception.rs`,
`flake8_simplify/rules/collapsible_if.rs`, `flake8_self/rules/private_member_access.rs`, `pylint/helpers.rs`,
`ruff_python_semantic` `analyze/{typing,type_inference,function_type}.rs`, `model.rs` (`only_binding`,
`lookup_attribute`, `lookup_symbol_in_scope`) and `scope.rs` (`Scope::add`)), then ran my own cases with Ruff and
`rerail-engine-1120 run-policies` on `docs/rule-syntax/packs/python/<id>.rerail` at `d5efa0477e`, with
`RERAIL_SOURCE_ROOT` set to a copy of that commit's `docs/rule-syntax` and `native/src/code_rule_dsl/typing/programs`.
The runs are on pt-w02 (`/tmp/pt-review-p11`): pt-w01 and pt-w07 have no `rerail-engine-1120`. Translation lines are
the finding's line (the engine names the node, not the column). No case uses a suppression comment.

All four rules have defects; each is listed with the file it was found on and both tools' output on it.

## `ruff-perf403-manual-dict-comprehension`: defects

Counterexample `perf403.py`:

```python
def walrus_iter(pairs):
    d = {}
    for k, v in (y := pairs):
        d[k] = v


def nested_tuple(items):
    d = {}
    for ((a, b), c), v in items:
        d[a, (b, c)] = v


def annotated_not_dict(items):
    d: int = {}
    for k, v in items:
        d[k] = v


def conditional_dict(items, c):
    d = {} if c else {1: 2}
    for k, v in items:
        d[k] = v


def walrus_dict(items):
    d = (e := {})
    for k, v in items:
        d[k] = v


def closure_then_rebound(items):
    d = {}
    for k, v in items:
        d[k] = v

    def g():
        return k

    k = 1


def annotation_first(items):
    d: dict
    d = {}
    for k, v in items:
        d[k] = v


def for_else(items):
    d = {}
    for k, v in items:
        d[k] = v
    else:
        pass


async def async_for(items):
    d = {}
    async for k, v in items:
        d[k] = v


m = {}


class K:
    for k, v in items:
        m[k] = v


m = {}
```

```text
Ruff:        perf403.py:22:9 PERF403
             perf403.py:28:9 PERF403
             perf403.py:34:9 PERF403
             perf403.py:52:9 PERF403
             perf403.py:60:9 PERF403
             perf403.py:68:9 PERF403
Translation: perf403.py:4 PERF403
             perf403.py:10 PERF403
             perf403.py:16 PERF403
             perf403.py:46 PERF403
             perf403.py:52 PERF403
             perf403.py:60 PERF403
```

1. **A walrus in the iterable (line 4, false).** Ruff returns early when `any_over_expr(iter, Expr::is_named_expr)`:
   an assignment expression is a syntax error in a comprehension's iterable. The rule has no such check.
2. **Nested tuples compared as flattened text (line 10, false).** `from_target` compares the key and the elements
   with parentheses, whitespace and comments removed, so the key `a, (b, c)` and the element `((a, b), c)` both
   become `a,b,c`. Ruff's `ComparableExpr` keeps the nesting: `Tuple(a, Tuple(b, c))` is not `Tuple(Tuple(a, b), c)`.
   Removing only the outermost parentheses (or comparing the nodes) fixes it.
3. **An annotated assignment is judged by its value (line 16, false).** `check_type` reads only the annotation of
   `x: T = v` (`Stmt::AnnAssign` → `match_annotation`). misc2-perf's `perf_typed_site` matches a `{}` or `{…: …}`
   value even when an annotation is present (only its `dict(…)` call clause checks `!perf_site_ann`), so
   `d: int = {}` is a dict to the translation. This is in the library, so PERF401 (`x: int = []`) gets it too.
4. **Value types `match_expr_type` infers (lines 22 and 28, missed).** `ResolvedPythonType::from` makes
   `{} if c else {1: 2}` (an `If` whose branches are both dicts), `{} or {}` (a `BoolOp`) and `(e := {})` (a `Named`)
   a dict. `perf_typed_site` takes a dictionary or dict comprehension display only. The JSON original's `dict-expr`
   had the conditional and walrus forms. Also a library change; `is_list` (PERF401) is the same.
5. **A read in a function defined after the loop resolves to the scope's last binding (line 34, missed).** Ruff
   visits function bodies after their enclosing scope, so `return k` in `g` reads `k = 1`, not the loop target; the
   loop target has no reference after the loop and Ruff reports. `perf_used_after` counts the bindings between the
   loop's end and the read's offset (none before `return k`), so the read counts. The rule's `nonlocal_after` already
   uses the last-binding reading for `nonlocal`; deferred reads need it too.
6. **An earlier annotation-only binding (line 46, false).** `d: dict` adds a `BindingKind::Annotation` binding, and
   `d = {}` shadows it (`Scope::add` records any earlier binding of the name), so `only_binding` is `None`. The
   translation reports it, so `perf_only_binding`'s count of earlier bindings is 1 there: the bare annotation is not
   a `binding` row [inference from the output; I did not query `binding`].
7. **A class body reads the module binding in force at that point (line 68, missed).** Class bodies are not
   deferred: `m` in `K` resolves to the first `m = {}`, which shadows nothing, so `only_binding` holds. The second
   clause of `perf_only_binding` (a read outside its binding's scope) requires the name's total binding count in that
   scope to be 1, which is only right for a function body, which Ruff reads after the scope is complete.

`for … else` and `async for` (lines 52, 60) match Ruff.

## `ruff-rse102-unnecessary-paren-on-raise-exception`: defects

Counterexample `rse102.py`:

```python
from errs import éclair, _ñame, Ünicode

raise éclair()
raise _ñame()
raise Ünicode()

raise Later()


def Later():
    return ValueError()


def in_function():
    raise Later2()


def Later2():
    return ValueError()


class lowc(Exception):
    pass


def outer():
    global lowc

    def inner():
        raise lowc()

    return inner


class K:
    def Make(self):
        return ValueError()

    def m(self):
        raise Make()


raise get_cls()()
raise len()
raise exit()


def Gone():
    return ValueError()


del Gone
raise Gone()


def Maker():
    return ValueError()


Maker: object
raise Maker()
```

Counterexample `rebound.py`:

```python
def Error():
    return ValueError()


class Error(Exception):
    pass


raise Error()
```

```text
Ruff:        rebound.py:9:12 RSE102
             rse102.py:5:14 RSE102
             rse102.py:7:12 RSE102
             rse102.py:40:19 RSE102
             rse102.py:43:16 RSE102
             rse102.py:44:10 RSE102
             rse102.py:45:11 RSE102
             rse102.py:53:11 RSE102
Translation: rse102.py:3 RSE102
             rse102.py:4 RSE102
             rse102.py:5 RSE102
             rse102.py:30 RSE102
             rse102.py:40 RSE102
             rse102.py:43 RSE102
             rse102.py:44 RSE102
             rse102.py:45 RSE102
```

1. **Bindings are not read in visit order (rebound.py:9, rse102.py:7 and 53, missed).** At module level Ruff's
   `lookup_attribute` sees the binding in force at the raise: in `rebound.py` the class (the last binding), at line 7
   nothing yet (`Later` is unbound; the name heuristic reports it), at line 53 `del Gone`'s `Deletion`, which
   `lookup_symbol_in_scope` turns into `Unbound`. `callee_kind` goes through `ref_site`, which is every binding in
   the scope with no flow pruning (lib/python/scopes), so a def anywhere in the scope makes the callee a factory.
   `rebound.py` is the realistic form: a factory that is later replaced by a class of the same name. The author's
   notes name the module-level case as not followed ("no corpus key differs"); it is a confirmed miss all the same.
   Function bodies are right: line 15 (`Later2` defined after the def that raises it) is silent in both.
2. **`global`/`nonlocal` in an enclosing scope (line 30, false).** `global lowc` in `outer` binds `lowc` in `outer`'s
   scope (`BindingKind::Global`), and `inner`'s lookup finds that binding first. It is neither a def nor a class, so
   the lowercase name is taken for a function. `declared` checks only the raise's own scope.
3. **Unicode lowercase (lines 3 and 4, false).** Ruff tests `char::is_lowercase` on the first character after one
   `_`. `factory`'s `match("_?[a-z].*", x)` is ASCII only, so `éclair` and `_ñame` are taken for classes.
   `Ünicode` (line 5) is right.

A method reading a class-body def (line 40: the class scope is not visible, so `Make` goes by its name), a call
callee (43), lowercase builtins (44, 45: Ruff reports them as builtins) and an annotation-only rebinding (60) match
Ruff.

## `ruff-sim102-collapsible-if`: defects

Counterexample `sim102.py`:

```python
from typing import TYPE_CHECKING


def f(a, b, c, d):
    if a:
        if b:
            if c:
                pass

    if a:
        x = 1
        y = 2
    elif b:
        if c:
            pass

    if TYPE_CHECKING:
        if a:
            if b:
                pass

    if __name__ == "__ma" "in__":
        if a:
            pass

    if __name__ == "\x5f_main__":
        if a:
            pass

    if a:
        pass
    elif b:
        if c:
            if d:
                pass
```

```text
Ruff:        sim102.py:5:5 SIM102
             sim102.py:6:9 SIM102
             sim102.py:13:5 SIM102
             sim102.py:18:9 SIM102
             sim102.py:32:5 SIM102
             sim102.py:33:9 SIM102
Translation: sim102.py:5 SIM102
             sim102.py:6 SIM102
             sim102.py:18 SIM102
             sim102.py:22 SIM102
             sim102.py:26 SIM102
             sim102.py:32 SIM102
             sim102.py:33 SIM102
```

1. **The `if`'s own body is checked when the branch is an `elif` (line 13, missed).** `nested_if_body` checks the
   length of the last branch's body only (`nested_if.body().len() > 1`), and that branch is the last `elif`. `branch`
   requires `short_if(s)` (the `if`'s own consequence at most one statement) for the `elif` case too, so an `if`
   with two statements followed by `elif b: if c: …` is missed. The rule's comment ("As in Ruff, the outer `if`'s own
   body must also be one statement") is wrong for the `elif` case; `short_if` belongs on the `(s, s, b)` clause only,
   where `plain_if` already requires a one-statement body.
2. **`__main__` is matched as source text (lines 22 and 26, false).** `is_main_check` compares the string literal's
   value: an implicit concatenation `"__ma" "in__"` and an escaped `"\x5f_main__"` are both `"__main__"`.
   `main_check` requires a single `string` node whose text matches `[uUrR]?["']+__main__["']+`.

Three nested `if`s give two findings in the 0.17.6 binary (lines 5 and 6, and 32 and 33 under an `elif`), as the rule
says. The tag's source has a parent check meant to drop the inner one, but the binary reports both, and the binary is
the ground truth. The `TYPE_CHECKING` exemption covers only the outer `if` in both tools (line 18).

## `ruff-slf001-private-member-access`: defects

Counterexample `slf001.py`:

```python
import abc
from typing import Self


class C:
    def __new__(cls):
        inst = object.__new__(cls)
        inst._a = 1
        inst2 = object().__new__(cls)
        inst2._b = 1
        return inst

    @abc.abstractstaticmethod
    def s(other):
        return other._c

    def m(self):
        x = Self()
        return x._d

    def n(self, *others: Self):
        return others._e

    def p(self):
        y: Self = make()
        z: int = C()
        return y._f, z._g

    def __eq__(self, o):
        return [o._h for _ in range(3)]

    f = lambda o: o._i
    j = C._j

    @classmethod
    def q(cls):
        with cls() as w:
            w._k
        if (v := cls()):
            v._l
```

```text
Ruff:        slf001.py:15:16 SLF001
             slf001.py:19:16 SLF001
             slf001.py:22:16 SLF001
             slf001.py:27:22 SLF001
             slf001.py:30:17 SLF001
             slf001.py:32:19 SLF001
             slf001.py:33:9 SLF001
Translation: slf001.py:8 SLF001
             slf001.py:10 SLF001
             slf001.py:22 SLF001
             slf001.py:27 SLF001
             slf001.py:30 SLF001
             slf001.py:38 SLF001
             slf001.py:40 SLF001
```

1. **`object.__new__(cls)` and `object().__new__(cls)` (lines 8 and 10, false).**
   `SameClassInstanceChecker::match_initializer` takes `X.__new__(…)` where X is `super(…)`, `object()` or `object`
   (builtin). misc2-self's `slf_instance_value` has `super().__new__(…)` only.
2. **`abc.abstractstaticmethod` (line 15, missed).** `function_type::is_static_method` matches
   `["" | "builtins", "staticmethod"] | ["abc", "abstractstaticmethod"]`. `slf_static` matches
   `builtins.staticmethod` only, so `other` is taken for the method's receiver.
3. **`Self()` (line 19, missed).** Ruff's initializer check takes a called name only when it is `cls`, `mcs` or the
   current class's name; `Self[T]()` passes through the subscript branch, but a plain `Self()` does not.
   `slf_instance_value`'s third clause accepts `slf_self(f)` for a called name.
4. **A class-level lambda's parameter (line 32, missed).** `is_method_receiver` needs the binding's statement to be
   a `FunctionDef`. A lambda parameter's statement is the assignment around it. `sig_param` reads any node with a
   `parameters` field, lambdas included, and the lambda's nearest scope is the class, so `slf_same_instance` takes
   `o` for a receiver.
5. **The class's own name in its body (line 33, missed).** Ruff adds the class binding after the body, so `C` in
   `j = C._j` resolves to nothing and SLF001 is reported. `slf_own_class`'s second clause (no binding, inside a class
   of that name) exempts it.
6. **`with` and walrus initializers (lines 38 and 40, false).** `check_type` reads `BindingKind::WithItemVar`
   (`with cls() as w`) and `NamedExprAssignment` (`(v := cls())`) through `match_initializer`. `slf_same_instance`'s
   variable clauses read `binding_value`; the output shows they do not reach these two forms [inference: I did not
   query `binding_value`].

`*others: Self` (22: `find_parameter` skips variadics), `y: Self = make()` / `z: int = C()` (annotation only, 25 and
27) and a comprehension in `__eq__` (30: its own scope, not the dunder's) match Ruff.

## The author's result file

`pt1120-python-11.result.json` has one `different` entry: `ruff-sim102-collapsible-if`, `right: translation`, key
`bandit/django-django-urls-resolvers.py-ed87b99ee423/django/urls/resolvers.py:341`. Ruff 0.17.6 on that corpus file
(`~/pt/corpora/real/samples-python/…`) reports no SIM102, and neither does the translation. The `elif` holding the
lone `if` is not the last branch, and Ruff checks only the last one. **Confirmed: the translation is right.**

The three `same` statuses hold on the corpora only. The defects above are outside them: PERF403 (7), RSE102 (3
causes), SIM102 (2), SLF001 (6). PERF403 3 and 4 and SLF001 1, 3 and 6 are in the shared libraries (misc2-perf,
misc2-self), so PERF401 and anything else built on misc2-self need the same fix.
