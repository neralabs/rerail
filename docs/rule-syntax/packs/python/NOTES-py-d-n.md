# py-d-n: 39 Ruff rules (pydocstyle D, pep8-naming N), hand-written

Every rule follows Ruff 0.16.9, read from its source at the commit the JSON was written from
(`wolfeel/.rerail/port-readiness/upstream/ruff/0be08a2…/crates/ruff_linter`): the rule function, the place in
`checkers/ast` that calls it (N806 and N815 run only when Ruff's current scope is a function or a class), and the
helpers it uses (`pydocstyle::helpers`, `pep8_naming::helpers`, `ruff_python_semantic::analyze::function_type`,
`class::is_metaclass`, `visibility`). Codes, messages, options and suppressions are the JSON's; the revision is 2.

The rules share three new libraries:

| file | lines | what |
|---|---|---|
| `lib/python/naming.rr` | 180 | case classes (lower, upper, camel, mixed, acronym, dunder); `pep8-naming.ignore-names` globs, with Ruff's defaults and `extend-ignore-names`; decorators with a call taken off (`dec_target`) and typing decorators (`typing_dec`); class/static methods by qualified name and by the `classmethod-decorators` / `staticmethod-decorators` options; Ruff's `function_type::classify` with `is_metaclass` (yes / maybe / no); first parameter; Store-context names and their scope; assignments that make a type; import aliases |
| `lib/python/docstrings.rr` | 170 | includes `naming.rr`. Ruff's definitions (MemberKind) and their visibility (the package path, `_` names, `__all__` bindings, setters and deleters); `pydocstyle.ignore-decorators`; `doc_missing`, the shared condition of D100-D107; the docstring (first statement, a plain str literal; one string for the D2xx-D4xx rules; a non-blank body); its body, prefix, quotes and summary line; its lines, blank lines, leading whitespace, source spans and indentation |
| `lib/python/docstrings-sections.rr` | 144 | section headers in the Google and NumPy conventions: Ruff's section names, header detection (leading words, `:` suffix, underline, end of paragraph, RST directive bodies skipped, the previous-section subsection tests, resolved exactly without recursion), the convention (the option, set once, or Ruff's per-docstring guess), section extents, the first content line, dashed underlines |

The 39 rules have 771 lines, comments and directives included: 15 to 50 lines each. `python3 site/check_rr.py`
prints `ok`.

## Scores

`rrlab score python <id> --all` cannot score these rules: none is in `python/conformance.json`, which leaves
rrlab with "cannot tell which upstream tool to score against". The recording also has no D or N findings (only
N999). So the scores below come from Ruff 0.16.9's own fixtures (`resources/test/fixtures/pydocstyle`,
`pep8_naming`) against its snapshots. They are matched at (path, line, code), with the settings each test in
`rules/<plugin>/mod.rs` uses: `ignore-decorators = ["functools.wraps"]` for pydocstyle, the `ignore_names` and
`classmethod_decorators` / `staticmethod_decorators` tests' options, and the D104 `__init__.py` fixture path.
They were run with `rrlab run`. Total: 265 TP, 0 FP, 0 FN.

A second check ran every rule, with no convention and with each one, on hand-written files and on a reviewer's
17 case directories, against the installed ruff 0.15.7. The files cover package paths with and without
`__init__.py`, `__all__` forms, walrus and match-capture stores, comprehensions, call bases and metaclasses,
Django model imports, `if`-nested `visit_` methods, form feeds and CRLF, RST directives and chains of nested
headers. They agree everywhere except V1, A1 and U1 below.

