# Review: python-02

Branch reviewed: `tbelanger/pack-translations-python-02` at `479f5f360f`. I read each `.rerail`, its JSON original, Ruff's rule documentation, rule source, helper source, and Ruff 0.17.6 fixtures before reading the author's result file. Ruff 0.17.6 and every translation counterexample ran on fleet boxes; `run-policies` used this branch's `docs/rule-syntax/lib`. Suppression-comment differences are excluded because `run-policies` does not apply them yet.

The standard corpus comparison says each translation now matches its JSON original on the recorded corpora. The independent cases below compare the translation with Ruff itself and expose behavior absent from those corpora.

| Rule | Verdict |
|---|---|
| `ruff-ruf017-quadratic-list-summation` | 3 defects |
| `ruff-ruf018-assignment-in-assert` | 6 defects |
| `ruff-ruf019-unnecessary-key-check` | 7 defects |
| `ruff-ruf020-never-union` | 4 defects |
| `ruff-ruf021-parenthesize-chained-operators` | 3 defects |
| `ruff-ruf022-unsorted-dunder-all` | 6 defects |
| `ruff-ruf026-default-factory-kwarg` | 1 defect |
| `ruff-ruf028-invalid-formatter-suppression-comment` | 10 defects |

For the concise outputs below, Ruff ran as `ruff check --isolated --no-cache --select RUFxxx --output-format concise <file>`. The translation ran as `rerail-engine run-policies <case-dir> <out-dir> <policy>`; its output is shown as `file:line CODE`, or `nothing` when `.diagnostics` was empty. Every code block is a complete file.

## `ruff-ruf017-quadratic-list-summation`

Ruff's `RUF017_0.py` and `RUF017_1.py` fixtures otherwise match, including builtin/import resolution, positional and keyword `start`, empty `list()`, and simple type strings.

### Defect 1: starred first argument with keyword `start` is missed

Ruff only requires `arguments.args.first()`; a starred expression counts. `ruff_arg(c, 0, _)` and `tstr_arg(c, 0, _)` reject it.

```python
# r017-star.py
sum(*xs, start=[])
```

- Ruff: `r017-star.py:1:1: RUF017 Avoid quadratic list summation`.
- translation: nothing.

The same miss occurs in `a: "sum(*xs, start=[])" = 1`.

### Defect 2: a keyword `start` does not override the positional value

Ruff's `find_argument_value("start", 1)` chooses the keyword first. The translation accepts either argument and reports the empty positional list.

```python
# r017-precedence.py
sum(x, [], start=[1])
```

- Ruff: `All checks passed!`
- translation: `r017-precedence.py:1 RUF017`.

The same false report occurs inside a simple type string.

### Defect 3: later module shadowing is applied too early in class bodies and defaults

```python
# r017-class-shadow.py
class C:
    value = sum(x, [])

sum = 0
```

- Ruff: `r017-class-shadow.py:2:13: RUF017 Avoid quadratic list summation`.
- translation: nothing.

Ruff resolves the class-body call in source order, before the module assignment. The shared `ruff_qual` later-binding clause requires the call's current scope to be the binding scope. A default argument such as `def f(a=sum(x, [])): ...; sum = 0` is missed for the same reason.

## `ruff-ruf018-assignment-in-assert`

Ruff's fixture matches, including reads confined to asserts and the ordinary post-assert read. The defects are in semantic-reference edge cases.

### Defect 1: an exception-handler binding after the walrus loses the earlier reference

```python
# r018-except.py
assert (x := 1)
try:
    pass
except ValueError as x:
    pass
print(x)
```

- Ruff: `r018-except.py:1:9: RUF018 Avoid assignment expressions in assert statements`.
- translation: nothing.

Ruff closes the handler with an unbound-exception binding that retains the previous binding. The translation treats every `except ... as x` as a hard chain break.

### Defect 2: a `for` iterable is ordered after its target

```python
# r018-for.py
assert (x := [1])
for x in x:
    pass
```

- Ruff: `r018-for.py:1:9: RUF018 Avoid assignment expressions in assert statements`.
- translation: nothing.

Ruff visits the iterable before binding the target. The translation compares source offsets, where the target text precedes the iterable.

### Defect 3: `global` followed by a local binding creates a false deferred reference

