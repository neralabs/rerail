# Python 12 rule review

Compared the translations with Ruff 0.17.6 on `junction-pt-w01`. I read Ruff's rule documentation, implementations, semantic-model code, and UP024/026/028/029/030/031/032/033 fixtures. Ruff and the translations agree on Ruff's own fixtures; the targeted cases below expose additional behavior. Outputs are `Ruff → run-policies`. Suppression comments are excluded because engine suppression is a separate known issue.

## `ruff-up024-os-error-alias`: defects

1. **Root-level relative imports become absolute.** Counterexample in root `a.py` (no package):
   ```python
   from .mmap import error
   raise error
   ```
   Ruff: no finding. Translation: UP024 line 2.

2. **Parenthesized callees are missed.**
   ```python
   import socket
   (IOError)(1)
   (socket.error)(1)
   ```
   Ruff: UP024 lines 2 and 3. Translation: no findings.

3. **A comprehension target does not shadow the element expression.**
   ```python
   values = [IOError() for IOError in errors]
   ```
   Ruff: no finding. Translation: UP024 line 1.

4. **An exception target does not shadow the handler type.**
   ```python
   try: pass
   except IOError as IOError: pass
   ```
   Ruff: no finding. Translation: UP024 line 2.

5. **Deletion is ignored.**
   ```python
   del IOError
   raise IOError
   ```
   Ruff: no finding. Translation: UP024 line 2.

6. **PEP 695 type parameters are ignored as bindings.**
   ```python
   def f[IOError]():
       raise IOError
   ```
   Ruff: no finding. Translation: UP024 line 2.

7. **Deferred binding order differs.**
   ```python
   try: pass
   except Exception as IOError: pass
   raise IOError
   ```
   Ruff: UP024 line 3. Translation: no finding. A module `raise IOError` before a deferred function body assigns `global IOError` has the same mismatch.

8. **Imported aliases in string annotations are missed.**
   ```python
   from socket import error
   x: "error(1)" = 1
   ```
   Ruff: UP024 line 2. Translation: no finding. `builtins.IOError` and `socket.error` inside string annotations have the same gap.

9. **Type-expression strings outside annotations are missed.**
   ```python
   from typing import cast
   import socket
   cast("socket.error(1)", x)
   ```
   Ruff: UP024 line 3. Translation: no finding.

10. **Deferred type-alias values are visited in the wrong order.**
    ```python
    type X = socket.error(1)
    import socket
    ```
    Ruff: UP024 line 1. Translation: no finding.

## `ruff-up026-deprecated-mock-import`: defects

1. **Type-expression strings outside annotations are missed.**
   ```python
   import mock
   from typing import cast
   cast("mock.mock.Y", 1)
   ```
   Ruff: UP026 lines 1 and 3. Translation: line 1 only. `List["mock.mock.Y"]` has the same gap.

2. **Type aliases and type-parameter bounds/defaults use the wrong deferred order.**
   ```python
   type X = mock.mock.Y
   import mock
   ```
   Ruff: UP026 lines 1 and 2. Translation: line 2 only. `def f[T: mock.mock.Y]()` and `class A[T = mock.mock.Y]` behave likewise.

3. **Match-pattern attributes are missed.**
   ```python
   import mock
   match x:
       case mock.mock.Y: pass
   ```
   Ruff: UP026 lines 1 and 3. Translation: line 1 only. Class and mapping patterns have the same gap.

4. **Stub class bases use the wrong deferred order.** Counterexample in a `.pyi` file:
   ```python
   class A(mock.mock.B): ...
   import mock
   ```
   Ruff: UP026 lines 1 and 2. Translation: line 2 only.

## `ruff-up028-yield-in-for-loop`: defects

1. **Exception-target unbinding does not restart the reference chain.**
   ```python
   def f(y):
       for x in y:
           yield x
       print(x)
       try: pass
       except Exception as x: pass
       x = 1
   ```
   Ruff: UP028 line 2. Translation: no finding. `except*` behaves likewise.

2. **Nested tuple targets are missed.**
   ```python
   def f(items):
       for x, (y, z) in items:
           yield x, (y, z)
   ```
   Ruff: UP028 line 2. Translation: no finding.