| rule | lines | snapshots TP / FP / FN | notes |
|---|---|---|---|
| ruff-d101-undocumented-public-class | 18 | 2 / 0 / 0 | P1 |
| ruff-d102-undocumented-public-method | 20 | 6 / 0 / 0 | P1 |
| ruff-d103-undocumented-public-function | 18 | 2 / 0 / 0 | P1 |
| ruff-d104-undocumented-public-package | 16 | 1 / 0 / 0 | P1 |
| ruff-d105-undocumented-magic-method | 18 | 1 / 0 / 0 | P1 |
| ruff-d106-undocumented-public-nested-class | 18 | 1 / 0 / 0 | P1 |
| ruff-d107-undocumented-public-init | 17 | 2 / 0 / 0 | P1 |
| ruff-d200-unnecessary-multiline-docstring | 18 | 9 / 0 / 0 |  |
| ruff-d205-missing-blank-line-after-summary | 20 | 2 / 0 / 0 |  |
| ruff-d209-new-line-after-last-paragraph | 18 | 2 / 0 / 0 |  |
| ruff-d212-multi-line-summary-first-line | 18 | 3 / 0 / 0 |  |
| ruff-d215-overindented-section-underline | 21 | 3 / 0 / 0 | V1, C1 |
| ruff-d300-triple-single-quotes | 18 | 12 / 0 / 0 |  |
| ruff-d301-escape-sequence-in-docstring | 19 | 5 / 0 / 0 |  |
| ruff-d402-signature-in-docstring | 18 | 3 / 0 / 0 |  |
| ruff-d403-first-word-uncapitalized | 18 | 12 / 0 / 0 |  |
| ruff-d404-docstring-starts-with-this | 18 | 4 / 0 / 0 | U1 |
| ruff-d406-missing-new-line-after-section-name | 17 | 3 / 0 / 0 | C1, V1 |
| ruff-d408-missing-section-underline-after-name | 17 | 1 / 0 / 0 | C1 |
| ruff-d409-mismatched-section-underline-length | 17 | 3 / 0 / 0 | C1 |
| ruff-d411-no-blank-line-before-section | 20 | 3 / 0 / 0 | C1 |
| ruff-d412-blank-lines-between-header-and-content | 24 | 5 / 0 / 0 | C1 |
| ruff-d418-overload-with-docstring | 17 | 3 / 0 / 0 |  |
| ruff-d419-empty-docstring | 15 | 3 / 0 / 0 |  |
| ruff-n801-invalid-class-name | 19 | 9 / 0 / 0 | A1 |
| ruff-n802-invalid-function-name | 30 | 11 / 0 / 0 | A1 |
| ruff-n803-invalid-argument-name | 20 | 7 / 0 / 0 | A1 |
| ruff-n804-invalid-first-argument-name-for-class-method | 21 | 16 / 0 / 0 |  |
| ruff-n805-invalid-first-argument-name-for-method | 21 | 60 / 0 / 0 |  |
| ruff-n806-non-lowercase-variable-in-function | 50 | 13 / 0 / 0 | C2, W1 |
| ruff-n807-dunder-function-name | 20 | 4 / 0 / 0 |  |
| ruff-n811-constant-imported-as-non-constant | 18 | 7 / 0 / 0 | A1, Q1 |
| ruff-n812-lowercase-imported-as-non-lowercase | 16 | 5 / 0 / 0 | A1, Q1 |
| ruff-n813-camelcase-imported-as-lowercase | 15 | 6 / 0 / 0 | A1, Q1 |
| ruff-n814-camelcase-imported-as-constant | 18 | 6 / 0 / 0 | A1, Q1 |
| ruff-n815-mixed-case-variable-in-class-scope | 22 | 12 / 0 / 0 | A1, C2 |
| ruff-n816-mixed-case-variable-in-global-scope | 18 | 9 / 0 / 0 | A1, C2 |
| ruff-n817-camelcase-imported-as-acronym | 25 | 5 / 0 / 0 | A1, Q1, E1 |
| ruff-n818-error-suffix-on-exception-name | 20 | 4 / 0 / 0 |  |

Columns: the section rules (C1) report on the source line of the header, from column 1. Ruff reports at the
header's name. Where two N806/N815/N816 findings share a line, the columns differ (C2). Everything else matches
at the column too.

## Deviations

- **A1, ASCII case.** The case classes test `A-Z` and `a-z` only. Ruff's case helpers fall back to Unicode
  `is_uppercase` / `is_lowercase` on non-ASCII text: `class Ééé` is N801 here and not in Ruff. Exact Unicode
  classes need `\p{Lu}`-style regexes, which rrlab does not have yet (R4).
