# py-up: 37 Ruff pyupgrade rules (UP), hand-written

Every rule follows Ruff 0.16.9, the version the JSON was written from. Where Ruff and the JSON differ, the rule
follows Ruff, and the sections below say so with a counterexample.

## How they were scored

The pack's recording (Data-layer-rule-expressivity) has no pyupgrade fixtures and no UP findings. A scratch
recording was made in the batch's scratchpad (`updata/`, not committed):
- the fixtures: the 83 files `upstream/ruff/fixtures.json` lists for these codes (`pyupgrade/*`, `ruff/redirects.py`),
  fetched at tag 0.16.9. They are byte-identical to the source the JSON was written from (wolfeel's
  `port-readiness/upstream/ruff/0be08a2…`).
- the findings: Ruff 0.16.9's (the release wheel), `ruff check --isolated --select UP0xx`, one code at a time, at
  Ruff's default target 3.10.

Each score below is `RRLAB_DATA=<that dir> rrlab score python <id> --all` at (path, line, column); the line-only
scores are the same. Ruff's own snapshot tests run at 3.14 unless a test sets a version, so they differ from this
recording on the version-gated rules. To keep the scores reproducible, commit these fixtures and the recording to
the pack's `upstream/ruff/`. Not scored: the mdtest cases in `resources/mdtest/pyupgrade/*.md`.

The target version: the JSON's `when: {pythonVersion: …}` reads `languageOptions.pythonVersion`, default 3.10
(DEFAULT_PYTHON_MINOR). Here it is the option `pythonVersion` ("3.12"), read by `target_minor` and `targets(m)` in
python/version.rr; an unparsable value falls back to 10. UP036 keeps its own `python-minor-version` option and falls back to the target. The
version-gated rules were also compared with Ruff 0.16.9 at other `--target-version`s.

Summary: 37 rules, 1,221 lines (18 to 81 each), plus 410 lines of libraries and one generated table.
- 26 rules are perfect on the recording.
- UP043 has no recorded findings at 3.10. At 3.13 it is 23 / 0 / 0 against Ruff.
- The other 10 miss only on harness gaps: string annotations, rrlab's `\N{…}` decoding, noqa in the middle of a
  comment (since fixed in rrlab), bare-CR line ends, and Ruff's repeated diagnostics at one key. The one exception is UP032_0.py:94, a
  library gap in `unparen` (F-L1 below).

## Scores

| rule | lines | TP / FP / FN | notes |
|---|---|---|---|
| ruff-up001-useless-metaclass-type | 24 | 2 / 0 / 0 | |
| ruff-up003-type-of-primitive | 41 | 6 / 0 / 0 | |
| ruff-up004-useless-object-inheritance | 20 | 24 / 0 / 0 | |
| ruff-up005-deprecated-unittest-alias | 30 | 4 / 0 / 0 |  |
| ruff-up006-non-pep585-annotation | 20 | 8 / 0 / 12 | I-H1 |
| ruff-up007-non-pep604-annotation-union | 34 | 18 / 0 / 4 | I-H1, I-D1 |
| ruff-up008-super-call-with-parameters | 51 | 32 / 0 / 0 | |
| ruff-up009-utf8-encoding-declaration | 40 | 7 / 1 / 0 | F-H1 |
| ruff-up010-unnecessary-future-import | 28 | 16 / 0 / 0 | I-D2, I-L1 |
| ruff-up011-lru-cache-without-parameters | 21 | 6 / 0 / 0 | D-UP011 |
| ruff-up012-unnecessary-encode-utf8 | 31 | 43 / 0 / 1 | F-H3 |
| ruff-up013-convert-typed-dict-functional-to-class | 31 | 13 / 0 / 0 | |
| ruff-up014-convert-named-tuple-functional-to-class | 32 | 6 / 0 / 0 | |
| ruff-up015-redundant-open-modes | 47 | 48 / 0 / 1 | F-H3, F-D1 |
| ruff-up018-native-literals | 50 | 51 / 2 / 2 | F-H4 |
| ruff-up019-typing-text-str-alias | 18 | 7 / 0 / 0 |  |
| ruff-up020-open-alias | 18 | 3 / 0 / 0 |  |
| ruff-up021-replace-universal-newlines | 29 | 3 / 0 / 0 | |
| ruff-up022-replace-stdout-stderr | 30 | 10 / 0 / 0 | |
| ruff-up023-deprecated-c-element-tree | 24 | 10 / 0 / 0 |  |
| ruff-up024-os-error-alias | 32 | 42 / 0 / 0 | I-D3 |
| ruff-up025-unicode-kind-prefix | 35 | 22 / 0 / 0 | |
| ruff-up026-deprecated-mock-import | 31 | 26 / 0 / 0 | I-D4 |
| ruff-up028-yield-in-for-loop | 44 | 20 / 0 / 0 | D-UP028 |
| ruff-up029-unnecessary-builtin-import | 41 | 7 / 0 / 0 |  |
| ruff-up030-format-literals | 36 | 32 / 0 / 1 | F-H2 |
| ruff-up031-printf-string-formatting | 38 | 67 / 0 / 1 | F-H3 |
| ruff-up032-f-string | 81 | 110 / 0 / 4 | F-H2, F-H3, F-L1, F-D2 |
| ruff-up033-lru-cache-with-maxsize-none | 21 | 9 / 0 / 0 | |
| ruff-up034-extraneous-parentheses | 41 | 14 / 0 / 0 | |
| ruff-up035-deprecated-import | 23 (+ 213-line table) | 55 / 0 / 5 | I-H2 |
| ruff-up036-outdated-version-block | 65 | 88 / 0 / 0 | D-UP036 |
| ruff-up039-unnecessary-class-parentheses | 21 | 7 / 0 / 0 | D-UP039 |
| ruff-up041-timeout-error-alias | 25 | 4 / 0 / 0 | 3.11 |
| ruff-up043-unnecessary-default-type-args | 26 | n/a (0 recorded at 3.10) | I-H3 |
| ruff-up049-private-type-parameter | 21 | 25 / 0 / 0 |  |
| ruff-up050-useless-class-metaclass-type | 21 | 13 / 0 / 0 | |

