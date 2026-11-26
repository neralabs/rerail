# Python 03 translation review

Reviewed `tbelanger/pack-translations-python-03` independently against Ruff 0.17.6. I read each Ruff rule's official documentation, implementation, and `crates/ruff_linter/resources/test/fixtures/ruff/RUFxxx.py`, then ran additional cases with Ruff and `~/pt/bin/rerail-engine run-policies` on `junction-pt-w01`. Paths in the observed output below are shortened to `case.py`.

I wrote the findings before opening `python-03.result.json`. That file marks all eight rules `same`; it has no `different` entry and therefore no `right` side to verify. The counterexamples below are outside its corpora.

## `ruff-ruf032-decimal-from-float-literal` — defect

[Ruff documentation](https://docs.astral.sh/ruff/rules/decimal-from-float-literal/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF032.py)

Ruff resolves the imported `Decimal` at the call before the later local class binding. The translation resolves the future class binding instead and misses the call.

```python
from decimal import Decimal

def f():
    value = Decimal(1.0)
    class Decimal: pass
    return value
```

Observed:

```text
ruff:         case.py:4:21: RUF032 `Decimal()` called with float literal argument
run-policies: (no diagnostics)
```

## `ruff-ruf033-post-init-default` — defect

[Ruff documentation](https://docs.astral.sh/ruff/rules/post-init-default/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF033.py)

The same order-sensitive import-resolution defect affects a dataclass decorator used before a later local binding.

```python
from dataclasses import dataclass as dc

def f():
    @dc
    class C:
        def __post_init__(self, value=1): pass
    class dc: pass
    return C
```

Observed:

```text
ruff:         case.py:6:39: RUF033 `__post_init__` method with argument defaults
run-policies: (no diagnostics)
```

## `ruff-ruf034-useless-if-else` — defect

[Ruff documentation](https://docs.astral.sh/ruff/rules/useless-if-else/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF034.py)

Ruff's comparable-expression logic compares the decoded bytes value, so adjacent bytes literals equal the combined literal. The translation's comparison does not equate these forms.

```python
value = b"a" b"b" if condition else b"ab"
```

Observed:

```text
ruff:         case.py:1:9: RUF034 Useless `if`-`else` condition
run-policies: (no diagnostics)
```

## `ruff-ruf037-unnecessary-empty-iterable-within-deque-call` — defect

[Ruff documentation](https://docs.astral.sh/ruff/rules/unnecessary-empty-iterable-within-deque-call/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF037.py)

The same order-sensitive import-resolution defect affects an imported `deque` alias used before a later local binding.

```python
from collections import deque as D

def f():
    value = D([])
    class D: pass
    return value
```

Observed:

```text
ruff:         case.py:4:13: RUF037 Unnecessary empty iterable within a deque call
run-policies: (no diagnostics)
```

## `ruff-ruf040-invalid-assert-message-literal-argument` — ok

[Ruff documentation](https://docs.astral.sh/ruff/rules/invalid-assert-message-literal-argument/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF040.py)

No defect found. Probes covered positive, float, complex, boolean, `None`, ellipsis, parenthesized, raw-bytes, and adjacent-bytes literals, plus unary numbers, strings, f-strings, lists, tuples, sets, and dictionaries. This minimal boundary case agrees:

```python
assert value, 1
assert value, -1
assert value, b"a" b"b"
assert value, "ok"
```

Observed:

```text
ruff:         case.py:1:15: RUF040 Non-string literal used as assert message
              case.py:3:15: RUF040 Non-string literal used as assert message
run-policies: case.py:1 RUF040
              case.py:3 RUF040
```

## `ruff-ruf046-unnecessary-cast-to-int` — two defects

[Ruff documentation](https://docs.astral.sh/ruff/rules/unnecessary-cast-to-int/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF046.py)

The translation reports line 3 from the future `before = 1` assignment, which Ruff does not infer at the earlier use. It also misses line 7: Ruff resolves the imported `math` alias at the call before the later local class binding.

```python
import math as m

false_positive = int(round(before, 2))
before = 1

def f():
    false_negative = int(m.ceil(1.2))
    class m: pass
    return false_negative
```

Observed:

```text
ruff:         case.py:7:22: RUF046 Value being cast to `int` is already an integer
run-policies: case.py:3 RUF046
```

Thus line 3 is translation-only and line 7 is Ruff-only.

## `ruff-ruf048-map-int-version-parsing` — defect

[Ruff documentation](https://docs.astral.sh/ruff/rules/map-int-version-parsing/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF048.py)

Inside a string annotation, Ruff combines adjacent separator literals into the string `"."`. The translation's type-string branch requires one string node and misses the concatenated form.

```python
value: "map(int, __version__.split('.' ''))"
```

Observed:

```text
ruff:         case.py:1:9: RUF048 `__version__` may contain non-integral-like elements
run-policies: (no diagnostics)
```

## `ruff-ruf049-dataclass-enum` — defect

[Ruff documentation](https://docs.astral.sh/ruff/rules/dataclass-enum/) · [Ruff fixture](https://github.com/astral-sh/ruff/blob/main/crates/ruff_linter/resources/test/fixtures/ruff/RUF049.py)

The order-sensitive import-resolution defect affects both the dataclass decorator and enum base when their imported aliases are used before later local bindings.

```python
from dataclasses import dataclass as dc
from enum import Enum as E

def f():
    @dc
    class C(E): pass
    class dc: pass
    class E: pass
    return C
```

Observed:

```text
ruff:         case.py:5:5: RUF049 An enum class should not be decorated with `@dataclass`
run-policies: (no diagnostics)
```