- **U1, D404 in rrlab.** D404 strips `[^\p{Alphabetic}\p{N}]` from the first word, Ruff's
  `normalize_word`, exactly. rrlab's regexes are `std::regex` (ECMAScript), which has no `\p{…}`, so in rrlab
  the class strips nothing and `"""(This) is."""` is missed. The snapshots have no such case. It is right once
  rrlab's regex functors are a Unicode engine (R4).
- **C1, section columns.** `doc_at(s, i, sp)` is the `src_line` span of body line i, so it starts at column 1.
  Keys are (path, line), which still match. Reporting at the name needs a span that starts at an offset into a
  line. See R2.
- **C2, same-line columns.** When two N806/N815/N816 findings share a line, both rows match at (path, line),
  but their columns differ from Ruff's.
- **P1, module privacy.** Ruff's package path: the directories up from the file while each holds an
  `__init__.py`, read from `file(...)` rows. A directory above the scanned root (rrlab: the files' common
  directory) is not seen, so a package that starts above it is read as starting at the root.
- **V1, 0.15.7 vs 0.16.9.** 0.16.9 skips headers inside RST directive bodies (`.. note::` followed by
  more-indented lines). 0.15.7 does not, so it reports D215 / D406 inside a `.. note::` that these rules, like
  0.16.9, do not. The rules follow 0.16.9.
- **W1, walrus targets.** rrlab gives a walrus target (`(W := 2)`) no `expr_ctx` row. `store_name` therefore
  adds `named_expression` names directly. A walrus inside a comprehension is in the comprehension's scope, as
  Ruff's current scope is, so N806 skips it, as Ruff does.
- **Q1, alias noqa.** Ruff reports N811-N817 at the alias, and a `# noqa` on the import statement's first line
  also suppresses it. The parent-statement noqa is not modelled. The finding sits on the alias's own line.
- **E1, import conventions.** N817 skips `import xml.etree.ElementTree as ET`, Ruff's default
  `flake8-import-conventions` alias. Other configured conventions are not an option here.
- **Metaclass ancestry.** `is_metaclass` marks a class "maybe" when a call is a base anywhere in its
  `any_base_class` walk. Here only the class's own bases are checked for calls. Metaclass bases are followed
  through `subclass` as before.
- `col` counts bytes, which is what Ruff's indentation comparison wants. `@slice` counts characters. The D
  rules use regexes and `strlen` only, so nothing mixes the two.
- `pydocstyle.property-decorators` matters only to D401 and D421, which are not in this batch. It is not read.
- Typing helpers (`@overload`, `@override`, `TypeVar`, `NamedTuple`) are resolved by qualified name through
  `sym_of`. Ruff also gates some of them on `seen_typing()`. That gate changes nothing when the name resolves.

## Duplicates to merge

- `typing_dec(f, x)` (`naming.rr`, now used by both D and N rules) is close to `typing_sym(e, x)` in
  `lib/python/typing.rr` (another batch), applied to a decorator.
- `doc_expr(o, e)` (`docstrings.rr`) and `py_docstring(o, e)` (`typing.rr`) both find the first-statement
  docstring. `doc_expr` also rejects bytes, f-strings and mixed concatenations, as Ruff's pydocstyle does.
- `doc_stub(d)` (`docstrings.rr`) and `stub_file(d)` (`typing.rr`) are the same.
- `doc_exports` / `doc_listed` (`docstrings.rr`) read `__all__` bindings only. pyflakes's `all_entry` also
  counts `.extend` / `.append`, which F822 wants and visibility does not. A shared `all_entry(s, x, how)` with the
  form would serve both.

## Change requests

- **R1, rrlab:** give walrus targets `expr_ctx(n, "store")`, as `ast.NamedExpr.target` has. Then `store_name`
  can drop its second rule.
- **R2, core:** a way to report on a sub-span of a line, or a span from a node plus an offset, so section rules
  can point at the header name rather than column 1.
- **R3, rrlab:** `str_part` and `match_all` have no rows in rrlab. These rules avoid both.
- **R4, rrlab:** Unicode property classes (`\p{Alphabetic}`, `\p{N}`, `\p{Lu}`) in the regex functors, as
  the Rust engine has. D404 needs them now (U1). A1 could then be made exact.
