# Rerail rule pack: Python

207 code rules for Python, ported from Ruff 0.16.9 (158 rules) and Bandit 1.9.4 (49 tests). Each is scored against the
tool's own findings on the tool's test fixtures, and against the labelled lines of the tool's real-code samples
([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace path, with
every disagreement declared in `divergences.json` (61 today, across 16 rules, 22 of them on six shipped rules: all but one on Ruff's
fixtures and Rerail's probes, and B406 on a Django sample, where Bandit is wrong). Every port honours its tool's
suppression comment, as the tool does: Ruff's `# noqa` (`suppress.noqa`) and Bandit's `# nosec` (`suppress.nosec`);
PGH004, which reports `noqa` comments, is the one exception, as in Ruff (`suppressionExemptions` in
`conformance.json`). 75 Ruff rules that Ruff's default selection leaves off ship off (`"on": false` in `pack.json`).
The conformance check ([conformance/README.md](../../conformance/README.md#the-gate)) gates the shipped rules. The
rename variants that test the ECMAScript packs against fitting to their tests (`holds-under-rename`) do not exist for
Python yet, so the real-code samples are the pack's only such evidence.

This is the reference pack for the [pack structure and validation contract](../../docs/pack-format.md): it is
checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and whether it is on; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), the cases Rerail wrote for it (`fixtures/rerail/`, listed in `UPSTREAM.md`: every shipped rule on lines carrying `# noqa` or `# nosec`, as a stub for rules Ruff reports only in stubs, and D104 exempt, since it reports once per package; and probes of inputs the ports once misread or their reviews pinned), its expected findings (`fixtures.expected.json`), the generator (`generate.py`, sharing `upstream/common.py`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |
| `candidates/rules/` | Ports still being evaluated (434 Ruff and Bandit ports): never shipped or listed in `pack.json`. 433 have a `conformance.json` port and are scored against the expected findings without gating; A005 (GPL-derived fixtures only) has none, and INP001 (tested only with a `namespace-packages` option) has no expected findings. The Conformance comment on a pull request says where each candidate stops |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/python.ts`) runs each tool's `generate.py`
over its fixtures, against the pinned tool versions, under CPython 3.14 (Bandit itself runs from its own install, on
CPython 3.13). The cases Rerail wrote (`fixtures/rerail/`) are authored: a change to them is staged into `publish/` with
the derived data.

```sh
npm run fixtures:derive -- --pack python                # every tool
npm run fixtures:derive -- --pack python --tool ruff    # one
npm run fixtures:stage -- --pack python --tool ruff     # copy what changed into upstream/ruff/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack python --rule ruff-f823-undefined-local --fixtures
```

## Not ported

Every stable Ruff 0.16.9 rule has a document here, shipped or candidate, except Pylint's and refurb's (whose
fixtures, from GPL plugins, are not copied; flake8-builtins' A005 has a candidate document with no fixtures for the
same reason, and its other five rules have none), flake8-bandit's (Bandit's own tests are ported instead) and these 66:

- **Layout a formatter owns** (Ruff's formatter rewrites it, or Ruff's docs call the rule redundant with the
  formatter): D201, D202, D203, D204, D206, D207, D208, D210, D211, E101, E501, E701, E702, E703, W191, W291, W292,
  W293, Q000, Q001, Q002, Q003, Q004, ISC002, COM812, COM819 and I001 (isort's canonical import rendering, with
  first-party sections read from the disk).
- **About Ruff itself, not Python source**: RUF100 and RUF104 report suppressions that silenced nothing, which needs
  every other Ruff rule's results; RUF101, RUF102 and RUF103 check Ruff's own `# noqa` and `# ruff:` comments against
  its rule registry; RUF200 checks pyproject.toml; E902 reports a file Ruff could not read.
- **Nothing to report at default settings**: ICN002, ICN003, TID251 and TID253 report only what a project lists in
  options that are empty by default, W505 only under a `max-doc-length` that is unset by default, I002 only for a
  `required-imports` list that is empty by default, and Ruff never reports F621 (its check is hard-coded off, and
  CPython rejects the case as a syntax error).
- **No fixture to record**: Ruff tests PD003, PD004, PD007, PD008, PD009, PD010, PD011, PD013, PD015 and CPY001 only
  on inline sources in its test modules, so there is nothing to copy, record or gate a port on.
- **Not expressible**: D214 compares a section header's indentation with the column the docstring opens at, which the
  engine does not read; D417 needs each parameter's name compared with the names a docstring section documents, a
  comparison between two nodes' text the code-rule language cannot make.
- **D213**, the opposite of D212 (ported): Ruff drops D213 when both are selected and turns it off under every
  docstring convention.
- **flake8-django** (DJ001, DJ003, DJ006, DJ007, DJ008, DJ012, DJ013) and **flake8-executable** (EXE001 to EXE005):
  Ruff's fixtures for them come from GPL-3.0 and LGPL-3.0 plugins, which licenses/ruff/UPSTREAM.md lists as not
  copied, so there are no upstream tests to gate a port on.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `ruff-async100-cancel-scope-no-checkpoint`

Port of Ruff's ASYNC100: a `with` statement whose last timeout-context item (`asyncio.timeout[_at]`, `anyio.move_on_after`/`fail_after`/`CancelScope`, `trio.move_on_after`/`move_on_at`/`fail_after`/`fail_at`/`CancelScope` calls) is the last item when the statement is `async with`, and whose body has no `yield` (nested scopes included) and no `await`, `async with`, `async for` or async comprehension outside nested functions and classes. Reported on the whole statement. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async105-trio-sync-call`

Port of Ruff's ASYNC105: a call to one of trio's async functions (Ruff's MethodName::is_async list) that is not the operand of an `await` (parentheses ignored), in any context. Reported on the call. Names resolve via `resolves` (M7).

### `ruff-async115-async-zero-sleep`

Port of Ruff's ASYNC115 (async-zero-sleep): a call to `trio.sleep` or `anyio.sleep` with exactly one argument, an int literal whose value is zero (any radix; parentheses allowed) as `seconds=` (trio), `delay=` (anyio) or the first positional argument, in any context, reported on the call. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async116-long-sleep-not-forever`

Port of Ruff's ASYNC116 (long-sleep-not-forever): a call to `trio.sleep` or `anyio.sleep` with exactly one argument, an int or float literal (any spelling; not complex, not signed; parentheses allowed) whose value is greater than 86400 as `seconds=` (trio), `delay=` (anyio) or the first positional argument, in any context, reported on the call. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async210-blocking-http-call-in-async-function`

Port of Ruff's ASYNC210 (blocking-http-call-in-async-function): a call to `urllib.request.urlopen`, `urllib3.request` or an `httpx`/`requests` module-level HTTP method inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async221-run-process-in-async-function`

Port of Ruff's ASYNC221 (run-process-in-async-function): a call to `os.system`, `os.posix_spawn[p]`, `subprocess.run`/`call`/`check_call`/`check_output`/`getoutput`/`getstatusoutput`, or an `os.spawn*` function whose mode (Ruff's find_argument_value("mode", 0): `arguments` in syntactic mode) is absent or `os.P_WAIT` (directly, or through a name whose value, read as find_assigned_value's lookup_symbol does (`resolves.binding` with `stopAtGlobal`), is `os.P_WAIT`) inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async222-wait-for-process-in-async-function`

Port of Ruff's ASYNC222 (wait-for-process-in-async-function): a call to `os.wait`, `os.wait3`, `os.wait4`, `os.waitid` or `os.waitpid` inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async230-blocking-open-call-in-async-function`

Port of Ruff's ASYNC230 (blocking-open-call-in-async-function): a call to the builtin `open` (`builtins.open`, `io.open`, `io.open_code`), or `.open()` on a `pathlib.Path(...)` call or on a name whose binding holds one (Ruff's find_binding_value through `resolves.binding`: a plain name assignment, annotated assignment or walrus value, an unpacked name's element of a tuple or list display, or a `with` item's context expression; and, as Ruff's match_value, an unpacked name paired with a display element by index where the engine gives no value: a set display, unequal arities, a position after a starred value), inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async250-blocking-input-in-async-function`

Port of Ruff's ASYNC250 (blocking-input-in-async-function): a call to the builtin `input` (the unshadowed name, or `builtins.input`) inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async251-blocking-sleep-in-async-function`

Port of Ruff's ASYNC251 (blocking-sleep-in-async-function): a call to `time.sleep` inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-b002-unary-prefix-increment-decrement`

Port of Ruff's B002 (unary-prefix-increment-decrement): a unary `+` whose operand is a unary `+`, or a unary `-` whose operand is a unary `-` (parentheses around the operand are transparent, as in Ruff's AST). Reported on the outer unary expression.

### `ruff-b003-assignment-to-os-environ`

Port of Ruff's B003 (assignment-to-os-environ): a plain assignment statement with exactly one target, that target being the attribute `os.environ` (the name `os`, not resolved; parentheses around the target are transparent). Chained assignments (several targets), annotated and augmented assignments are not checked, as in Ruff. Reported on the attribute.

### `ruff-b004-unreliable-callable-check`

Port of Ruff's B004 (unreliable-callable-check): `hasattr(x, "__call__")` with exactly two positional arguments, or `getattr(x, "__call__")` / `getattr(x, "__call__", default)` with two or three, no keyword arguments, the second argument a str literal (not bytes, an f- or t-string; parentheses transparent) whose value is `__call__` (escapes decoded, implicit concatenations joined). The callee (parentheses transparent) resolves to the builtin `hasattr`/`getattr` (Ruff's `resolve_builtin_symbol`, via `resolves`): the unshadowed name, or that attribute of the `builtins` module through the file's imports. Reported on the call.

### `ruff-b005-strip-with-multi-characters`

Port of Ruff's B005 (strip-with-multi-characters): `.strip`/`.lstrip`/`.rstrip` called with exactly one positional argument that is a str literal (not bytes, an f- or t-string; escapes decoded, implicit concatenations joined) whose value repeats a character, tested with a backreference (`(.).*\1`). Reported on the call.

### `ruff-b007-unused-loop-control-variable`

Port of Ruff's B007 (unused-loop-control-variable) with the default `dummy-variable-rgx`: a name bound by a `for` loop's target (itself, or inside tuple/list/starred targets) that does not match `^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$` and never occurs as a name expression in the loop body (the `else` clause does not count; attribute members, keyword-argument names, def/class names, parameters, import and global/nonlocal names and except-handler names are not name expressions). The message says `may not be used` when the body calls `locals`/`globals`/`vars`/`exec`/`eval` (by spelling; Ruff checks they are builtins). Match-statement capture, keyword-pattern, star and `as` names are not name expressions either. Reported on the target name.

### `ruff-b009-get-attr-with-constant`

Port of Ruff's B009 (getattr-with-constant): `getattr(obj, "name")` with exactly two positional arguments and no keyword arguments, `obj` not starred, the second a str literal (not bytes, an f- or t-string; escapes decoded, implicit concatenations joined) whose value is a Python identifier (Unicode XID, not a keyword) and not a mangled private name (`__x` without a trailing `__`). The callee (parentheses transparent) resolves to the builtin `getattr` (Ruff's `match_builtin_expr`): the unshadowed name, or `getattr` of the `builtins` module through the file's imports. Reported on the call.

### `ruff-b010-set-attr-with-constant`

Port of Ruff's B010 (set-attr-with-constant): `setattr(obj, "name", value)` used as an expression statement, with exactly three positional arguments and no keyword arguments, `obj` not starred, the name a str literal (not bytes, an f- or t-string; escapes decoded, implicit concatenations joined) whose value is a Python identifier (Unicode XID, not a keyword), not `__debug__` and not a mangled private name. The callee (parentheses transparent) resolves to the builtin `setattr` (Ruff's `match_builtin_expr`): the unshadowed name, or `setattr` of the `builtins` module through the file's imports. Reported on the call.

### `ruff-b011-assert-false`

Port of Ruff's B011 (assert-false): an `assert` statement whose test is the literal `False` (Ruff's `is_const_false`; parentheses transparent), reported on the literal. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/B011.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-b013-redundant-tuple-in-exception-handler`

Port of Ruff's B013 (redundant-tuple-in-exception-handler): the type of an `except` (or `except*`) handler is a parenthesized tuple with exactly one element that is not starred. Parentheses around the tuple are transparent, and an `as` name does not matter. Reported on the tuple.

### `ruff-b014-duplicate-handler-exception`

Port of Ruff's B014 (duplicate-handler-exception): the type of an `except` (or `except*`) handler is a tuple in which the same dotted name (a name or an attribute chain of names, Ruff's `UnqualifiedName`) occurs more than once (parentheses around an element are transparent). Reported once on the tuple, whatever the number of duplicates.

### `ruff-b016-raise-literal`

Port of Ruff's B016 (raise-literal): the exception of a `raise` statement is a literal (Ruff's `is_literal_expr`: string, bytes, number, `True`/`False`, `None`, `...`; not an f-string or t-string, including an implicit concatenation with one; parentheses transparent). Reported on the literal.

### `ruff-b020-loop-variable-overrides-iterator`

Port of Ruff's B020 (loop-variable-overrides-iterator): a name in a `for` loop's target that also occurs as a name in its iterable, where comprehensions in the iterable contribute only the iterables of their `for` clauses and lambdas the children of their body (Ruff's `NameFinder` walks the body's children, so `lambda: x` adds nothing). A name that is a (non-variadic) parameter of a lambda in the iterable is exempt unless it occurs again after that lambda, as Ruff's walk removes it and a later occurrence adds it back. Reported on each such target name (Ruff reports only the last occurrence of a name repeated in the target).

### `ruff-b021-f-string-docstring`

Port of Ruff's B021 (f-string-docstring): the first statement of a module, function or class body is an expression statement whose value is an f-string (an `f`-prefixed literal, or an implicit concatenation containing one; parentheses transparent). Comments before it are not statements. Reported on the statement.

### `ruff-b022-useless-contextlib-suppress`

Port of Ruff's B022 (useless-contextlib-suppress): a call to `contextlib.suppress` with no positional arguments, reported on the call. The callee (parentheses transparent) resolves to `contextlib.suppress` with `resolves` (imports and shadowing as Ruff's semantic model sees them).

### `ruff-b023-function-uses-loop-variable`

Port of Ruff's B023 (function-uses-loop-variable): a name loaded in the body of a function or lambda defined inside a loop (for, while, or comprehension), when the function neither takes it as a parameter nor stores it anywhere in its body, and the loop assigns it (assignment, augmented or annotated assignment, for or comprehension target; not inside nested functions or lambdas). The function is the first def or unsafe lambda on the way down from the loop; a lambda is safe when it is returned directly, called immediately, a positional argument of `filter`/`reduce`/`map`/`functools.reduce`, or the value of a `key=` keyword. Reported on the name.

### `ruff-b025-duplicate-try-block-exception`

Port of Ruff's B025 (duplicate-try-block-exception): an exception name (a name or dotted attribute chain) in an `except`/`except*` clause, as the type or a direct element of a tuple type, that an earlier handler of the same `try` already names; within one tuple only its first occurrence counts. Names compare structurally. Reported on the repeated name.

### `ruff-b026-star-arg-unpacking-after-keyword-arg`

Port of Ruff's B026 (star-arg-unpacking-after-keyword-arg): a `*` unpacking argument of a call that comes after the call's first keyword argument (`name=value` or `**mapping`). Reported on the starred argument.

### `ruff-b028-no-explicit-stacklevel`

Port of Ruff's B028 (no-explicit-stacklevel): a call to `warnings.warn` with no `stacklevel` (keyword or third positional argument: find_argument_value, through `arguments` in syntactic mode), no non-empty `skip_file_prefixes`, and no `*`/`**` unpacking. The callee resolves with `resolves` (imports and shadowing as Ruff's semantic model sees them). Reported on the callee, inside any parentheses. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/B028.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-b029-except-with-empty-tuple`

Port of Ruff's B029 (except-with-empty-tuple): an `except`/`except*` clause whose type is the empty tuple `()` (inside any parentheses, with or without `as`). Reported on the whole clause.

### `ruff-b030-except-with-non-exception-classes`

Port of Ruff's B030 (except-with-non-exception-classes): flattens an `except`/`except*` type as Ruff's `flatten_iterables` does (a top-level tuple's elements; `*` unpacking of tuples and lists; both sides of `+`, splicing tuple sides and unpacking starred sides) and reports each resulting expression that is not a name, attribute, subscript or call. Parentheses and an `as` binding (`except E as e`, tree-sitter's as_pattern) are transparent.

### `ruff-b032-unintentional-type-annotation`

Port of Ruff's B032 (unintentional-type-annotation): an annotated statement without a value whose target is a subscript of a name (`d[k]: v`) or an attribute of a name other than `self` (`o.a: v`), inside any parentheses (tree-sitter reads a parenthesized target as a one-element `tuple_pattern`). Reported on the statement.

### `ruff-b033-duplicate-value`

Port of Ruff's B033 (duplicate-value): a literal element of a set display (string, bytes, number, boolean, `None`, `...`, not an f- or t-string; parentheses are transparent) equal to an earlier literal element. Equality is Ruff's HashableExpr (`compare` `text: "structure-hash"`): numbers fold by value (`True == 1 == 1.0 == 0x1`), strings by decoded value (`'a' == "\x61"`, implicit concatenation joined), and an int of 2**64 or more by its token.

### `ruff-b034-re-sub-positional-args`

Port of Ruff's B034 (re-sub-positional-args): a call to `re.sub` or `re.subn` with more than three positional arguments, or to `re.split` with more than two (starred arguments count). The callee resolves with `resolves` (imports and shadowing as Ruff's semantic model sees them). Reported on the call.