3. **Parentheses that Ruff removes remain semantically significant.** Representative counterexamples:
   ```python
   def a(z):
       for a, b in z:
           yield (a), (b)

   def b(y):
       for (x) in y:
           (yield x)
   ```
   Ruff: UP028 lines 2 and 6. Translation: no findings.

4. **A parenthesized target can be mistaken for a one-tuple.**
   ```python
   def f(y):
       for (x) in y:
           yield x,
   ```
   Ruff: no finding. Translation: UP028 line 2.

5. **Duplicate target names are treated as safe.**
   ```python
   def f(z):
       for x, x in z:
           yield x, x
   ```
   Ruff: no finding. Translation: UP028 line 2.

6. **Several real reads are absent from `read_elsewhere`.**
   ```python
   def f(y):
       for x in y:
           yield x
       x += 1
   ```
   Ruff: no finding. Translation: UP028 line 2. A nested `nonlocal x` declaration and a pre-loop bare annotation whose binding carries a read produce the same false positive.

7. **Global/nonlocal targets are resolved beyond Ruff's current scope.**
   ```python
   def f(y):
       global x
       for x in y:
           yield x

   def g():
       return x
   ```
   Ruff: UP028 line 3. Translation: no finding. Nonlocal targets and a nested function that declares `global x` have analogous mismatches.

8. **Conditional-delete detection crosses a nested function boundary.**
   ```python
   try:
       pass
   except E:
       def f(y):
           for x in y:
               yield x
           print(x)
           del x
           x = 1
   ```
   Ruff: UP028 line 5. Translation: no finding.

9. **The iterable's read is attributed to the loop binding.**
   ```python
   def f():
       for x in x:
           yield x
   ```
   Ruff: UP028 line 2. Translation: no finding.

10. **The translation requires a function scope.**
    ```python
    for x in y:
        yield x
    ```
    Ruff: UP028 line 1. Translation: no finding. The source is invalid at module/class scope, so this is low priority.

## `ruff-up029-unnecessary-builtin-import`: defects

1. **Global writes in another function are counted as module bindings.**
   ```python
   def f():
       global str
       str = 1
   from builtins import str
   ```
   Ruff: UP029 line 4. Translation: no finding.

2. **Nested nonlocal writes are counted as bindings in the import's function.**
   ```python
   def f():
       def g():
           nonlocal str
           str = 2
       from builtins import str
   ```
   Ruff: UP029 line 5. Translation: no finding.

3. **A module-level `global` suppresses the rule.**
   ```python
   global str
   from builtins import str
   ```
   Ruff: UP029 line 2. Translation: no finding.

4. **Bare function annotations are not bindings.**
   ```python
   def f():
       str: type
       from builtins import str
   ```
   Ruff: no finding. Translation: UP029 line 3.

5. **Deletion is treated as restoring the builtin.**
   ```python
   try:
       del str
   except NameError:
       pass
   from builtins import str
   ```
   Ruff: no finding. Translation: UP029 line 5.

6. **Whitespace in dotted module names prevents recognition.**
   ```python
   from six . moves import zip
   from six.moves  .builtins import str
   ```
   Ruff: UP029 lines 1 and 2. Translation: no findings.

## `ruff-up030-format-literals`: defects

1. **Unicode-name escape braces are parsed as format fields.**
   ```python
   r"\N{x} {0}".format(a)
   ```
   Ruff: UP030 line 1. Translation: no finding. Conversely, `"\\N{0}".format(a)` produces no Ruff finding but UP030 in the translation.

2. **Only `!r`, `!s`, and `!a` conversions are accepted.**
   ```python
   "{0!x}".format(a)
   ```
   Ruff: UP030 line 1. Translation: no finding.

3. **A `]` in an attribute part is rejected.**
   ```python
   "{0.a]}".format(a)
   ```
   Ruff: UP030 line 1. Translation: no finding.

4. **Indices larger than `usize` are still parsed as indices.**
   ```python
   "{0}{99999999999999999999999}".format(a)
   ```
   Ruff: no finding. Translation: UP030 line 1.

5. **Implicitly concatenated string annotations are missed.**
   ```python
   t: "'{0}' '{1}'.format(a, b)"
   ```
   Ruff: UP030 line 1. Translation: no finding.

## `ruff-up031-printf-string-formatting`: defects

