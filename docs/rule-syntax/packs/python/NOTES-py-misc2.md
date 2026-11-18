# py-misc2: Ruff's smaller plugins, hand-written

70 rules from 15 plugins: flake8-annotations (ANN), flake8-unused-arguments (ARG), perflint (PERF), numpy
(NPY), FastAPI (FAST), flake8-type-checking (TC), flake8-import-conventions (ICN), flake8-async (ASYNC),
flake8-2020 (YTT), flake8-errmsg (EM), flake8-gettext (INT), flake8-todos (TD), pygrep-hooks (PGH), flake8-self
(SLF) and flake8-slots (SLOT). Written in four groups; each section below is one group, with its libraries,
scores, deviations, change requests and duplicates.

## How it was scored

The shared rrlab recording covers only ASYNC among these plugins. Every rule was scored against Ruff 0.16.9's
own test snapshots at ruff@0be08a206f9c (the version the JSON pins), on the fixtures those snapshots point at,
matched on (file, line). The script is a scratch tool (`msnap.py`, not committed). Default-settings snapshots
are used, plus the option tests named in the tables, run with that option. ASYNC was also scored with
`rrlab score python <id> --all`. Each group also diffed its rules against the installed Ruff 0.15.7 on
counterexamples.

Result: 69 of 70 rules are perfect on their snapshots. ANN401 has 2 FN, both from rrlab having no `parsed`
rows (D-A2). ANN002 and ANN003 have no default-settings snapshot case (checked ad hoc against 0.15.7).

## Group A: flake8-annotations (ANN) and flake8-unused-arguments (ARG)

Libraries:

- `lib/python/annotations.rr` (about 190 lines): which defs ANN checks (overload implementations skipped),
  methods and their decorators, Ruff's visibility, the checked parameters, the return code per def, and
  `ann_any_type` (Ruff's type_hint_resolves_to_any).
- `lib/python/misc2-args.rr` (about 120 lines): Ruff's function_type classification, stubs (is_stub and
  is_not_implemented_stub_with_variable), `locals()`, dummy names, and `arg_unused`. It includes annotations.rr.

Scores are TP/FP/FN at (file, line) against Ruff 0.16.9's snapshots. Only tests run at default settings are
used, plus the ignore/enforce-variadic-names tests for ARG002 with their option. Ad hoc: every ANN and ARG
code was also diffed at line and column against the installed Ruff 0.15.7, on 40 hand-written defs (scratch).
They matched except for D-A2.

| rule | lines | 0.16.9 snapshots | deviations |
|---|---|---|---|
| ANN001 missing-type-function-argument | 20 | 4/0/0 (defaults, suppress_none_returning) | D-A1 |
| ANN002 missing-type-args | 17 | 0/0/0 (no default-settings case); ad hoc 3/0/0 vs 0.15.7 | D-A1 |
| ANN003 missing-type-kwargs | 17 | 0/0/0 (no default-settings case); ad hoc 2/0/0 vs 0.15.7 | D-A1 |
| ANN201 missing-return-type-undocumented-public-function | 21 | 52/0/0 (defaults, auto_return_type, allow_overload, allow_nested_overload, shadowed_builtins, simple_magic_methods) | D-A1, D-A3 |
| ANN202 missing-return-type-private-function | 21 | 1/0/0 (same) | D-A1, D-A3 |
| ANN204 missing-return-type-special-method | 20 | 17/0/0 (same) | D-A1, D-A3 |
| ANN205 missing-return-type-static-method | 18 | 3/0/0 (same) | D-A3 |
| ANN206 missing-return-type-class-method | 18 | 3/0/0 (same) | D-A3 |
| ANN401 any-type | 24 | 15/0/2 (defaults) | D-A1, D-A2 |
| ARG002 unused-method-argument | 21 | 8/0/0 (ARG.py); 2/0/0 enforce_variadic_names; 2/0/0 ignore_variadic_names (with the option) | |
| ARG003 unused-class-method-argument | 20 | 1/0/0 (ARG.py) | |
| ARG004 unused-static-method-argument | 21 | 3/0/0 (ARG.py) | |
| ARG005 unused-lambda-argument | 17 | 1/0/0 (ARG.py) | |

12 of 13 are perfect. ANN401 has 2 FN, both from the rrlab harness (D-A2).