## Classes, calls and version blocks (12 rules)

All 12 are exact on the recording.

- **D-UP011, UP033.** Ruff turns UP011 off below a 3.8 target and UP033 below 3.9. The UP011 JSON has no such
  gate: `@functools.lru_cache()` at `--target-version py37` is clean in Ruff (checked). Both resolve the decorator
  in Ruff's walk order. In UP011.py, `@lru_cache(maxsize=None)` below `def lru_cache(…)` is not functools'
  (sym_at).
- **D-UP028.** Ruff 0.16.9 reports a loop whose target is declared `global` or `nonlocal` (UP028_0.py:179, 187),
  where the JSON's message says to keep it. A loop variable read anywhere else in its scope stops the finding:
  before the loop, after it, or from a nested function (UP028_1.py:56-113). So does a second loop on the same name
  that reads it (UP028_0.py:65, 88, 91).
- **D-UP036.** The rule is derived from Ruff 0.16.9 on six operators and 17 specifiers. A scratch probe gives 0 FP /
  0 FN against Ruff at 3.10 and at 3.12.
  - At the minimum itself, `(3, 10)` and `(3, 10, 0)` settle only `<` and `>=`. `> (3, 10)` and `<= (3, 10)` are
    left.
  - `==` and `!=` with a tuple are never reported. With an int over 255 they give the invalid-specifier finding
    (UP036_0.py:191).
  - Reversed operands (`(3, 9) <= sys.version_info`), conditional expressions and `and` chains are not reported.
- **D-UP039.** A comment between the parentheses does not exempt the class (UP039.py:46). The fix is offered only
  where there is none.
- UP008 models CPython's implicit `__class__` cell for `builtins.super(C, self)`. The method, or a function nested in
  it, must load `super` or `__class__`, and neither may bind `__class__`, a bare annotation included
  (UP008.py:178-289). The class must be named by its full class path (`Outer.Inner`), not a suffix of it
  (UP008.py:350-383).
- UP003, UP004 and UP050 read the builtins in walk order (builtin_at). An `object = A` above a class makes its
  `object` base clean; a rebinding below it does not. A star import (`from os import *`) does not hide a builtin,
  as in Ruff.
- UP013 and UP014 report only field names that are identifiers and not keywords. UP013 also skips keys with a `__`
  prefix (name mangling); UP014 skips any `_` prefix. UP014 takes the fields as a list only
  (`NamedTuple("P", [("x", int)])`; a tuple of fields is clean in Ruff), and neither reports an annotated
  assignment (`P: type = NamedTuple(…)`).