### `ruff-b039-mutable-contextvar-default`

Port of Ruff's B039 (mutable-contextvar-default): the `default=` of a `contextvars.ContextVar(...)` call (the callee may be subscripted) is a list, dict or set display or comprehension, or a call to anything other than a function Ruff knows returns an immutable value (`is_immutable_return_type`: `datetime`, `decimal`, `fractions`, `operator`, `pathlib.Path`, `types.MappingProxyType`, `re.compile`, immutable builtins, and Ruff's option `flake8-bugbear.extend-immutable-calls`). Names resolve with `resolves` (imports, and builtins only when unshadowed). Reported on the default, inside any parentheses.

### `ruff-b904-raise-without-from-inside-except`

Port of Ruff's B904 (raise-without-from-inside-except): a `raise X` without `from` in an `except`/`except*` body, reached only through `if`/`elif`/`else`, `with`, `for` and `while` bodies (not their `else`) and `match` cases; nested `try`, `def` and `class` are not entered. `raise name` re-raising the handler's own `as` name is exempt. Reported on the raise statement.

### `ruff-b905-zip-without-explicit-strict`

Port of Ruff's B905 (zip-without-explicit-strict), from target version 3.10 (`when`; `expression.rs:792`): a call to builtin `zip` (bare `zip`, or `zip` from `builtins`) with no `strict=` keyword (find_keyword, through `arguments` in syntactic mode), at least two positional arguments or a `*` argument, and no infinite iterator argument (`itertools.cycle`/`count`, `repeat` without a finite `times`). `zip` and the itertools names resolve with `resolves`, so a shadowed `zip` is not reported. Reported on the call. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/B905.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-e711-none-comparison`

Port of Ruff's E711 (none-comparison) with default settings: `None` compared with `==` or `!=`, reported on the `None` literal (inside any parentheses). The first operand is checked when the second is not a non-singleton constant; each later `None` when the operand before its operator is not one (Ruff's `is_constant_non_singleton`: string, bytes and number literals, and tuples of constants). Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/E711.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-e721-type-comparison`

Port of Ruff's E721 (type-comparison): a comparison with an `==` or `!=` operator whose operand on either side is a type (a call whose callee resolves to the builtin `type` or `builtins.type`, or a name from Ruff's builtin type and exception list that resolves to the builtin) while neither side is a dtype (`x.dtype`, or a call of a callee resolving to `numpy.dtype`), reported on the whole comparison once per offending operator (a chained comparison can report twice, as in Ruff).

### `ruff-e722-bare-except`

Port of Ruff's E722 (bare-except) with default settings: an `except` clause that names no exception type, unless its body re-raises with a bare `raise` statement. Reported on the `except` keyword, as Ruff does. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/E722.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f405-undefined-local-with-import-star-usage`

Port of Ruff's F405 (undefined-local-with-import-star-usage) on the engine's name resolution (M10a `resolves.unresolved: "star"`): a reference, or a string entry of a module `__all__`, that no binding in scope and no builtin resolves and that a visible star import may bind, reported on the name or the string; and such a name inside a string Ruff parses as a type definition (M11a `parse` with `annotation`, nested strings included), reported on each such name inside the string.

### `ruff-f407-future-feature-not-defined`

Port of Ruff's F407 (future-feature-not-defined): a name imported from `__future__` (also through a relative `from .__future__`, matched by the module's text with `resolves.imported` and `relative: "text"`, as Ruff ignores the level) that is not one of CPython's future features (`is_known_future_feature` in ruff_python_parser's semantic_errors.rs), reported on the alias (`name as alias` included) or on the `*` of a star import.

### `ruff-f501-percent-format-invalid-format`

Port of Ruff's F501 (percent-format-invalid-format): a `%`-format string the parser refuses for any reason but an unsupported format character (an incomplete specifier, an unclosed mapping key, a width or precision too big). The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) left of `%`, read with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser).

### `ruff-f502-percent-format-expected-mapping`

Port of Ruff's F502 (percent-format-expected-mapping): a valid `%`-format string with a `%(key)` placeholder applied to a list, tuple or set display, a list or set comprehension or a generator. The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) left of `%`, read with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser).

### `ruff-f503-percent-format-expected-sequence`

Port of Ruff's F503 (percent-format-expected-sequence): a valid `%`-format string consuming more than one positional value (placeholders without a key, and `*` widths and precisions) applied to a dict display or comprehension. The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) left of `%`, read with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser).

### `ruff-f504-percent-format-extra-named-arguments`

Port of Ruff's F504 (percent-format-extra-named-arguments): a valid `%`-format string without positional placeholders applied to a dict display without `**` with a string-literal key no `%(key)` placeholder names. `formatArgs` reads the format string left of `%` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser), and the right operand as Ruff's strings.rs does.

### `ruff-f505-percent-format-missing-argument`

Port of Ruff's F505 (percent-format-missing-argument): a valid `%`-format string without positional placeholders applied to a dict display without `**`, all of whose keys are string literals, that passes no value for some `%(key)` placeholder. `formatArgs` reads the format string left of `%` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser), and the right operand as Ruff's strings.rs does.

### `ruff-f506-percent-format-mixed-positional-and-named`

Port of Ruff's F506 (percent-format-mixed-positional-and-named): a valid `%`-format string with both a `%(key)` placeholder and a positional value (a placeholder without a key, or a `*` width or precision). The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) left of `%`, read with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser).

### `ruff-f507-percent-format-positional-count-mismatch`

Port of Ruff's F507 (percent-format-positional-count-mismatch): a valid `%`-format string without `%(key)` placeholders whose positional values differ in number from a tuple display without starred items, or, with any count but one, from a right operand Ruff's ResolvedPythonType infers as one non-tuple value; with no placeholder, any right operand but a tuple display or one Ruff types as a tuple (`(1,) + (2,)`). `formatArgs` reads the format string left of `%` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser), and the right operand as Ruff's strings.rs does. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F507.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f508-percent-format-star-requires-sequence`

Port of Ruff's F508 (percent-format-star-requires-sequence): a valid `%`-format string with a `*` width or precision applied to a dict display or comprehension. The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) left of `%`, read with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser).

### `ruff-f509-percent-format-unsupported-format-character`

Port of Ruff's F509 (percent-format-unsupported-format-character): a `%`-format string the parser refuses at an unsupported conversion character. The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) left of `%`, read with the `percentFormat` grammar (a port of ruff_python_literal's cformat parser).

### `ruff-f521-string-dot-format-invalid-format`

Port of Ruff's F521 (string-dot-format-invalid-format): a `.format` call on a str literal the format parser refuses (unmatched braces, fields nested deeper than one level, an empty attribute or index, an unclosed `[`, a bad conversion or format spec). The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) receiving `.format`, read with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it).

### `ruff-f522-string-dot-format-extra-named-arguments`

Port of Ruff's F522 (string-dot-format-extra-named-arguments): a `.format` call without `**` on a valid format string, passing a keyword argument no named field (top level or nested in a spec) reads. `formatArgs` reads the format string receiving `.format` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it), and the call's arguments as Ruff's strings.rs does.

### `ruff-f523-string-dot-format-extra-positional-arguments`

Port of Ruff's F523 (string-dot-format-extra-positional-arguments): a `.format` call on a valid format string with a positional argument (not starred; counted by syntactic index, past a starred one too) that no automatic or numbered field reads. `formatArgs` reads the format string receiving `.format` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it), and the call's arguments as Ruff's strings.rs does. A call whose only argument is a bare generator (`"{a}".format(x for x in y)`, with no argument list for `formatArgs` to read) is matched separately: its one positional argument is unused when no field reads index 0. Known divergence: a `.format` call written inside a string annotation (`y: "'{0}'.format(a, b)"`) is not reported, because `formatArgs` is not available inside `parse: annotation`. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F523.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f524-string-dot-format-missing-arguments`

Port of Ruff's F524 (string-dot-format-missing-arguments): a `.format` call without `*` or `**` on a valid format string with a field (top level or nested in a spec) whose index is past the positional arguments or whose name no keyword argument passes. `formatArgs` reads the format string receiving `.format` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it), and the call's arguments as Ruff's strings.rs does. A call whose only argument is a bare generator (`"{1}".format(x for x in y)`, with no argument list for `formatArgs` to read) is matched separately: any field reading an index other than 0, or a name, is missing. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F524.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f525-string-dot-format-mixing-automatic`

Port of Ruff's F525 (string-dot-format-mixing-automatic): a `.format` call on a valid format string with both an automatically numbered field (`{}`) and a manually numbered one (`{0}`), nested fields in specs included. The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) receiving `.format`, read with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it).

### `ruff-f541-f-string-missing-placeholders`

Port of Ruff's F541 (f-string-missing-placeholders) with default settings: every f-string part of a string expression (an implicit concatenation counts as one expression) when no f-string part of that expression has a replacement field. `{{`/`}}` escapes are not placeholders; nested f-strings inside a replacement field or format spec are their own expressions.

### `ruff-f601-multi-value-repeated-key-literal`