Review fixes (ARG, checked against the Ruff 0.16.9 binary on the reviewer's cases, which now match line for
line, and rescored as above): `.pyi` files are skipped; a dunder `__new__` is checked (as ARG004) only when
undecorated, since `@staticmethod`/`@classmethod` make it an ordinary dunder, which Ruff skips; a bare
annotation `x: int` neither reads nor rebinds a parameter; a `global x` on a parameter (a syntax error) is
left alone, as Ruff does.

### Deviations

- **D-A1: flake8-annotations options.** The JSON has no options, so the rules do what Ruff does at its
  defaults. Ruff's `suppress-dummy-args`, `suppress-none-returning`, `mypy-init-return`, `allow-star-arg-any`
  and `ignore-fully-untyped` (all false by default) are not offered. Adding them would be one guard each in
  annotations.rr. ARG keeps both of its JSON options (`dummy-variable-rgx`,
  `flake8-unused-arguments.ignore-variadic-names`), and both were checked.
- **D-A2: quoted annotations (harness gap).** rrlab has no `parsed` rows, so a string annotation never parses
  there. annotations.rr follows Ruff:
  - a top-level string that does not parse is not reported;
  - a string member of a Union that does not parse counts as Any.
  
  Under rrlab this gives the two FN, `def f(a: "Union[str, bytes, Any]")` (line 154) and
  `def quoted_escape(x: "'in\x74'")` (line 172). Ruff reports line 172 because it refuses to parse a nested
  quote that contains an escape, so that member counts as Any. The engine's `parsed` must refuse it the same
  way for this to match.
  
  It also gives one ad-hoc FP against 0.15.7: `def h2(b: Union[int, "Foo"])`. Ruff parses `"Foo"` and does not
  report it. With a real `parsed`, neither the FN nor the FP happens.
- **D-A3: no fixes.** Ruff offers unsafe fixes for the return codes: an inferred `-> T`, and `-> None` for
  `__init__` and the simple dunders. The JSON has no fixes, and the inference (Ruff's auto_return_type over
  returns and terminal flow) is not vocabulary we have, so none are offered.
- **Overload order (approximation, no fixture difference).** Ruff skips a def whose name matches the @overload
  def *visited just before it*, in its own order: a def's body is visited after its enclosing scope. Here
  "just before" is the latest def or class that starts earlier in the same def body (or module), nested
  class bodies included. That is Ruff's order except at the first definition of a body, where Ruff's
  predecessor is the last definition of the previous body. All the overload fixtures match.
- **Private modules.** Ruff decides from the module path when it knows the package root, and otherwise from
  the file name. Here a file name or a `pkg` segment that starts with `_` and is not a dunder makes the module
  private.

### Library and core change requests

None needed.

### Duplicates to merge

- `annt_typing(e, x)` (annotations.rr) means the same as typing.rr's `typing_sym(e, x)` and upgrade.rr's
  `typing_ref`, but resolves through ruff.rr's `ruff_qual` instead of `sym_of`.
- `argt_abstract` (misc2-args.rr) overlaps typing.rr's `abstract(f)`, which has the same four abc decorators.
- `ann_method(f)` is typing.rr's `pyi_method(_, f, _)`.
- `argt_method`/`argt_dec` is Ruff's function_type, as typing.rr's `method_kind` is. method_kind matches a
  decorator by its name text (`decorated`); this one resolves it and also counts abc.abstractstaticmethod and
  abc.abstractclassmethod, as Ruff does.

### Note for the coordinator

While I was scoring, an in-progress edit to `lib/python/ruff.rr` (another batch) briefly declared
`ann_head(ann, h)`. That clashes with ast.rr's `ann_head(a: node, s: symbol)` and stopped every rule that
includes ruff.rr from compiling. It was gone on the next run. If it comes back, the clash is in ruff.rr.

## Group B: perflint (PERF), NumPy (NPY), FastAPI (FAST), flake8-type-checking (TC), import conventions (ICN)

Libraries (all new; no file outside this group's list was edited):

- `lib/python/misc2-perf.rr` (123 lines): for-loop shapes, Ruff's only_binding and the binding in force
  (resolve_name during the walk), what a binding stores (through a chain `a = b = v`) and its annotation head, is_list / is_dict, and reads
  of a loop target after the loop (is_unused). FAST uses its binding helpers too.
- `lib/python/misc2-numpy.rr` (51 lines): references to `numpy.*` by qualified name (Name, Attribute, and
  dotted case patterns through misc2-case) and NPY201's fallback guards (Ruff's is_guarded_by_try_except).