- Fixes kept, as in the JSON: UP001 removes the whole `__metaclass__ = type` statement; UP003 replaces
  `type("")` with `str` (and so on), but offers no fix where `str` is rebound in a scope that sees the call, and
  pads with a space where a name or keyword follows directly (`type("")in y` becomes `str in y`); UP011 drops the
  empty parentheses; UP039 drops `()` only where no comment is between them.

## Imports, aliases and typing (14 rules)

Scored with `rrlab score python <id> --all` against the scratch recording (Ruff 0.16.9, `--isolated`, target
3.10, the 82 pyupgrade fixtures). Version cases were checked against Ruff 0.16.9 with `--target-version` (cmp
column: the targets compared, every one 0 FP / 0 FN unless noted).


13 of 14 are exact on every recorded (path, line, column); the misses are harness gaps only.

### Harness gaps

- **I-H1, string annotations.** rrlab has no `parsed` rows, so names inside `"List[str]"` are never seen. Every FN
  of UP006 (12) and UP007 (4) is inside a string annotation (UP006_0.py:29-57, UP006_3.py:7, UP007_1.py:29, 33,
  91). The rules read the parsed tree through names' `annotation`/`sym_at`, so the engine should report them; their
  columns (inside the string, or at the string for concatenated ones) are the engine's to place.
  Since fixed: rrlab b402cbfa8 gives `parsed` rows, and python/names' `parsed_in` starts a parsed name's lookup at
  its string's scope. UP006 is 17 / 0 / 3 on Ruff's snapshots (`rrlab score --snapshots`); the 3 FNs are names in
  a string nested in a parsed one (UP006_0.py:49, 53, 57), which rrlab does not parse yet.
- **I-H2, duplicate keys.** Ruff gives UP035.py:44 six diagnostics (one per target module and per renamed name),
  all at 44:1. The rule reports the statement once: same key, so the five extra recorded rows are FNs only by
  count.