Port of Ruff's F601 (multi-value-repeated-key-literal) with default settings: a literal dictionary key (string, bytes, number, boolean, None, Ellipsis, tuple, f-string, signed number, real +/- imaginary; Ruff's is_literal_key) equal to an earlier key of the same dict display, reported on each repeat (parentheses are transparent). Equality is Ruff's HashableExpr (`compare` `text: "structure-hash"`): numbers fold by value (`1 == 1.0 == True`, `1 + 0j == 1`), strings by decoded value, tuples of constants element by element, anything else structurally as ComparableExpr (names without NFKC normalization). Dict displays inside string annotations are checked too, as Ruff does (`parse: annotation`). In a plain annotation the repeat is reported on the inner key. In an escaped or implicitly concatenated annotation it is reported on the whole string, where Ruff's ranges put it, once per string even when Ruff reports several repeats there. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F601.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f602-multi-value-repeated-key-variable`

Port of Ruff's F602 (multi-value-repeated-key-variable) with default settings: a dictionary key that is a bare name equal to an earlier key of the same dict display, reported on each repeat (parentheses transparent).

### `ruff-f622-multiple-starred-expressions`

Port of Ruff's F622 (multiple-starred-expressions, a semantic syntax error) with default settings: a tuple or list assignment target (assignment, `for`, comprehension `for`, `with ... as`), at any nesting, with two or more starred elements of its own. Reported on the target tuple or list, parentheses included.

### `ruff-f632-is-literal`

Port of Ruff's F632 (is-literal) with default settings: `is` or `is not` with an operand that is a non-singleton constant (string, bytes or number literal, tuple of constants) or a list, set or dict display or comprehension, one finding per offending operator reported on the whole comparison, as in Ruff; also a comparison inside a string Ruff parses as a type definition (a string annotation, `cast`/`TypeVar` arguments, `List["..."]`), reported on the comparison inside the string, once per offending operator; strings nested in one (`List['1 is x']` inside a string annotation) are read too. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F632.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f633-invalid-print-syntax`

Port of Ruff's F633 (invalid-print-syntax) with default settings: the left operand of `>>` when it resolves to the builtin `print` (or `builtins.print`), reported on `print`. tree-sitter-python parses a statement `print >> f, x` as a Python 2 print statement whose `print` is a keyword token, not a name reference, so name resolution cannot be asked there; that form falls back to a file-wide test: no binding of `print` anywhere in the file (assignment, loop, with/except and walrus targets, def/class names, parameters, imports, global/nonlocal).

### `ruff-f701-break-outside-loop`

Port of Ruff's F701 (break-outside-loop, a semantic syntax error) with default settings: `break` with no enclosing `for`/`while` body between it and the nearest function or class definition. A loop's `else` block does not count as inside that loop.

### `ruff-f702-continue-outside-loop`

Port of Ruff's F702 (continue-outside-loop, a semantic syntax error) with default settings: `continue` with no enclosing `for`/`while` body between it and the nearest function or class definition. A loop's `else` block does not count as inside that loop.

### `ruff-f706-return-outside-function`

Port of Ruff's F706 (return-outside-function, a semantic syntax error) with default settings: `return` whose nearest enclosing function or class body is not a function body (module level, or directly in a class body).

### `ruff-f707-default-except-not-last`

Port of Ruff's F707 (default-except-not-last) with default settings: a bare `except:` handler followed by another handler of the same `try`. Reported on the `except` keyword.

### `ruff-f822-undefined-export`