- `lib/python/misc2-fastapi.rr` (129 lines): route defs (Ruff's is_fastapi_route), their parameters, FastAPI
  markers, `Annotated` arguments, and FAST003's dependency resolution (Dependency::from_parameter).
- `lib/python/misc2-typecheck.rr` (138 lines): TYPE_CHECKING blocks, runtime type definitions (Ruff's
  in_type_definition outside a TYPE_CHECKING block) and `|` unions.
- `lib/python/misc2-case.rr` (19 lines): `case_value_name(e, q)`, a dotted name used as a value in a case
  pattern (`case six.PY3:`), with its qualified name. Used by NPY (npy_ref) and YTT202.

Scores are TP/FP/FN at (file, line) against Ruff 0.16.9's snapshots at their own settings. Those snapshots
run at Python 3.14 (perflint at 3.10), and the default `pythonVersion` here is 3.10. Where that matters, the
3.13 side of the `deferred_annotations_diff` snapshot is used too. Ad hoc: the installed Ruff 0.15.7 was
run on the same fixtures, by line.

| rule | lines | 0.16.9 snapshots | deviations |
|---|---|---|---|
| PERF101 unnecessary-list-cast | 47 | 13/0/0 | |
| PERF102 incorrect-dict-iterator | 41 | 12/0/0 (incorrect-dict-iterator) | D-B4 |
| PERF203 try-except-in-loop | 32 | 1/0/0 (at 3.10; silent from 3.11 as Ruff) | |
| PERF401 manual-list-comprehension | 32 | 29/0/0; 0.15.7 on PERF401.py + PERF402.py 31/31 | D-B5 |
| PERF402 manual-list-copy | 21 | 1/0/0; 0.15.7 4/4 | |
| PERF403 manual-dict-comprehension | 47 | 22/0/0; 0.15.7 22/22 | D-B5 |
| NPY001 numpy-deprecated-type-alias | 28 | 7/0/0 | D-B2 |
| NPY002 numpy-legacy-random | 20 + 63 table | 52/0/0 | |
| NPY003 numpy-deprecated-function | 25 | 10/0/0 | |
| NPY201 numpy2-deprecation | 24 + 132 table | 71/0/0 | D-B2, D-B3 |
| FAST001 fast-api-redundant-response-model | 41 | 0/0/0 at 3.14; 10/0/0 at 3.13 (deferred diff) | D-B3, D-B5, D-B6 |
| FAST002 fast-api-non-annotated-dependency | 28 | 27/0/0; 0.15.7 equal at 3.10 and 3.14 | D-B3 |
| FAST003 fast-api-unused-path-parameter | 41 | 27/0/0 at 3.14; at 3.13, 34 = 27 plus the diff's 7; 0.15.7 equal at 3.10 and 3.14 | D-B3, D-B7 |
| TC005 empty-type-checking-block | 25 | 4/0/0 | D-B8 |
| TC006 runtime-cast-value | 24 | 13/0/0 | |
| TC010 runtime-string-union | 30 | 2/0/0 at 3.14 | D-B1 |
| ICN001 unconventional-import-alias | 32 + 19 table | 14/0/0 (defaults); 0.15.7 on all fixtures 64/64 | D-B3 |

Generated tables, each with `gen-` and `check-` scripts that round-trip against the JSON with 0 differences:

- ICN001 `.aliases.rr`: 16 conventional aliases.
- NPY002 `.methods.rr`: `npy002_method(q, g)` and `npy002_advice(g, m)`, 50 functions in 8 groups.
- NPY201 `.members.rr`: `npy201_member(q, g)`, `npy201_advice(g, m)` and `npy_replacement(q, new)`, 70
  members in 56 groups.

### Deviations

- **D-B1, TC010 at the default target.** At `pythonVersion` 3.10 there are 3 more findings than the snapshot
  (TC010_2.py lines 4, 5 and 12), because the snapshot runs at 3.14, where annotations are deferred. Ruff 0.16.9
  with `--target-version py310` agrees with the rule (TC010_1 1/1, TC010_2 5/5).
- **D-B2, string annotations.** NPY001 and NPY201 also check names inside string annotations
  (`x: "np.float"`) in Ruff. That needs `parsed` / `string_annotation`, which rrlab does not model, so that path
  is not written and cannot be scored.
- **D-B3, fixes not offered.** These rules do not offer Ruff's fixes:
  - ICN001: the rename.
  - NPY201: the edits that add an import.
  - FAST001: removing the argument.
  - TC006: quoting the type (it must rewrite quotes inside it).
  - FAST002: rewriting to `Annotated`.
  - FAST003: adding the parameter.

  The JSON has no fix for any of them either.
- **D-B4, PERF102 comprehensions.** Ruff also checks comprehensions, but only in preview. Only for statements
  are covered.
- **D-B5, comparison by text.** PERF401 and PERF403 compare expressions by their text, and FAST001 compares
  types by their text after removing whitespace and comments. This approximates Ruff's ComparableExpr and
  name-by-binding comparison. The difference shows only when two spellings mean the same expression in a way
  that text cannot show, such as a string with different quotes (`d['k']` against `d["k"]` in PERF403): Ruff
  calls them equal and this rule does not. There is no known difference on the fixtures, or against
  Ruff 0.15.7 on them.
- **D-B6, FAST001 deferral.** The rule is silent under `from __future__ import annotations` or `pythonVersion`
  3.14 and later. Ruff resolves the return annotation's names only after the walk, so they never match the
  decorator's, which is why the default 3.14 snapshot is empty.
- **D-B7, FAST003 position.** The finding is reported at the path string, not at the `{…}` inside it.
  rrlab's `match` captures are symbols, not spans, and `match_all` is not modelled. The line is the same for a
  one-line path. Path parameters are read from the literal's source text, not its decoded value, so an escape
  sequence inside a `{…}` differs (none in the fixtures).
- **D-B8, TC005 fix.** The fix is `delete(i)`. The applier drops it when the block is its parent's only
  statement, as Ruff does (it would leave an empty body).

- **Review fixes (Ruff 0.16.9 binary on the review cases, line and column).**
  - TC010 follows Ruff's in_type_definition: the type roots are cast, TypeVar (constraints, `bound=`,
    `default=`), NewType, TypeAliasType (value and `type_params=`), TypeForm, the functional NamedTuple and
    TypedDict, the TypeAlias value, the items of a generic subscript (typing generics and the stdlib ones:
    `list`, `dict`, `collections.*`, `collections.abc.*`, `re.Pattern`, …, but not inside a `type X = …`
    statement), and annotations that run: a def's in any scope, a variable's only at module or class scope,
    not in stubs or when deferred. `Literal["a" | "b"]` is reported (only Annotated metadata is skipped), and a
    `|` inside `Union[…]` is nested. Review cases: tc.py 13/13, tc2.py 9/9, tc3.py 0/0, st.pyi 3/3.
  - NPY001: a builtin's name bound in any enclosing scope (not only the nearest) shadows it.
  - PERF: `perf_site_value` follows chained assignments, so `a = b = [1]` is a list for both names. PERF101
    keeps the copy only for `del x[…]` as one of the del's own targets (not `del x[0][1]` or `del y[x[0]]`).
    Review cases p1–p3, a1, a2: equal to 0.16.9 (21, 11, 1, 0, 1 findings).
  - FAST001 is reported at the whole `response_model=T` keyword argument, as Ruff is (FAST001.py 10/10 at
    py313).
  - YTT202 also reports `case six.PY3:` (through misc2-case).

