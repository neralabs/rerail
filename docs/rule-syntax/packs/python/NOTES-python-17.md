# NOTES python-17: D101, D102, D103, D105, D107, D200, D205, D209

Ground truth: Ruff 0.17.6 on a fleet box (`ruff check --select <CODE> --no-cache`). The comparison gate is `ptcheck` against
the Python examples, recorded fixtures and samples-python corpus.

New library: `lib/python/python-17.rerail`, shared by D101, D102, D103, D105 and D107. It extends the existing
interfaces without editing them: `all_entry` accepts bare tuples, set arguments and parenthesized entries;
`typing_dec` and `doc_accessor` see transparent parentheses; and `tuple_pseudodoc` identifies a one-string
tuple statement that `python/ast` currently accepts as a docstring.

## Per rule

- **D101** (different; right: translation). The shared `all_entry` fix restores the original and Ruff findings for
  classes exported by `__all__ = 'Foo', ('f')`, `__all__ += (('Zed')),` and
  `__all__ = tuple({'Foo', ('f')}) + list(['Zed'])`. The remaining two translation-only fixture keys follow Ruff:
  `ruff/pyupgrade/UP045_1.py:35` and `:43` each define an undocumented public `ServiceRefOrValue`; Ruff reports D101
  for both, while the JSON original does not.
- **D102** (same). Exported classes now make their public methods visible. Parentheses are transparent for exemptions:
  `@(override) async def i(...)` is not D102, nor are `@((x)).setter def x(...)` and `@(x.deleter) def x(...)`.
  Ruff does report the counterexamples `@y.setter def x(...)`, `@self.x.setter def x(...)` and `@x.getter def x(...)`.
- **D103** (different; right: translation). Exported functions in the bare/set `__all__` forms are now found, and a
  first statement such as `"dоc",` is a tuple rather than a docstring. The remaining translation-only keys follow
  Ruff. Counterexample: every public function in `ruff/flake8_bugbear/B012.py` lacks a docstring; Ruff reports D103
  (for example `def a()` at line 1), while the JSON original omits those keys.
- **D105** (same). The corrected bare-tuple `__all__` recognizes Pydantic's
  `__all__ = 'BaseModel', 'create_model'`, restoring its twelve undocumented magic methods. A one-string tuple is
  not documentation: `def __iter__(self): "x",` is D105, as Ruff reports.
- **D107** (same). Parenthesized qualified decorators now keep overrides exempt. Ruff reports a plain undocumented
  `def __init__`, but not `@(typing.override) def __init__`; the former translation reported both.
- **D200** (same). The rule now uses `doc_line`/`doc_nlines`, so LF, CRLF and bare CR have identical semantics.
  Counterexample: `def f():\r    """\r    One.\r    """` is D200 in Ruff.
- **D205** (same). The first non-blank `doc_line` is the summary and the next is the description; their index gap must
  contain exactly one blank line. Counterexample: `"""Summary.\r\r\r    Desc.\r    """` has two blank lines and
  Ruff reports D205.
- **D209** (same). A checked triple-quoted docstring is reported when its final body line and an earlier body line are
  both non-blank, using universal-newline `doc_line` facts. Counterexample: `"""Summary.\r\r    Detail"""` is D209.

## Library changes proposed

- `python/pyflakes.rerail`'s `all_entry` should include `expression_list` and `set`, and should apply `unparen` to each
  sequence child.
- `python/naming.rerail`'s `typing_dec` should resolve the unparenthesized decorator expression.
- `python/docstrings.rerail`'s `doc_accessor` should unparenthesize both the decorator and the receiver of
  `.setter`/`.deleter`.
- `python/ast.rerail`'s `doc_stmt` should reject a one-element bare tuple (`"text",`).

No engine primitive or new `.decl` is needed beyond these reusable library relations.
