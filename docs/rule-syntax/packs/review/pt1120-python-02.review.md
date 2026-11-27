# Review: batch pt1120-python-02 (PYI045, PYI047, PYI049, PYI051, PYI053–PYI056)

Ground truth: Ruff 0.17.6 (`ruff check --isolated --no-cache --select <CODE>`), its rule sources and fixtures at tag
0.17.6. Translation: `rerail-engine-1120 run-policies` (the boxes' #1120 engine, sha256 `aa28dc0b…`) on the branch's
`.rerail` files at d634de0418, run on a fleet box. Ruff's columns are given; run-policies gives lines only. Every
counterexample below was run as written.

The suite agrees with the author on its corpora: `ptcheck --ref tbelanger/pt1120-python-02 -- --pack python --rules
<the eight>` gives 8 same (examples, fixtures 1963 files, samples-python 2313 files). The defects below are outside
those corpora.

| Rule | Verdict |
|---|---|
| ruff-pyi045-iter-method-return-iterable | defects (name resolution) |
| ruff-pyi047-unused-private-type-alias | defects |
| ruff-pyi049-unused-private-typed-dict | defects |
| ruff-pyi051-redundant-literal-union | defects |
| ruff-pyi053-string-or-bytes-too-long | defects |
| ruff-pyi054-numeric-literal-too-long | ok (one count difference) |
| ruff-pyi055-unnecessary-type-union | defect (`_typeshed`) |
| ruff-pyi056-unsupported-method-call-on-all | ok |

## Shared cause: `typing_mod` lacks `_typeshed`

`python/scopes`' `typing_mod` is `typing` and `typing_extensions`. Ruff's `match_typing_qualified_name`
(ruff_python_semantic `model.rs`) also matches `_typeshed`. So `typing_sym` misses `_typeshed.TypeAlias`, `.TypedDict`,
`.Literal` and `.Union`. This shows up in PYI047, PYI049, PYI051, PYI053 and PYI055 below. The originals list
`_typeshed.*` in their `resolves.import` sets.

## ruff-pyi045-iter-method-return-iterable: defects

`chain`/`seen`/`ruff_site` say that a name bound nowhere at visit time is read "once the module is complete", in
the method's own scope chain. Ruff does something else. The definition rules run in `analyze::definitions`, after the
traversal. There `resolve_qualified_name` finds no resolved name (`resolve_name`), so it falls back to `lookup_symbol`
in the semantic model's *current* scope. That scope is not the method's. It is the global scope, or the scope of
the last lambda that `analyze::deferred_lambdas` restored. Every lambda is pushed to `analyze.lambdas` (checker
`mod.rs`), and nothing restores the scope before `analyze::definitions`. So a binding that comes later in the class
or in the enclosing function is never seen:

`a_classlater.py`
```python
class A:
    def __iter__(self) -> It[int]: ...
    from collections.abc import Iterable as It
```

`b_funclater.py`
```python
def f():
    class E:
        def __iter__(self) -> It[int]: ...
    from collections.abc import Iterable as It
```

`c_lambda.py`
```python
class A:
    def __iter__(self) -> It[int]: ...
def g():
    from collections.abc import Iterable as It
    return lambda: 0
```

```text
Ruff:        c_lambda.py:2:27 PYI045
Translation: a_classlater.py:2 PYI045
             b_funclater.py:3 PYI045
```

`a_classlater` and `b_funclater` are translation-only. `c_lambda` is a miss: Ruff resolves `It` from the lambda's
scope inside `g`. The rule's own example (`Early` bound after the class, at module level) holds only because the
fallback scope is the global one there. In a stub or under `from __future__ import annotations`, the annotation is
visited deferred and resolves normally; the rule's `late` is right for those.

## ruff-pyi047-unused-private-type-alias: defects

Ruff's `is_used` is per binding: references resolved when the expression is visited, copied forward to the binding
that shadows it (`add_binding`). A `del` makes the scope's last binding a `Deletion`, which `is_assignment()`
rejects. The translation reads "used" as any load of the name in the scope (`read_in`), at any position. For
`type` statements it reads it as any identifier with that text outside the statement (`type_alias_read`).

`pyi047.py`
```python
from typing import TypeAlias
import _typeshed
type _B = list[_B]
_H: TypeAlias = "list[_H]"
type _C = int
def f(_C): return _C
print(_A)
_A: TypeAlias = int
_E: TypeAlias = int
del _E
_T: _typeshed.TypeAlias = int
```

`pyi047.pyi`
```python
from typing import TypeAlias
X: TypeAlias = list[[0 for _a in () if b._a]]
Y: TypeAlias = list[[0 for _c in () if f(_c=1)]]
```

```text
Ruff:        pyi047.pyi:2:28 PYI047      (X: `_a`)
             pyi047.pyi:3:28 PYI047      (Y: `_c`)
             pyi047.py:5:6 PYI047        (_C)
             pyi047.py:8:1 PYI047        (_A)
             pyi047.py:11:1 PYI047       (_T)
Translation: pyi047.py:3 PYI047
             pyi047.py:4 PYI047
             pyi047.py:9 PYI047
```

- Line 3, `type _B = list[_B]`: translation-only. The deferred value's `_B` resolves to the alias, so the alias is
  used. `type_alias_read` excludes every identifier inside the statement.
- Line 4, `_H: TypeAlias = "list[_H]"`: translation-only. The string value is a type definition that Ruff parses
  later, and its `_H` resolves to the binding.
- Line 5, `type _C`: missed. The only `_C` reads are the parameter's own (`def f(_C): return _C`), and
  `type_alias_read` counts them by text.
- Line 8, `_A`: missed. `print(_A)` runs before the binding exists and resolves to nothing.
- Line 9, `_E` then `del _E`: translation-only. The last binding is a deletion.
- Line 11, `_typeshed.TypeAlias`: missed (shared cause).
- Stub lines 2–3: missed. `unread_comp_var` counts any identifier with the variable's text as a read: the attribute
  name `b._a` and the keyword `_c=` are not reads.

## ruff-pyi049-unused-private-typed-dict: defects

Same causes as PYI047 (`unused_private`, `unread_comp_var`, `typing_mod`). In addition, Ruff matches a base with
`match_typing_expr(base, "TypedDict")`, which does not see through a subscript. `class_base_sym` reads through
subscripts (`Protocol[T]`).

`pyi049.py`
```python
from typing import TypedDict
import typing
import _typeshed
_B = TypedDict("_B", {"x": "_B"})
print(_C)
class _C(TypedDict): ...
class _D(TypedDict): ...
del _D
class _E(typing.TypedDict[int]): ...
class _G(_typeshed.TypedDict): ...
```

`pyi049.pyi`
```python
from typing import TypedDict
TD = TypedDict("TD", {"x": int for _a in () if b._a})
class P(TypedDict, **{k: 1 for _d in () if f(_d=1)}): ...
```

```text
Ruff:        pyi049.pyi:2:36 PYI049      (TD: `_a`)
             pyi049.pyi:3:32 PYI049      (P: `_d`)
             pyi049.py:6:7 PYI049        (_C)
             pyi049.py:10:7 PYI049       (_G)
Translation: pyi049.py:4 PYI049
             pyi049.py:7 PYI049
             pyi049.py:9 PYI049
```

- Line 4: translation-only. The field type `"_B"` is a string type definition that resolves to `_B`.
- Line 6: missed. The read comes before the binding.
- Line 7: translation-only. `del _D` follows it.
- Line 9: translation-only. A subscripted base is not a TypedDict to Ruff.
- Line 10: missed (`_typeshed`).
- Stub lines 2–3: missed. The attribute and keyword names are not reads.

## ruff-pyi051-redundant-literal-union: defects

Ruff (`redundant_literal_union`) takes a Literal's members from its slice only, unpacking a tuple. A nested
`Literal[...]` member is a subscript, so `match_literal_type` gives `None`. The translation goes through
`literal_leaf`, which descends into nested Literals. Ruff's `as_literal_expr` takes an implicitly concatenated bytes
literal as a `BytesLiteral`, and Ruff checks the unions inside string type definitions. The translation has no
clause over `str_tree`, unlike PYI055.

`pyi051.py`
```python
from typing import Literal
import _typeshed
a: Literal[Literal["a"]] | str
b: Literal[1, Literal[2]] | int
c: Literal[b"a" b"b"] | bytes
d: "Literal[4] | int"
e: list["Literal[5] | int"]
f: _typeshed.Literal["a"] | str
```

```text
Ruff:        pyi051.py:4:12 PYI051   (`1`)
             pyi051.py:5:12 PYI051
             pyi051.py:6:13 PYI051
             pyi051.py:7:18 PYI051
             pyi051.py:8:22 PYI051
Translation: pyi051.py:3 PYI051     (`"a"`)
             pyi051.py:4 PYI051     (`1`)
             pyi051.py:4 PYI051     (`2`)
```

- Line 3, and the `2` on line 4: translation-only (nested Literal).
- Line 5: missed (`b"a" b"b"`).
- Lines 6–7: missed (string type definitions, top-level and nested).
- Line 8: missed (`_typeshed.Literal`).

## ruff-pyi053-string-or-bytes-too-long: defects

Ruff counts an f-string as the sum of its parts (`count_f_string_chars`). A plain part counts its decoded
characters. An f-string part counts its literal elements' decoded characters plus each interpolation's source range.
The translation has two gaps. `long_literal`'s f-string clause requires `kind(s, "string")` and
`!inside_concat(s)`, so an implicit concatenation that contains an f-string is never measured. `fstring_body`
measures the source text between the quotes, so escapes (`\n`, 2 characters for 1) and doubled braces (`{{`, 2 for
1) count double. `typed_call` goes through `typing_sym`, so a `_typeshed.TypedDict` field is not a type to it.

`pyi053.pyi`
```python
import _typeshed
b = f"" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
c = f"\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n"
e = f"{{aaaaaaaaaa}}{{aaaaaaaaaa}}{{aaaaaaaaaa}}{{aaaaaaaaaa}}"
T = _typeshed.TypedDict("T", {"a": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"})
```

```text
Ruff:        pyi053.pyi:2:5 PYI053
Translation: pyi053.pyi:3 PYI053
             pyi053.pyi:4 PYI053
             pyi053.pyi:5 PYI053
```

- Line 2: missed. The value is 60 characters across three parts.
- Line 3: translation-only. The value is 30 characters.
- Line 4: translation-only. The value is 48 characters.
- Line 5: translation-only. The value is a field type to Ruff.

Agreements checked: `Annotated` metadata, `list["…"]` and `dict[str, Literal["…"]]` at runtime, `cast`, a TypeVar
bound, bytes with `\x41` escapes, `@warnings.deprecated("…" + "")`, bare bytes and f-string statements, and
`total=` / NamedTuple fields.

## ruff-pyi054-numeric-literal-too-long: ok

Same on everything tried: `list["Literal[…]"]` and `cast("…")` at runtime, a TypeVar bound, an f-string
interpolation, `u"…"`, a line continuation inside the string, a `\x31` escape, a triple-quoted multi-line string,
nested strings, and `Literal["12345678907"]`.

One count difference. In an unplaced string, Ruff reports each number at the string's range. The translation
reports one finding per string, so a line-keyed comparison does not see it:

```python
from typing import Literal
e: "Literal[1, 2]" ""
```
```text
Ruff:        pyi054.pyi:2:4 PYI054   (twice)
Translation: pyi054.pyi:2 PYI054     (once)
```

## ruff-pyi055-unnecessary-type-union: defect

Agrees on type-parameter bounds and defaults, `type X = …`, `if TYPE_CHECKING` / `if typing.TYPE_CHECKING`,
`builtins.type`, a local annotation in a method of a class in a function, nested unions, a runtime parameter, plain
and unplaced strings (`|` reported, `Union` not), and a shadowed `type` in a stub. One miss, the shared `_typeshed`
cause:

`pyi055.py`
```python
import _typeshed
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    x = _typeshed.Union[type[int], type[str]]
```
```text
Ruff:        pyi055.py:4:9 PYI055
Translation: (none)
```

## ruff-pyi056-unsupported-method-call-on-all: ok

Same on: an f-string interpolation, a non-string annotation (`y: __all__.extend([1])`), `list["…"]` at runtime,
`cast("…" "", 1)`, a nested string annotation, `Literal["…"]` (neither reports it), a bare decorator
`@__all__.append` (neither reports it), `[*__all__.append(1)]`, a lambda body, and a parenthesised `__all__`
across lines.

## The author's result file

`pt1120-python-02.result.json` lists all eight rules as `same` and has no `different` entry, so there is no
`right` side to check. The suite run above reproduces `same` on the corpora. The eight older `different` entries in
`equivalence.json` (each `right: "unknown"`) are stale, as the suite says. The defects above are outside the corpora,
so `same` there does not mean the rules follow Ruff.

The author's notes were in the batch's commit diff, which I read before writing these cases. None of the
counterexamples above repeats a case from the notes. PYI045's contradicts the notes' account of a name bound later.
The notes' nested-Literal cases are tuples, and a nested Literal without a tuple is still reported.