### misc2-typecheck.rr exports (stable interface for other batches)

`tc_block(i, b)`, `tc_test(t)`, `tc_in_block(n)`, `tc_typing_call(c, x)`, `tc_cast_type(c, t)`,
`tc_runtime_typedef(e)`, `tc_typedef_root(e)`, `tc_ann_owner(r, o)`, `tc_bitor(u)`, `tc_union(u)`,
`tc_nested(u)`, `tc_union_member(u, m)`, `tc_side(u, m)`, `tc_strip(c, m)`. It includes
`python/upgrade-typing.rr` and `python/ruff.rr`. Internal helpers (not for other batches): `tc_items`,
`tc_lazy`, `tc_generic_sub`, `tc_generic`, `tc_up_wrap`.

`tc_runtime_typedef(e)` now means "e is a type expression Python evaluates at run time": a `tc_typedef_root`
outside a TYPE_CHECKING block. See request 5 below for typing-context.rr's `deferred_ann`.

Other exports:

- misc2-case.rr: `case_value_name(e, q)`.
- misc2-perf.rr: `perf_for`, `perf_body`, `perf_call_stmt`, `perf_method_call`, `perf_mentions`,
  `perf_only_binding`, `perf_latest_site`, `perf_site_value`, `perf_chain_right`, `perf_site_ann`, `perf_ann_head`,
  `perf_typed_site`, `perf_list_site`, `perf_dict_site`, `perf_target_name`, `perf_used_after`,
  `perf_unused`.