1. **`%b` is accepted in a `str` format.**
   ```python
   "%b" % x
   ```
   Ruff: no finding. Translation: fixable UP031 line 1.

2. **Mapping-key parentheses are not balanced.**
   ```python
   "%(a(b))s" % x
   ```
   Ruff: fixable UP031 line 1. Translation: no finding. The unclosed `"%(a(b)s" % x` produces the inverse mismatch.

3. **Width and precision overflow are accepted.**
   ```python
   "%2147483648f" % x
   ```
   Ruff: no finding. Translation: fixable UP031 line 1.

4. **Some multi-line dictionaries are classified as fixable.**
   ```python
   x = (
       "%(a)s"
   ) % {"a": [
       1]}
   ```
   Ruff: manual UP031 line 2. Translation: fixable UP031 line 1.

5. **A Python 3.14 t-string right operand is accepted.**
   ```python
   "%s" % t"x"
   ```
   Ruff: no finding. Translation: fixable UP031 line 1.

6. **Implicitly concatenated string annotations are missed.**
   ```python
   z: "'%s' '%s' % (a, b)"
   ```
   Ruff: fixable UP031 line 1. Translation: no finding.

## `ruff-up032-f-string`: defects

1. **Escaped braces make `split_field` reject valid conversions.**
   ```python
   "{{ {}".format(a)
   ```
   Ruff: UP032 line 1. Translation: no finding.

2. **Hex-escaped braces are decoded before field parsing.**
   ```python
   "\x7b}".format(a)
   ```
   Ruff: no finding. Translation: UP032 line 1.

3. **Walrus expressions are not effectful.**
   ```python
   "{x} {x}".format(x=(y := 1))
   ```
   Ruff: no finding. Translation: UP032 line 1.

4. **Invalid/keyword attribute parts are accepted.**
   ```python
   "{0.class}".format(a)
   ```
   Ruff: no finding. Translation: UP032 line 1.

5. **Unsafe subscript keys are accepted.**
   ```python
   "{0[it's]}".format(a)
   ```
   Ruff: no finding. Translation: UP032 line 1. A backslash in the key behaves likewise.

6. **A multi-line bare generator argument is rejected.**
   ```python
   "{}".format(
       x for x in y
   )
   ```
   Ruff: UP032 line 1. Translation: no finding.

7. **Concatenated/escaped annotation strings use the wrong inner ranges.**
   ```python
   z: "'{}'" ".format(a)"
   ```
   Ruff: no finding. Translation: UP032 line 1.

8. **Type-expression strings outside annotations are missed.**
   ```python
   from typing import cast
   cast("'{}'.format(a)", q)
   ```
   Ruff: UP032 line 2. Translation: no finding. `TypeVar`, `NewType`, and `TypeAlias` forms have the same gap.

9. **`Annotated` metadata is parsed as a type expression.**
   ```python
   from typing import Annotated
   x: Annotated[int, "'{}'.format(a)"]
   ```
   Ruff: no finding. Translation: UP032 line 2.

10. **Cross-file re-exports are followed.** `mymod.py`:
    ```python
    from django.utils.translation import gettext
    ```
    `use.py`:
    ```python
    from mymod import gettext
    gettext("{}".format(a))
    ```
    Ruff: UP032 in `use.py` line 2. Translation: no finding.

## `ruff-up033-lru-cache-with-maxsize-none`: defects

1. **Class decorators are reported.**
   ```python
   import functools

   @functools.lru_cache(maxsize=None)
   class C: pass
   ```
   Ruff: no finding. Translation: UP033 line 3.

2. **Cross-file re-exports are followed.** `mymod.py`:
   ```python
   from functools import lru_cache
   ```
   `use.py`:
   ```python
   from mymod import lru_cache

   @lru_cache(maxsize=None)
   def f(): pass
   ```
   Ruff: no finding. Translation: UP033 in `use.py` line 3.

## Author result check

`python-12.result.json` has one `different` entry: `ruff-up032-f-string`, with `right: "translation"`. I checked its `pyflakes/F525.py`-style key using:

```python
"{} {1}".format(1, 2)
```

Ruff 0.17.6 reports no UP032 finding, and run-policies reports `diagnostics: []`. The stated right side is correct: mixed automatic and numbered fields are invalid for `str.format`, and the translation correctly leaves them alone.