```python
# r018-global.py
def f():
    global x
    x = 2
    return x

assert (x := 1)
```

- Ruff: `All checks passed!`
- translation: `r018-global.py:6 RUF018`.

Ruff's later function binding shadows its `Global` binding. The translation homes the read to the module walrus.

### Defect 4: deferred forward references are missed

```python
# r018-forward.py
def f(a: "T"):
    pass

assert (T := int)
```

- Ruff: `r018-forward.py:4:9: RUF018 Avoid assignment expressions in assert statements`.
- translation: nothing.

Module string annotations, `cast("T", value)`, and future-annotations expressions fail the same way. Ruff resolves these after the scope is complete; the rule has no type-string/deferred-annotation reads.

### Defect 5: a bare annotation does not participate in the binding chain

```python
# r018-bare-annotation.py
x: int
print(x)
assert (x := 1)
```

- Ruff: `r018-bare-annotation.py:3:9: RUF018 Avoid assignment expressions in assert statements`.
- translation: nothing.

Ruff's `Annotation` binding receives the read, then the walrus copies that binding's references. The translation creates no `ev` for a bare annotation.

### Defect 6: a lambda body that is exactly the name is not deferred

```python
# r018-lambda.py
g = lambda: q
assert (q := 1)
```

- Ruff: `r018-lambda.py:2:9: RUF018 Avoid assignment expressions in assert statements`.
- translation: nothing.

`deferred` uses strict `within(n, body)`. When `n` is the lambda body node itself, it does not match. `lambda: q + 0` is correctly reported.

## `ruff-ruf019-unnecessary-key-check`

The upstream fixture and ordinary boolean contexts, tuple keys, side-effect filtering, and simple literal comparisons match. The following gaps are in `ComparableExpr`, type strings, and boolean-test propagation.

### Defect 1: adjacent f-string or bytes pieces are not normalized

```python
# r019-fstring-concat.py
d = {}
if f"a" "b" in d and d[f"ab"]:
    pass
```

- Ruff: `r019-fstring-concat.py:2:4: RUF019 Unnecessary key check before dictionary access`.
- translation: nothing.

`b"a" b"b"` versus `b"ab"`, and mixed plain/f-string pieces, fail for the same reason. Ruff joins comparable bytes and normalized interpolated-string literal pieces; `compare.rerail` only joins plain strings.

### Defect 2: f-string debug text is ignored

```python
# r019-debug.py
d = {}
x = 1
if f"{x=}" in d and d[f"{x =}"]:
    pass
```

- Ruff: `All checks passed!`
- translation: `r019-debug.py:3 RUF019`.

The expressions print different debug text. Ruff includes that text in comparability; the translation does not.

### Defect 3: imaginary numbers compare by spelling

```python
# r019-imaginary.py
d = {}
if 1j in d and d[1.0j]:
    pass
```

- Ruff: `r019-imaginary.py:2:4: RUF019 Unnecessary key check before dictionary access`.
- translation: nothing.

Ruff compares complex values. The translation compares lowercased source text.

### Defect 4: large integers in different bases compare as equal

```python
# r019-bigint.py
d = {}
if 0x10000000000000000 in d and d[18446744073709551616]:
    pass
```

- Ruff: `All checks passed!`
- translation: `r019-bigint.py:2 RUF019`.

For integers beyond `u64`, Ruff's comparable integer retains token text; the translation canonicalizes the value.

### Defect 5: a comma in a subscript comment is treated as a trailing comma

```python
# r019-comment-comma.py
d = {}
k = 1
if k in d and d[k  # a,
]:
    pass
```

- Ruff: `r019-comment-comma.py:3:4: RUF019 Unnecessary key check before dictionary access`.
- translation: nothing.

The `tuple_key` regex reads the comma inside `# a,` as syntax.

### Defect 6: key checks in parsed type strings are missed

```python
# r019-type-string.py
d = {}
k = 1
x: "[a for a in [] if k in d and d[k]]"
```

- Ruff: `r019-type-string.py:3:23: RUF019 Unnecessary key check before dictionary access`.
- translation: nothing.

A `cast("[a for ...]", value)` first argument fails similarly. The JSON original had a `check-in-string` path; the translation has none.