- misc2-numpy.rr: `npy_ref`, `npy_callee`, `npy_replacement` (declared there, rows from NPY201's table),
  `npy_guarded`, `npy_in_handler`, `npy_catches`.
- misc2-fastapi.rr: `fast_route`, `fast_param`, `fast_strip`, `fast_dependency`, `fast_annotated`,
  `fast_dep_param`, `fast_dep_unknown`.

### Library and core change requests

1. **rrlab: model `match_all`.** It lowers to `rr_never` today, so every rule that uses it scores 0. FAST003
   works around it with a recursive `match` over the rest of the string.
2. **rrlab: `match` captures on a node should be spans.** core.rr says "On a node or span, X is a span", but
   rrlab binds the capture as a symbol, so a finding cannot be placed at the `{…}` (D-B7).
3. **rrlab: support the `(?s)` inline flag in `match`.** `match("(?s).*\\{(?<k>…)…", s)` gives no rows, while
   `[\\s\\S]*` works. Either support the flag or have check_rr reject it.
4. **core.rr: a perf-independent "binding in force" helper.** `perf_latest_site` and `perf_only_binding` are
   Ruff's resolve_name and only_binding, which FAST also needs. Suggested text for `python/names.rr`:
   `// ruff_latest_site(n, site): the binding of n's name in force at n (Ruff's resolve_name during the walk).`
   Then misc2-perf would re-export it.
5. **typing-context.rr (another batch's file): `deferred_ann` applies the scope test to defs.** It uses
   `tc_ann_owner` with a "not in a function scope" test for def annotations too, but Ruff evaluates a def's
   parameter and return annotations at run time in any scope (TC010's defect 2). Only variable annotations
   depend on the scope. Suggested: keep the scope test only on the assignment-annotation branch.

### Duplicates to merge

- `fast_strip` (misc2-fastapi) and `tc_strip` (misc2-typecheck) both remove parentheses and the `type`
  wrapper. fast_strip also lets an unset value through. They could be merged into one helper in ruff.rr.
- `perf_ann_head` (misc2-perf) overlaps with ast.rr's annotation-head helpers. The earlier `ann_head` clash
  was renamed to avoid it, so it was not merged.

## Group C: flake8-async (ASYNC) and flake8-2020 (YTT)

Libraries:

- `lib/python/misc2-async.rr` (90 lines): the async context, blocking calls, the trio, anyio and asyncio API tables, and checkpoints in a body. It also has `async_name_value`, the value of the binding a name reads, built on pathlib.rr's `pth_seen` and `site_init`.
- `lib/python/misc2-version.rr` (47 lines): `sys.version` and `sys.version_info` subscripts, and single comparisons.

How the rules were scored:

- **0.16.9 snapshots:** `msnap.py`, which uses default-settings snapshots and matches on (file, line).
- **ASYNC recording:** `rrlab score python <id> --all`, from the recording in the sibling Data-layer-rule-expressivity checkout. It covers every fixture, rerail probes included.
- **Installed Ruff 0.15.7:** run on ad-hoc counterexamples. They cover parentheses, chained comparisons, `[:3:]`, a non-ASCII string, rebinding, lambdas and class bodies in an async def, decorators of nested defs, `with … as` items, and float exponents. Every finding matched Ruff's.

| rule | lines | 0.16.9 snapshots | rrlab --all | deviations |
|---|---|---|---|---|
| ASYNC100 cancel-scope-no-checkpoint | 31 | 11/0/0 | 15/0/0 | |
| ASYNC105 trio-sync-call | 18 | 25/0/0 | 50/0/0 | |
| ASYNC115 async-zero-sleep | 18 | 14/0/0 | 22/0/0 | C1 |
| ASYNC116 long-sleep-not-forever | 30 | 18/0/0 | 20/0/0 | C2 |
| ASYNC210 blocking-http-call-in-async-function | 24 | 24/0/0 | 27/0/0 | |
| ASYNC221 run-process-in-async-function | 38 | 24/0/0 (ASYNC22x) | 27/0/0 | |
| ASYNC222 wait-for-process-in-async-function | 17 | 7/0/0 (ASYNC22x) | 9/0/0 | |
| ASYNC230 blocking-open-call-in-async-function | 25 | 12/0/0 | 17/0/0 | |
| ASYNC250 blocking-input-in-async-function | 16 | 4/0/0 | 6/0/0 | |
| ASYNC251 blocking-sleep-in-async-function | 17 | 1/0/0 | 3/0/0 | |
| YTT101 sys-version-slice3 | 16 | 3/0/0 | no recording | |
| YTT102 sys-version2 | 15 | 2/0/0 | no recording | |
| YTT103 sys-version-cmp-str3 | 17 | 5/0/0 | no recording | C3 |
| YTT201 sys-version-info0-eq3 | 18 | 4/0/0 | no recording | |
| YTT202 six-py3 | 21 | 2/0/0; review case `case six.PY3:` equal to 0.16.9 | no recording | |
| YTT203 sys-version-info1-cmp-int | 16 | 2/0/0 | no recording | |
| YTT204 sys-version-info-minor-cmp-int | 16 | 2/0/0 | no recording | |
| YTT301 sys-version0 | 15 | 2/0/0 | no recording | |
| YTT302 sys-version-cmp-str10 | 16 | 5/0/0 | no recording | C3 |
| YTT303 sys-version-slice1 | 15 | 2/0/0 | no recording | |

All 20 rules are perfect on both oracles. They total 397 lines, plus 137 lines of library.

### Deviations (group C)

- **C1 (ASYNC115 follows Ruff, not the JSON):** the JSON's `number` util accepts an int or float whose value is 0. Ruff accepts only the int literal 0 (`Number::Int(Int::ZERO)`). Counterexample: Ruff 0.15.7 does not report `await trio.sleep(0.0)`, and the rule does not either.
- **C2 (ASYNC116 float literals):** the core `value` relation has no float rows in rrlab, so a float is compared by `to_float` on its text, with `_` removed.
  - A positive exponent of three significant digits, such as `1e100` or `1e999` (leading zeros aside: `1e010` is read by `to_float`), is reported directly when the mantissa is not 0. Ruff's f64 is then a huge number or `inf`, which is over a day.
  - An exponent like `e-100` is not reported, which is correct.
  - An int over 86400 is tested by its digits. Ruff also reports an int too large for a u64; so does the rule, through six or more digits.
- **C3 (YTT103 and YTT302 string length):** they split at a string length of 1. Ruff measures that length in UTF-8 bytes (`str::len`), and Soufflé's `strlen` also counts bytes, so `sys.version > "é"` is YTT103 in both.
- **Review fixes (Ruff 0.16.9 binary):** ASYNC100 treats a `yield` as a checkpoint but not `yield from` (Ruff's is_yield_expr), so `with trio.fail_after(1): yield from foo()` is reported; ASYNC230 takes the receiver without parentheses, so `(p).open()` is reported; ASYNC116 reads `1e010`; YTT's subscripts take their value without parentheses (`(sys.version)[:1]`). The review's ASYNC and YTT case files now match 0.16.9 line for line, including YTT202's `case six.PY3:` (through `case_value_name` in misc2-case.rr), except ASYNC220 (not in this batch).
- **Seen modules:** YTT202, ASYNC105, ASYNC115 and ASYNC116 test `seen_module(six | trio | anyio)` in Ruff. They need no separate test here, because each one already requires the callee or name to resolve, through an import, to a `six.`, `trio.` or `anyio.` qualified name.
- **No fixes, except ASYNC105:** the JSON has no fixes. ASYNC105 offers Ruff's unsafe "Add `await`" fix as a suggestion, only inside an async def. Ruff's fixes for ASYNC115 (`lowlevel.checkpoint()`) and ASYNC116 (`sleep_forever()`) are not offered: they need an import edit (`get_or_import_symbol`), and the libraries have no generic form of that edit.

### Library and core change requests (group C)

- **The `value` relation has no float rows in rrlab:** core says `value` normalises numbers. A float row, such as a canonical decimal or Python's `repr(float)`, plus a numeric comparison on it would replace ASYNC116's `to_float` text workaround. That is a harness gap, not a core change.
- **Soufflé raises an error on a bad `to_number` or `to_float` instead of failing the row:** this happened on `to_number("18446744073709551616")` and `to_float("1e999")`. Soufflé's join order does not respect a guard written before the conversion in the same body, so rules filter through a relation first (`async_float`). Proposal for core.rr's value-functions section: "to_number and to_float on text they cannot represent give no row (the atom is false), never an evaluation error."

### Duplicates to merge (group C)

- `ytt_sys(e, x)` is `ruff_qual(e, q), split_dot(q, "sys", x)`. Fold it into a general `sys_attr` if another batch adds one.
- `async_name_value(n, v)`: the value of the binding in effect at a name, as Ruff's find_binding_value gives it. pathlib.rr has the same thing only for "only binding" (`only_site`). If another library has a "latest binding value" relation, merge the two.
- `async_ctx(n)` is Ruff's `in_async_context`. It looks like what bugbear and the other async-aware libraries would want, so move it to a shared library if any of them has the same relation.

## Group D: flake8-errmsg (EM), flake8-gettext (INT), flake8-todos (TD), pygrep-hooks (PGH), flake8-self (SLF), flake8-slots (SLOT)

Libraries:

- `lib/python/misc2-messages.rr` (86 lines): a call's first argument and what it builds (str, bytes, f-string,
  `.format`, printf), Ruff's literal expressions, the raised call, typing.cast, and translation calls.
- `lib/python/misc2-todo.rr` (72 lines): Ruff's TodoDirective parse (tag, author, colon, description) and
  issue links in the comment block.
- `lib/python/misc2-noqa.rr` (51 lines): Ruff's noqa lexer (inline and file-wide blankets, file exemptions),
  own-line comments, and blanket `type: ignore`.
- `lib/python/misc2-self.rr` (80 lines): the receiver tests of SLF001.
- `lib/python/misc2-slots.rr` (41 lines): classes without `__slots__` (a parenthesized `(__slots__) = …`
  counts), positional bases (not `metaclass=` or `**kw`), enums.
- `lib/python/misc2-todo-span.rr`: **to delete**, unused. It held the span-anchored TD version (D-D4), which
  rrlab cannot lower; `rm` is blocked here.

Scores are TP/FP/FN at (file, line) against Ruff 0.16.9's default-settings snapshots (plus the option tests
named). Every rule was also diffed against the installed Ruff 0.15.7 on two scratch files of edge cases
(`# foo # FIXME`, `#\tTODO`, `@name` authors, comment blocks with HACK between, `# noqa:` with no codes,
`# type: ignore []`, `raise E(("x"))`, `*args`, concatenated strings, `1 .format()`, `del obj._q`, lambdas and
comprehensions in methods, staticmethod receivers, `*bases`, `if`-nested `__slots__`). They matched, except
one line noted under D-D1.

| rule | lines | 0.16.9 snapshots | deviations |
|---|---|---|---|
| EM101 raw-string-in-exception | 23 | 13/0/0 (defaults, string_exception); 7/0/0 custom (max-string-length 20) | D-D2 |
| EM102 f-string-in-exception | 19 | 3/0/0 | |
| EM103 dot-format-in-exception | 19 | 4/0/0; 4/0/0 custom | |
| INT001 f-string-in-get-text-func-call | 21 | 16/0/0 | D-D1 |
| INT002 format-in-get-text-func-call | 20 | 16/0/0 | |
| INT003 printf-in-get-text-func-call | 20 | 16/0/0 | |
| TD001 invalid-todo-tag | 18 | 3/0/0 | D-D3, D-D4 |
| TD002 missing-todo-author | 20 | 4/0/0 | D-D3, D-D4 |
| TD003 missing-todo-link | 19 | 6/0/0 | D-D3, D-D4 |
| TD004 missing-todo-colon | 18 | 5/0/0 | D-D3, D-D4 |
| TD005 missing-todo-description | 17 | 4/0/0 | D-D3, D-D4 |
| TD006 invalid-todo-capitalization | 19 | 3/0/0 | D-D3, D-D4 |
| TD007 missing-space-after-todo-colon | 17 | 5/0/0 | D-D3, D-D4 |
| PGH003 blanket-type-ignore | 18 | 7/0/0 | |
| PGH004 blanket-noqa | 26 | 9/0/0 (PGH004_0 to _3) | D-D5 |
| PGH005 invalid-mock-access | 35 | 19/0/0 | |
| SLF001 private-member-access | 50 | 12/0/0 (SLF001, _1, _2); 2/0/0 ignore_names (with the option) | D-D6 |
| SLOT000 no-slots-in-str-subclass | 19 | 1/0/0 | |
| SLOT001 no-slots-in-tuple-subclass | 23 | 4/0/0 | |
| SLOT002 no-slots-in-namedtuple-subclass | 24 | 4/0/0 | |

All 20 are perfect.

### Deviations (group D)

- **D-D1 (INT, version difference):** `import gettext; gettext.gettext(f"{x}")` is INT001 in 0.16.9 (its
  snapshot line 22, `gettext_mod.gettext(…)`), not in 0.15.7. The rule follows 0.16.9. A qualified callee
  needs its import before the call at module level, as Ruff resolves names in order there (fixture line 10:
  `gettext_fn(f"…")` before `from gettext import gettext as gettext_fn` is not reported). ngettext's second
  argument is checked only in Ruff's preview, so not here.
- **D-D2 (options added):** the EM and INT JSONs declare no options. The rules read Ruff's, named as the
  other packs name them: `flake8-errmsg.max-string-length` (default 0, a length in UTF-8 bytes as Ruff's
  `len`), `flake8-gettext.function-names` (replaces the default `_`, `gettext`, `ngettext`) and
  `flake8-gettext.extend-function-names`. No fixes: the JSONs have none, and Ruff's EM fix needs a fresh
  binding name (`msg`, else `msg_0`, …).
- **D-D3 (TD word characters):** Ruff's first word is a run of Unicode alphanumerics, and it trims Unicode
  whitespace. Here the tag must be followed by nothing, a space or tab, or ASCII punctuation, so `# TODOé: x` and
  `# TODO2` are not directives, as in Ruff. The one remaining difference is a non-ASCII punctuation mark right
  after the tag (`# TODO：x`): Ruff takes it as a tag and these rules do not. The tag regex is the whole of
  Ruff's section loop: the leftmost `#`, then `#`s and whitespace, then the tag (misc2-todo.rr explains why that
  is the same).
- **D-D4 (TD anchors and TD006 fix):** the TD findings sit on the comment node, so their column is the
  comment's start where Ruff's is the tag's; the line is the same, which is what the findings contract compares.
  TD006's fix (safe, as Ruff's; the JSON has none) replaces the tag alone, as Ruff's does. Since the python merge,
  TD001-TD007 report at the tag (python/misc2-todo-span's `todo_at`, a match capture on the comment, which rrlab
  binds as a span since 08d977aa5), so the column is Ruff's too (TD006.py: 5:3, 6:3, 7:9).
  On the review cases `td.py` and `td2.py` all seven rules equal Ruff 0.16.9 by line and code (60 and 1).