Port of Ruff's F822 (undefined-export) with default settings, on the engine's name resolution (M10a `resolves.unresolved` on `__all__` entries): a string entry of a static module `__all__` definition (Ruff's extract_dunder_all_names) (an implicit concatenation is one entry) that names no module binding (a bare annotation included) and no builtin, in a module without a star import. `__init__.py` is exempt (Ruff reports it only in preview). Reported on the string. A parenthesized string element (`[("a")]`, `((('a')))`) and the elements of a bare tuple value (`__all__ += "a", "b"`, `__all__ = ("a"), "b"`) are entries, as in Ruff's parenthesis-free AST.

### `ruff-f823-undefined-local`

Port of Ruff's F823 (undefined-local) with default settings, on the engine's name resolution (M10a `resolves.beforeLocal`, Ruff's undefined_local): a function's first read of a name that resolves to an enclosing function's or the module's binding which the function's own later binding of the name shadows, where that binding is the function's first and latest of the name (not `global`, `nonlocal` or `del`). Reported on the read. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F823.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f842-unused-annotation`

Port of Ruff's F842 (unused-annotation) with default settings (`dummy-variable-rgx` = `^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$`), on the engine's usedness (M10a `binds` with form `annotation`, Ruff's unused_annotation): the latest binding of a name in a function or lambda scope that is a bare annotation (`x: int`, `(x): int`) nothing reads, not a dummy name. Names in strings Ruff parses as type definitions are reads (M11a).

### `ruff-f901-raise-not-implemented`

Port of Ruff's F901 (raise-not-implemented) with default settings: the exception of a `raise` (not its `from` cause) that is the name `NotImplemented` or a call whose callee is that name, parentheses transparent. Reported on the name. Ruff matches the name itself, not the builtin binding.

### `ruff-pie794-duplicate-class-field-definition`

Port of Ruff's PIE794 (duplicate-class-field-definition): in a class body, an assignment or annotated assignment to a name `N` whose value does not mention `N`, after an earlier such definition of `N` in the same body.

### `ruff-pie796-non-unique-enums`

Port of Ruff's PIE796 (non-unique-enums): in a class with an `enum.Enum` base, a plain assignment whose value equals the value of an earlier assignment in the same body as Ruff's ComparableExpr (`text: "structure"`: literals by type and value, `1 == 0x1`, `'a' == "a"`, `-1 == -(1)`, not `1 == 1.0 == True`; names compared without NFKC normalization); `enum.auto()` values (alone or in a tuple) are skipped, and in stubs (`.pyi`) so are `...` and `typing.cast(T, ...)`. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/PIE796.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it, and its `.py`/`.pyi` split read per file (`ast.file.path`): the suite's `select_when` gate holds wherever a matching file exists anywhere in the workspace, so in a repository it would read stubs as modules and modules as stubs.

### `ruff-pie800-unnecessary-spread`

Port of Ruff's PIE800 (unnecessary-spread): a dict display spread with `**` into another dict display, reported on the inner dict (inside any parentheses).

### `ruff-pie804-unnecessary-dict-kwargs`

Port of Ruff's PIE804 (unnecessary-dict-kwargs): a call argument `**{...}` whose dict display is a single `**x` spread, empty, or has only keys that are str literals (not bytes, f- or t-strings; escapes decoded, implicit concatenations joined) whose value is a Python identifier (not a keyword). Reported on the `**` argument.

### `ruff-pie807-reimplemented-container-builtin`

Port of Ruff's PIE807 (reimplemented-container-builtin): a parameterless `lambda` whose body is an empty list or dict display.

### `ruff-pie810-multiple-starts-ends-with`

Port of Ruff's PIE810 (multiple-starts-ends-with): an `or` chain with two or more operands `R.startswith(x)` (or two `R.endswith(x)`) on the same name `R`, each with one positional argument and no keywords, where the argument is not a name bound to a tuple (Ruff's is_bound_to_tuple through `resolves.binding`, read as lookup_symbol does with `stopAtGlobal`: a `def`'s `*args`, a tuple-typed value, `tuple(...)`, or a `tuple`/`Tuple` annotation or return annotation). Reported on the whole chain, once per such group.

### `ruff-ret501-unnecessary-return-none`

Port of Ruff's RET501 (unnecessary-return-none): `return None` in a function whose every `return` is bare or returns `None` (nested functions and classes are separate; generators and functions whose body is a lone `return` are skipped), unless the function has a return annotation other than `None` or a property decorator (`property`, `enum.property`, `functools.cached_property`, `abc.abstractproperty`, `types.DynamicClassAttribute`, resolved through the file's bindings and imports). Reported on the `return` statement.

### `ruff-ret502-implicit-return-value`

Port of Ruff's RET502 (implicit-return-value): a bare `return` in a function that also has a `return` with a value other than `None` (nested functions and classes are separate; generators and functions whose body is a lone `return` are skipped), reported on the `return`.

### `ruff-ret505-superfluous-else-return`

Port of Ruff's RET505 (superfluous-else-return): the first `elif`/`else` clause of an `if` statement inside a function whose `if` body has a direct `return` statement and no earlier direct raise/continue/break statement, reported on the `elif`/`else` keyword. As in Ruff, only the `if` body and the first clause are compared, nested functions and classes are separate, and generator functions and functions whose body is a lone `return` are skipped.

### `ruff-ret506-superfluous-else-raise`

Port of Ruff's RET506 (superfluous-else-raise): the first `elif`/`else` clause of an `if` statement inside a function whose `if` body has a direct `raise` statement and no earlier direct return/continue/break statement, reported on the `elif`/`else` keyword. As in Ruff, only the `if` body and the first clause are compared, nested functions and classes are separate, and generator functions and functions whose body is a lone `return` are skipped.

### `ruff-ret507-superfluous-else-continue`

Port of Ruff's RET507 (superfluous-else-continue): the first `elif`/`else` clause of an `if` statement inside a function whose `if` body has a direct `continue` statement and no earlier direct return/raise/break statement, reported on the `elif`/`else` keyword. As in Ruff, only the `if` body and the first clause are compared, nested functions and classes are separate, and generator functions and functions whose body is a lone `return` are skipped.

### `ruff-ret508-superfluous-else-break`

Port of Ruff's RET508 (superfluous-else-break): the first `elif`/`else` clause of an `if` statement inside a function whose `if` body has a direct `break` statement and no earlier direct return/raise/continue statement, reported on the `elif`/`else` keyword. As in Ruff, only the `if` body and the first clause are compared, nested functions and classes are separate, and generator functions and functions whose body is a lone `return` are skipped.

### `ruff-sim101-duplicate-isinstance-call`

Port of Ruff's SIM101 (duplicate-isinstance-call): two adjacent operands of one `or` chain that are calls of the builtin `isinstance(T, C)` (exactly two positional arguments, no keywords) with equal targets (Ruff's ComparableExpr, `text: "structure"`; names without NFKC normalization), reported on the whole `or` chain once per run of adjacent such calls with equal targets. A parenthesized `or` is its own chain, as in Ruff's AST. `isinstance` must resolve to the builtin (`match_builtin_expr`: the unshadowed name, or `builtins.isinstance` through an import).

### `ruff-sim107-return-in-try-except-finally`

Port of Ruff's SIM107 (return-in-try-except-finally): the first top-level `return` of a `finally` body when the `try` body or an `except` handler body has a top-level `return`.

### `ruff-sim109-compare-with-tuple`

Port of Ruff's SIM109 (compare-with-tuple): an `or` chain with two or more operands `N == name` (one `==`, both sides names) for the same `N`, anywhere in the chain; not reported when a comment lies inside the chain or follows it on its last line with only white space between (Ruff's has_comments). Reported on the whole `or` chain, once per such name.

### `ruff-sim112-uncapitalized-environment-variables`

Port of Ruff's SIM112 (uncapitalized-environment-variables): a str literal (not bytes, an f- or t-string; escapes decoded, implicit concatenations joined) whose value has an ASCII lowercase letter, other than `https_proxy`/`http_proxy`/`no_proxy`, used as `os.environ[...]` or as the first argument of a call resolving to `os.environ.get`/`os.getenv`, after Ruff's traversal has visited an absolute import of `os` (M10a `resolves.seenBefore`). Reported on the string.

### `ruff-sim113-enumerate-for-loop`

Port of Ruff's SIM113 (enumerate-for-loop): `N += 1` as a top-level statement of a non-async `for` body without `continue`, where the binding `N` reads (`resolves.binding`) is a name assignment of `0` (plain, annotated, or an unpacked element of a display, paired by index as Ruff's match_value where the engine gives no value) whose parent statement is the loop's, `N` is bound only once inside the loop (`binds` with `bindings` within the loop: deletions and comprehension walruses count, comprehension targets, nested functions' locals and keyword arguments do not), and no load of `N` outside the loop in its scope, at or after the initial assignment, reads the initial binding (whose references Ruff's rebinding inherits) or a binding inside the loop. The loop and the initial assignment are found again by node identity (`compare` with `text: "node"`), so identical loops in one scope stay apart.

### `ruff-sim208-double-negation`

Port of Ruff's SIM208 (double-negation): `not` applied to another `not` expression, through any parentheses, reported on the outer `not` expression (each outer `not` of a longer chain).

### `ruff-sim210-if-expr-with-true-false`

Port of Ruff's SIM210 (if-expr-with-true-false): a conditional expression whose body is the literal `True` and whose `else` arm is `False` (through any parentheses), reported on the whole expression. Ruff's message and fix differ when the test is a comparison or `bool` is shadowed; what is flagged does not.

### `ruff-sim211-if-expr-with-false-true`

Port of Ruff's SIM211 (if-expr-with-false-true): a conditional expression whose body is the literal `False` and whose `else` arm is `True` (through any parentheses), reported on the whole expression.

### `ruff-sim212-if-expr-with-twisted-arms`

Port of Ruff's SIM212 (if-expr-with-twisted-arms): `B if not A else A` where the negated test operand and the `else` arm are the same plain name (through any parentheses), reported on the whole expression.

### `ruff-sim300-yoda-conditions`

Port of Ruff's SIM300 (yoda-conditions): a comparison with one `==`, `!=`, `<`, `<=`, `>` or `>=` whose left operand is more constant-like than its right, on Ruff's three levels (`ConstantLikelihood`, flake8_simplify/rules/yoda_conditions.rs:76-136): literals (not f- or t-strings) are definite; a name or attribute whose name is cased uppercase is probable; tuples, lists and dicts (keys and values, `**` values), binary operations and unary `+`/`-`/`~` take the least of their parts (empty is definite); everything else, sets included, is unlikely. Reported on the comparison.

### `ruff-sim905-split-static-string`

Port of Ruff's SIM905 (split-static-string): `.split(...)` or `.rsplit(...)` called on a str literal (implicit concatenations included; not bytes, f- or t-strings), where `sep` (keyword, else first positional argument) is absent, `None` or a str literal, and `maxsplit` (keyword, else second positional argument) is absent or an integer literal, optionally negated, that fits in an i32 (`get_maxsplit_value`). Reported on the call.

### `ruff-sim910-dict-get-with-none-default`

Port of Ruff's SIM910 (dict-get-with-none-default): `.get(key, None, ...)` whose second positional argument is `None`, with no keywords, called on a dict display or dict comprehension (through parentheses), or on a name whose binding is a dict (Ruff's `is_dict` through `resolves.binding`: a `def`'s `**kwargs`, a dict-typed value or `dict(...)`, a `dict`/`Dict` annotation or return annotation; unpacking carries no value).

### `ruff-t100-debugger`

Port of Ruff's T100 (debugger): imports of debugger modules (`is_debugger_import`) and of debugger functions (`from M import n` with `M.n` in `is_debugger_call`), once per imported name, matched by module with `resolves.imported`, relative imports by the module's text (`relative: "text"`: Ruff ignores the level). It also flags calls whose callee resolves to a debugger function (`resolve_qualified_name` through the file's imports, or the builtin `breakpoint`).

### `bandit-b101-assert-used`

Port of Bandit 1.9.4's B101 assert_used with its default configuration, which skips no file (tests included; scope is every file).

### `bandit-b102-exec-used`

As in Bandit, the callee is a bare name whose qualified name through the file's import aliases is `exec`: `exec` itself unless an import aliases it to another name, or a name imported as the module `exec` (`import exec as x`). `from m import exec as y` does not count.

### `bandit-b113-request-without-timeout`

Port of Bandit 1.9.4's B113 request_without_timeout: a `requests.get/options/head/post/put/patch/delete` call without `timeout`, or a `requests`/`httpx` call (httpx also covers `request`, `stream`, `Client`, `AsyncClient`) whose `timeout` keyword is `None`. Bandit reads `timeout` as a keyword only. httpx calls without `timeout` are not flagged (httpx has a default). Port of the kernel test b113_requests_without_a_timeout_or_with_none (rerail/native/src/code_rule_dsl/value/rules.rs).

### `bandit-b303-md5`

Covers PyCrypto/PyCryptodome `MD2/MD4/MD5/SHA.new` and cryptography `hashes.MD5/SHA1`. `hashlib` is Bandit's B324, which the pack does not ship yet.

### `bandit-b307-eval`

Same alias resolution as B102: a bare `eval` unless an import rebinds it, or a name bound by `import eval as x`.

### `bandit-b317-xml-bad-sax`

Only matches `xml.sax.parse/parseString/make_parser` calls that are direct children of an expression statement.

### `bandit-b505-weak-cryptographic-key`

Port of Bandit 1.9.4's B505 weak_cryptographic_key with Bandit's medium thresholds: DSA/RSA below 2048 bits or an EC curve below 224 bits (SECP192R1, SECT163K1, SECT163R2), by keyword or position, across cryptography, Crypto and Cryptodome. Port of the kernel test b505_weak_key_sizes_by_keyword_or_position. `# nosec` honoured.

### `bandit-b507-ssh-no-host-key-verification`

Only fires once an import whose name contains "paramiko" has been walked (Bandit's substring test, in Bandit's source-order walk). Flags a call whose function name is set_missing_host_key_policy (attribute name, bare name, or a name aliased by an earlier import to one ending in set_missing_host_key_policy) and whose first positional argument is a name, attribute or call naming AutoAddPolicy or WarningPolicy.

### `bandit-b615-huggingface-unsafe-download`

Requires a transformers, datasets or huggingface_hub-like import before the call. Allowed (not flagged): nonliteral revisions, hexadecimal revisions of at least seven characters, and local-path first arguments; all other unpinned downloads are flagged.

### `bandit-b103-set-bad-file-permissions`

Any call whose callee name contains `chmod`, with a literal mode. World-write (0o002) is reported by its own clause (higher severity in Bandit); group-write, group-execute or world-execute (0o031 bits) by the other.

### `bandit-b406-import-xml-sax`

Port of Bandit 1.9.4's B406 import_xml_sax. Divergence: an import of only xml.sax.saxutils (`from xml.sax.saxutils import ...`, `from xml.sax import saxutils`, aliased or parenthesised) is not reported. saxutils holds XML writing and escaping helpers (escape, quoteattr, unescape, XMLGenerator) and no parser, so defusedxml has nothing to replace it with. `import xml.sax.saxutils` is still reported, since it also binds xml.sax and its parse functions.

### `bandit-b610-django-extra-used`

Port of Bandit 1.9.4's B610 django_extra_used: a call of `extra` whose `where` or `tables` is not a list of plain strings, or whose `select` is not a dict of plain strings, bound to QuerySet.extra's six positional slots. Port of the kernel test b610_extra_with_a_non_literal_where_tables_or_select. `# nosec` honoured. Divergence: Bandit reports a call of any function or method named `extra`; the port reports only a method call whose receiver looks like a Django QuerySet or Manager: a name or attribute named `objects`, `qs`, `QS`, `queryset`, `querysets`, `manager` or `managers`, alone or after an underscore-joined prefix (`User.objects`, `self.queryset`, `base_qs`, `_default_manager`); a call of `get_queryset`; a method call on such a receiver (`User.objects.filter(...)`); or `self` or `super()` inside a class with a base whose name ends in `QuerySet` or is `Manager`, `BaseManager` or `BaseUserManager`, optionally dotted (`models.QuerySet`); a base that only ends in `Manager` (`AbstractContextManager`, `PluginManager`, `TreeManager`) does not count. A bare `extra(...)` call and `extra` on any other receiver are not reported.

### `ruff-air302-airflow3-moved-to-provider`

Port of Ruff's AIR302 (airflow3-moved-to-provider): reports each read of one of the 182 deprecated Airflow core names that Airflow 3.0 moved into a provider package. Names resolve through plain imports, aliases (including `import a.b.Name as X`), module aliases and attribute chains, as Ruff's qualified-name resolution does. A name is reported at the name, an attribute at its last segment. Attribute stores, deletes and loop targets are reported, as in Ruff. A bare-name store, `del` target or augmented-assignment target is not. Names inside string type annotations (`def f(h: "PostgresHook")`, nested strings, `cast("X", ...)`) are reported at the name inside the string. There are 89 clauses, one per provider, version and replacement name. Each message names the provider package to install and the import to use. A use is not reported when it sits in the except fallback of a try whose body imports or reads the replacement. Name uses need an ImportError or ModuleNotFoundError handler and attribute uses an AttributeError handler, in any handler of the nearest try. As in Ruff, a nested tuple of exception types does not count. The replacement is compared as Ruff compares it: the exact module and name for an import, the exact qualified name for an attribute. The 2 FAB modules that share names are checked against their own module. Divergences: dotted match patterns (`case m.S3Hook():`) are not reported. A guarded name used inside a string annotation is still reported. Very large files can exceed the engine's work budget. The document is about 234 KiB pretty-printed: the 182-name table, 89 clauses each with its own string-annotation reader, and per-replacement guard utilities. The compact gate passes.

### `ruff-air311-airflow3-suggested-update`

Port of Ruff's AIR311 (airflow3-suggested-update): reports Airflow names that Airflow 3.0 deprecated but still supports and that have a newer equivalent. These are Dataset, DatasetAlias, DatasetAll, DatasetAny and DatasetOrTimeSchedule, renamed to the Asset family; the airflow.sdk moves of DAG, dag, task, Variable, Connection, chain, BaseOperatorLink, BaseSensorOperator and so on; the airflow.sdk.bases, airflow.sdk.io and airflow.sdk.definitions.param homes; and expand_alias_to_datasets. Names resolve through imports, aliases (including `import a.b.Name as X`) and attribute chains, as Ruff does. Param, ParamsDict and DagParam match at any depth under airflow.models, as Ruff's `..` pattern does. A bare-name `del` or augmented-assignment target is not reported. Names inside string type annotations are reported at the name inside the string. A name is not reported in the except fallback of a try whose body imports or reads the replacement, with the module and name compared exactly. Name uses need an ImportError or ModuleNotFoundError handler and attribute uses an AttributeError handler. A nested tuple of exception types does not count. DatasetAliasEvent has no replacement and no guard. Keyword clauses report the first matching keyword's name, on a callee resolved through parentheses and aliases: `sla_miss_callback` on a call to any `airflow.**.DAG` or `dag`; `datasets` on DatasetOrTimeSchedule (rename it to `assets`); and `sla` on `airflow.operators.**.*Operator` or `airflow.providers.**.operators.**.*Operator`. The `operators` segment is read from the import's module or name and from the attribute chain, not from an alias or the statement text. The sla keywords have no replacement, so the message says so. Divergences: dotted match patterns (`case ds.DatasetAny():`) are not reported. An aliased Param-family name inside a string annotation is missed. A guarded name inside a string annotation is still reported. The document is about 58 KiB, mostly per-replacement guard utilities. The compact gate passes.

### `ruff-air312-airflow3-suggested-to-move-to-provider`

Port of Ruff's AIR312 (airflow3-suggested-to-move-to-provider): it reports a loaded name, an attribute chain, or a name or attribute chain inside a string type annotation (nested strings such as `"list['X']"` included) when it resolves through the file's imports to one of the 31 core Airflow names Ruff lists as moved to apache-airflow-providers-standard (the `airflow.hooks.*`, `airflow.operators.*`, `airflow.sensors.*` and `airflow.triggers.*` names in its table). Like Ruff's `seen_module(AIRFLOW)` gate, it reports only in a file with an absolute import of `airflow` or one of its submodules. Aliases and shadowing follow the scope table. A name is reported on itself and an attribute chain on its last name, which is Ruff's `attr` range. A plain name used as a store or delete target is not reported (`del X`, `X += 1`, also through parentheses, tuples and lists). A store or delete through an attribute is reported, as in Ruff. Ruff's `is_guarded_by_try_except` is modelled as follows. A name is skipped when all of these hold. The binding it reads is an import inside an except handler (a def or class body inside the handler clears that, as Ruff clears the flag there; a lambda does not). The nearest try around that import catches builtin ImportError or ModuleNotFoundError, directly, parenthesized, or in a top-level tuple, with or without `as`. A bare `except:`, `except Exception`, a nested tuple or `except A or B` does not count. That try's body imports the same real name with `from ... import` from Ruff's mapped provider module. The import can be nested at any depth in the body, but not in the try's else or finally or in another handler. The real name of `import a.b.X as Y` is `X`. The module is matched by text, so relative dots are ignored. The mapping keeps Ruff's irregular `airflow.providers.standard.sensor.bash` and `sensors.time` targets. An attribute chain, in code or in a string annotation, is skipped when it sits in such a handler, the nearest try catches AttributeError, and that try's body reaches the same name as an attribute chain through the provider path. A name in a string annotation is a forward reference, so, like Ruff's global-first lookup, it is skipped when the module's live binding of the name is such a guarded import. When the module does not import the name, it is skipped when such a guarded import is the live binding of an enclosing function, or of the class whose body holds the annotation (a method's parameter annotations included). Divergences: the airflow-import gate does not depend on order, while Ruff counts only the imports it has visited before the check. A dotted class or value in a match pattern (`case bash.BashOperator():`) is not reported. In a string annotation that lists several deprecated names, a guarded name hides the unguarded names after it. For a string annotation inside a function, a guarded import in a function nested inside that function counts as if it were in the enclosing function, and so does one with a shadowing binding between it and the annotation.

### `ruff-ann002-missing-type-args`

Port of Ruff's ANN002 (missing-type-args): reports an unannotated `*args` parameter (any name, including a bare `*` followed by a comment and the name on the next line), anchored on the starred parameter, in functions, methods and nested functions; a bare `*` keyword-only marker and lambdas are not reported. A method's receiver skip does not apply to `*args` (`def only_star(*args)` in a class is reported). The implementation following `@overload` stubs of the same name (typing, typing_extensions or _typeshed, through compound-statement blocks and the tail of a preceding class body) is skipped, while the stubs themselves are checked. Divergences: a nested function that is the first definition of its body, preceded in Ruff's breadth-first definition order by a same-named `@overload` elsewhere, is reported where Ruff skips it; an `@overload` used before its module-level import is not recognised. Default settings only (allow-star-arg-any and suppress-dummy-args are not modelled).

### `ruff-ann003-missing-type-kwargs`

Port of Ruff's ANN003 (missing-type-kwargs): reports an unannotated `**kwargs` parameter (any name, with spacing or comments after the `**`), anchored on the double-starred parameter, in functions, methods and nested functions; lambdas are not reported. A method's receiver skip does not apply to `**kwargs`. The implementation following `@overload` stubs of the same name (typing, typing_extensions or _typeshed, through compound-statement blocks and the tail of a preceding class body) is skipped, while the stubs themselves are checked. Divergences: a nested function that is the first definition of its body, preceded in Ruff's breadth-first definition order by a same-named `@overload` elsewhere, is reported where Ruff skips it; an `@overload` used before its module-level import is not recognised. Default settings only (allow-star-arg-any and suppress-dummy-args are not modelled).

### `ruff-ann201-missing-return-type-undocumented-public-function`

Port of Ruff's ANN201 (missing-return-type-undocumented-public-function): reports the name of a public function or method that has no return annotation. Public means: the module is public (`file.module` has no dotted component starting with `_` unless it is dunder-wrapped; outside a package the file stem decides), the function name does not start with `_`, it is not nested in another function, and its enclosing classes are all public and not nested in functions; a top-level function or outermost class must also be listed when the module declares `__all__`. A module declares `__all__` when, outside any function or class, `__all__` (optionally parenthesized) is the first target of a plain or annotated assignment (annotation alone included) or the target of an augmented assignment. `x = __all__ = [...]`, tuple or list unpacking, walrus, `for` targets and `global __all__` in a function declare nothing, and `.extend`/`.append`/`.remove` neither declare nor list names. A name is listed when a string literal, optionally parenthesized, spells it among the elements Ruff extracts from such a declaration's value: list, tuple or bare tuple elements; `list(...)`/`tuple(...)` (also subscripted, builtin only) with exactly one positional list, set or tuple argument and no keywords; the value of a walrus; any operand of a left-nested binary-operator chain; and the final value of a chained assignment. Non-string elements, starred elements and other calls add nothing, so their names are unlisted (as in Ruff). Static and class methods belong to ANN205 and ANN206 and are skipped. Ruff resolves those decorators as follows: when the file never imports `builtins`, the bare name counts unless the module binds that name anywhere, even after the class (an annotation without a value or an `except ... as` name does not count), because Ruff looks it up in the module's final state; function- or class-level rebindings do not count. When the file imports `builtins`, the decorator must resolve at its site to the builtin or `builtins.<name>`. `__init__` and other dunder methods (ANN204) are also skipped. Property setters and deleters (`@name.setter`/`@name.deleter` naming the method itself, with any parentheses around `name` or the whole decorator) are private. The implementation following `@overload` stubs of the same name is skipped, while the stubs themselves are reported; stubs, abstract methods and empty bodies are still reported (only Ruff's autofix differs). Private-module behaviour is covered by local probes only (recorded probe names cannot start with an underscore). Divergences: an implicitly concatenated string in `__all__` is not read as one name; an `@overload` used before its module-level import is not recognised; a file the Python grammar cannot read (Ruff's flake8_bugbear/B012.py fixture, whose `match *0, 1, *2:` subject turns 99% of the file into error regions) is skipped, so its findings are missed. Default settings only (suppress-none-returning, ignore-fully-untyped and mypy-init-return are not modelled).

### `ruff-ann202-missing-return-type-private-function`

Port of Ruff's ANN202 (missing-return-type-private-function): reports the name of a private function or method without a return annotation. Private means any of the following: a nested function (any function inside a function); a module-level function whose name starts with `_` or that is not listed in a declared `__all__`; a method whose name starts with `_` but is not dunder-wrapped (dunder methods are ANN204; `_`, `__x_` and `_x__` are private); a method of a private class (name starting with `_`, nested in a function, inside a private class, or an outermost class unlisted by `__all__`); a property setter or deleter naming the method itself (parentheses around the name or the whole decorator allowed); and every function and method of a private module (`file.module` has a dotted component starting with `_` that is not dunder-wrapped; outside a package the file stem decides). The private-module case is the second clause, which shares its utilities with the first through a `define`. A module declares `__all__` when, outside any function or class, `__all__` (optionally parenthesized) is the first target of a plain or annotated assignment (annotation alone included) or the target of an augmented assignment. `x = __all__ = [...]`, tuple or list unpacking, walrus, `for` targets and `global __all__` in a function declare nothing, and `.extend`/`.append`/`.remove` neither declare nor list names. A name is listed when a string literal, optionally parenthesized, spells it among the elements Ruff extracts from such a declaration's value: list, tuple or bare tuple elements; `list(...)`/`tuple(...)` (also subscripted, builtin only) with exactly one positional list, set or tuple argument and no keywords; the value of a walrus; any operand of a left-nested binary-operator chain; and the final value of a chained assignment. Non-string elements, starred elements and other calls add nothing, so their names are unlisted (as in Ruff). Static and class methods belong to ANN205 and ANN206 and are skipped. Ruff resolves those decorators as follows: when the file never imports `builtins`, the bare name counts unless the module binds that name anywhere, even after the class (an annotation without a value or an `except ... as` name does not count), because Ruff looks it up in the module's final state; function- or class-level rebindings do not count. When the file imports `builtins`, the decorator must resolve at its site to the builtin or `builtins.<name>`. The implementation following `@overload` stubs of the same name is skipped, while the stubs themselves are reported. Divergences: a nested function that is the first definition of its body, preceded in Ruff's breadth-first definition order by a same-named `@overload` elsewhere, is reported where Ruff skips it; a local `staticmethod`/`classmethod` rebinding in the scope of the file's last lambda, loop or comprehension, which Ruff applies file-wide, is ignored; an implicitly concatenated string in `__all__` is not read as one name; an `@overload` used before its module-level import is not recognised; a file the Python grammar cannot read (Ruff's flake8_bugbear/B012.py fixture, whose `match *0, 1, *2:` subject turns 99% of the file into error regions) is skipped, so its findings are missed.

### `ruff-ann204-missing-return-type-special-method`

Port of Ruff's ANN204 (missing-return-type-special-method): reports the name of a `def` that sits directly in a class body (also inside `if`/`try`/`for`/`while`/`with`/`match` blocks of that body) when it has no `->` return annotation and is named `__init__` or any other name that starts and ends with two underscores (so `__`, `___`, `__str__` and `__eq__` count, `__x__y` and `x__` do not). Matches Ruff's default settings: `__init__` is reported even when its arguments are typed. Functions decorated with the builtin `classmethod` or `staticmethod` are left to the class-method and static-method rules, as Ruff checks those two first. The builtins are recognised as Ruff's match_builtin_expr does: when the file never imports `builtins`, a bare name is the builtin unless the module scope binds it (a module-level annotation without a value and an `except ... as` name do not count, as Ruff skips the first and restores the builtin after the second; class-scope and function-scope bindings do not count either); when the file imports `builtins` anywhere, names resolve lexically, so `builtins.x`, aliases and class- or function-scope rebindings are honoured. The implementation `def` that follows an `@overload` set (typing, typing_extensions or _typeshed overload, through any import alias) is skipped, judged in Ruff's definition order (a nested class's last overload counts, a nested function's does not; any other def or class in between ends the run). Nested functions, functions in method bodies and lambdas are not methods. Works on .py and .pyi. Divergence: Ruff reads the bare decorator names from the scope of the last lambda it analysed, so a `staticmethod`/`classmethod` binding around that lambda changes Ruff's verdict file-wide; the document reads the module scope.

### `ruff-ann205-missing-return-type-static-method`

Port of Ruff's ANN205 (missing-return-type-static-method): reports the name of a `def` that sits directly in a class body (also inside `if`/`try`/`for`/`while`/`with`/`match` blocks of that body), has no `->` return annotation and is decorated with the builtin `staticmethod`, whatever its name (dunder names included, e.g. a static `__new__`). A method that is also decorated with the builtin `classmethod` is not reported, because Ruff checks the class-method rule first. The builtins are recognised as Ruff's match_builtin_expr does: when the file never imports `builtins`, a bare name is the builtin unless the module scope binds it (a module-level annotation without a value and an `except ... as` name do not count, as Ruff skips the first and restores the builtin after the second; class-scope and function-scope bindings do not count either); when the file imports `builtins` anywhere, names resolve lexically, so `builtins.x`, aliases and class- or function-scope rebindings are honoured. The implementation `def` that follows an `@overload` set of the same name (typing, typing_extensions or _typeshed overload, any alias) is skipped, judged in Ruff's definition order. Functions outside classes (nested functions, functions in method bodies) are not methods and are not reported. Works on .py and .pyi. Default settings only (`suppress-none-returning` off). Divergence: Ruff reads the bare decorator name from the scope of the last lambda it analysed, so a `staticmethod` binding around that lambda (a parameter, or a function-local or `global` assignment) silences Ruff file-wide; the document reads the module scope.

### `ruff-ann206-missing-return-type-class-method`

Port of Ruff's ANN206 (missing-return-type-class-method): reports the name of a `def` that sits directly in a class body (also inside `if`/`try`/`for`/`while`/`with`/`match` blocks of that body), has no `->` return annotation and is decorated with the builtin `classmethod`, whatever its name and whether or not it is also a `staticmethod` (Ruff checks the class-method rule first). The builtins are recognised as Ruff's match_builtin_expr does: when the file never imports `builtins`, a bare name is the builtin unless the module scope binds it (a module-level annotation without a value and an `except ... as` name do not count, as Ruff skips the first and restores the builtin after the second; class-scope and function-scope bindings do not count either); when the file imports `builtins` anywhere, names resolve lexically, so `builtins.x`, aliases and class- or function-scope rebindings are honoured. The implementation `def` that follows an `@overload` set of the same name (typing, typing_extensions or _typeshed overload, any alias) is skipped, judged in Ruff's definition order. Functions outside classes are not methods. Works on .py and .pyi. Default settings only (`suppress-none-returning` off). Divergence: Ruff reads the bare decorator name from the scope of the last lambda it analysed, so a `classmethod` binding around that lambda silences Ruff file-wide; the document reads the module scope.

### `ruff-c420-unnecessary-dict-comprehension-for-iterable`

Port of Ruff's C420 (unnecessary-dict-comprehension-for-iterable): reports a dict comprehension with exactly one `for` clause, no `if` clause and no `async for`, whose key is the same expression as its loop target. The comparison works like Ruff's ComparableExpr: parentheses, comments and tuple-versus-bare-tuple differences are ignored. So `{(a, b): 1 for a, b in x}` and `{(a, b): 1 for ((a, b)) in x}` match, but `{(a, b): 1 for [a, b] in x}` does not. A target that contains an attribute or subscript (`for o.a in`, `for s[0] in`) is skipped. The value must be shareable: it must not contain, at any depth (f-string fields included), a lambda, list, dict or set literal, comprehension, generator, `await`, `yield`, call or walrus. It must also not read a name bound by the loop target (`{a: a.b for a in x}` is skipped, `{a: obj.a for a in x}` is reported). Builtins such as `None` or `NotImplemented`, and names from enclosing scopes or outer comprehensions, are allowed. Comprehensions inside string annotations and other string type definitions are reported too, with Ruff's span. Elsewhere the span is the comprehension, without enclosing parentheses. A shadowed `dict` does not suppress the report. Divergences: these are not reported: a key/target pair with a starred element (`{(a, *b): 1 for a, *b in x}`), and a target with a one-element parenthesized pattern nested inside a tuple or list (`for (a, (b)) in x`), both because of engine limits in the structural comparison. In a string type definition, a value that names the loop target is always treated as reading it. Ruff instead resolves it to a module binding of that name when one exists, and then reports.

### `ruff-d102-undocumented-public-method`

Port of Ruff's D102 (undocumented-public-method): reports the name of a method (a def or async def directly in a class body, also under if/try/for/with inside it) whose body does not start with a docstring (forms as D101). The method is public when its name does not start with an underscore, or is exactly __new__ or __call__ (other dunders are D105, __init__ is D107); no enclosing class name starts with an underscore; no function encloses the method's class or any class around it; the outermost class is listed in __all__ when the module assigns one (read as for D101; nested classes ignore __all__, as in Ruff); and the module is public (as D100). Skipped as in Ruff: methods decorated with typing, typing_extensions or _typeshed overload or override (a name or attribute resolved through imports, parenthesized or not; a call such as @overload() does not count), and the accessors @<own name>.setter / .deleter, also with any parentheses around the name or the whole decorator, which Ruff treats as private; @x.setter() or @other.setter or @self.x.setter do not exempt. Stub files (.pyi) are skipped. The span is the method name. Divergences: the __all__ entry engine gap and dotted-stem edge described for D101, and a decorator used above a later import of overload or override (which fails at import time) is reported where Ruff exempts it.

### `ruff-d104-undocumented-public-package`

Port of Ruff's D104 (undocumented-public-package): reports, at the very start of the file (1:1, zero width), an __init__.py whose first statement is not a docstring (forms as D100: plain, raw or u-prefixed string, parenthesized or implicitly concatenated; not bytes, f-string or t-string, a tuple or an expression on a string; a comment or byte-order mark before it is fine). Only files named exactly __init__.py count (an __init__.pyi stub is skipped like every stub), and the package must be public: no name in its module path (sent by the host inside a package; otherwise the file stem) starts with an underscore without being a dunder, so pkg/_priv/__init__.py is private while __init__ itself is public. The per-rule harness cannot send a module path, so Ruff's private-package chain is exercised there only for the init stem; with a module path the document is silent on private packages (checked directly against the engine). Defaults assumed.

### `ruff-d105-undocumented-magic-method`

Port of Ruff's D105 (undocumented-magic-method): reports the name of a method whose name starts and ends with two underscores (including `__` and `___`), other than `__init__`, `__new__` and `__call__` (D107 and D102 cover those), defined in the body of a public class (also inside if, try, for, with, while and match blocks of that body) when the body's first statement, ignoring comments, is not a docstring. A docstring is a plain string, optionally with an `r` or `u` prefix, parenthesized, or an implicit concatenation of such strings; f-strings, byte strings, t-strings, a tuple like `"doc",` and a string after another statement do not count. A class is public when its name does not start with an underscore, the classes enclosing it are public, it is not defined inside a function, and, for a module-level class, it is listed by the module's `__all__` whenever the module assigns `__all__` (the engine's export binding, so `=` and `+=` count, `.extend` and `.append` do not, and a bare `__all__: T` annotation exports nothing, as in Ruff; a parenthesized `(__all__) = [...]` target is not recognized, so such a module is treated as having no `__all__`). Methods decorated with `overload` or `override` from `typing`, `typing_extensions` or `_typeshed`, and a property accessor decorated `@name.setter` or `@name.deleter` with its own name (parentheses allowed), are exempt. `.pyi` files are skipped, and so are modules with a private name: one clause checks the file name when the host sends no module path, a second one the dotted module path inside a package (the port harness only exercises the first). The `ignore-decorators` and `property-decorators` settings are not supported; defaults only.

### `ruff-d107-undocumented-public-init`

Port of Ruff's D107 (undocumented-public-init): reports the name of an `__init__` method defined in the body of a public class (also inside if, try, for, with, while and match blocks of that body, and in public nested classes) when the body's first statement, ignoring comments, is not a docstring. A docstring is a plain string, optionally with an `r` or `u` prefix, parenthesized, or an implicit concatenation of such strings; f-strings, byte strings, t-strings, a tuple like `"doc",` and a string after another statement do not count. A class is public when its name does not start with an underscore, the classes enclosing it are public, it is not defined inside a function, and, for a module-level class, it is listed by the module's `__all__` whenever the module assigns `__all__` (the engine's export binding, so `=` and `+=` count, `.extend` and `.append` do not, and a bare `__all__: T` annotation exports nothing, as in Ruff; a parenthesized `(__all__) = [...]` target is not recognized, so such a module is treated as having no `__all__`). An `__init__` decorated with `overload` or `override` from `typing`, `typing_extensions` or `_typeshed`, or with `@__init__.setter` or `@__init__.deleter`, is exempt. `.pyi` files are skipped, and so are modules with a private name: one clause checks the file name when the host sends no module path, a second one the dotted module path inside a package (the port harness only exercises the first). The `ignore-decorators` setting is not supported; defaults only.