### Defect 7: Ruff treats the first argument of invalid multi-argument `bool` calls as a test

```python
# r019-bool-extra.py
d = {}
k = 1
value = bool(k in d and d[k], 1)
```

- Ruff: `r019-bool-extra.py:3:14: RUF019 Unnecessary key check before dictionary access`.
- translation: nothing.

The program raises `TypeError`, so this is low severity. Ruff sets boolean-test context on the first argument; `bool_test` requires `call_only`.

## `ruff-ruf020-never-union`

The upstream fixture, ordinary PEP 604 and `typing.Union` structures, aliases from `typing`/`typing_extensions`, and simple type strings match.

### Defect 1: `_typeshed` typing names are missing

```python
# r020-typeshed.py
from _typeshed import Never
x: Never | int
```

- Ruff: `r020-typeshed.py:2:4: RUF020 Never | T is equivalent to T`.
- translation: nothing.

`from _typeshed import Union; Union[Never, int]` is also missed. Ruff's typing-name matcher accepts `typing`, `typing_extensions`, and `_typeshed`; `never_q` and `union_q` omit `_typeshed`.

### Defect 2: a comma in a comment is treated as a trailing union comma

```python
# r020-comment-comma.py
from typing import Never, Union
Union[(Never, int)  # a,
]
```

- Ruff: `r020-comment-comma.py:2:8: RUF020 Union[Never, T] is equivalent to T`.
- translation: nothing.

`trailing_comma` reads the comment comma as syntax, the same cause as RUF019 defect 5.

### Defect 3: a deferred function body ignores a later module rebinding

```python
# r020-later-rebind.py
from typing import Never

def f():
    return Never | int

Never = int
```

- Ruff: `All checks passed!`
- translation: `r020-later-rebind.py:4 RUF020`.

Ruff analyzes the function after the module and resolves `Never` to the final assignment. Shared `ruff_qual` considers only bindings before the textual reference.

### Defect 4: complex type strings have a count/behavior mismatch

```python
# r020-complex-string.py
from typing import Never
y: "Never | " "int"
```

- Ruff: `All checks passed!`
- translation: `r020-complex-string.py:2 RUF020`.

With an undefined other member, Ruff emits two findings while the translation emits one. Escaped spellings such as `"Never | \x69nt"` have the same divergence. This is low severity and partly reflects Ruff's unstable range/name handling for complex string annotations, but it is still observable parity behavior.

## `ruff-ruf021-parenthesize-chained-operators`

Ruff's fixture matches exactly (12 findings), as do ordinary expressions, simple string annotations, positional `cast`, TypeVar bounds/constraints, `NewType`, and normal parenthesization.

### Defect 1: several Ruff type-definition sites have no `type_root`

```python
# r021-cast-keyword.py
from typing import cast
value = cast(typ="a or b and c", val=1)
```

- Ruff: `r021-cast-keyword.py:2:23: RUF021 Parenthesize a and b expressions ...`.
- translation: nothing.

The same miss occurs in `TypeVar(default=...)`, `TypeAliasType`, functional `NamedTuple`/`TypedDict`, `TypeForm`, `mypy_extensions.Arg` and relatives, and a TypedDict class's `extra_items=`.

### Defect 2: strings passed to ordinary calls inside a type are parsed as types

```python
# r021-call-argument.py
x: f("a or b and c")
```

- Ruff: `All checks passed!`
- translation: `r021-call-argument.py:1 RUF021`.

Ruff visits an ordinary call's arguments as non-type definitions. `type_ctx` propagates through every child. Keyword arguments and an ordinary call inside an outer string annotation fail similarly.

### Defect 3: Ruff loses parentheses in concatenated or escaped annotations

```python
# r021-complex-parentheses.py
long_variable_name_to_shift_offsets: "a or (b and c)" ""
```

- Ruff: `r021-complex-parentheses.py:1:38: RUF021 Parenthesize a and b expressions ...`.
- translation: nothing.

Ruff assigns parsed nodes the whole complex string's range, so its token check no longer observes the inner parentheses. An escaped `"a or (b \x61nd c)"` behaves the same. Shorter prefixes can make Ruff panic instead; those panic-only cases are not counted.

## `ruff-ruf022-unsorted-dunder-all`