- **D-D7 (file-wide `# ruff: noqa` in rrlab; fixed in rrlab 08d977aa5):** an own-line `# ruff: noqa` (or
  `# flake8: noqa`) silences every other code in the file in Ruff. rrlab honoured line `# noqa` only, so PGH003 and
  the other rules reported on review case `pgh.py` lines 14-21 where Ruff is silent. rrlab now honours the
  file-level forms.
- **D-D5 (PGH004 suppression):** the JSON has no `suppress` for PGH004 and the rule keeps none: Ruff does not
  let a blanket `# noqa` hide its own PGH004 (fixture line 1, `x = 1  # noqa`, is reported). A file whose own-line
  `# ruff: noqa: …` lists PGH004 is exempt (PGH004_3). As Ruff's lexer, the list ends at the first word that is
  not a code: `# ruff: noqa: F401 not PGH004` does not exempt. The message for "codes without a colon" is chosen by a
  regex that approximates Ruff's code lexer; both messages are the same finding key.
- **D-D6 (SLF001 approximations):** Ruff's `pep8-naming.classmethod-decorators`/`staticmethod-decorators` are
  not read (the custom_method_decorators snapshot is not scored); only `builtins.staticmethod` makes a def
  static. Where a name has no binding before it in its own function, Ruff looks it up outside; here only an
  enclosing class of that name is found (fixture line 40, `Foo._private_thing` before `Foo = Bar()`).

### Library and core change requests (group D)

1. **Reporting on part of a node (future).** Ruff reports TD001–TD007 at the tag inside the comment, and
   TD006's fix replaces only the tag. That needs a finding and an edit anchored on a span taken from a node's
   text: a `match` capture bound as a span (core.rr says so; rrlab binds text). With it, the TD rules would
   use `todo_at(c, D)` (the deleted misc2-todo-span.rr: the same tag regex, `D` a span) as their anchor and
   fix `replace(D, `TODO`)`.

### Duplicates to merge (group D)

- `msg_str_lit` (misc2-messages.rr) is a str literal, adjacent ones joined, as pyflakes' `plain_str` without
  the value; `msg_literal` is Ruff's is_literal_expr, close to pyflakes' `py_literal` (which also takes
  f-strings and `-1`).
- `own_line_comment` (misc2-noqa.rr) may exist in another batch's comment library (misc-comments.rr).
- `slots_enum` (misc2-slots.rr) is typing.rr's `class_ancestor` test behind `in_enum`, for a class node.

