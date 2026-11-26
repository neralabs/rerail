# NOTES python-24: PT026, PT030

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select <CODE> --no-cache`). Checked with `ptcheck` against
the integration branch: PT030 `same`; PT026 `different` by one fixture key, where the translation follows Ruff.

## ruff-pt026-pytest-use-fixtures-without-parameters (different, right: translation)

No change. The one difference is `ruff/rerail/probes/PT026-bare-name-and-trailing-comment.py:23`,
`@pytest.mark.usefixtures()` on `class TestE: ...`. The original reports it, because its `inside` accepts a
`class_definition` as well as a `function_definition`. Ruff runs PT026 only on a def's decorators, so it reports
lines 6, 10, 18 and 27 of that file, not 23. The translation reports the same four lines, since it requires
`kind(f, "function_definition")`. Counterexample: `@pytest.mark.usefixtures()` / `class TestE: ...` gets no
finding from Ruff.

## ruff-pt030-pytest-warns-too-broad (same)

Baseline: 4 original-only on fixtures. Ruff agreed with the original on all four:

- `PT030-fstring-match-forms.py:11, 12, 21`: `match=f"{f''}"`, `match=f"{'' ''}"`,
  `match=f"{''}{None}{f'{None}'}"`. Ruff's `is_empty_or_null_string` recurses into every f-string field, and an
  implicit concatenation of `''` is an empty str. python/pytest's `pytest_empty_or_null` follows only one level and
  takes only a single `''` or `None` as a field. The rule now has its own recursive check (`full`, the complement,
  which keeps the recursion positive). Probes run through Ruff, and the translation agrees on all of them:
  `{'':{'a'}}` (format spec ignored), `{'' !r:{x}}`, `{f'{(f"{None}")}'}`, `{u''}`, `{r''}`, `{''''''}`, and
  `("" f"{('' '')}")` are empty. `{f'a'}`, `{'' 'a'}`, `{x}`, `{'\n'}` and `{f'{None}{1}'}` are not, and neither is
  `f"{{{''}}}"`, where `{{` is literal text.
- `RUF061-string-annotations.py:9`: `x: "pytest.warns(UserWarning, func)" = 1`. Ruff parses a string type
  definition and runs PT030 on the tree. The translation now checks calls in those trees (python-01's
  `typedef_node`, so `Literal[...]` strings and strings starting with a space are left out).

Ruff's behaviour, followed, beyond the corpus (probes on the box):

| Case | Ruff | Translation |
|---|---|---|
| `x: "pytest.warns(UserWarning)" "" = 1`, `"pytest.warns(\x55serWarning)"`, `"list['pytest.warns(UserWarning)' '']"` (a complex annotation, or a string nested in one) | not reported | not reported (`deep_complex`) |
| `r"…"`, `"""…"""`, `u"…"` annotations | reported | reported |
| `from os import *` then `pytest.warns(UserWarning)` | reported | reported (`miss`, not python/names' `builtin_ref`, which leaves out modules with a star import) |
| `from builtins import UserWarning as UW` then `pytest.warns(UW)` | not reported: the name shown is "builtins.UserWarning" | not reported |
| `z = cast("pytest.warns(DeprecationWarning)", 1)`, `T: TypeAlias = "pytest.warns(UserWarning)"` | reported | **not reported** (library gap 2) |

In python/pytest, `pytest_shown` strips "builtins." from every qualified name, so with the library's
`pytest_broad`, `pytest.warns(UW)` above would be reported. The rule uses its own `shown`. PT011
(`pytest_broad("raises", …)`) likely has the same divergence [inference, not run].

## Library changes proposed

1. python/pytest `pytest_empty_or_null`: recurse as Ruff does (this rule's `looked`/`piece`/`blank`/`full`), and
   read an empty str literal from its text (`blank`), since `value` has no rows in a parsed tree.
2. python/names `parsed_in` covers annotation strings only. Names in the other strings of python/typing's
   `type_ctx` (`cast`'s type, a `TypeAlias` value, a TypeVar bound, `NewType`'s base) do not resolve, so
   `exc_call` fails in them. Wanted: the same scope start (`wstart`) for every `typedef_node` tree.
3. python/names `builtin_ref`: the `!star(m)` guard does not match Ruff, which resolves a builtin even when the
   module has `from x import *`. It also has no rows in a parsed tree, because `docof` gives the fragment's
   document, which has no `modnode`. Wanted: take the module of the host document.
4. python/pytest `pytest_shown`: keep "builtins." for a name imported from `builtins`. Only a bare builtin
   reference drops it.

## Engine gaps (found here, worked around in the rule)

1. A negation (or a `count`) of a relation that reads no parse, written in a rule that does read one, holds on a node
   of a parsed tree even when the relation has the row there. With `broad_warn` reading `parsed_in`,
   `PT030(…) :- broad_warn(c, w), !arg(c, 1, _)` and `… arg(c, 1, _)` both held for
   `x: "pytest.warns(UserWarning, func)"`, and `count : { item(c, _, _) }` gave both 0 and 2. The rule moves each
   negation into a relation that reads no parse (`form`, `loose`) and joins it positively.
2. As in NOTES-python-05: `kind(l, "argument_list")` does not hold in a parsed tree, so python/exceptions'
   `exc_arg`/`exc_argument` have no rows there. The rule counts arguments itself (`item`, `arg`, `warning`).
   `value` has no rows there either (`blank` reads the text).