### `ruff-d403-first-word-uncapitalized`

Port of Ruff's D403 (first-word-uncapitalized): for the docstring of a function or method (not a class or module; plain, raw or unicode strings only, not implicit concatenation), takes the first whitespace-delimited word after leading whitespace, strips trailing `.`, `!` and `?`, and reports when it starts with an ASCII lower-case letter and the rest are ASCII letters or apostrophes (so `it's`, `iPhone` and `x.` are reported, `'tis`, `é`, `foo-bar`, `x2`, `foo,` and `foo.bar` are not). Reports the whole docstring literal.

### `ruff-d418-overload-with-docstring`

Port of Ruff's D418 (overload-with-docstring): reports the name of a function that has a non-empty docstring and an `@overload` decorator, anchored at the function name (not the docstring). A decorator is `@overload` when it is exactly an identifier or attribute that resolves through the file's imports to `typing.overload`, `typing_extensions.overload` or `_typeshed.overload` (aliased imports such as `from typing import overload as ov`, `import typing as t` with `@t.overload`, and `@(overload)` in parentheses all count; locally rebound or shadowed names, a call `@overload()`, look-alikes such as `@overloaded` or an `overload` from another module do not), in any position of the decorator list, on sync and async functions, methods and nested functions. The docstring is a single plain `r`/`u` string (optionally parenthesized) that is the first statement of the body and contains something other than whitespace; empty docstrings, f-strings, bytes, implicit concatenations and strings that are not the first statement do not count. Stub files (`.pyi`) are skipped as Ruff does. Names resolve in the decorator's own scope, as Ruff's do: an `overload` or `typing` imported in a class body counts, a later `del overload` does not undo it, a star import does not count. No known divergence; 31 probe files agree with live Ruff finding for finding.

