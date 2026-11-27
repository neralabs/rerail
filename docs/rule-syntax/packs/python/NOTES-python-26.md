# Python translation batch 26

The seven rules follow Ruff. Fleet `ptcheck` results and exact remaining corpus counts are recorded in
`python-26.result.json`. Ground truth used `~/.local/bin/ruff check --select … --no-cache` on the reported fixture or
corpus file.

## Shared vocabulary

`lib/python/python-26.rerail` adds the ANN concepts shared by this batch:

- builtin `staticmethod` and `classmethod` resolution after a module/class bare annotation or an expired `except … as`
  target;
- receiver selection using that decorator resolution;
- Ruff-compatible `__all__` extraction for parenthesized strings, set displays passed to `tuple`, validated
  `list`/`tuple` calls, and non-chained assignments;
- parenthesized property setter/deleter recognition; and
- return-code selection using those facts.

No engine primitive or existing-library edit is required.

## `ruff-ann001-missing-type-function-argument`

Follows Ruff. A resolved static method has no implicit receiver, even when the builtin name previously appeared only
in a bare annotation or as an exception target. Counterexample:

```python
staticmethod: object
try:
    pass
except Exception as staticmethod:
    pass
class C:
    @staticmethod
    def __new__(cls): ...  # ANN001 for cls
```

The old translation treated `cls` as a receiver. `ann_ruff_param` uses the corrected decorator resolution.

## `ruff-ann201-missing-return-type-undocumented-public-function`

Follows Ruff's visibility. Parenthesized strings and supported constructor expressions in `__all__` make definitions
public:

```python
__all__ = "C", ("f")
class C:
    def m(self): ...  # ANN201
def f(): ...          # ANN201
```

The JSON original omits valid ANN201 findings in unrelated Ruff fixtures such as `flake8_bugbear/B012.py`; Ruff reports
those functions, so any such remaining translation-only findings are intentional and the translation is right.

## `ruff-ann202-missing-return-type-private-function`

Follows Ruff's inverse visibility and accessor handling. Invalid `__all__` constructors do not export their apparent
string arguments, chained assignments are not evaluated as `__all__`, and parentheses do not hide a matching setter
or deleter:

```python
__all__ = list(["hidden"], ["also_hidden"])
class C:
    @((value)).setter
    def value(self, new): ...  # ANN202
```

The JSON original omits valid nested-function findings in `flake8_bugbear/B012.py`; Ruff reports them, so any such
remaining translation-only findings are intentional and the translation is right.

## `ruff-ann204-missing-return-type-special-method`

Follows Ruff's decorator precedence. A dunder resolved as a static or class method is ANN205 or ANN206, not ANN204:

```python
classmethod: object
try:
    pass
except Exception as classmethod:
    pass
class C:
    @classmethod
    def __init_subclass__(cls): ...  # ANN206, not ANN204
```

The shared return classifier removes the former ANN204 false positives. No remaining divergence is expected.

## `ruff-ann205-missing-return-type-static-method`

Follows Ruff. A module/class bare annotation and an expired exception target do not shadow the builtin decorator:

```python
staticmethod: object
class C:
    @staticmethod
    def make(): ...  # ANN205
```

When both decorators resolve, classmethod wins for the return code, matching Ruff. No remaining divergence is expected.

## `ruff-ann206-missing-return-type-class-method`

Follows Ruff. Expired exception targets do not prevent classmethod resolution, and classmethod wins over staticmethod
for the return code:

```python
class C:
    @classmethod
    @staticmethod
    def make(cls): ...  # ANN206
```

The same definition still exposes `cls` to ANN001 because its staticmethod decorator means there is no implicit
receiver. No remaining ANN206 divergence is expected.

## `ruff-ann401-any-type`

Follows Ruff for invalid nested quoted annotations. A quoted annotation whose parsed expression is another string
resolves to Any:

```python
def quoted_escape(x: "'in\\x74'"): ...  # ANN401
```

The JSON original misses valid Ruff findings on Pydantic's `json_schema.py` at lines 207, 2029, 2127, and 2393. Ruff
reports each line, so those translation-only findings are intentional and the translation is right. The original also
exceeds its clause budget on that corpus; this is an original-rule limitation, not an engine gap in the translation.
