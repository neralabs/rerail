# Python rule batch 04 review

Reviewed against Ruff 0.17.6 documentation, implementation, upstream fixtures, targeted `ptcheck` runs, and independent probes. `run-policies` commands used the reviewed `.rerail` files from `tbelanger/pack-translations-python-04`.

## `ruff-ruf051-if-key-in-dict-del`: defect

Ruff compares numeric literal keys by numeric value. The translation's float key normalization only removes underscores and adds a leading zero before `.`, so it misses equivalent spellings such as `1e1` and `10.0` (and likewise `3j` and `3.0j`).

Counterexample `case.py`:

```python
d = {}
if 1e1 in d:
    del d[10.0]
```

Ruff:

```text
case.py:3:5: RUF051 Use `pop` instead of `key in dict` followed by `del dict[key]`
Found 1 error.
```

Translation (`run-policies`):

```text
0 diagnostic(s)
```

## `ruff-ruf057-unnecessary-round`: ok

The upstream fixture and independent cases covered integer expressions, inferred integer names, the maximum signed-64-bit `ndigits`, the first out-of-range value, negative `ndigits`, and a shadowed `round`. Ruff and `run-policies` both reported only the integer expression and the inferred integer with `9223372036854775807`.

## `ruff-ruf058-starmap-zip`: ok

Independent default-target cases covered empty `zip`, a generator argument, all-starred and partly-starred `zip` arguments, a starred first `starmap` argument, and `**kwargs`. Ruff and `run-policies` reported the same four calls. The Python 3.14 `strict=` branch is explicitly version-gated in the translation.

## `ruff-ruf059-unused-unpacked-variable`: defect

The translation hard-codes Ruff's default dummy-variable pattern instead of honoring `lint.dummy-variable-rgx` (an option present in the JSON original).

`pyproject.toml`:

```toml
[tool.ruff.lint]
dummy-variable-rgx = "^second$"
```

Counterexample `case.py`:

```python
def consume(values):
    first, second = values
    return first
```

Ruff:

```text
All checks passed!
```

Translation (`run-policies`):

```text
1 diagnostic(s)
case.py:2 RUF059
```

## `ruff-ruf060-in-empty-collection`: defect

Ruff recursively recognizes a one-argument empty constructor only for `set` and `frozenset`. The translation applies that recursion to every empty collection constructor, producing false positives for `list([])`, `tuple(())`, `dict({})`, `str("")`, `bytes(b"")`, and `bytearray(b"")`.

Counterexample `case.py`:

```python
1 in list([])
```

Ruff:

```text
All checks passed!
```

Translation (`run-policies`):

```text
1 diagnostic(s)
case.py:1 RUF060
```

## `ruff-ruf061-legacy-form-pytest-raises`: ok

Independent calls to `pytest.raises`, `pytest.warns`, and `pytest.deprecated_call`, both as context managers and in legacy function form, matched Ruff 0.17.6. `ptcheck` listed three recorded original-only keys, but current Ruff reports none at those keys; running Ruff directly on all three recorded files reported only `RUF043-import-forms-and-other-calls.py:15`, which the translation also reports. The recorded oracle entries are stale tool output, not translation misses.

## `ruff-ruf063-access-annotations-from-class-dict`: defect

Ruff disables RUF063 below Python 3.10 when `lint.typing-extensions` is false. The translation has no target-version or typing-extensions gate.

`pyproject.toml`:

```toml
[tool.ruff]
target-version = "py39"

[tool.ruff.lint]
typing-extensions = false
```

Counterexample `case.py`:

```python
thing.__dict__["__annotations__"]
```

Ruff:

```text
All checks passed!
```

Translation (`run-policies`):

```text
1 diagnostic(s)
case.py:1 RUF063
```

## `ruff-ruf064-non-octal-permissions`: defect

The translation resolves a `pathlib.Path` receiver through an earlier assignment even after the name is rebound. Ruff uses the current binding and does not treat the final receiver as a path.

Counterexample `case.py`:

```python
from pathlib import Path

path = Path("a")
path = object()
path.chmod(644)
```

Ruff:

```text
All checks passed!
```

Translation (`run-policies`):

```text
1 diagnostic(s)
case.py:5 RUF064
```

## Author result verification

`python-04.result.json` has one `different` entry: RUF061, with `right: "translation"`. Confirmed. Ruff 0.17.6 reported no RUF061 at each recorded right-side sample key:

- `flake8_bugbear/B017_0.py:51`
- `rerail/probes/B017-comment-before-argument.py:3`
- `rerail/probes/RUF043-import-forms-and-other-calls.py:14`

On the third file Ruff reported line 15, the actual legacy-form call; `run-policies` reports that call too. Suppression-only differences were not treated as translation defects because `run-policies` does not yet apply suppression comments.