### `ruff-g001-logging-string-format`

Port of Ruff's G001 (logging-string-format): a logging call whose message is a `.format(...)` call on a literal receiver. A logging call is `<obj>.debug/info/warning/warn/error/exception/critical(...)`, where the message is the `msg=` keyword, else the first positional argument before any starred one. It is also `<obj>.log(...)`, where the message is `msg=`, else the second positional argument before any starred one. Here `<obj>` (parentheses transparent) is one of: a name that resolves through the file's imports to `logging` or `flask.current_app.logger`; a call to `logging.getLogger` or `logging.Logger`; or a name or attribute chain that resolves to no import, definition or builtin and whose last segment starts with `log`/`LOG` or ends with `logger`/`logging`/`LOGGER`/`LOGGING`. Names resolve as Ruff's semantic model does: function and lambda bodies see the module's final bindings, methods skip the class scope, an unconditional `del` or a finished `except ... as` name unbinds, and a match capture, loop target, walrus or `with ... as` rebinds. A bare call to one of those `logging` functions, as after `from logging import info, log` or in a file whose stem is `logging`, also counts. The receiver of `.format` can be a str or bytes literal, an implicit concatenation with no f- or t-string part, a number, `True`, `False`, `None` or `...` (parentheses transparent). f-strings, names and unary minus do not count. The finding covers the whole `.format(...)` call without its enclosing parentheses. Ruff's `lint.logger-objects` setting is not modelled; this matches its default (empty). Two divergences from Ruff: a logging call written inside a quoted annotation is not reported, and the logger-name test reads identifiers as written, so a compatibility spelling that Ruff NFKC-normalizes (`ｌｏｇ.info(...)`) is not treated as a logger.

### `ruff-g002-logging-percent-format`

Port of Ruff's G002 (logging-percent-format): a logging call whose message is a binary `%` expression with any operands, numeric modulo included (parentheses transparent). The logging call is detected as in G001: level methods and `.log` on a `logging`/flask/`getLogger`/`Logger` object, on an unresolved name chain whose last segment looks like a logger, or bare `logging` functions imported by name. The message is the `msg=` keyword, else the positional message slot before any starred argument. The finding covers the `%` expression without its enclosing parentheses. A `%` nested under a top-level `+` is not reported (that is G003), and `(a + b) % c` is. Names resolve as Ruff's semantic model does, including later module rebindings seen from function bodies, `del` and `except ... as` unbinding, and `global`/`nonlocal`. Ruff's `lint.logger-objects` setting is not modelled; this matches its default (empty). Two divergences from Ruff: a logging call inside a quoted annotation is not reported, and identifiers that Ruff NFKC-normalizes (`ｌｏｇ.info(...)`) are not treated as loggers.

### `ruff-g003-logging-string-concat`

Port of Ruff's G003 (logging-string-concat): a logging call whose message is a binary `+` expression with any operands, numeric addition included (parentheses transparent). The logging call is detected as in G001: level methods and `.log` on a `logging`/flask/`getLogger`/`Logger` object, on an unresolved name chain whose last segment looks like a logger, or bare `logging` functions imported by name. The message is the `msg=` keyword, else the positional message slot before any starred argument. The finding covers the `+` expression without its enclosing parentheses. A `+` nested under a top-level `%` is not reported (that is G002). Names that resolve to an import other than `logging`/`flask.current_app.logger`, or to a def, class or builtin, are never treated as loggers, whatever they are called. Resolution follows Ruff's semantic model: function and lambda bodies see the module's final bindings, methods and inner comprehensions skip the class scope, and a comprehension's first iterable reads the class scope. Ruff's `lint.logger-objects` setting is not modelled; this matches its default (empty). Two divergences from Ruff: a logging call inside a quoted annotation is not reported, and identifiers that Ruff NFKC-normalizes (`ｌｏｇ.info(...)`) are not treated as loggers.

### `ruff-g004-logging-f-string`

Port of Ruff's G004 (logging-f-string): a logging call whose message is an f-string, or an implicit concatenation with at least one f-string part (any prefix case, raw included; parentheses transparent). The logging call is detected as in G001: level methods and `.log` on a `logging`/flask/`getLogger`/`Logger` object, on an unresolved name chain whose last segment looks like a logger, or bare `logging` functions imported by name. The message is the `msg=` keyword, else the first positional argument (the second for `.log`) before any starred argument. `**{"msg": ...}` passes nothing, and `logging.log(f"...", level=10)` has no message. The finding covers the whole string or concatenation. t-strings are not reported, and neither is an f-string that is only an operand of `+`/`%` or the argument of `.format`. Ruff's `lint.logger-objects` setting and its preview-only autofix are not modelled. Two divergences from Ruff: a logging call inside a quoted annotation is not reported, and identifiers that Ruff NFKC-normalizes (`ｌｏｇ.info(f"{x}")`) are not treated as loggers.

### `ruff-n803-invalid-argument-name`

Port of Ruff's N803 (invalid-argument-name): reports every parameter whose name contains an alphabetic character that is not lowercase (Rust char::is_lowercase semantics, with NFKC-foldable characters modelled by explicit ranges), for def and async def parameters (positional-only, regular, keyword-only, `*args`, `**kwargs`, with or without annotation and default) and for lambda parameters. The reported span is Ruff's: the whole parameter including its annotation and default (`B: int = 3`, `C=1`) and the leading stars of `*Args: int` and `**Kw`. Skipped: pep8-naming's default ignore names (setUp, tearDown, ..., maxDiff), and every parameter of a def directly in a class scope (also under if, try, for, with or match in the class body) decorated with typing, typing_extensions or _typeshed `override` resolved through imports (aliases and parentheses work, the call form `@override()` is not exempt). As in Ruff the override exemption is class-scope only: `@override` on a module-level or nested function, or on a def nested inside an override method, is still reported, and `overload` is not an exemption. Two clauses differ only in message: function parameters and lambda parameters. Divergences: user-configured ignore-names are unreadable; an ignore-listed name spelled with NFKC-foldable compatibility characters would be reported here but skipped by Ruff; and the harness's not-overfit gate flags the default ignore-names list, which Ruff spells in pep8_naming/settings.rs rather than in this rule's file.

### `ruff-pth100-os-path-abspath`

Port of Ruff's PTH100 (os-path-abspath): reports a call whose callee resolves through the file's imports to os.path.abspath (from os import path, import os.path as osp, from os.path import abspath and aliases all resolve; a name shadowed by a parameter, local binding or later rebinding does not), at the callee (inside the parentheses when the callee is parenthesized). Like Ruff it fires only when the call has exactly one argument, either the first positional or the path keyword (a lone generator expression counts); extra arguments, unknown keywords, starred and double-starred arguments are not reported. Divergences, both engine gaps: a call inside a string annotation (x: "os.path.abspath(p)") is not reported, and a lone starred call in a list or set display ([*os.path.abspath(p)]) is not reported because the grammar reads it as a call on a splat.

### `ruff-pth101-os-chmod`

Port of Ruff's PTH101 (os-chmod): reports a call resolving through imports to os.chmod, at the callee, unless Ruff's two exemptions hold. Exemption one: a dir_fd keyword whose value is not a literal None (parentheses transparent). Exemption two: the path argument (the path keyword, else the first positional before any starred argument; absent when only *args or **kwargs are passed, which Ruff still reports) looks like a file descriptor, read as Ruff's is_file_descriptor does: an integer literal (not imaginary, not negated), or a name (also through parentheses and through the callee of a call on a name) whose only binding is an assignment, walrus or with item whose value Ruff's type inference calls an int, an int-annotated assignment, an int-annotated non-variadic parameter, or a def returning int; or Cls.attr / Cls.Inner.attr (parentheses allowed) where the latest binding of the attribute in that class's scope, at any depth of its body but not in nested scopes, is one of those (a later loop, augmented assignment, import, with, del or def rebinding cancels it). Int inference mirrors Ruff's ResolvedPythonType, not evaluation: integer literals, int/bool arithmetic except / and @ (so 2 ** -1 counts), unary operators over ints or bools, conditionals and and/or whose arms are ints or bools with at least one int, a walrus, and a builtin int(...) call only at the top of the value (1 + int(s) does not count). Annotations count through int or builtins.int, parentheses and a subscript of int, not Optional[int], Annotated[int, ...] or a string; a bare `x: int` with no value does not count for a name, as in Ruff. Divergences: class nesting beyond two levels, Ruff's zip pairing for a starred unpacking target, and the engine gaps for string annotations and a lone starred call in a list or set display.

### `ruff-pth102-os-mkdir`

Port of Ruff's PTH102 (os-mkdir): reports a call resolving through imports to os.mkdir, at the callee (inside the parentheses when parenthesized), unless a dir_fd keyword is passed with a value other than a literal None (parentheses transparent). Like Ruff it reports before looking at the arguments, so a call with no arguments, starred arguments or **kwargs is reported too; there is no file-descriptor exemption. Divergences, both engine gaps: a call inside a string annotation (x: "os.mkdir(p)") is not reported, and a lone starred call in a list or set display ([*os.mkdir(p)]) is not reported because the grammar reads it as a call on a splat.

### `ruff-pth103-os-makedirs`

Port of Ruff's PTH103 (os-makedirs): reports every call whose callee resolves through imports to os.makedirs (import os, from os import makedirs, aliases; not a shadowed name), at the callee (inside the parentheses when parenthesized). Like Ruff it has no argument conditions: Ruff reports before looking at the arguments and has no dir_fd or file-descriptor exemption, so calls with no arguments, starred arguments, **kwargs or any keyword are reported. Divergences, both engine gaps: a call inside a string annotation (x: "os.makedirs(p)") is not reported, and a lone starred call in a list or set display ([*os.makedirs(p)]) is not reported because the grammar reads it as a call on a splat.

### `ruff-pth104-os-rename`

Port of Ruff's PTH104 (os-rename): reports a call resolving through imports to os.rename, at the callee (inside the parentheses when parenthesized), unless a src_dir_fd or a dst_dir_fd keyword is passed with a value other than a literal None (parentheses transparent). Like Ruff it reports before looking at the other arguments, so calls with no arguments, starred arguments or **kwargs are reported too; there is no file-descriptor exemption. Divergences, both engine gaps: a call inside a string annotation (x: "os.rename(a, b)") is not reported, and a lone starred call in a list or set display ([*os.rename(a, b)]) is not reported because the grammar reads it as a call on a splat.

### `ruff-pth105-os-replace`

