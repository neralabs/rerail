# pt1120-python-05: D212, D300, D403, D406, D418, D419 against their originals

Checked with `ptcheck --ref tbelanger/pack-translations-1120` (engine `rerail-engine-1120`), ground truth ruff 0.17.6
(`ruff check --select <CODE> --no-cache`). All six are `same` on examples, fixtures and samples-python. Every
difference the baseline recorded was the translation's error.

New library: `lib/python/docstrings-ruff-reads.rerail`, `ruff_doc(s)`: a docstring Ruff reads. Its statement is no
one-string tuple, and its file has no tree-sitter ERROR node. Every rule here requires it.

| Rule | Baseline (fixtures) | Cause | Fix |
|---|---|---|---|
| ruff-d212-multi-line-summary-first-line | 3 original-only | the newline after the quotes was `\r?\n`; a lone `\r` breaks a line too | `[\r\n]` |
| ruff-d300-triple-single-quotes | 3 translation-only | `"x",` read as a docstring (2); a docstring in a file that does not parse (1) | `ruff_doc` |
| ruff-d403-first-word-uncapitalized | 1 translation-only | `"x",` read as a docstring | `ruff_doc` |
| ruff-d406-missing-new-line-after-section-name | 2 translation-only | Google docstrings read as NumPy, through the library's `tie` | the rule's own `numpy_doc` |
| ruff-d418-overload-with-docstring | 1 original-only | `from _typeshed import overload` not read as `overload` | `overload_dec` |
| ruff-d419-empty-docstring | 1 translation-only | a docstring in a file that does not parse | `ruff_doc` |

## Per rule

- **D212.** `probes/D200-bare-cr-line-endings.py:2`: `def f():\r    """\r    One.\r    """`. Ruff reports D212.
  The body starts with `\r`, and the old `\\r?\\n` did not match it.
- **D300, D403.** `probes/PYI058-tuple-expression-statements.py:19` (`"x",`) and
  `probes/RUF002-tuple-statement-is-not-docstring.py:2` (`"dоc",`). Ruff reports nothing: each is a tuple
  statement. python/ast's `doc_stmt` counts named children only, so it took the statement for a lone string.
  python/python-17's `tuple_pseudodoc` names the case, and `ruff_doc` excludes it.
- **D300, D419: files that do not parse.** Ruff reports only the syntax error in such a file. A valid `""" """` above an
  unterminated `"a` gives no D419 (ruff 0.17.6, probed on a box). The cases: `flake8_implicit_str_concat/ISC_syntax_error.py:2`
  (`"a" "b`) and `ISC_syntax_error_2.py:2` (`'' '`). tree-sitter parses the first string as a statement and the rest
  as an ERROR node. `ruff_doc` drops docstrings in a file with an ERROR node.
  - Not core's `syntax_error`, CPython's verdict and the closer match: a policy that reads it withholds every
    syntax-error file from every policy of the same `run-policies` run, not just its own. Two probe policies run
    together showed it: the one that did not read it lost those files too. A pack run would change every other
    Python rule's findings.
  - A file whose only error tree-sitter recovers as a MISSING token is still read. No corpus case.
- **D406.** `pydocstyle/D215.py:2` (`TODO:` over `-`, a module docstring) and
  `probes/D407-style-by-section-counts.py:16` (4 Google sections, 3 NumPy). Ruff reports nothing: each docstring is
  Google, and D406 checks NumPy only.
  - The library's `doc_style` gave both "google" and "numpy". Its `tie(s)` writes `g = count : {google}, g = count :
    {numpy}, g > 0`, and the engine does not hold the second count to the first. A probe policy on the box showed
    `tie` for D215.py with counts 1 and 0, so every docstring with a Google section was a tie, and a tie with no hint
    is NumPy.
  - The rule now states the convention itself (`numpy_doc`): the option, else NumPy parameters, else no Google args
    and NumPy sections ≥ Google ones, a tie going to NumPy unless the library's `tie_by` says Google.
  - It reads the sections as NumPy ones (`doc_sec(s, "numpy", i)`, `head_line`), as `doc_section` would with the
    right convention.
- **D418.** `probes/D418-overload-import-forms.py:41`: `from _typeshed import overload as ov3`, then `@ov3`. Ruff
  reports D418. `typing_dec` reads `typing` and `typing_extensions` only. `overload_dec` adds `_typeshed.overload`.
  python/python-17, included through the new library, gives `typing_dec` its parenthesised decorators, as the JSON
  and Ruff read them.

## Library changes proposed (not made: existing files)

- `python/docstrings-sections.rerail`, `tie`: `g = count : { doc_sec(s, "google", _) }, n = count : { doc_sec(s,
  "numpy", _) }, g = n, g > 0`. Every rule that reads `doc_style` or `doc_section` (D405-D417 and others) reads Google
  docstrings as NumPy too. D406's `numpy_doc` is then `doc_style(s, "numpy")` again.
- `python/syntax.rerail`, `doc_stmt`: refuse a statement whose text ends in `,` (python-17's
  `tuple_pseudodoc`), so `docstring` itself is Ruff's. `ruff_doc`'s first test then goes.
- `python/naming.rerail`, `typing_dec`: `_typeshed` with `typing` and `typing_extensions`, as Ruff's typing members.

## Engine gaps

- `x = count : {…}` with x already bound does not test the count against x (the `tie` case above). It should be
  refused, or read as a comparison.
- Reading core's `syntax_error` in one policy withholds the syntax-error files from every policy of the run. Per
  policy, D2xx-D4xx could read it (`!syntax_error(d, _, _)`) and drop the ERROR-node test.