Ruff's fixture matches exactly (35 findings). Ordinary assignments, annotations, `+=`, `.extend`, scope boundaries, and the common isort/natural-order cases also match.

### Defect 1: parenthesized `__all__` names are missed

```python
# r022-parenthesized-name.py
(__all__) = ["b", "a"]
```

- Ruff: `r022-parenthesized-name.py:1:13: RUF022 __all__ is not sorted`.
- translation: nothing.

`(__all__) += ...` and `(__all__).extend(...)` fail for the same reason: the name check does not unparenthesize.

### Defect 2: Ruff's constant length is UTF-8 bytes, not characters

```python
# r022-byte-length.py
__all__ = ["A", "É"]
```

- Ruff: `r022-byte-length.py:1:11: RUF022 __all__ is not sorted`.
- translation: nothing.

Ruff treats one-character `É` as a constant because it is two UTF-8 bytes and places it before class `A`. The translation's character count does not. The reverse order produces a translation-only finding.

### Defect 3: only ASCII lowercase excludes a translation constant

```python
# r022-unicode-lowercase.py
__all__ = ["ΩΩ", "Éé"]
```

- Ruff: `All checks passed!`
- translation: `r022-unicode-lowercase.py:1 RUF022`.

Ruff's Unicode `is_cased_uppercase` classifies `Éé` as a class. The translation excludes only `[a-z]`, so it calls `Éé` a constant. `Aª` and `Aß` expose the same bug; reversing this file's entries gives a missed report.

### Defect 4: lowercasing is not Unicode's Uppercase property

```python
# r022-titlecase.py
__all__ = ["b", "ǅǅ"]
```

- Ruff: `All checks passed!`
- translation: `r022-titlecase.py:1 RUF022`.

`ǅ` is titlecase, not uppercase, but lowercasing changes it. Conversely, `ℂ` has the Uppercase property without a lowercase mapping, which yields a missed Ruff finding.

### Defect 5: digit runs of ten or more collide in `nat_key`

```python
# r022-long-digits.py
__all__ = ["a9999999999", "a10000000000"]
```

- Ruff: `All checks passed!`
- translation: `r022-long-digits.py:1 RUF022`.

The translation maps every length of ten or more to `:` and then compares digits. Reversing these entries gives a miss. The marker also collides with a literal colon (`["a1234567890", "a:"]`).

### Defect 6: `.extend` inside a parsed annotation string is missed

```python
# r022-type-string.py
x: "__all__.extend(['b', 'a'])"
```

- Ruff: `r022-type-string.py:1:20: RUF022 __all__ is not sorted`.
- translation: nothing.

Ruff visits the parsed call with the module scope captured at the annotation. `all_checked` has no type-string path.

## `ruff-ruf026-default-factory-kwarg`

Ruff's fixture matches (14 findings), as do runtime call values, imports/shadowing, arguments, and ordinary simple type strings.

### Defect 1: an inner simple string is checked inside a complex outer type string

```python
# r026-complex-outer.py
from collections import defaultdict
from typing import List

x: "List['defaultdict(default_factory=int)']" ""
```

- Ruff: `All checks passed!`
- translation: two findings at `r026-complex-outer.py:4`.

Ruff does not inspect any nested type string when the outer annotation string is concatenated or escaped. `tstr_simple(s)` tests only the innermost string, and the concatenated parse reaches it twice. The un-concatenated control emits one finding in both tools.

## `ruff-ruf028-invalid-formatter-suppression-comment`

Ruff's fixture matches (11 findings). The rule diverges where Ruff's suppression-comment visitor assigns comments to AST nodes, plus alternate-body and directive parsing details.

### Defect 1: the directive regex accepts prefixes Ruff rejects

```python
# r028-prefixed.py
x = [
    # noqa: E501 # fmt: off
    1,
]
```

- Ruff: `All checks passed!`
- translation: `r028-prefixed.py:3 RUF028`.

Ruff requires off/on/yapf directives to occupy the comment after one leading `#`; only `skip` can appear in a later `#`-separated segment. `## fmt: off` and `## fmt: on` are also false reports.

### Defect 2: an own-line comment after the module's last node is visited

```python
# r028-eof.py
x = 1
# fmt: skip
```