Port of Ruff's PTH105 (os-replace): reports a call resolving through imports to os.replace, at the callee (inside the parentheses when parenthesized), unless a src_dir_fd or a dst_dir_fd keyword is passed with a value other than a literal None (parentheses transparent). Like Ruff it reports before looking at the other arguments, so calls with no arguments, starred arguments or **kwargs are reported too; there is no file-descriptor exemption, and str.replace or Path.replace are not matched. Divergences, both engine gaps: a call inside a string annotation (x: "os.replace(a, b)") is not reported, and a lone starred call in a list or set display ([*os.replace(a, b)]) is not reported because the grammar reads it as a call on a splat.

### `ruff-pth106-os-rmdir`

Port of Ruff's PTH106 (os-rmdir): reports a call whose callee resolves, through imports and aliases with any number of parentheses around the callee, to `os.rmdir`, and only when the call has exactly one argument, written either as a positional argument that is not starred or as the keyword `path=`. A generator-expression argument counts as one argument. `os.rmdir()`, `os.rmdir(p, p)`, `os.rmdir(*a)`, `os.rmdir(**kw)` and `os.rmdir(foo=p)` are not reported, as in Ruff (which requires one argument and finds its value as the `path` keyword or the first non-starred positional argument). The finding is anchored on the callee expression inside any parentheses, as Ruff reports call.func. Shadowed or rebound names are not reported, as in Ruff. Calls written inside a string type definition (`x: "os.rmdir(p)"`, a quoted return annotation, a string nested in one) are reported too, as Ruff parses those strings as code: through parse: annotation, anchored on the callee inside the string for a simple string and on the whole string for a complex one (concatenated or escaped), where, as in Ruff, the call is reported only when its arguments name nothing other than the callee's own head name. Divergences: a `nonlocal` rebinding of the module name in a sibling function hides the import here but not in Ruff; and in a complex string annotation Ruff decides by the last name visited, the document by every name in the arguments.

### `ruff-pth107-os-remove`

Port of Ruff's PTH107 (os-remove): reports a call whose callee resolves, through imports and aliases with any number of parentheses around the callee, to `os.remove`, and only when the call has exactly one argument, written either as a positional argument that is not starred or as the keyword `path=`. A generator-expression argument counts as one argument. `os.remove()`, `os.remove(p, p)`, `os.remove(*a)`, `os.remove(**kw)` and `os.remove(foo=p)` are not reported, as in Ruff (which requires one argument and finds its value as the `path` keyword or the first non-starred positional argument). The finding is anchored on the callee expression inside any parentheses, as Ruff reports call.func. Shadowed or rebound names are not reported, as in Ruff. Calls written inside a string type definition (`x: "os.remove(p)"`, a quoted return annotation, a string nested in one) are reported too, as Ruff parses those strings as code: through parse: annotation, anchored on the callee inside the string for a simple string and on the whole string for a complex one (concatenated or escaped), where, as in Ruff, the call is reported only when its arguments name nothing other than the callee's own head name. Divergences: a `nonlocal` rebinding of the module name in a sibling function hides the import here but not in Ruff; and in a complex string annotation Ruff decides by the last name visited, the document by every name in the arguments.

### `ruff-pth108-os-unlink`

Port of Ruff's PTH108 (os-unlink): reports a call whose callee resolves, through imports and aliases with any number of parentheses around the callee, to `os.unlink`, and only when the call has exactly one argument, written either as a positional argument that is not starred or as the keyword `path=`. A generator-expression argument counts as one argument. `os.unlink()`, `os.unlink(p, p)`, `os.unlink(*a)`, `os.unlink(**kw)` and `os.unlink(foo=p)` are not reported, as in Ruff (which requires one argument and finds its value as the `path` keyword or the first non-starred positional argument). The finding is anchored on the callee expression inside any parentheses, as Ruff reports call.func. Shadowed or rebound names are not reported, as in Ruff. Calls written inside a string type definition (`x: "os.unlink(p)"`, a quoted return annotation, a string nested in one) are reported too, as Ruff parses those strings as code: through parse: annotation, anchored on the callee inside the string for a simple string and on the whole string for a complex one (concatenated or escaped), where, as in Ruff, the call is reported only when its arguments name nothing other than the callee's own head name. Divergences: a `nonlocal` rebinding of the module name in a sibling function hides the import here but not in Ruff; and in a complex string annotation Ruff decides by the last name visited, the document by every name in the arguments.

### `ruff-pth109-os-getcwd`

Port of Ruff's PTH109 (os-getcwd): reports every call whose callee resolves, through imports and aliases with any number of parentheses around the callee, to `os.getcwd` or `os.getcwdb`, whatever arguments it has (Ruff reports before it looks at the arguments). There are two clauses because the advice differs: `os.getcwd()` returns a string and `os.getcwdb()` returns bytes. The finding is anchored on the callee expression inside any parentheses, as Ruff reports call.func. Shadowed or rebound names are not reported, as in Ruff. Calls written inside a string type definition (`x: "os.getcwd()"`, a quoted return annotation, a string nested in one) are reported too, as Ruff parses those strings as code: through parse: annotation, anchored on the callee inside the string for a simple string and on the whole string for a complex one (concatenated or escaped), where, as in Ruff, the call is reported only when its arguments name nothing other than the callee's own head name. Divergences: a `nonlocal` rebinding of the module name in a sibling function hides the import here but not in Ruff; and in a complex string annotation Ruff decides by the last name visited, the document by every name in the arguments.

### `ruff-pth110-os-path-exists`

Port of Ruff's PTH110 (os-path-exists): reports a call whose callee resolves, through imports and aliases with any number of parentheses around the callee, to `os.path.exists`, and only when the call has exactly one argument, written either as a positional argument that is not starred or as the keyword `path=`. A generator-expression argument counts as one argument. `os.path.exists()`, `os.path.exists(p, p)`, `os.path.exists(*a)`, `os.path.exists(**kw)` and `os.path.exists(foo=p)` are not reported, as in Ruff (which requires one argument and finds its value as the `path` keyword or the first non-starred positional argument); `posixpath.exists` and `ntpath.exists` are not reported either. The finding is anchored on the callee expression inside any parentheses, as Ruff reports call.func. Shadowed or rebound names are not reported, as in Ruff. Calls written inside a string type definition (`x: "os.path.exists(p)"`, a quoted return annotation, a string nested in one) are reported too, as Ruff parses those strings as code: through parse: annotation, anchored on the callee inside the string for a simple string and on the whole string for a complex one (concatenated or escaped), where, as in Ruff, the call is reported only when its arguments name nothing other than the callee's own head name. Divergences: a `nonlocal` rebinding of the module name in a sibling function hides the import here but not in Ruff; and in a complex string annotation Ruff decides by the last name visited, the document by every name in the arguments.

### `ruff-pth111-os-path-expanduser`

Port of Ruff's PTH111 (os-path-expanduser): reports a call whose callee resolves, through imports and aliases with any number of parentheses around the callee, to `os.path.expanduser`, and only when the call has exactly one argument, written either as a positional argument that is not starred or as the keyword `path=`. A generator-expression argument counts as one argument. `os.path.expanduser()`, `os.path.expanduser(p, p)`, `os.path.expanduser(*a)`, `os.path.expanduser(**kw)` and `os.path.expanduser(foo=p)` are not reported, as in Ruff (which requires one argument and finds its value as the `path` keyword or the first non-starred positional argument); `posixpath.expanduser` and `ntpath.expanduser` are not reported either. The finding is anchored on the callee expression inside any parentheses, as Ruff reports call.func. Shadowed or rebound names are not reported, as in Ruff. Calls written inside a string type definition (`x: "os.path.expanduser(p)"`, a quoted return annotation, a string nested in one) are reported too, as Ruff parses those strings as code: through parse: annotation, anchored on the callee inside the string for a simple string and on the whole string for a complex one (concatenated or escaped), where, as in Ruff, the call is reported only when its arguments name nothing other than the callee's own head name. Divergences: a `nonlocal` rebinding of the module name in a sibling function hides the import here but not in Ruff; and in a complex string annotation Ruff decides by the last name visited, the document by every name in the arguments.

### `ruff-pth112-os-path-isdir`

Port of Ruff's PTH112 (os-path-isdir): flags a call whose callee resolves, unshadowed and through any import form or alias, to os.path.isdir, including a parenthesized callee; the report is on the callee expression (parentheses excluded), as in Ruff. Like Ruff it needs exactly one argument, found as the first positional argument (before any starred one) or as the keyword s; so isdir(a, b), isdir(path=p), isdir(*args), isdir(**kw) and isdir() are silent, while isdir(s=p), isdir(p,) and a lone generator-expression argument are reported. Names from star imports, posixpath/ntpath and attribute look-alikes do not match. Divergence: Ruff also lints expressions inside string (forward-reference) annotations; the engine does not parse those, so a call written inside a quoted annotation is not reported.

### `ruff-pth113-os-path-isfile`

Port of Ruff's PTH113 (os-path-isfile): flags a call whose callee resolves, unshadowed and through any import form or alias, to os.path.isfile, including a parenthesized callee; the report is on the callee expression (parentheses excluded), as in Ruff. Like Ruff it needs exactly one argument, found as the first positional argument (before any starred one) or as the keyword path; so isfile(a, b), isfile(s=p), isfile(*args), isfile(**kw) and isfile() are silent, while isfile(path=p), isfile(p,) and a lone generator-expression argument are reported. Names from star imports, posixpath/ntpath and attribute look-alikes do not match. Divergence: Ruff also lints expressions inside string (forward-reference) annotations; the engine does not parse those, so a call written inside a quoted annotation is not reported.

### `ruff-pth114-os-path-islink`

Port of Ruff's PTH114 (os-path-islink): flags a call whose callee resolves, unshadowed and through any import form or alias, to os.path.islink, including a parenthesized callee; the report is on the callee expression (parentheses excluded), as in Ruff. Like Ruff it needs exactly one argument, found as the first positional argument (before any starred one) or as the keyword path; so islink(a, b), islink(s=p), islink(*args), islink(**kw) and islink() are silent, while islink(path=p), islink(p,) and a lone generator-expression argument are reported. Names from star imports, posixpath/ntpath and attribute look-alikes do not match. Divergence: Ruff also lints expressions inside string (forward-reference) annotations; the engine does not parse those, so a call written inside a quoted annotation is not reported.

### `ruff-pth115-os-readlink`

Port of Ruff's PTH115 (os-readlink): flags a call whose callee resolves, unshadowed and through any import form or alias, to os.readlink, including a parenthesized callee; the report is on the callee expression (parentheses excluded), as in Ruff. Ruff skips it below Python 3.9, mirrored by when.pythonVersion min 3.9 (the default is 3.10; Ruff goes silent under --target-version py38). Like Ruff it needs exactly one argument, found as the first positional argument (before any starred one) or as the keyword path, so any dir_fd keyword, even dir_fd=None, makes the call silent, as does readlink(a, b), readlink(s=p), readlink(*args), readlink(**kw) and readlink(); readlink(path=p), readlink(p,) and a lone generator-expression argument are reported. The message also tells the agent to leave bytes paths alone, since Path rejects them. Divergence: Ruff also lints expressions inside string (forward-reference) annotations; the engine does not parse those, so a call written inside a quoted annotation is not reported.

### `ruff-pth116-os-stat`

Port of Ruff's PTH116 (os-stat): flags a call whose callee resolves, unshadowed and through any import form or alias, to os.stat, including a parenthesized callee; the report is on the callee expression, as in Ruff. The path is found as Ruff's find_argument_value does (keyword path, else the first positional before any starred one); no path (os.stat(), os.stat(*a), os.stat(**kw)) is silent, extra arguments are allowed, and a dir_fd keyword that is not the literal None (parentheses ignored) silences the call. Ruff's file-descriptor exemption is mirrored: the path is silent when it is an int literal (not 1j, not -1), a name whose only binding is int-typed, a call chain rooted at such a name, or a Class.attr or Class.Inner.attr whose class scope's latest binding of attr is int-typed (the latest binding anywhere in the class scope, so a later rebinding inside an if, for, with or an augmented assignment counts, as does a tuple unpack or a decorated def). A binding is int-typed as Ruff's is_int judges it: a plain, unpacked or walrus assignment of an int-typed value, a with item whose context expression is one, an annotated assignment, bare annotation or non-variadic parameter annotated int (or int[...], or builtins.int; a string annotation or a shadowed int does not count), or a def returning int. An int-typed value follows Ruff's type inference structurally: int literals, unary + - ~ and the operators + - * % // ** & | ^ << >> over int or bool operands, conditional expressions and and/or chains whose arms are int or bool with at least one int (the condition is ignored), walrus values and parentheses, or a direct int(...) call; / and @ are not int, nor is any expression containing a name or another call. Divergences: chains through three or more class levels (A.B.C.attr) are not followed, so Ruff's silence there is missed (extra report); Ruff also lints expressions inside string annotations, which the engine does not parse.

### `ruff-pth117-os-path-isabs`

Port of Ruff's PTH117 (os-path-isabs): flags a call whose callee resolves, unshadowed and through any import form or alias, to os.path.isabs, including a parenthesized callee; the report is on the callee expression (parentheses excluded), as in Ruff. Like Ruff it needs exactly one argument, found as the first positional argument (before any starred one) or as the keyword s; so isabs(a, b), isabs(path=p), isabs(*args), isabs(**kw) and isabs() are silent, while isabs(s=p), isabs(p,) and a lone generator-expression argument are reported. Names from star imports, posixpath/ntpath and attribute look-alikes do not match. Divergence: Ruff also lints expressions inside string (forward-reference) annotations; the engine does not parse those, so a call written inside a quoted annotation is not reported.

### `ruff-pth118-os-path-join`

