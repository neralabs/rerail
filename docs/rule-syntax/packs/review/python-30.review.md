# python-30 translation review

Reviewed against Ruff 0.17.6 on `junction-pt-w01`. I read Ruff's rule documentation, implementations, and upstream fixtures for PERF101, PERF102, PERF401, PERF402, EM101, EM102, and EM103. Suppression comments are excluded because `run-policies` does not apply them yet.

## `ruff-perf101-unnecessary-list-cast`: defects

Ruff and the translation disagree on deferred module binding lookup, nested `nonlocal`, parenthesized mutation calls, bare deletion, keyword splats, `with` bindings, and nested unpacking.

Counterexample `review_perf101.py`:

```python
values = [1]
def sees_old_global():
    for item in list(values): pass
values = make_value()

def sees_later_global():
    for item in list(later): pass
later = [1]

def nested_nonlocal():
    values = [1]
    for item in list(values): pass
    def inner():
        nonlocal values
        values = None

def parenthesized_mutation():
    values = [1]
    for item in list(values):
        (values).append(item)

def bare_delete():
    values = [1]
    for item in list(values):
        del values

def keyword_splat(kw):
    values = [1]
    for item in list(values, **kw): pass

def with_binding():
    with [1] as values:
        for item in list(values): pass

def nested_unpacking():
    (other, values), tail = ([0], [1]), 2
    for item in list(values): pass
```

Observed output:

```text
Ruff:         PERF101 at lines 7, 12, 29, 33, 37
run-policies: PERF101 at lines 3, 19, 24
```

Specific mismatches:

- Line 3 is a translation false positive; Ruff analyzes the deferred function against the final module binding, `make_value()`.
- Line 7 is a translation false negative; Ruff sees the later module list binding.
- Line 12 is a false negative; a `nonlocal values` in a nested function does not make the outer loop target nonlocal.
- Line 19 is a false positive; Ruff's AST removes the receiver parentheses and sees `values.append(...)` as a mutation.
- Line 24 is a false positive; Ruff's final binding is the deletion, so it cannot establish a list-valued iterable.
- Lines 29, 33, and 37 are false negatives. Ruff ignores call keywords when checking the single positional `list` argument and resolves `with` and recursively nested assignment bindings.

## `ruff-perf102-incorrect-dict-iterator`: defects

The translation's unused-name analysis omits Ruff references created by deferred closures, `del`, and augmented assignment. It also treats a nested list pattern as an unused tuple and rejects `items(**kwargs)`, although Ruff permits keywords when the positional argument list is empty.

Counterexample `review_perf102.py`:

```python
def closure_before(mapping):
    def show(): print(value)
    for key, value in mapping.items(): print(key)

def deleted_value(mapping):
    for key, value in mapping.items():
        print(key)
        del value

def augmented_value(mapping):
    for key, value in mapping.items():
        print(key)
        value += 1

def nested_list_pattern(mapping):
    for key, (_, [_, _]) in mapping.items(): print(key)

def keyword_splat(mapping, kw):
    for key, value in mapping.items(**kw): print(key)
```

Observed output:

```text
Ruff:         PERF102 at line 19
run-policies: PERF102 at lines 3, 6, 11, 16
```

Lines 3, 6, 11, and 16 are translation false positives. Line 19 is a translation false negative.

## `ruff-perf401-manual-list-comprehension`: defects

The translation reports a loop whose iterable contains a named expression, which Ruff deliberately skips. Its list-receiver inference is also narrower than Ruff's: it misses list-valued `+`, `or`, conditional expressions, nested unpacking, and walrus bindings.

Counterexample `review_perf401.py`:

```python
def walrus_iterable(values):
    result = []
    for item in (items := values): result.append(item * 2)

def list_addition(values):
    result = [] + []
    for item in values: result.append(item * 2)

def list_or(values):
    result = [] or []
    for item in values: result.append(item * 2)

def list_conditional(values, condition):
    result = [] if condition else []
    for item in values: result.append(item * 2)

def nested_unpacking(values):
    (other, result), tail = ([0], []), 1
    for item in values: result.append(item * 2)

def walrus_binding(values):
    (result := [])
    for item in values: result.append(item * 2)
```

Observed output:

```text
Ruff:         PERF401 at lines 7, 11, 15, 19, 23
run-policies: PERF401 at line 3
```

Line 3 is a translation false positive. Lines 7, 11, 15, 19, and 23 are translation false negatives.

## `ruff-perf402-manual-list-copy`: defect

The same incomplete list-receiver inference misses every tested nontrivial Ruff list binding.

Counterexample `review_perf402.py`:

```python
def list_addition(values):
    result = [] + []
    for item in values: result.append(item)

def list_or(values):
    result = [] or []
    for item in values: result.append(item)

def list_conditional(values, condition):
    result = [] if condition else []
    for item in values: result.append(item)

def nested_unpacking(values):
    (other, result), tail = ([0], []), 1
    for item in values: result.append(item)

def walrus_binding(values):
    (result := [])
    for item in values: result.append(item)
```

Observed output:

```text
Ruff:         PERF402 at lines 3, 7, 11, 15, 19
run-policies: no diagnostics
```

All five are translation false negatives.

## `ruff-em101-raw-string-in-exception`: defect

`flake8-errmsg.max-string-length` is a UTF-8 byte length in Ruff, but the translation counts Unicode scalar values for `str` literals.

Counterexample `review_em101.py`, with `flake8-errmsg.max-string-length = 2`:

```python
def unicode_length():
    raise RuntimeError("é")

def ascii_control():
    raise RuntimeError("ab")
```

Observed output:

```text
Ruff:         EM101 at lines 2 and 5
run-policies: EM101 at line 5
```

Line 2 is a translation false negative: `"é"` is two UTF-8 bytes. `run-policies` has no options argument, so this probe hardcoded the resolved threshold `2` into an otherwise identical scratch policy; the shipped policy run with `run-rr` and `{"flake8-errmsg.max-string-length": 2}` produced the same line-5-only output. At the default threshold, Ruff and `run-policies` matched on empty, ASCII, Unicode, escaped, raw, bytes, concatenated, parenthesized, `from`, typing-cast, starred, keyword, and line-continuation cases.

## `ruff-em102-f-string-in-exception`: ok

Ruff and `run-policies` both reported lines 5, 6, 7, 8, and 14 of the review case. The case covered plain and adjacent f-strings, raw f-strings, `raise ... from`, a cause-only f-string, typing-cast, a leading starred argument, a keyword argument, and a backslash continuation.

## `ruff-em103-dot-format-in-exception`: ok

Ruff and `run-policies` both reported lines 2 through 11 and line 15 of the review case. The case covered string, bytes, adjacent and parenthesized receivers; integer, float, complex, boolean, `None`, and ellipsis literals; and exclusions for f-strings, unary literals, chained calls, and a `.format()` appearing only in the `from` cause.

## Author result audit

`python-30.result.json` marks all seven rules `same`. It contains no `different` entries, so there are no `right` sides or sample keys to validate. Those corpus-equivalence statuses do not cover the handcrafted Ruff mismatches above.
