# pt1120-python-05 translation review

Ground truth: Ruff 0.17.6 (`ruff check --isolated --no-cache --select D212,D300,D403,D406,D418,D419`). I read the
rules at the 0.17.6 tag (`pydocstyle/rules/{multi_line_summary_start,triple_quotes,capitalized,sections,if_needed,
not_empty}.rs`, `pydocstyle/helpers.rs` (`get_section_contexts`, `find_underline`), `docstrings/{extraction,
sections}.rs`, `checkers/ast/analyze/definitions.rs`, `ruff_python_ast` `str.rs` (`is_triple_quote`) and
`docstrings.rs`, `ruff_python_semantic` `model.rs` (`resolve_qualified_name`, `match_typing_qualified_name`)), then ran
my own cases with Ruff and `rerail-engine-1120 run-policies` on the six `docs/rule-syntax/packs/python/<id>.rerail`
at `be9cd34cd5`, with that commit's `docs/rule-syntax` and `native/src/**/*.rerail` as the engine's root. The runs
are on pt-w02 (`/tmp/rv05`): pt-w01, pt-w07 and ts-prisma-b have no `rerail-engine-1120`. Translation lines are the
finding's line. No case uses a suppression comment.

163 files, all six rules on each: prefixes, `\r` / `\r\n` / form-feed bodies, parentheses, implicit concatenation,
one-string tuples, comments before the docstring, bytes and f-strings, stubs, escapes in the body, Unicode whitespace
(U+00A0, U+0085, U+2003, U+2028, U+3000 against U+001C, U+200B, U+FEFF) for D403 and D419, D403's word shapes
(`iOS`, `e.g.`, `don't`, `what?!`, `x_1`, `wo.rd`, non-ASCII letters), 19 bindings of `overload` for D418, 43 section
layouts for D406 (ties, sub-sections, directives, underlines, lowercase names, with `\r` and `\r\n` copies of 31 of
them), and 116 files of other syntax below an empty docstring. Apart from the defects below, every finding agreed.

Not run: `pydocstyle.convention`. `run-policies` gives a policy no options, so D406's `option(...)` clauses are
untested; read against `get_section_contexts` and `sections` they look right. Ruff's `lint.typing-modules` (more
modules whose `overload` counts for D418) is not read by the translation and cannot be passed either.

## All six rules: the syntax-error gate (`python/pt1120-python-05`'s `py05_unparsed`)

The batch withholds a file when tree-sitter leaves an ERROR node in it, standing in for Ruff, which reports no lint
finding in a file it cannot parse. The two disagree both ways.

### Ruff cannot parse the file, tree-sitter gives no ERROR: the translation reports, Ruff does not

Counterexample `py2_print.py` (the same body on every file below):

```python
from typing import overload
print "hello"


def empty():
    """ """


def first_line():
    """
    Summary.
    """


def quotes():
    'Summary.'


def lower():
    """return the total."""


@overload
def over(x: int) -> int:
    """Doc."""


def sections():
    """Sum.

    Returns:
    -------
    int
    """
```

```
ruff:        py2_print.py:2:7: invalid-syntax: Simple statements must be separated by newlines or semicolons
translation: py2_print.py:6 D419, :10 D212, :16 D300, :20 D403, :24 D418, :31 D406
```

With `print("hello")` on line 2 (`valid_print.py`) both give those six findings, so the gate is the difference.
`default_order.py` (line 2 `def g(a=1, b):` and `    pass`) is the same: Ruff `2:12: invalid-syntax: Parameter
without a default cannot follow a parameter with a default`, the translation the six findings.

Each of these statements, put below an empty docstring (`def f():` / `    """ """`), makes Ruff report only the syntax
error and the translation still report D419:

| Statement | Ruff |
|---|---|
| `print "hello"`, `exec "x = 1"`, `x = 10L`, `x = ur"abc"` | Simple statements must be separated by newlines or semicolons |
| ``x = `1` `` | Got unexpected token `` ` `` |
| `x = 1 <> 2` | Expected an expression |
| `x = 0777` | Invalid decimal integer literal |
| `async = 1` | Expected `def`, `with` or `for` to follow `async`, found `=` |
| `a, b += 1` | Expected `,`, found `+=` |
| `def g(*):` | Expected one or more keyword parameter after `*` separator |
| `def g(a=1, b):` | Parameter without a default cannot follow a parameter with a default |
| `del g()` | Invalid delete target |
| `g(x for x in y, 1)` | Unparenthesized generator expression cannot be used here |
| `g(**k, *a)` | Iterable argument unpacking cannot follow keyword argument unpacking |
| `x := 1` | Expected a statement |
| a tab-indented line under a space-indented one | unindent does not match any outer indentation level |
| `x = 1 \` at the end of the file | unexpected EOF while parsing |

Python 2 files are the likely real case. (Semantic errors such as `def g(a, a)`, `nonlocal x` at module level or
`*a = [1]` do not stop Ruff's lint; both sides report there, as they should.)

### Ruff parses the file, tree-sitter gives an ERROR: Ruff reports, the translation does not

`lazy_import.py`: line 2 `lazy import json` (PEP 810), the same body:

```
ruff:        lazy_import.py:6:5 D419, 10:5 D212, 16:5 D300, 20:5 D403, 24:5 D418, 31:5 D406
translation: (none)
```

`dict_unpack_comp.py`: line 2 `x = {**d for d in [{}]}` (PEP 798's dict form): the same six from Ruff, none from the
translation. `lazy from os import path` is the same. PEP 798's list, set and generator forms (`[*a for a in b]`)
parse in tree-sitter, and both sides report there.

## `ruff-d212-multi-line-summary-first-line`: the syntax gate only

Otherwise agrees: `r"""`, `u"""` and `'''` openers (Ruff's `is_triple_quote` takes every prefix), a lone `\r` and
`\r\n` after the opener, `"""` followed by spaces or a backslash (not reported), parentheses, blank bodies (D419 only).

## `ruff-d300-triple-single-quotes`: the syntax gate only

Otherwise agrees: `"…"`, `'…'`, `'''…'''`, `R'''…'''`, `U"…"`, text with `"""` (wants `'''`), text with both
(`'''Says """ and \''' here.'''`, not reported), `"Says \"\"\" here."` (wants `"""`), empty `''` (D419 only).

## `ruff-d403-first-word-uncapitalized`: the syntax gate only

Otherwise agrees, on methods, nested and async functions, not classes or modules, and on the word shapes above,
including Unicode whitespace after the word (`return\u00a0x.` and `return\u0085x.` reported, `return\u001cx.` not).

## `ruff-d406-missing-new-line-after-section-name`: the syntax gate only

Otherwise agrees on every layout I tried with the convention unset, including the tie-breaks the rule rewrites
(`numpy_doc`): a first Google section with an underline, with a colon, with neither and a later one with a colon,
counts that differ through Google-only (`Note`, `Example`, `Todo`) and NumPy-only (`Receives`, `Short Summary`,
`Other Params`) names.

## `ruff-d418-overload-with-docstring`: defects

1. The syntax gate (above).
2. A binding of `overload` (or of `typing`) in the same scope between the import and the decorator is not seen.
   Ruff reads the decorator's name as resolved where it is used (`resolve_qualified_name`, the `resolve_name` the
   visit recorded) and finds the later binding, which is not `typing.overload`; the translation's `typing_dec` /
   `sym_of` still answers `typing.overload`.

   Counterexample `d418_shadow.py`:

   ```python
   from typing import overload


   def overload(fn):
       return fn


   @overload
   def f(x: int) -> int:
       """Doc."""
   ```

   ```
   ruff:        All checks passed!
   translation: d418_shadow.py:9 D418
   ```

   Counterexample `d418_rebind.py`:

   ```python
   import typing
   from typing import overload

   overload = lambda fn: fn
   typing = None


   @overload
   def f(x: int) -> int:
       """Doc."""


   @typing.overload
   def g(x: int) -> int:
       """Doc."""
   ```

   ```
   ruff:        All checks passed!
   translation: d418_rebind.py:9 D418, d418_rebind.py:14 D418
   ```

   The same holds (Ruff nothing, the translation D418 at the def) when the binding between them is `class
   overload:`, `for overload in []:`, `with open("x") as overload:`, `if (overload := None):`, `del overload`, or
   `from functools import wraps as overload`; and when `def overload(fn)` comes first and `from typing import
   overload` only after the decorated def (Ruff resolved the name to the def during the visit). A binding in another
   scope is handled: a shadowing def or parameter in an enclosing function, or a class attribute, gives nothing on
   either side. A binding after the decorated def, `except … as overload:` before it, and the import only after the
   def (no binding during the visit; Ruff then looks the name up at the end) are reported by both.

Otherwise agrees: `typing`, `typing_extensions`, `_typeshed` (from-import and attribute), aliases (`import typing as
t`, `as ov_ext`), `@(overload)`, `@overload()`, methods, stacked decorators, `if TYPE_CHECKING:` imports, an import
inside the enclosing function, `.pyi` stubs (not reported), and no D418 for an empty or blank docstring, a bytes
"docstring", a one-string tuple, or an implicitly concatenated one.

## `ruff-d419-empty-docstring`: the syntax gate only

Otherwise agrees: `""`, `''`, `""""""`, `r""" """`, `u'  '`, parentheses, a blank multi-line body, one-line bodies
(`def i(): ""`, `class K: """ """`), Unicode whitespace (U+00A0, U+0085, U+2003 + U+2028, U+3000, `\x0b\x0c`
reported; U+001C, U+200B, U+FEFF not), `"""\n"""` with an escaped newline (not empty), and nothing for `"" ""`,
`"",`, `f""" """` or `b""" """`.

## The author's result file

`pt1120-python-05.result.json` lists all six as `same`, with no `different` entry, so there is no `right` side to
check. `ptcheck --ref tbelanger/pt1120-python-05` at `be9cd34cd5` agrees: 6 same, 0 different (examples, fixtures
1963 files, samples-python 2313 files). Those compare against the JSON originals, not Ruff, and no corpus file has
any of the cases above.

The NOTES name one gap in the gate: a file whose only tree-sitter error is a MISSING token. The forms above go
further: tree-sitter reads much of Python 2 and several invalid Python 3 statements without an ERROR or MISSING node,
and gives an ERROR on some valid 3.15 syntax. The NOTES' proposed engine fix (each policy reading core's
`syntax_error` for its own files) would follow CPython's verdict instead of tree-sitter's, which agrees with Ruff on
every Ruff-rejected form in the table [INFERENCE: not run]. On `lazy import` and `{**d for d in …}` it agrees only
if that CPython is 3.15.
