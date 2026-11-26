# Python 07 translation review

Ground truth: Ruff 0.17.6. I read Ruff's rule documentation, implementation, and 0.17.6 fixtures, then ran additional cases with Ruff and `rerail-engine run-policies` on `junction-pt-w01`.

## `ruff-pyi024-collections-named-tuple`: defects

The translation resolves references by textual order, but Ruff resolves names in deferred function bodies against the end-of-module binding. It also misses string type expressions passed to `typing.cast`.

Counterexample `pyi024.py`:

```python
from collections import namedtuple
from typing import cast

def deferred():
    return namedtuple("P", "x")

namedtuple = None

from collections import namedtuple as nt
casted = cast("nt", 1)
```

```text
Ruff:        pyi024.py:10:16 PYI024
Translation: pyi024.py:5 PYI024
```

Thus line 5 is a translation-only false positive and line 10 is a missed Ruff finding.

## `ruff-pyi026-type-alias-without-annotation`: defect

`in_enum` follows a class declared later and suppresses the member, while Ruff resolves the base at the subclass definition and reports it.

Counterexample `pyi026.pyi`:

```python
from enum import Enum

class Child(Later):
    RED = None

class Later(Enum): ...
```

```text
Ruff:        pyi026.pyi:4:5 PYI026
Translation: no diagnostics
```

## `ruff-pyi030-unnecessary-literal-union`: defects

Counterexample `pyi030.py`:

```python
from typing import Literal, TypeAlias, Union

a: Union[(Literal[1], Literal[2])]
Alias: TypeAlias = "Literal[1] | Literal[2]"
b: "dict[Literal[1] | Literal[2], int]" ""
```

```text
Ruff:        pyi030.py:3:4 PYI030
             pyi030.py:4:21 PYI030
             pyi030.py:5:4 PYI030
Translation: no diagnostics
```

The translation misses: Ruff opening a parenthesized tuple used as `Union`'s sole slice; parsed string annotations used as `TypeAlias` values; and a complex concatenated string whose literal union is nested inside another generic.

## `ruff-pyi032-any-eq-ne-annotation`: defect

Ruff treats `_typeshed.Any` as `typing.Any`; `typing_sym` does not.

Counterexample `pyi032.py`:

```python
import _typeshed

class C:
    def __eq__(self, other: _typeshed.Any) -> bool: ...
```

```text
Ruff:        pyi032.py:4:29 PYI032
Translation: no diagnostics
```

## `ruff-pyi033-legacy-type-comment`: ok

Ruff and the translation agreed on spacing around `type:`, empty and whitespace-only annotations, `# type:#comment`, `# type: #comment`, `ignore`/`ignored`, and a comment whose first directive is `noqa`.

## `ruff-pyi036-bad-exit-annotation`: defects

Four independent resolution/parsing gaps:

```python
# stringized.py
from types import TracebackType
class C:
    def __exit__(self, typ: "type[BaseException] | None", exc: "BaseException | None", tb: "TracebackType | None"): ...

# type_name.py
from types import TracebackType
class C:
    def __exit__(self, type: type[BaseException] | None, value: BaseException | None, tb: TracebackType | None): ...

# star_import.py
from ctypes import *
class C:
    def __exit__(self, *args: object): ...

# optional.py
from typing import Optional
class C:
    def __exit__(self, typ: Optional[type[BaseException], int], value: object, tb: object): ...
```

```text
Ruff:
optional.py:4:29 PYI036

Translation:
stringized.py:4 PYI036  (first, second, and third arguments)
type_name.py:4 PYI036   (first argument)
star_import.py:4 PYI036 (`*args`)
```

- Ruff passes annotations through `match_maybe_stringized_annotation`; the translation calls `type_bare` on the string node and reports all three valid stringized annotations.
- A parameter named `type` shadows the builtin only for the translation's lookup from the function scope.
- Any star import prevents the translation's builtin resolution, so valid `object` is rejected.
- `optional_ok` does not require exactly one `Optional` argument, so it accepts invalid `Optional[type[BaseException], int]` that Ruff reports.

## `ruff-pyi041-redundant-numeric-union`: defects

Three independent cases:

```python
# singledispatch.py
from functools import singledispatch
@singledispatch
def dispatch(value): ...
@dispatch.register
def _(value: int | float): ...

# star_import.py
from os.path import *
def f(value: int | float): ...

# nested_tuple.py
from typing import Union
def f(value: Union[(int, Union[(float, str)])]): ...
```

```text
Ruff:
singledispatch.py: no diagnostics
star_import.py:3:14 PYI041
nested_tuple.py:3:14 PYI041

Translation:
singledispatch.py:6 PYI041
star_import.py: no diagnostics
nested_tuple.py: no diagnostics
```

Ruff skips the first parameter of a registered `singledispatch` implementation; the translation does not. A star import suppresses the translation's builtin resolution. The translation also fails to open the nested parenthesized tuple as Ruff's union traversal does.

## `ruff-pyi044-future-annotations-in-stub`: ok

Ruff and the translation agreed on a plain import, an aliased `annotations` import, an import mixed with another future feature, and a future import without `annotations`.

## Author result validation

The three `different` entries have the correct `right: "translation"` side:

- PYI024: Ruff reports `def a(self) -> "namedtuple": ...` after `from collections import namedtuple`.
- PYI030: Ruff does not report `"list[int] | Literal[1]" ""`.
- PYI044: Ruff does not report `from ..__future__ import annotations` or `from . __future__ import annotations`.

The result compares the translation with the JSON original, not Ruff. Its corpora omit the counterexamples above, so its `same` statuses and its three explained differences do not establish tool parity.