- Ruff: `All checks passed!`
- translation: `r028-eof.py:2 RUF028`.

Ruff's visitor never captures this trailing top-level own-line comment. A file containing only the comment behaves the same.

### Defect 3: intervening comment indentation is ignored

```python
# r028-min-indent.py
def f():
    x = [  # fmt: off
        1
    ]
# explanation
    # fmt: on
```

- Ruff: only `r028-min-indent.py:2:12 RUF028`.
- translation: findings on lines 2 and 6.

Ruff uses the minimum indentation of comments between the entered expression and formatter comment. The translation uses only the formatter comment's column.

### Defect 4: every later alternate clause is treated as the first

```python
# r028-second-elif.py
def f():
    if a:
        pass
    elif b:
        pass
    # fmt: off
    elif c:
        pass
```

- Ruff: `All checks passed!`
- translation: `r028-second-elif.py:6 RUF028`.

Ruff applies this reason only to the first alternate body. Later `else`, second `except`, and `else`/`finally` after an `except` produce the same false report.

### Defect 5: a deeper own-line comment after a decorator is misattached

```python
# r028-decorator-indent.py
@dec
    # fmt: off
def f():
    pass
```

- Ruff: `All checks passed!`
- translation: `r028-decorator-indent.py:2 RUF028`.

Ruff attaches the deeper comment to the decorator node, where it is valid. The translation treats it as after a decorator.

### Defect 6: an end-of-line off comment on a bare decorator is rejected

```python
# r028-decorator-trailing.py
@dec  # fmt: off
def f():
    pass
```

- Ruff: `All checks passed!`
- translation: `r028-decorator-trailing.py:1 RUF028`.

Ruff permits trailing `off` on the decorator. The translation classifies it as inside an expression.

### Defect 7: parentheses Ruff drops make valid statement comments look expression-local

```python
# r028-return-annotation.py
def f() -> (  # fmt: off
    int
):
    pass
```

- Ruff: `All checks passed!`
- translation: `r028-return-annotation.py:1 RUF028`.

Ruff's enclosing node is the valid `FunctionDef`; tree-sitter retains a parenthesized expression. AnnAssign annotations, `for` targets, parenthesized case patterns/guards, type-alias values, parenthesized `with` items, and a skip after a with-item before its comma expose the same mismatch.

### Defect 8: `fmt: skip` followed by another segment is missed

```python
# r028-skip-suffix.py
def f():
    # fmt: skip # noqa: E501
    pass
```

- Ruff: `r028-skip-suffix.py:2:5: RUF028 it cannot be on its own line`.
- translation: nothing.

Ruff recognizes `skip` in a `#`-separated segment. The translation's match requires it at the end.

### Defect 9: arbitrary misalignment above an alternate body is missed

```python
# r028-misaligned.py
if True:
    pass
  # fmt: off
else:
    pass
```

- Ruff: `r028-misaligned.py:3:3: RUF028 it cannot be directly above an alternate body`.
- translation: nothing.

Ruff compares the comment indentation with the preceding statement. The translation reports only when comment and clause columns are equal.

### Defect 10: comments at the start of alternate bodies after handlers are missed

```python
# r028-handler-else.py
def f():
    try:
        pass
    except Exception:
        pass
    else:
        # fmt: off
        pass
```

- Ruff: `r028-handler-else.py:8:9: RUF028 it cannot be directly above an alternate body`.
- translation: nothing.

A `finally` body has the same miss. An explanatory comment between `# fmt: off` and an `else` clause is also missed because the clause is no longer the formatter comment's immediate sibling.

## The author's result file

`python-02.result.json` marks all eight rules `same`. It contains no `different` entry, so there is no `right` side to validate against Ruff.

Pinned fleet `ptcheck` runs independently confirmed `same` for RUF018, RUF019, RUF020, and RUF026 across the examples, 1,963 recorded fixture files, and 2,313 sample files. Their only gate problem is the stale `different` entry in the pack's equivalence configuration: each says the rule now reports the same findings as its JSON original and should be removed from `different`.

That corpus equality is narrower than tool parity. The counterexamples above compare directly with Ruff 0.17.6; they are shapes absent from the recorded corpora, and in several cases the JSON original shares the translation's gap.