- **I-H3, UP043.** The recording's target (3.10) gives no findings. Checked at 3.13 by `rrlab run --options
  '{"pythonVersion": "3.13"}'` against Ruff 0.16.9 `--target-version py313`.

### Deviations from the JSON (the rules follow Ruff 0.16.9)

- **I-D1, UP007.** Ruff drops `Union[…]` with a quoted member (or one inside a tuple member) only where the
  expression is evaluated: `def f(x: Union["str", int])` and `x = Union["a", "b"]` are not reported, but a local
  `x: Union["a", "b"]` inside a def is (never evaluated). Also skipped: `Union[*Ts]`, and any member that is
  `NamedTuple`. Under a stub the annotation is unevaluated but runtime code is not (`x = Union["a", "b"]` in a
  .pyi is not reported). Counterexamples: UP007_1.py:37, 41, 47 (not reported) vs 49 (reported).
- **I-D2, UP010.** A name counts as used when some load of it in the file resolves to no other binding;
  `__all__` does not count (UP010_1.py:5, `generator_stop` in `__all__`, is reported). A name imported `as`
  another is not reported (`from __future__ import division as d`: Ruff silent).
- **I-D3, UP024 and UP041** resolve names in Ruff's walk order (upgrade.rr's sym_at): `from mmap import error`
  then later `from foo import error` makes `except (OSError, error)` after the second import clean
  (UP024_0.py:73). WindowsError is not in names' builtins; an unbound `WindowsError` counts (Ruff counts it as a
  builtin). `(mmap).error` resolves through the parentheses (UP024_0.py:87).
- **I-D4, UP026** is not a list (two module spellings), so it has no generated table; its three JSON clauses are
  three rules with the JSON's messages. The attribute form is reported only as `mock.mock.X`, and only in a file
  that imports `mock` (`import mock` / `from mock import …`); a bare `mock.mock` or a `mock.mock.X` in a file
  without the import is clean in Ruff.
- UP043 fires in stubs at any target, and not for `from __future__ import annotations` below 3.13 (Ruff 0.16.9,
  checked).
- UP049 skips `type X[_T] = …` aliases, as Ruff does.
- **I-D5, 3.14 annotations.** From a 3.14 target annotations are deferred (PEP 649), so every typing-only
  annotation counts as late (`late_annotation`) and `runtime_annotation` is empty: UP006 and UP007 then report
  quoted members as Ruff does at 3.14 (its own snapshot target). Checked at 3.8, 3.10 and 3.14.
- UP005's fix (`assertEquals` to `assertEqual`, …) is kept as in the JSON.

### Limits

- **I-L1, UP010** approximates "used": it does not tie the load to the scope of the `__future__` import (the import
  creates no binding in python/names).

### Vocabulary added

- `lib/python/upgrade-imports.rr` (56 lines): `up_qual(e, q)` (sym_at, plus `(m).x`), `exc_site(e, how)`
  (except type, raise, call function), `handler_tuple(t, e)`, `from_stmt(i, m)`, `from_name(i, a, x, l)`,
  `plain_import(i, a, m, l)`.
- `lib/python/upgrade-typing.rr` (64 lines): `rewritable(n, m)` (one condition: stub, or target, or 3.7+ with future annotations on a typing-only node),
  `pep585(x, b)`, `sub_of(s, q)`, `up_typing_sub(s, x)`, `type_arg(s, i, a)`, `up_type_nargs(s, k)` (one view of
  subscript and generic_type, comments skipped, `X[(a, b)]` flattened), `runtime_annotation(n)`.
- UP035 table: `packs/python/ruff-up035-deprecated-import.moves.rr`, generated by
  `gen-ruff-up035-deprecated-import.py`, checked by `check-ruff-up035-deprecated-import.py` (prints ok).

## Strings and format strings (11 rules)

Shared vocabulary: `lib/python/upgrade-format.rr` (73 lines): string literals (`str_lit`, `fstr_lit`,
`lit_piece`, `wide_escape`, `ascii_str`, `u_piece`), UTF-8 codec names (`utf8_name`, `utf8_only_arg`), and
percent-format extras over pyflakes-format (`pct_conv`, `rhs_form`). It includes python/upgrade.rr, which holds
the one argument view (`up_arg`, `up_kw`, `up_kwarg`, `up_argc`, `up_splat`, `kwsplat`, `nkw`), and
python/pyflakes-format.rr, which holds `format_call`, the `brace_*` relations and `call_pos`/`call_kw`/….

Scores: `rrlab score python <id> --all` over the 82 pyupgrade fixtures of Ruff 0.16.9, recorded with Ruff 0.16.9's
CLI (`--isolated`, target 3.10). Line and line+column scores are the same for every rule. Every rule was also run
on probe files and compared with Ruff 0.16.9 at 3.8, 3.10 and 3.14; each probe matched except
`\N{…}` (F-H2) and a comment inside parentheses (F-L1) in UP032.


Harness gaps (the rule is right; rrlab is not the engine):
- F-H1, UP009_4.py:1 (fixed in rrlab 08d977aa5). `# coding=utf8  # noqa: UP009` is one comment; rrlab honoured
  `noqa` only at a comment's start, Ruff anywhere in it. rrlab now matches Ruff; UP009 is 7 / 0 / 0 on the snapshot.
- F-H2, value decoding. rrlab's `value` is approximate: it leaves `\N{snowman}` undecoded, so its braces read as a `{snowman}` field (UP030_0.py:22 and
  UP032_0.py:135 are FNs). The engine's decoder (str_part) is exact.
- F-H3, string annotations. `x: '"foo".encode("utf-8")'` and the like (UP012.py:86, UP015_1.py:3, UP031_0.py:131,
  UP032_0.py:270): rrlab does not model `parsed`, so code inside a string annotation has no tree.
- F-H4, UP018_CR.py: the file ends its lines with bare `\r`. Ruff counts them as line ends; rrlab's tree positions
  count `\n` only, so both findings land on line 1 (2 FP, 2 FN).

Library gap:
- F-L1, UP032_0.py:94: `(\n # comment\n "{a}" … )` — python/ast's `unparen` takes child 0, which is the comment, so
  `format_call` (python/pyflakes-format) does not see the literal. See the change request below.