Port of Ruff's PTH118 (os-path-join): reports a call whose callee resolves through the file's imports to os.path.join or os.sep.join (import aliases, from-imports, `import os.path`, parenthesized callees; a name shadowed by a parameter, local, class attribute, comprehension variable, except or match capture, or rebound by a later assignment, is not reported, and a star import does not resolve), at the callee's span, whatever the arguments (none, starred, keyword). Calls written inside string annotations are reported too, one finding per call. Three clauses give different advice: os.path.join without an unpacked positional argument (use the / operator), os.path.join with a starred argument (use joinpath, as Ruff's own message does) and os.sep.join. posixpath and ntpath are not os.path to Ruff and are not reported. No known divergence from Ruff on the shipped tests or on 42 probe files.

### `ruff-pth119-os-path-basename`

Port of Ruff's PTH119 (os-path-basename): reports a call whose callee resolves through the file's imports to os.path.basename and that has exactly one argument, which is a plain positional argument, a lone generator expression or the keyword p (Ruff's call.arguments.len() == 1 plus find_argument_value("p", 0)); zero or two or more arguments, a starred or double-starred argument and any other keyword name (path=, foo=) are not reported, as in Ruff. The finding is at the callee's span, parentheses excluded. Calls written inside string annotations are reported too. Matches live Ruff on the shipped tests and on 42 probe files (aliases, from-imports, shadowing, parentheses, comments, line continuations, stubs, noqa, unicode columns, string annotations).

### `ruff-pth120-os-path-dirname`

Port of Ruff's PTH120 (os-path-dirname): reports a call whose callee resolves through the file's imports to os.path.dirname and that has exactly one argument, which is a plain positional argument, a lone generator expression or the keyword p; any other argument shape (none, two or more, starred, double-starred, another keyword name) is not reported, as in Ruff's check_os_pathlib_single_arg_calls. The finding is at the callee's span, parentheses excluded. Calls inside string annotations are reported too. Matches live Ruff on the shipped tests and on 42 probe files.

### `ruff-pth121-os-path-samefile`

Port of Ruff's PTH121 (os-path-samefile): reports every call whose callee resolves through the file's imports to os.path.samefile, whatever its arguments (Ruff reports before it looks at them, so zero, one, starred or extra arguments still fire), at the callee's span. Calls written inside string annotations are reported too, one finding per call. Matches live Ruff on the shipped tests and on 42 probe files; no known divergence.

### `ruff-pth122-os-path-splitext`

Port of Ruff's PTH122 (os-path-splitext): reports every call whose callee resolves through the file's imports to os.path.splitext, whatever its arguments, at the callee's span (parentheses excluded); shadowed or rebound names and star imports are not resolved, as in Ruff. Calls written inside string annotations are reported too, one finding per call. A local variable aliasing the function is not resolved, as in Ruff. Matches live Ruff on the shipped tests and on 42 probe files.

### `ruff-pth123-builtin-open`

Port of Ruff's PTH123 (builtin-open): reports a call whose callee is the unshadowed builtin open or builtins.open (also from builtins import open, import builtins as b, parenthesized callees; io.open, codecs.open, os.open, a def open, a class-scope open or an imported open are not), at the callee's span. Arguments are read as Ruff's find_argument_value reads them (the keyword first, else the positional slot before any starred argument; **kwargs passes nothing). Not reported, as in Ruff: closefd passed as anything but literal True, opener passed as anything but None, and a file argument that looks like a file descriptor: an integer literal (not an imaginary one); a name whose binding shadows no earlier one and is an int-annotated parameter or variable (subscripted int too), an assignment, walrus or with-item of an expression Ruff's type inference calls int (integer literals, unary and arithmetic or bitwise operators over ints and bools, and/or and conditionals whose branches are ints or bools with at least one int, int(...)), an element unpacked from a tuple, list or set literal, or a def returning int; a call, or call of a call, of such a name; or Cls.attr and Outer.Inner.attr where the latest class-scope binding of attr is such an int binding (an augmented assignment, del or str rebinding after it is not). As in Ruff, calls inside string type definitions (annotations, cast/TypeVar arguments, strings nested in them) are reported too, through parse: annotation, with the same closefd and opener exclusions (keyword or 7th/8th positional). There, a file argument that is an integer, a name, an attribute or a call is not reported, because bindings cannot be read inside a parsed annotation. A string with an escape sequence or an implicit concatenation is not read (Ruff reports those on the whole string). Divergences: see the reason field.

### `ruff-pyi011-typed-argument-default-in-stub`

Port of Ruff's PYI011 (typed-argument-default-in-stub): in `.pyi` files only, reports the default value of an annotated positional-only, regular or keyword-only parameter unless it is simple: int, float, complex, str, bytes, bool, None or `...` literals (not f-strings or t-strings), `-` applied to a number or to `math.inf`/`e`/`pi`/`tau`, `a + bj` or `a - bj` with a real part of at most 10 characters (optionally negated) and an imaginary part, `math.inf`/`nan`/`e`/`pi`/`tau` and the listed `sys` attributes resolved through imports, and a list, tuple, set or dict of at most 10 such simple elements without nesting, starring, `**` or comprehensions. Parentheses around a default are transparent and the reported span excludes them; unannotated defaults are left to PYI014. As in Ruff, which analyses parameters after the whole stub is read, `math.pi` or `sys.stdout` is also accepted when `math` or `sys` is unbound at the def but imported later at module level (unless a parameter of that function has the module's name). Known divergence: a module imported later under an alias (`import math as m` after `m.pi` is used) is still reported.

### `ruff-pyi014-argument-default-in-stub`

Port of Ruff's PYI014 (argument-default-in-stub): in .pyi files, reports the default of an unannotated parameter (functions and lambdas) that Ruff does not consider simple: anything but `...`, numeric, string, bytes, None and bool literals (f-strings and t-strings are not simple), `-` applied to a number or to math.inf/e/pi/tau, `real + complex` or `real - complex` with a real part of at most 10 characters, lists, tuples, sets and dicts of at most 10 such elements (comments and line continuations are not counted), and the allowed math and sys attributes (math.pi, sys.stdin...) resolved through imports. The report covers the default without its parentheses. Parameters with annotations are left to PYI011. Known divergence: a default that names a module imported only later in the stub (`def f(x=math.pi): ...` followed by `import math`) is reported, while Ruff resolves it through the later import.

### `ruff-pyi021-docstring-in-stub`

Port of Ruff's PYI021 (docstring-in-stub): flags, in .pyi files only (a file-path condition; the scope stays open so the document loads), the string literal that is the first statement of a module, a class body or a function body, including async functions, decorated definitions, one-line bodies such as `def f(): """doc"""`, parenthesized strings and implicit concatenation of plain strings. Comments before the string do not count as statements. Strings with a b, f or t prefix, a concatenation containing one, a string that is not the first statement, and strings in if/with/try/for/while/match bodies are not reported, as in Ruff. The finding spans the string literal. No divergence from Ruff found.

### `ruff-pyi054-numeric-literal-too-long`

Port of Ruff's PYI054 (numeric-literal-too-long): in `.pyi` stub files, reports every integer, float or imaginary number literal whose source text is longer than ten characters (the sign of a negative literal is a separate unary operator and is not counted), anywhere in the file, including inside string annotations and forward-reference strings, where the literal itself is the reported range. A string annotation that is implicitly concatenated or contains an escape sequence is parsed as Ruff parses it, with every node placed at the whole string, so every number in it, of any length, is reported at the string when the string is longer than ten characters; this also applies to such a string nested inside a plain string annotation. Files that are not stubs are never checked. Divergences: several numbers in one escaped or concatenated string annotation are reported once (Ruff repeats the diagnostic at the same range), and the string's length is counted in characters where Ruff counts bytes.

### `ruff-slf001-private-member-access`

Port of Ruff's SLF001 (private-member-access): matches attribute accesses whose attribute starts with an underscore but is not a dunder (`__x__`), a sunder (`_x_`), `_` or `__`, and is not in `flake8-self.ignore-names` (default `_make`, `_asdict`, `_replace`, `_fields`, `_field_defaults`, `_name_`, `_value_`). It skips accesses inside annotations, `os._exit`, and code whose scope is a dunder operator method (`__eq__`, `__add__`, ...). That scope covers the method body plus the defaults and decorators of nested functions and lambdas and a comprehension's first iterable. It does not cover nested function, lambda or class bodies, class bases, or later comprehension clauses. Receivers `self`, `cls`, `mcs` and `super()` calls (also parenthesized, including `(super)()`) are exempt. A class name read inside that same class's body is exempt, except in a comprehension the class body runs itself. Instances of a class are exempt, checked by syntax as Ruff does: the first parameter of an instance method, classmethod or `__new__`; names bound by annotation, typed parameter, return annotation, assignment, walrus or `with ... as` to `C`, `Self`, `C[...]`, `Annotated[C, ...]`, `cls()`, `mcs()`, `C()`, `C[...]()`, `super().__new__()` or a `self` parameter. As in Ruff, a class name only counts when the access is inside the body of a class with that name, wherever the binding is. In `match` patterns it reports dotted value and class patterns: the whole pattern when its last segment is private, else the prefix up to the first private segment. Divergences: at most one finding per dotted `case` pattern (`case a._b._c.d:` gives `a._b` only; `case self._a._b.c:` gives none); no findings inside strings Ruff parses as type definitions outside annotations (`cast("foo._x", y)`, TypeVar bounds, `list["foo._x"]`); a few binding cases that only arise in code that fails at runtime.

### `ruff-b015-useless-comparison`

Port of Ruff's B015 (useless-comparison): an expression statement whose value is a comparison (parentheses transparent). When the statement is the last one of a function body the message asks whether the result was meant to be returned. Reported on the comparison.

### `ruff-f402-import-shadowed-by-loop-var`

Port of Ruff's F402 (import-shadowed-by-loop-var) on the engine's shadowed-binding pairs (M10a `binds.redefines`): a `for` statement's loop variable (any nesting of its target) whose shadowed binding, in its own scope or the enclosing function or module scope, is an import (used or not), on the same fork (Ruff's same_branch). Reported on the loop variable.

### `ruff-f704-yield-outside-function`

Port of Ruff's F704 (yield-outside-function, a semantic syntax error) with default settings: `yield`, `yield from` or `await` whose scope does not allow it. Scopes are found by climbing: a function or lambda body allows both; a class body or the module allows neither; a list, set or dict comprehension rejects `yield` and is transparent for `await`; a generator expression rejects `yield` and allows `await`. The first iterable of a comprehension, parameter defaults, decorators and class bases belong to the enclosing scope. Reported on the expression.

### `ruff-sim102-collapsible-if`

Port of Ruff's SIM102 (collapsible-if) with default settings: an `if` whose body (or whose last `elif` clause's body, when the last clause is an `elif`) is exactly one `if` without `elif`/`else`, when the `if`'s own body is one statement, the tested condition is not `__name__ == "__main__"` or a boolean literal, and the `if` is not a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Every `if` of a nested chain is reported, as Ruff does (its parent-chain exemption compares the parent with itself and never applies). Reported at the `if` or `elif` keyword.

### `ruff-sim103-needless-bool`

Port of Ruff's SIM103 (needless-bool): an `if`/`else`, a trailing `elif`/`else`, or an `if` followed by a `return` statement, whose two branches are single `return True` / `return False` statements with opposite values, unless the `if` is a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Reported at the `if` (or the last `elif`).

### `ruff-sim117-multiple-with-statements`

Port of Ruff's SIM117 (multiple-with-statements): a `with` whose body is exactly one `with` of the same kind (both `async` or neither), unless either has a single item that calls `asyncio.timeout`/`timeout_at`, `anyio.CancelScope`/`fail_after`/`move_on_after` or `trio.fail_after`/`fail_at`/`move_on_after`/`move_on_at` (`resolve_qualified_name`); only the outermost `with` of a mergeable chain is reported.

### `ruff-sim118-in-dict-keys`

Port of Ruff's SIM118 (in-dict-keys): `key in X.keys()` / `key not in X.keys()` (a single comparison), a non-async `for key in X.keys()` and a comprehension `for key in X.keys()`, where `.keys()` has no arguments and `X` is not the name `self`. Reported from the key (with its parentheses).

### `ruff-sim222-expr-or-true`

Port of Ruff's SIM222 (expr-or-true), `is_short_circuit` in flake8_simplify/rules/ast_bool_op.rs:736. Over the values of one `or` run (tree-sitter nests them to the left; parentheses start a new run), a value whose truthiness is unknown is skipped when the run is not in a boolean test, or when it has a side effect (`contains_effect`). The first value that is not skipped and is truthy (`True`, a non-empty literal, ...), or whose successor is, triggers; the finding starts at the value after the last skipped value before it (the whole expression when none), as Ruff's `furthest`. Truthiness follows `Truthiness::from_expr`, including f-string emptiness and `list(...)`-style calls. The finding spans that value only; Ruff's range runs to the end of the expression. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/SIM222.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-sim223-expr-and-false`

Port of Ruff's SIM223 (expr-and-false), `is_short_circuit` in flake8_simplify/rules/ast_bool_op.rs:736. Over the values of one `and` run (tree-sitter nests them to the left; parentheses start a new run), a value whose truthiness is unknown is skipped when the run is not in a boolean test, or when it has a side effect (`contains_effect`). The first value that is not skipped and is falsy (`False`, `None`, zero, an empty literal, ...), or whose successor is, triggers; the finding starts at the value after the last skipped value before it (the whole expression when none), as Ruff's `furthest`. Truthiness follows `Truthiness::from_expr`, including f-string emptiness and `list(...)`-style calls. The finding spans that value only; Ruff's range runs to the end of the expression.
