# Python 15 rule review

Reviewed `tbelanger/pack-translations-python-15` at `b14c12c5ee` against Ruff 0.17.6. I read Ruff's rule documentation, implementations, and fixtures under `crates/ruff_linter/resources/test/fixtures/flake8_comprehensions/`. I also ran each rule through `ptcheck`; all eight matched Ruff on the recorded corpora. The defects below require cases absent from those corpora.

Known suppression-comment differences are excluded.

## `ruff-c409-unnecessary-literal-within-tuple-call` — ok

The translation matches Ruff's name-only builtin lookup, exact single positional argument, rejection of keywords and splats, list/tuple literal kinds, parenthesis handling, and parsed string type definitions. Ruff's C409 fixture and custom positive/negative cases produced no mismatch.

## `ruff-c411-unnecessary-list-call` — ok

The translation matches Ruff's name-only builtin lookup, one-list-comprehension argument requirement, rejection of additional positional or keyword arguments, and parsed string type definitions. Ruff's C411 fixture and custom positive/negative cases produced no mismatch.

## `ruff-c413-unnecessary-call-around-sorted` — defects

`/tmp/python15-review/counterexamples/c413/case.py`:

```python
from builtins import list as builtin_list
import builtins
xs = [2, 1]
alias_case = builtin_list(sorted(xs))
qualified_string_case: "builtins.list(sorted(xs))"
```

Ruff:

```text
case.py:4:14: C413 Unnecessary `list()` call around `sorted()`
case.py:5:25: C413 Unnecessary `list()` call around `sorted()`
```

Translation (`run-policies`):

```json
[]
```

1. **Underreport: imported builtin aliases.** Ruff's `resolve_builtin_symbol` follows `from builtins import list as builtin_list`; `p15_builtin_call` accepts an unbound canonical name or an attribute, not an imported alias.
2. **Underreport: qualified builtins in string type definitions.** The file path uses `p15_builtin_call`, but the parsed-string path uses name-only `p15_str_call`, so `builtins.list(...)` is missed inside a string annotation.

## `ruff-c414-unnecessary-double-cast-or-process` — defects

`/tmp/python15-review/counterexamples/c414/case.py`:

```python
from builtins import set as builtin_set
xs = [2, 1]
alias_case = builtin_set(list(xs))
string_case: "set(list(xs))"
```

Ruff:

```text
case.py:3:14: C414 Unnecessary `list()` call within `set()`
case.py:4:15: C414 Unnecessary `list()` call within `set()`
```

Translation (`run-policies`):

```json
[]
```

1. **Underreport: imported builtin aliases.** Ruff resolves the outer alias to `builtins.set`; the translation does not.
2. **Underreport: all string type definitions.** C414 uses `p15_builtin_call` only. That relation excludes parsed string-definition nodes, so even canonical `set(list(xs))` is missed there.

## `ruff-c416-unnecessary-comprehension` — defects

`/tmp/python15-review/counterexamples/c416/case.py`:

```python
pairs = [("a", 1)]
parenthesized_names = {(k): (v) for k, v in pairs}
conditional_dict = {} if pairs else {}
conditional_type = [(k, v) for k, v in conditional_dict.items()]
direct_dict = {}
items_with_argument = [(k, v) for k, v in direct_dict.items(1)]
deferred_dict = []
def deferred_lookup():
    return [(k, v) for k, v in deferred_dict.items()]
deferred_dict = {}
```

Ruff:

```text
case.py:2:23: C416 Unnecessary dict comprehension (rewrite using `dict()`)
case.py:4:20: C416 Unnecessary list comprehension (rewrite using `list()`)
case.py:6:23: C416 Unnecessary list comprehension (rewrite using `list()`)
case.py:9:12: C416 Unnecessary list comprehension (rewrite using `list()`)
```

Translation (`run-policies`):

```json
[]
```

1. **Underreport: parenthesized dict-comprehension names.** Ruff's AST removes parentheses around `k` and `v`; the translation requires the key and value fields themselves to be identifiers instead of unparenthesizing them.
2. **Underreport: incomplete dict type inference.** Ruff's `typing::is_dict` recognizes an if-expression whose branches are dictionaries. `dict_ref` recognizes only direct dictionary literals/comprehensions and `dict(...)` calls.
3. **Underreport: `.items()` arguments are restricted when Ruff does not restrict them.** Ruff identifies a call to a known dict's `.items` without checking its arguments. `dict_items` requires zero positional and keyword arguments.
4. **Underreport: deferred function lookup uses the wrong binding.** Ruff analyzes the function after the module and resolves `deferred_dict` to its final `{}` binding. `dict_ref` selects the earlier `[]` binding at the function's source position.

## `ruff-c417-unnecessary-map` — defects

`/tmp/python15-review/counterexamples/c417/case.py`:

```python
from builtins import map as builtin_map
xs = [1, 2]
alias_case = builtin_map(lambda x: x + 1, xs)
string_case: "map(lambda x: x + 1, xs)"
late_binding_case = map(lambda x: lambda: [x for x in xs], xs)
```

Ruff:

```text
case.py:3:14: C417 Unnecessary `map()` usage (rewrite using a generator expression)
case.py:4:15: C417 Unnecessary `map()` usage (rewrite using a generator expression)
```

Translation (`run-policies`):

```json
[{"path":"case.py","line":5,"code":"C417"}]
```

1. **Underreport: imported builtin aliases.** Ruff's builtin resolver follows `from builtins import map as builtin_map`; `p15_builtin_call` does not accept that alias.
2. **Underreport: string type definitions.** C417 uses `p15_builtin_call` only, so canonical `map(...)` is missed in parsed string definitions.
3. **Overreport: nested-comprehension shadowing differs from Ruff's late-binding guard.** Ruff's syntactic visitor sees the nested lambda and a load of the outer parameter name, so it suppresses line 5. The translation resolves that load to the comprehension target and reports C417.

## `ruff-c418-unnecessary-literal-within-dict-call` — ok

The translation matches Ruff's name-only builtin lookup, one dict-literal-or-comprehension argument requirement, rejection of additional positional or keyword arguments, and parsed string type definitions. Ruff's C418 fixture and custom positive/negative cases produced no mismatch.

## `ruff-c419-unnecessary-comprehension-in-call` — defects

`/tmp/python15-review/counterexamples/c419/case.py`:

```python
from builtins import any as builtin_any
import builtins
xs = [1, 2]
alias_case = builtin_any([x > 0 for x in xs])
qualified_string_case: "builtins.any([x > 0 for x in xs])"
```

Ruff:

```text
case.py:4:26: C419 Unnecessary list comprehension
case.py:5:38: C419 Unnecessary list comprehension
```

Translation (`run-policies`):

```json
[]
```

1. **Underreport: imported builtin aliases.** Ruff resolves `builtin_any` to `builtins.any`; the translation does not.
2. **Underreport: qualified builtins in string type definitions.** Direct `any(...)` strings are handled, but the string path is name-only and misses `builtins.any(...)`.

## Author result comparison

`python-15.result.json` contains no `different` entries: all eight rules are marked `same`, so there were no right-side sample keys to validate.

The `same` claims are contradicted by the Ruff 0.17.6 and `run-policies` outputs above for C413, C414, C416, C417, and C419. `NOTES-python-15.md` also explicitly says C414 and C417 do not check string type definitions, despite Ruff doing so; those known underreports are not represented in the result file.