Deviations from the JSON (the rules follow Ruff 0.16.9):
- F-D1, UP015: the JSON reports at the `mode=` keyword; Ruff reports at the string (`open(name="f", mode="U")` is
  1:23, not 1:18). It also reads only the builtin `open` and `aiofiles.open`, in Ruff's source order
  (upgrade.rr's `builtin_at`): `io.open("f", "r")` and `pathlib.Path(p).open("r")` are not reported, and neither is a
  call in a function when a later module-level `open = …` rebinds it. Only positional arguments before the
  first `*` count (`open(*a, "r")` is clean). The mode must be one Ruff's `OpenMode` parses: no repeated flag,
  not both `b` and `t`, exactly one of `r`/`U`, `w`, `x`, `a`, so `rU+` is reported and `rw` is not.
- F-D2, UP032 conditions taken from Ruff's behaviour, checked on probes: an argument holding any string literal
  stops the rewrite (`"{}".format(a['b'])`; target 3.10, before PEP 701); an argument used by two fields must have
  no call, await, subscript or binary operator (`"{0} {0}".format(a[0])` and `(a + b)` are not reported, `(-a)` and
  `(a.b)` are); an argument written over several lines stops it (UP032_0.py:178, 186); a field split across adjacent
  literals stops it (`'{' '0}'.format(1)`, UP030_0.py:34); only django's `gettext`/`gettext_lazy` count as
  translation calls (`_()`, `ngettext`, `pgettext`, `gettext.gettext` do not); Ruff's line-length check
  (`fits_or_shrinks`, 88 columns) is modelled for a call on one line only. A multi-line call is not measured:
  Ruff reports UP032_0.py:159, whose f-string is 90 columns, so its measure of multi-line calls is not the plain
  one, and no fixture needs it.
- UP009 follows Ruff on encodings: the name must be exactly `utf-8` or `utf8` (case-sensitive: `UTF-8` is another
  encoding), and a second coding comment in the leading comment block that names another encoding stops the
  finding even on line 3 (`# coding: utf-8` / `# hello` / `# coding: ascii` is not reported). The coding regex is
  greedy where PEP 263's is lazy; they differ only on a line that says "coding" twice.
- UP012: Ruff's UTF-8 names are `utf-8`, `utf8`, `utf_8`, `u8`, `utf`, `cp65001`, any case (`utf8_ucs2` is not).
  A literal with a `\N`, `\u` or `\U` escape, or an octal escape above `\377`, is reported by neither variant,
  even with a UTF-8 argument (`wide_escape`; `"\477".encode("utf-8")` is UP012.py:128, clean in Ruff).
- UP018: Ruff leaves a call inside an f-string nested in another (`f"{f'{int()}'}"`), and reads `str(object=…)`
  and `complex(real=…)`, no other keyword.
- UP021, UP022: `subprocess.run` only (not Popen or check_output), and only from target 3.7 (`targets(7)`).
- UP031: Ruff leaves `S % R` when R is a list, set, operator, lambda, `-1`, comparison, conditional, comprehension
  or walrus (`rhs_form`); Ruff 0.16 reports names, attributes, subscripts and calls (no longer false negatives).
  The two JSON variants are kept, as two violations with one code.
- UP034: Ruff's scan is over tokens and skips what an attempt passed over, so only the outermost of nested doubles
  is reported (`print((("a")))` once), and a double inside a failed attempt is not (`f(((1)), 2)`).
- UP025: as the JSON. Under `from __future__ import annotations` or in a stub, a string that is a whole annotation
  is left (in a stub, a `TypeAlias` value too); `x: list[u"int"]` is still reported.
- UP030: the variant for string annotations (`format-literals-annotation`) has no separate relation; the main
  rule covers annotation strings once the engine parses them.

Fixes kept: UP009 (delete the whole coding line) and UP025 (drop the prefix), both `fix safe` as in Ruff. The
others have no fix in the JSON. UP009's edit deletes the line's span (`src_line`), so the line ending is left to
the applier; see the change request below.

## Fixes

Seven rules keep the JSON's fix, all `fix safe` as in Ruff (rrlab drops fixes, so they are not scored):

| rule | fix |
|---|---|
| UP001 | remove the `__metaclass__ = type` statement |
| UP003 | `type("")` to `str` (and the other primitives); none where `str` is rebound, a space added before a glued name |
| UP005 | rename the deprecated unittest alias (`assertEquals` to `assertEqual`) |
| UP009 | delete the coding line (its span; the line ending is the applier's, see below) |
| UP011 | drop the empty parentheses of `@lru_cache()` |
| UP025 | drop the `u` prefix |
| UP039 | drop `()` after a class name, only with no comment between them |

## Vocabulary

| file | lines | what |
|---|---|---|
| `lib/python/version.rr` | 24 | the target version: `target_minor(m)` from the option `pythonVersion` ("3.N" or "3.N.P"; absent or unparsable, 10), `targets(m)` for every minor up to it |
| `lib/python/upgrade.rr` | 193 | `up_stub`, `future_annotations` (leading run only), `typing_ref`, `typing_only`, `late_annotation`, names in Ruff's walk order (`sym_at`, `builtin_at`, `runs_at_import`), the call arguments (`up_arg`, `up_npos`, `up_kw`, `up_kwarg`, `up_argc`, `up_splat`, `kwsplat`, `nkw`), `identifier`/`field_name`/`py_keyword`, `functional_def`. Includes python/pyflakes.rr and python/version.rr |
| `lib/python/upgrade-imports.rr` | 56 | `up_qual`, `exc_site`, `handler_tuple`, `from_stmt`, `from_name`, `plain_import` |
| `lib/python/upgrade-typing.rr` | 64 | `rewritable`, `pep585`, `sub_of`, `up_typing_sub`, `type_arg`, `up_type_nargs`, `runtime_annotation` |
| `lib/python/upgrade-format.rr` | 73 | string literals (`str_lit`, `lit_piece`, `wide_escape`, …), UTF-8 codec names, percent-format extras |
| `packs/python/ruff-up035-deprecated-import.moves.rr` | 213 | UP035's table (gen/check scripts beside it; the check prints ok) |

### sym_at: names in Ruff's order

Ruff binds names while it walks the module and defers function bodies to the end. sym_of sees every binding in a
scope, so a later `type = str` or `def lru_cache` further down would hide a finding. sym_at(e, q) follows Ruff:
- At import time (module and class bodies, decorators, defaults, runtime annotations) a load sees only the
  bindings above it in its scope.
- A function body is read after the whole module. Each enclosing scope offers its last binding of the name; the
  function's own scope is still read in walk order. Stores are not uses.
  - `from typing import List` / `def f(): return List[int]()` / `if x: List = list`: no UP006.
  - A function that imports `List`, uses it, rebinds `List = list`, then uses it again: only the first use is
    reported.
- A class body with no earlier binding of the name falls through to the enclosing scope:
  `class C: @lru_cache() …; lru_cache = 1` is UP011, and `class D: x = type(1); type = 2` is UP003.
- builtin_at(e, x) is sym_at reaching no binding plus `builtin(x)`. A star import does not hide a builtin.

Cost: a name with one binding above the read takes a direct path (`up_once`); only names bound more than once
walk the scope chain. UP011 on 800 probe files: 2.8 s of souffle CPU with sym_at against 2.2 s with sym_of (1.27x).

## Duplicates to merge

- These were renamed with the `up_` prefix (LIB-CLASH.md). The same idea is in another batch's library:
  - `up_stub` and `up_typing_module`: typing.rr's `stub` and `typing_module`;
  - `up_typing_sub`: typing.rr's `typing_sub`;
  - `up_type_nargs`: ruff-classes.rr's `type_nargs`;
  - `up_qual`: ruff.rr's `qual`;
- `str_shadowed` (UP003's fix guard) is a narrow form of names.rr's `shadowed`.
- `target_minor`/`targets` now live in python/version.rr; other batches that gate on the version should include
  it instead of reading `pythonVersion` themselves.

## Library and core change requests

- **python/ast.rr, unparen (F-L1, UP032_0.py:94).** Skip comments inside parentheses:
  `unparen(e, m) :- kind(e, "parenthesized_expression"), child(e, i, _), !kind(i, "comment"), unparen(i, m).`
- **python/names.rr, not_name.** Names in a `from __future__ import` count as ast_names today. Add
  `not_name(n) :- kind(i, "future_import_statement"), within(n, i).` UP010 works around it.
- **python/names.rr, builtins.** Ruff treats `WindowsError` as a builtin on every platform. Add
  `builtin("WindowsError").` UP024 falls back on `miss` until then.
- **Fix edits.** A `delete_line(l)` edit, or `src_line` spans that include the line ending, so that UP009's fix
  removes the line and not only its text.
- **rrlab (harness, not the engine).** `parsed` rows for string annotations (UP006, UP007, UP012, UP015, UP031,
  UP032); exact string decoding (`\N{…}`); `# noqa` anywhere in a comment; bare `\r` line ends.
