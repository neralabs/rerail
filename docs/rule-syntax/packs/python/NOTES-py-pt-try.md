# NOTES: py-pt-try (Ruff flake8-pytest-style PT0xx and tryceratops TRY)

37 rules, hand-written from the rerail.policy/v1 JSON and Ruff 0.16.9's source (the commit the JSON was written
from). Two new libraries: `lib/python/exceptions.rr` (Ruff's view of a call, handlers, raises, logging calls) and
`lib/python/pytest.rr` (fixture and mark decorators, parametrize names, `with pytest.raises/warns` blocks, the
fixture-body walk, ComparableExpr). `python3 site/check_rr.py` reports nothing in these files.

## How the rules were scored

The pack's recording (`fixtures.expected.json`) has no flake8_pytest_style or tryceratops fixtures, and
`rrlab score python <id>` cannot score these rules: they are not in `packs/python/conformance.json`, and the python
pack has two upstream tools, so rrlab cannot tell which one to score against. The rules were scored instead on
Ruff 0.16.9's own fixtures (`crates/ruff_linter/resources/test/fixtures/{flake8_pytest_style,tryceratops}`)
against every non-preview snapshot of each rule, matched on (file, line) and on (file, line, column). A snapshot
taken under a setting runs with that setting through `rrlab run --options`. For example, PT006_csv runs with
`{"parametrize-names-type": "csv"}` and PT011_glob_prefix with `{"raises-require-match-for": ["pickle.*"]}`.

The reviewer's extra cases (PT006, PT007, PT014, PT023, PT026, PT027, PT028, TRY004, TRY300) also agree with the
installed Ruff 0.15.7 on line and column. So do ad-hoc cases for TRY201, PT001, PT003, PT011, PT012, PT015, PT016,
PT019, PT021 and PT022.

The scores use the committed rrlab, which now has the fix to its lowering of `has`.

## Options

The rules read Ruff's settings for these plugins, with Ruff's names and defaults:

| Option | Default | Rules |
|---|---|---|
| `fixture-parentheses` | false | PT001 |
| `mark-parentheses` | false | PT023 |
| `parametrize-names-type` | `tuple` (or `csv`, `list`) | PT006 |
| `parametrize-values-type` | `list` (or `tuple`) | PT007 |
| `parametrize-values-row-type` | `tuple` (or `list`) | PT007 |
| `raises-require-match-for` | BaseException, Exception, ValueError, OSError, IOError, EnvironmentError, socket.error | PT011 |
| `raises-extend-require-match-for` | empty | PT011 |
| `warns-require-match-for` | Warning, UserWarning, DeprecationWarning | PT030 |
| `warns-extend-require-match-for` | empty | PT030 |

The `*-require-match-for` lists are globs over the qualified name, written as Ruff prints it (`ValueError`,
`socket.error`). pytest.rr holds the defaults and the glob test (`pytest_broad`).

## Rules

| Rule | Lines | Variants | TP by line / by column | Deviations |
|---|---|---|---|---|
| ruff-pt001-pytest-fixture-incorrect-parentheses-style | 22 | 2 | 12 / 12 | none |
| ruff-pt002-pytest-fixture-positional-args | 17 | 1 | 2 / 2 | none |
| ruff-pt003-pytest-extraneous-scope-function | 19 | 1 | 8 / 8 | none |
| ruff-pt006-pytest-parametrize-names-wrong-type | 38 | 4 | 66 / 66 | none |
| ruff-pt007-pytest-parametrize-values-wrong-type | 37 | 5 | 48 / 52 | none |
| ruff-pt008-pytest-patch-with-lambda | 31 | 1 | 12 / 12 | none |
| ruff-pt009-pytest-unittest-assertion | 33 | 1 | 36 / 36 | none |
| ruff-pt010-pytest-raises-without-exception | 18 | 1 | 1 / 1 | none |
| ruff-pt011-pytest-raises-too-broad | 29 | 5 | 28 / 28 | a1 |
| ruff-pt012-pytest-raises-with-multiple-statements | 21 | 1 | 11 / 11 | none |
| ruff-pt013-pytest-incorrect-pytest-import | 22 | 1 | 3 / 3 | none |
| ruff-pt014-pytest-duplicate-parametrize-test-cases | 26 | 1 | 7 / 10 | a2 |
| ruff-pt015-pytest-assert-always-false | 45 | 1 | 17 / 17 | a3 |
| ruff-pt016-pytest-fail-without-message | 23 | 1 | 7 / 7 | a3 |
| ruff-pt017-pytest-assert-in-except | 19 | 1 | 2 / 2 | none |
| ruff-pt018-pytest-composite-assertion | 22 | 1 | 22 / 22 | none |
| ruff-pt019-pytest-fixture-param-without-value | 38 | 1 | 4 / 4 | none |
| ruff-pt020-pytest-deprecated-yield-fixture | 17 | 3 | 9 / 9 | none |
| ruff-pt021-pytest-fixture-finalizer-callback | 26 | 1 | 2 / 2 | a4 |
| ruff-pt022-pytest-useless-yield-fixture | 26 | 1 | 3 / 3 | none |
| ruff-pt023-pytest-incorrect-mark-parentheses-style | 19 | 2 | 13 / 13 | none |
| ruff-pt024-pytest-unnecessary-asyncio-mark-on-fixture | 17 | 1 | 4 / 4 | none |
| ruff-pt025-pytest-erroneous-use-fixtures-on-fixture | 17 | 1 | 2 / 2 | none |
| ruff-pt026-pytest-use-fixtures-without-parameters | 20 | 1 | 2 / 2 | a8 |
| ruff-pt027-pytest-unittest-raises-assertion | 27 | 2 | 17 / 17 | a9 |
| ruff-pt028-pytest-parameter-with-default-argument | 28 | 2 | 13 / 15 | none |
| ruff-pt030-pytest-warns-too-broad | 27 | 5 | 20 / 20 | a1 |
| ruff-pt031-pytest-warns-with-multiple-statements | 18 | 1 | 11 / 11 | none |
| ruff-try002-raise-vanilla-class | 20 | 1 | 4 / 4 | a1 |
| ruff-try003-raise-vanilla-args | 34 | 1 | 4 / 4 | a5 |
| ruff-try004-type-check-without-type-error | 57 | 1 | 36 / 36 | a1 |
| ruff-try201-verbose-raise | 20 | 1 | 3 / 3 | none |
| ruff-try203-useless-try-except | 32 | 1 | 12 / 12 | none |
| ruff-try300-try-consider-else | 57 | 1 | 1 / 1 | none |
| ruff-try301-raise-within-try | 31 | 1 | 6 / 6 | a6 |
| ruff-try400-error-instead-of-exception | 19 | 1 | 11 / 11 | a7 |
| ruff-try401-verbose-log-message | 28 | 1 | 20 / 22 | none |

There are 58 snapshot variants. All of them have 0 FP and 0 FN, by line and by column. The column TP can exceed
the line TP because two findings can share a line. The rules are 1000 lines in all. The libraries add 349
lines (pytest.rr 180, exceptions.rr 169).

Where the JSON and the tool differ, the rules follow Ruff's source. The messages keep the JSON's wording and fill
in its `$X` placeholders. Some codes had several clauses in the JSON: PT001, PT006, PT007, PT011, PT012, PT013,
PT016, PT020, PT023, PT030, PT031 and TRY201. Each of these is one violation: the finding key is the same, and the
message is picked per case.

## Approximations (no counterexample in the fixtures)

- **a1. Qualified names.** Names resolve through names.rr's `sym_of`. It gives "builtins.X" both for an unshadowed
  builtin and for an explicit `import builtins; builtins.X`. Ruff's QualifiedName prints these as "X" and
  "builtins.X". The default patterns (PT011, PT030) and TRY002/TRY004 match only the first. So
  `pytest.raises(builtins.ValueError)` is PT011 here but not in Ruff.
- **a2. ComparableExpr** (`pytest_same`). Two atoms match when they have the same value. Two other elements
  match when they have the same kinds, operators and children, compared in order. Parentheses, comments, spacing
  and quotes are ignored. An implicit concatenation is one string (`"ab"` is `"a" "b"`). A float compares by value
  (`1.0` is `1.00`). The kinds of string stay apart (`b"a"` and `f"a"` are not `"a"`). An implicit concatenation
  that mixes f-string and plain pieces was not checked. When lib/python/compare.rr lands, its `cmp_same` should
  replace `pytest_same` (see "Duplicates to merge").
- **a3. Empty f-strings.** An f-string is empty when it has no text and every field is a plain `''` (`f""`,
  `f"{''}"`). A nested f-string field, such as `f"{f''}"`, is not seen as empty.
- **a4. PT021.** The finding is at the last `request.addfinalizer` call the walk sees, by `earlier`. That is
  Ruff's visit order on the engine. In rrlab it is start order, and the two differ only for a call nested in a
  decorator or a default.
- **a5. TRY003.** "Whitespace" is the regex class `\s` and the escapes `\t \n \r \v \f`. Rust's
  `char::is_whitespace` also covers Unicode spaces.
- **a6. TRY301.** A raised class is compared with a handled one by dotted text, which is ComparableExpr on names
  and attributes. A handled type that is a call or a subscript never matches.
- **a7. TRY400/TRY401.** Ruff's `logger-objects` setting (default empty) is not an option here.
- **a8. PT026 on classes.** Ruff checks marks on a class only while PT023 is enabled. So under `--select PT026`
  alone, `@pytest.mark.usefixtures()` on a class is not reported, but under `--select PT` it is (checked with Ruff
  0.15.7). The rule follows `--select PT026` and reports functions only. A finding on a class would need the
  rule to know whether PT023 is selected.
- **a9. PT027 targets.** Ruff reads a with item's `as` target only when it is a plain name. A with item bound to
  an attribute, a tuple or a subscript (`as self.cm`, `as (a, b)`, `as cm[0]`) is never reported. The rule skips
  it too.

## Library and core changes

1. **names.rr `builtin`**: the builtin table lacks 32 of Python 3.13's exception classes. TRY004 needs four of
   them (MemoryError, ReferenceError, SyntaxError, SystemError: 4 FNs). exceptions.rr carries the list as
   `exc_more_builtin` and resolves a name from it when nothing in the file binds it. That fallback should become
   rows of `builtin`:
   `BaseExceptionGroup BlockingIOError BrokenPipeError BytesWarning ChildProcessError ConnectionAbortedError
   ConnectionRefusedError ConnectionResetError EncodingWarning ExceptionGroup FileExistsError FloatingPointError
   FutureWarning ImportWarning IndentationError InterruptedError IsADirectoryError MemoryError NotADirectoryError
   PendingDeprecationWarning ProcessLookupError PythonFinalizationError ReferenceError ResourceWarning
   StopAsyncIteration SyntaxError SyntaxWarning SystemError TabError UnboundLocalError UnicodeTranslateError
   UnicodeWarning`.
   Once they are there, drop `exc_more_builtin` and the second `exc_qual` rule.
2. **ast.rr `unparen`** takes a parenthesized expression's child 0. When a comment comes first, that child is the
   comment:
   ```
   assert (
       # comment
       a and b
   )
   ```
   This was PT018's one FN. The proposed rule:
   `unparen(e, m) :- kind(e, "parenthesized_expression"), child(e, i, _), !kind(i, "comment"), unparen(i, m).`
   exceptions.rr has the fixed form as `exc_unparen`. PT015 and PT018 use it. Every user of unparen has the same
   gap.

## Harness gaps (rrlab)

- **`join` captures a rule variable named `s`.** The generated `rrjoinN_*` rules use `s` themselves. A
  `join … : { … }` in a rule whose own variable is `s` fails to type-check. pytest.rr avoids `s` there.
- **`(?s)` is not mapped** onto std::regex, so `match("(?s).*\\s.*", v)` matches nothing. The rules write
  `[\\s\\S]*` instead, which means the same thing in both dialects.

## Duplicates to merge

These have the same meaning as relations in other batches' new libraries. They are kept under this batch's
prefixes for now.

- exceptions.rr `exc_qual` ≈ ruff.rr `ruff_qual`. ruff.rr also models rebinding order.
- exceptions.rr `exc_call` ≈ ruff.rr `call_q`.
- exceptions.rr `exc_arg` ≈ ruff.rr `ruff_arg`.
- pytest.rr `pytest_str` ≈ pyflakes.rr `plain_str`.
- ruff-try300's private `may_effect` ≈ ruff.rr's `effect`/`pure` (contains_effect).
- pytest.rr `pytest_same` (ComparableExpr) ≈ compare.rr `cmp_same` / `cmp_same_ask`, once that library
  lands. PT014 should then use it and drop `pytest_atom`, `pytest_kid`, `pytest_pair` and
  `pytest_differ`.
- ruff-pt015's private `always_false` ≈ the falsy half of ruff-classes.rr's `truth`/`truth_of` (Truthiness).
