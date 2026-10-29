# Rerail rule pack: Python, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `bandit-b104-hardcoded-bind-all-interfaces`

Matches any Python string constant whose decoded value is exactly `0.0.0.0`, regardless of where it is used.

### `bandit-b108-hardcoded-tmp-directory`

Matches string constants (and implicitly concatenated strings as a whole) whose decoded value starts with `/tmp`, `/var/tmp` or `/dev/shm`; strings that are bare expression statements (docstrings) are skipped.

### `bandit-b103-set-bad-file-permissions`

Any call whose callee name contains `chmod`, with a literal mode. World-write (0o002) is reported by its own clause (higher severity in Bandit); group-write, group-execute or world-execute (0o031 bits) by the other.

### `bandit-b202-tarfile-unsafe-members`

Old description claimed: allow `filter="data"`, otherwise classify missing members, members supplied by a call, and other members expressions separately. The current clause does none of that — it flags every `extractall` on a value bound from `tarfile.open`.

### `bandit-b324-hashlib-insecure-functions`

hashlib `md4/md5/sha/sha1` and `hashlib.new("md4"|"md5"|"sha"|"sha1")` unless `usedforsecurity=False`; `crypt.crypt`/`crypt.mksalt` with `METHOD_CRYPT`, `METHOD_MD5`, `METHOD_BLOWFISH`, or with no salt/method at all.

### `bandit-b406-import-xml-sax`

Port of Bandit 1.9.4's B406 import_xml_sax. Divergence: an import of only xml.sax.saxutils (`from xml.sax.saxutils import ...`, `from xml.sax import saxutils`, aliased or parenthesised) is not reported. saxutils holds XML writing and escaping helpers (escape, quoteattr, unescape, XMLGenerator) and no parser, so defusedxml has nothing to replace it with. `import xml.sax.saxutils` is still reported, since it also binds xml.sax and its parse functions.

### `bandit-b501-request-with-no-cert-validation`

Covers requests HTTP-verb calls (get, options, head, post, put, patch, delete) and httpx request/stream/Client/AsyncClient calls whose `verify` keyword is the literal False or the string 'False'. The callee root is read as Bandit's call qualname does: through the head's import alias, with relative imports named by their module text and the dots ignored, otherwise by the head's own text.

### `bandit-b506-yaml-load`

Port of Bandit 1.9.4's B506 yaml_load: `yaml.load` without `Loader`, or with a `Loader` (by keyword or second position) other than SafeLoader or CSafeLoader. Port of the kernel test b506_yaml_load_without_a_safe_loader. `# nosec` honoured.

### `bandit-b604-any-other-function-with-shell-equals-true`

A `shell=` keyword counts as truthy per Bandit's has_shell: anything but a falsy constant (False, None, a zero number, an empty string or bytes), an empty list or an empty dict. Calls to subprocess.Popen/call/check_call/check_output/run are excluded (covered by B602).

### `bandit-b610-django-extra-used`

Port of Bandit 1.9.4's B610 django_extra_used: a call of `extra` whose `where` or `tables` is not a list of plain strings, or whose `select` is not a dict of plain strings, bound to QuerySet.extra's six positional slots. Port of the kernel test b610_extra_with_a_non_literal_where_tables_or_select. `# nosec` honoured. Divergence: Bandit reports a call of any function or method named `extra`; the port reports only a method call whose receiver looks like a Django QuerySet or Manager: a name or attribute named `objects`, `qs`, `QS`, `queryset`, `querysets`, `manager` or `managers`, alone or after an underscore-joined prefix (`User.objects`, `self.queryset`, `base_qs`, `_default_manager`); a call of `get_queryset`; a method call on such a receiver (`User.objects.filter(...)`); or `self` or `super()` inside a class with a base whose name ends in `QuerySet` or is `Manager`, `BaseManager` or `BaseUserManager`, optionally dotted (`models.QuerySet`); a base that only ends in `Manager` (`AbstractContextManager`, `PluginManager`, `TreeManager`) does not count. A bare `extra(...)` call and `extra` on any other receiver are not reported.

### `bandit-b613-trojansource`

Closest AST-only approximation of Bandit B613: only bidirectional-control characters inside parsed Python string literals are detected (not comments or other tokens).

### `ruff-async109-async-function-with-timeout`

Port of Ruff's ASYNC109: a positional-only, positional or keyword-only parameter named `timeout` of an `async def` not decorated with `override` from `typing`, `_typeshed` or `typing_extensions`, reported on the parameter. `asyncio.timeout` exists from 3.11, so below 3.11 a parameter is flagged only after an absolute import of `trio` or `anyio` has been seen (Ruff's seen_module, M10a `resolves.seenBefore`; async_function_with_timeout.rs: `module == AsyncModule::AsyncIo && target_version < PY311`).

### `ruff-async110-async-busy-wait`

Port of Ruff's ASYNC110 (async-busy-wait): a `while` loop whose body is a single statement `await <call>` where the callee resolves to `trio.sleep`/`sleep_until`, `anyio.sleep`/`sleep_until` or `asyncio.sleep`, reported on the loop. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async212-blocking-http-call-httpx-in-async-function`

Port of Ruff's ASYNC212 (blocking-http-call-httpx-in-async-function): a blocking method call on a name whose binding holds an `httpx.Client(...)` call or is annotated with `httpx.Client` (directly or in `|`/`Optional`/`Union`: Ruff's check_type through `resolves.binding`) inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7). The binding is Ruff's only_binding (`resolves.binding` with `only`: it shadows no earlier binding of the name in its scope).

### `ruff-async220-create-subprocess-in-async-function`

Port of Ruff's ASYNC220 (create-subprocess-in-async-function): a call to `subprocess.Popen`, `os.popen`, or an `os.spawn*` function whose mode (Ruff's find_argument_value("mode", 0): `arguments` in syntactic mode) is given and is not `os.P_WAIT` (directly, or through a name whose value, read as find_assigned_value's lookup_symbol does (`resolves.binding` with `stopAtGlobal`), is `os.P_WAIT`) inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async240-blocking-path-method-in-async-function`

Port of Ruff's ASYNC240 (blocking-path-method-in-async-function): a call to an `os.path` function, or a method on a `pathlib` path (a `Path(...)`-family call, or a name whose binding holds such a call or is annotated with a path class, directly or in `|`/`Optional`/`Union`: Ruff's check_type through `resolves.binding`), except Ruff's list of pure methods, inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7). A bare name imported from `os.path` resolves through `prefix`. The binding is Ruff's only_binding (`resolves.binding` with `only`: it shadows no earlier binding of the name in its scope).

### `ruff-b006-mutable-argument-default`

Port of Ruff's B006 (mutable-argument-default), stable behaviour: a function parameter default that is a list, dict or set display or comprehension, or a call of `dict`/`list`/`set` or `collections.Counter`/`OrderedDict`/`defaultdict`/`deque` (a subscripted callee judged by its value; parentheses transparent), unless the parameter's annotation is immutable by Ruff's `is_immutable_annotation`: an immutable builtin, `typing` or `collections.abc` type (generic ones also subscripted), `None`, `X | Y`, `Union[...]` of immutables, `Optional[X]`, `Annotated[X, ...]`. Names resolve with `resolves` (imports, their scope and order, and unshadowed builtins, as Ruff's semantic model sees them); `builtins.X` counts only where Ruff's table lists it. Stub files are skipped. Lambdas are not checked (Ruff runs B006 on `def` only). Reported on the default. Ruff's option `flake8-bugbear.extend-immutable-calls` adds immutable annotations.

### `ruff-b008-function-call-in-default-argument`

Port of Ruff's B008 (function-call-in-default-argument): every call inside a parameter default (of a `def` or a `lambda`), not looking inside lambdas, unless the parameter's annotation is immutable (Ruff's `is_immutable_annotation`, as in B006) or the callee is a known mutable constructor (`dict`/`list`/`set`, `collections.Counter`/`OrderedDict`/`defaultdict`/`deque`) or immutable-returning callable (`datetime` types, `decimal.Decimal`, `fractions.Fraction`, `operator` getters, `pathlib.Path`, `types.MappingProxyType`, `re.compile`, immutable builtins; a subscripted callee judged by its value). Names resolve with `resolves` (imports, their scope and order, and unshadowed builtins, as Ruff's semantic model sees them); `builtins.X` counts only where Ruff's table lists it. A bare-name callee whose binding (`resolves.binding`) is `X = NewType(name, <immutable annotation>)` (Ruff's `is_immutable_newtype_call`: a plain assignment, the type as `tp=` or the second positional argument) is exempt where that binding shadows no earlier binding of the name in its scope (Ruff's only_binding: `resolves.binding` with `only`) and is not an unpacking. Reported on each call. Ruff's option `flake8-bugbear.extend-immutable-calls` adds immutable calls and annotations.

### `ruff-b012-jump-statement-in-finally`

Port of Ruff's B012 (jump-statement-in-finally): a `return`, `break` or `continue` statement in a `finally` block, reached through the bodies of nested `if` (not `elif`/`else`), `try` (not handlers, `else` or `finally`), `with` and `match` cases; `return` also through `while`/`for` bodies (not their `else`), where `break`/`continue` stop. Function and class bodies stop the walk. Reported on the statement.

### `ruff-b015-useless-comparison`

Port of Ruff's B015 (useless-comparison): an expression statement whose value is a comparison (parentheses transparent). When the statement is the last one of a function body the message asks whether the result was meant to be returned. Reported on the comparison.

### `ruff-b017-assert-raises-exception`

Port of Ruff's B017 (assert-raises-exception): `<x>.assertRaises(...)` or `pytest.raises(...)` whose exception (first positional argument, or `exception=` / `expected_exception=`) resolves to the builtin `Exception` or `BaseException` (the unshadowed name, or through an import of `builtins`; parentheses transparent). `pytest.raises` is exempt with `match=` or a string/bytes literal second positional argument. Reported on a `with` item that has no `as` target, and on any such call with at least two positional arguments or a `func=` keyword (the call form). `pytest.raises` and the exception names resolve with `resolves` (imports, their scope and order, and shadowing, as Ruff's semantic model sees them).

### `ruff-b018-useless-expression`

Port of Ruff's B018 (useless-expression): an expression statement (parentheses transparent; a bare tuple counts) that is not a comparison, a string or f-string literal, or `...`, and has no effect in Ruff's `contains_effect` sense: no call other than an argument-less `set()`/`list()`/`tuple()`/`dict()`/`frozenset()`, no `await`, `yield`, walrus, comprehension, generator or subscript, and no binary operation with an operand that is not a literal or display, anywhere inside (lambda bodies included). An attribute access with an effect is reported as a useless attribute access. The builtin names must resolve to the unshadowed builtins (`resolves`, Ruff's `has_builtin_binding`). Reported on the expression.

### `ruff-b019-cached-instance-method`

Port of Ruff's B019 (cached-instance-method): a decorator `functools.cache` / `functools.lru_cache` (called or not) on an instance method: a function defined directly in a class body, not decorated with `staticmethod`/`classmethod` (or `abc.abstractstaticmethod`/`abc.abstractclassmethod`), not `__new__`, `__init_subclass__` or `__class_getitem__`, in a class that does not list an `enum` base (`Enum`, `Flag`, `IntEnum`, `IntFlag`, `StrEnum`, `ReprEnum`, `CheckEnum`). Names resolve with `resolves` (imports, their scope and order, and shadowing, as Ruff's semantic model sees them); a base naming a class defined in the file is followed to that class's bases (`resolves.binding` on its definition, cycles fail). Reported on the decorator.

### `ruff-b024-abstract-base-class-without-abstract-method`

Port of Ruff's B024 (abstract-base-class-without-abstract-method): a class with exactly one base or keyword, which is `abc.ABC` as a base or `abc.ABCMeta` as `metaclass=`, whose body has no method decorated with an `abc` abstract decorator and no annotated attribute without a value. Names resolve with `resolves` (imports and shadowing as Ruff's semantic model sees them). Not in stubs. Reported on the class name.

### `ruff-b027-empty-method-without-abstract-decorator`

Port of Ruff's B027 (empty-method-without-abstract-decorator): in a class whose only base or keyword is `abc.ABC` or `metaclass=abc.ABCMeta`, a method directly in the class body whose body is only `pass`, string literals and `...`, with no `abc` abstract decorator and no `typing.overload` / `typing_extensions.overload` decorator. Names resolve with `resolves` (imports and shadowing as Ruff's semantic model sees them). Not in stubs. Reported on the whole definition, decorators included.

### `ruff-b031-reuse-of-groupby-generator`

Partial port of Ruff's B031 (reuse-of-groupby-generator): in the body of `for k, g in itertools.groupby(...)`, a use of `g` inside a nested loop body or list/set/dict comprehension, or after an earlier statement (or the enclosing `if` test or `match` subject) that uses it. Ruff's per-branch use counting (maximum over if/match branches), resets at break/continue/return and reassignment of `g` are not modelled.

### `ruff-b035-static-key-dict-comprehension`

Port of Ruff's B035 (static-key-dict-comprehension): the key of a dict comprehension is constant: a literal, a name the comprehension's generators never store (targets and walrus targets, nested ones included), or an attribute, subscript, operator or tuple built only from such parts. Reported on the key, inside any parentheses.

### `ruff-b911-batched-without-explicit-strict`

Port of Ruff's B911 (batched-without-explicit-strict), from target version 3.13 (`when`; `batched_without_explicit_strict.rs:72`): a call to `itertools.batched` with no `strict=` keyword (find_keyword, through `arguments` in syntactic mode) whose first positional argument exists, is not `*` unpacking and is not an infinite iterator. The callee resolves with `resolves` (imports, their scope and order as Ruff's semantic model sees them). Reported on the call.

### `ruff-b912-map-without-explicit-strict`

Port of Ruff's B912 (map-without-explicit-strict), from target version 3.14 (`when`; `expression.rs:797`): a call to builtin `map` (bare `map`, or `map` from `builtins`) with no `strict=` keyword (find_keyword, through `arguments` in syntactic mode), at least three positional arguments (a function and two iterables) or a `*` argument, and no infinite iterator among the arguments after the first. `map` and the itertools names resolve with `resolves`, so a shadowed `map` is not reported. Reported on the call.

### `ruff-ble001-blind-except`

Port of Ruff's BLE001 (blind-except): the first builtin `Exception` or `BaseException` (`resolve_builtin_symbol`: the unshadowed name or `builtins.`) in an `except` clause's type, searching nested tuples, reported on it, unless the handler body (not entering nested `try`, `def` or `class`) re-raises (`raise`, `raise e`, `raise ... from e` with the handler's name) or logs the exception: an expression-statement call of `exception`, or of another logger method with `exc_info` not known falsy, on a logger candidate (a value resolving to `logging` or `flask.current_app.logger`, an inline `logging.getLogger(...)`/`Logger(...)`, or a name whose last part starts with `log`/`LOG` or ends with `logger`/`logging` and that has no qualified name: `resolves.import: true`, Ruff's `resolve_qualified_name(..).is_some()`), or a name resolving to `logging.<method>`.

### `ruff-e402-module-import-not-at-top-of-file`

Port of Ruff's E402 (module-import-not-at-top-of-file): an `import` or `from ... import` statement directly at module level after an import boundary, reported on the statement. A boundary is any earlier module-level statement other than an import, the module docstring (a plain string literal as the first statement), a single-target assignment to a dunder name, and the allowed path/environment setup calls (`sys.path` mutation, `os.environ` edits, `os.putenv`/`unsetenv`, `matplotlib.use`, `pytest.importorskip`, `site.addsitedir`); `if`, `try`, `with` and `match` blocks and everything inside them are never boundaries. Qualified names are resolved through the file's imports as Ruff resolves them at that point of the module.

### `ruff-e712-true-false-comparison`

Port of Ruff's E712 (true-false-comparison) with default settings: `True` or `False` compared with `==` or `!=`, reported on the whole comparison. Operands are checked as for E711; `True == True` (same literal on both sides of `==`) is not reported.

### `ruff-e713-not-in-test`

Port of Ruff's E713 (not-in-test): `not` applied to a comparison whose only operator is `in` (the comparison may be parenthesized), reported on the comparison inside any parentheses; also inside a string Ruff parses as a type definition (M11a `parse` with `annotation`, nested strings included), reported on each such comparison inside the string.

### `ruff-e714-not-is-test`

Port of Ruff's E714 (not-is-test): `not` applied to a comparison whose only operator is `is` (the comparison may be parenthesized), reported on the comparison inside any parentheses; also inside a string Ruff parses as a type definition (M11a `parse` with `annotation`, nested strings included), reported on each such comparison inside the string.

### `ruff-e731-lambda-assignment`

Port of Ruff's E731 (lambda-assignment): a plain or annotated assignment statement with a single name target whose value is a `lambda` (parentheses are transparent), reported on the statement. Chained assignments are skipped, as are annotated assignments whose enclosing scope is a class body.

### `ruff-e741-ambiguous-variable-name`

Port of Ruff's E741 (ambiguous-variable-name): a name `l`, `I` or `O` bound as a variable: assignment, augmented-assignment, annotated, `for`/comprehension, `with ... as`, `except ... as` and walrus targets (through tuple, list and starred unpacking), `global`/`nonlocal` names, function and lambda parameters, and `type` alias names. Not match-pattern captures, imports, type parameters or `del`. Stub files (`.pyi`) are skipped.

### `ruff-f401-unused-import`

Port of Ruff's F401 (unused-import) on the engine's usedness (M10a `binds`): the latest binding of a name made by `import` or `from ... import` (not `__future__`, relative ones included as Ruff ignores their level; not `*`) in a module or function scope that nothing reads, not an explicit re-export (`x as x`, a name a static `__all__` lists), not under `global`, not an import of a module Ruff's option `pyflakes.allowed-unused-imports` lists (or of its submodules or members). Reads through a submodule's alias (`import pyarrow.csv` read as `pa.csv`) and `__all__` entries count. Reported on the unaliased import item's dotted name or on the alias. An `__all__` that may list names only at run time (a call, conditional or comprehension) leaves usedness unknown; names in strings Ruff parses as type definitions are reads (M11a).

### `ruff-f402-import-shadowed-by-loop-var`

Port of Ruff's F402 (import-shadowed-by-loop-var) on the engine's shadowed-binding pairs (M10a `binds.redefines`): a `for` statement's loop variable (any nesting of its target) whose shadowed binding, in its own scope or the enclosing function or module scope, is an import (used or not), on the same fork (Ruff's same_branch). Reported on the loop variable.

### `ruff-f404-late-future-import`

Port of Ruff's F404 (late-future-import): a `from __future__ import` (also relative, by the module's text with `resolves.imported` and `relative: "text"`, as Ruff ignores the level) after the futures boundary, reported on the statement. The boundary is closed by any statement other than a `__future__` import or a string-literal module docstring in first position (ruff_python_parser semantic_errors.rs `visit_stmt`); a compound statement closes it before its body is visited, so a nested `__future__` import is always late.

### `ruff-f631-assert-tuple`

Port of Ruff's F631 (assert-tuple) with default settings: an `assert` whose test (not its message) is a non-empty tuple, parenthesized any number of times. Reported on the whole statement.

### `ruff-f634-if-tuple`

Port of Ruff's F634 (if-tuple) with default settings: the test of an `if` or `elif` branch (not a conditional expression, not `while`) that is a non-empty tuple, reported on the tuple (inside any extra parentheses).

### `ruff-f704-yield-outside-function`

Port of Ruff's F704 (yield-outside-function, a semantic syntax error) with default settings: `yield`, `yield from` or `await` whose scope does not allow it. Scopes are found by climbing: a function or lambda body allows both; a class body or the module allows neither; a list, set or dict comprehension rejects `yield` and is transparent for `await`; a generator expression rejects `yield` and allows `await`. The first iterable of a comprehension, parameter defaults, decorators and class bases belong to the enclosing scope. Reported on the expression.

### `ruff-f722-forward-annotation-syntax-error`

Port of Ruff's F722 (forward-annotation-syntax-error) with default settings: a string Ruff parses as a type definition (a string annotation, a `type` value, a `cast`/`TypeVar` argument, a typing generic's `"..."` slice), read with the `annotation` grammar. Three checks: a string Ruff's parser refuses (the parse is one `ERROR` node), reported on the string; each semantic syntax error Ruff's checker reports in a string it parsed (`await` or `yield` outside a function, a walrus rebinding a comprehension variable, duplicate arguments, `__debug__` stored, a name used before its `global`, ...: an `ERROR` node after the expression), reported at the offending node; and Ruff's second error on an `await` evaluated outside any function ("`await` statement outside of a function": the string's host is not in a function body with no class between them, and the `await` is not in a lambda's body or a generator expression's element, conditions or later iterables), at the `await`. Strings nested in one (`List['a b']` inside a string annotation) are read too. Not inside a function decorated with a `no_type_check` that resolves to `typing`, `_typeshed` or `typing_extensions` (Ruff's match_typing_expr): its parameters, defaults, return annotation and body. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F722.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it. Divergence: a string after the first element of a subscript whose generic resolves to a jaxtyping import (`Float[Array, "b n"]`, `jaxtyping.Float[...]`, `jt.Shaped[...]`) is a jaxtyping shape, which jaxtyping parses with its own grammar, not a forward annotation, so it is not reported. Ruff 0.16.9 reports it. A shape inside a string annotation (`"Float[Array, 'b n']"`) is still reported.

### `ruff-f811-redefined-while-unused`

Port of Ruff's F811 (redefined-while-unused) with default settings, on the engine's shadowed-binding pairs (M10a `binds.redefines`): a binding (not a loop variable) whose shadowed binding is unused, on the same fork, where Ruff's Binding::redefines allows the pair (an import redefines an import, a definition or an assignment; a submodule import only the same qualified name, `sameImport`; an assignment only an import or a definition; any other binding an import, a definition or an assignment). Same scope: a dummy name is exempt unless the shadowed binding is an import, and a `def` decorated with an `overload` that resolves to `typing`, `_typeshed` or `typing_extensions` is never redefined. Across scopes: only a shadowed import that is no explicit re-export, and not from a class body. Reported on the redefining name. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F811.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f821-undefined-name`

Port of Ruff's F821 (undefined-name), on the engine's name resolution (M10a `resolves.unresolved`): a reference with no binding in scope (or one an unconditional `del` unbound), no builtin of the target version or of Ruff's option `builtins` and no visible star import, outside a `try` body whose handler catches the builtin `NameError`; and such a name inside a string Ruff parses as a type definition (M11a `parse` with `annotation`: string annotations, `cast`/`TypeVar` arguments, typing generics' string slices, nested strings), reported on each such name inside the string. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F821.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f841-unused-variable`

Port of Ruff's F841 (unused-variable), on the engine's usedness (M10a `binds`): the latest binding of a name in a function or lambda scope made by `=` (plain or annotated), `+=`, `:=`, `with ... as name`, a match capture or a `type` alias (Ruff's Assignment, NamedExprAssignment and WithItemVar) that nothing reads, not under `global`/`nonlocal`, not an unpacked assignment (Ruff's is_unpacking_assignment: a name in a tuple-like target whose value is no display, or in a chain with a non-tuple target, a walrus in the value of an all-tuple unpacking, a `with` tuple target), not a dummy (Ruff's option `dummy-variable-rgx`) or traceback name; and any `except ... as name` nothing reads, not a dummy name. A `locals()` call in the function leaves its assignments' usedness unknown (Ruff's USES_LOCALS; not the `except` names'). Names in strings Ruff parses as type definitions are reads (M11a). Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F841.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-n999-invalid-module-name`

Port of Ruff's N999 (invalid-module-name): a `.py` or `.pyi` file in a package (the host sent its module path) whose name is not a valid module name (`is_module_name`: `[a-z_][a-z0-9_]*`, not a keyword), reported at the start of the file. The name is the file's stem (`moduleName`), or for an `__init__` file the package root's directory (`package`: Ruff reads `package.path()`, the top package, not the file's own directory), as on disk. In a `versions` or `migrations` directory a leading digit is allowed (`is_migration_name`). Names matching `pep8-naming.ignore-names` (Ruff's option, with its defaults setUp, tearDown, ...) are exempt.

### `ruff-pie790-unnecessary-placeholder`

Port of Ruff's PIE790 (unnecessary-placeholder): a `pass` statement, or an expression statement that is only `...`, in a body (module, class, function or compound-statement block) of two or more statements. An `...` is kept in a `TYPE_CHECKING` block, a `typing.Protocol` class or an `abc.abstractmethod`-decorated function, and there Ruff stops scanning the body, so a later `pass` is kept too.

### `ruff-pie808-unnecessary-range-start`

Port of Ruff's PIE808 (unnecessary-range-start): `range(0, stop)` (an int literal zero, any spelling) with exactly two positional arguments and no keywords, where `range` resolves to the builtin (`match_builtin_expr`). Reported on the `0`.

### `ruff-ret503-implicit-return`

Port of Ruff's RET503 (implicit-return): a function that has a `return` with a value other than `None` and whose last statement can fall through (Ruff's has_implicit_return: recursively the last statement of each `if`/`elif`/`else`, `for`/`while` `else`, `match` case and `with` body; an `if` without `else` and a loop without `else` fall through; `while True`, `assert False`, `return`, `raise`, `try` and calls of known no-return functions do not). No-return functions are `exit`/`quit` (builtin, `sys`, `_thread`, `pytest`), `os`/`posix` `_exit`/`abort`, `_winapi.ExitProcess`, `pytest.fail`/`skip`/`xfail`, `assert_never` from `typing`/`_typeshed`/`typing_extensions`, and functions defined before the analysed function with a `NoReturn`/`Never` return annotation; the callee names are resolved through the file's bindings and imports. Reported on the function, from its first decorator.

### `ruff-ret504-unnecessary-assign`

Port of Ruff's RET504 (unnecessary-assign): `return x` whose preceding statement (comments aside) is `x = value` with a single name target, or a `with` block (not using `contextlib.suppress`) whose last statement is such an assignment, reported on the returned name. Skipped when `x` is declared `global`/`nonlocal` or bare-annotated in the function, referenced in a `finally` enclosing the return (a read or `del` resolving to a binding in the function's scope), or when the function scope's latest binding of `x` has a reference from another scope (`binds` with `latest` and `nestedUse`, which inherits the references of the bindings it shadows); generators and functions whose body is a lone `return` are skipped.

### `ruff-sim102-collapsible-if`

Port of Ruff's SIM102 (collapsible-if) with default settings: an `if` whose body (or whose last `elif` clause's body, when the last clause is an `elif`) is exactly one `if` without `elif`/`else`, when the `if`'s own body is one statement, the tested condition is not `__name__ == "__main__"` or a boolean literal, and the `if` is not a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Every `if` of a nested chain is reported, as Ruff does (its parent-chain exemption compares the parent with itself and never applies). Reported at the `if` or `elif` keyword.

### `ruff-sim103-needless-bool`

Port of Ruff's SIM103 (needless-bool): an `if`/`else`, a trailing `elif`/`else`, or an `if` followed by a `return` statement, whose two branches are single `return True` / `return False` statements with opposite values, unless the `if` is a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Reported at the `if` (or the last `elif`).

### `ruff-sim105-suppressible-exception`

Port of Ruff's SIM105 (suppressible-exception) at target 3.14: a `try` with one simple statement (expression, assignment, `del`, `assert`, import or `pass`), exactly one `except` handler whose body is only `pass` or `...`, no `else` or `finally`, and whose handled types are all names or attribute chains (or none). Reported on the `try` statement.

### `ruff-sim108-if-else-block-instead-of-if-exp`

Partial port of Ruff's SIM108 (if-else-block-instead-of-if-exp): an `if`/`else` whose two bodies are each one plain assignment `N = value` to the same name (no yield/await value), unless it is a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Missing: Ruff reports only when the generated ternary fits the line length.

### `ruff-sim110-reimplemented-builtin`

Partial port of Ruff's SIM110 (reimplemented-builtin): in a function scope, a `for` loop whose body is one `if` (no `elif`/`else`) returning a boolean literal, followed by a `return` of the opposite literal in the loop's `else` or as the next statement; the loop target and the `if` test contain no `await`/`yield`. Missing: Ruff reports only when the generated `return any(...)`/`all(...)` fits the line length.

### `ruff-sim114-if-with-same-arms`

Port of Ruff's SIM114 (if-with-same-arms): two consecutive `if`/`elif` branches whose bodies are equal as Ruff's ComparableStmt (`text: "structure"`; names without NFKC normalization) and whose body ranges hold the same comment texts in the same order, reported at the first branch. A branch's body range runs from the end of its test's line to the end of its last statement's line (M12a `layout`): a comment on the test's line or after the last statement's line is left out. Up to 3 comments per branch are compared; a branch with more is not reported.

### `ruff-sim115-open-file-with-context-handler`

Port of Ruff's SIM115 (open-file-with-context-handler): a call of a file-opening function (builtin `open`, `io.open`, `codecs.open`, `tempfile.TemporaryFile`, `tarfile.open`, `dbm.*.open`, ... from Ruff's table) or of `open` on a `pathlib.Path`/`zipfile.ZipFile`/`lzma.LZMAFile`/`tarfile.TarFile` instance created in place, outside a `with` or `return` statement, not immediately `.close()`d, not passed to `enter_context` under `with contextlib.ExitStack()` (or the async variant) or to unittest's `enterContext`, and not directly in `__enter__`. Names resolve through the file's imports and the builtin scope (`resolve_qualified_name`). Reported on the callee.

### `ruff-sim116-if-else-block-instead-of-dict-lookup`

Port of Ruff's SIM116 (if-else-block-instead-of-dict-lookup): an `if` whose test and every `elif` test is `N == <literal>` (one `==`, the same name `N`, a string/bytes/number/boolean/None/ellipsis literal), each branch (and any `else`) being one `return` whose value has no side effect (Ruff's `contains_effect`), with at least three distinct literals (Ruff's ComparableLiteral, `text: "structure-parts"`: type and value, `1 == 0x1`, `'a' == "a"`, not `1 == 1.0`; an implicit concatenation part by part), unless `N` resolves to `sys.version_info`/`sys.platform`.

### `ruff-sim117-multiple-with-statements`

Port of Ruff's SIM117 (multiple-with-statements): a `with` whose body is exactly one `with` of the same kind (both `async` or neither), unless either has a single item that calls `asyncio.timeout`/`timeout_at`, `anyio.CancelScope`/`fail_after`/`move_on_after` or `trio.fail_after`/`fail_at`/`move_on_after`/`move_on_at` (`resolve_qualified_name`); only the outermost `with` of a mergeable chain is reported.

### `ruff-sim118-in-dict-keys`

Port of Ruff's SIM118 (in-dict-keys): `key in X.keys()` / `key not in X.keys()` (a single comparison), a non-async `for key in X.keys()` and a comprehension `for key in X.keys()`, where `.keys()` has no arguments and `X` is not the name `self`. Reported from the key (with its parentheses).

### `ruff-sim201-negate-equal-op`

Port of Ruff's SIM201 (negate-equal-op): `not` applied to a comparison (through any parentheses) whose only operator is `==`, reported on the `not` expression. Exempt, as Ruff: when the enclosing statement is an `if` whose body is a single `raise` (Ruff's `is_exception_check` reads the `if` statement's own body, so an `elif` test counts with it), and when the nearest enclosing scope is a function named `__eq__`, `__ne__`, `__lt__`, `__le__`, `__gt__` or `__ge__` (lambdas, comprehensions and nested functions are their own scopes).

### `ruff-sim202-negate-not-equal-op`

Port of Ruff's SIM202 (negate-not-equal-op): `not` applied to a comparison (through any parentheses) whose only operator is `!=`, reported on the `not` expression. Exempt, as Ruff: when the enclosing statement is an `if` whose body is a single `raise` (Ruff's `is_exception_check` reads the `if` statement's own body, so an `elif` test counts with it), and when the nearest enclosing scope is a function named `__eq__`, `__ne__`, `__lt__`, `__le__`, `__gt__` or `__ge__` (lambdas, comprehensions and nested functions are their own scopes).

### `ruff-sim220-expr-and-not-expr`

Port of Ruff's SIM220 (expr-and-not-expr): an `and` expression (one run of the operator; parentheses start a new one) with a value `not A` and a value `A`, the same plain name (through parentheses), and no side effect anywhere in it (Ruff's `contains_effect`). Reported on the whole `and` expression, once per such pair.

### `ruff-sim221-expr-or-not-expr`

Port of Ruff's SIM221 (expr-or-not-expr): an `or` expression (one run of the operator; parentheses start a new one) with a value `not A` and a value `A`, the same plain name (through parentheses), and no side effect anywhere in it (Ruff's `contains_effect`). Reported on the whole `or` expression, once per such pair.

### `ruff-sim222-expr-or-true`

Port of Ruff's SIM222 (expr-or-true), `is_short_circuit` in flake8_simplify/rules/ast_bool_op.rs:736. Over the values of one `or` run (tree-sitter nests them to the left; parentheses start a new run), a value whose truthiness is unknown is skipped when the run is not in a boolean test, or when it has a side effect (`contains_effect`). The first value that is not skipped and is truthy (`True`, a non-empty literal, ...), or whose successor is, triggers; the finding starts at the value after the last skipped value before it (the whole expression when none), as Ruff's `furthest`. Truthiness follows `Truthiness::from_expr`, including f-string emptiness and `list(...)`-style calls. The finding spans that value only; Ruff's range runs to the end of the expression. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/SIM222.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-sim223-expr-and-false`

Port of Ruff's SIM223 (expr-and-false), `is_short_circuit` in flake8_simplify/rules/ast_bool_op.rs:736. Over the values of one `and` run (tree-sitter nests them to the left; parentheses start a new run), a value whose truthiness is unknown is skipped when the run is not in a boolean test, or when it has a side effect (`contains_effect`). The first value that is not skipped and is falsy (`False`, `None`, zero, an empty literal, ...), or whose successor is, triggers; the finding starts at the value after the last skipped value before it (the whole expression when none), as Ruff's `furthest`. Truthiness follows `Truthiness::from_expr`, including f-string emptiness and `list(...)`-style calls. The finding spans that value only; Ruff's range runs to the end of the expression.

### `ruff-sim401-if-else-block-instead-of-dict-get`

Port of Ruff's SIM401 (if-else-block-instead-of-dict-get): `if K in D: V = D[K] else: V = DEFAULT` (or the `not in` form with the arms swapped), and the conditional-expression form, where the default has no side effect (`contains_effect`). Operands are compared by source text, where Ruff compares expressions (`ComparableExpr`). The statement form needs `D` to be a name whose binding is a dict (`is_dict` through `resolves.binding` with `only`: Ruff's is_known_to_be_of_type_dict, whose only_binding asks that the binding shadows no earlier one). Partial: Ruff also asks that the suggested `V = D.get(K, DEFAULT)` fits the line length.

### `ruff-sim911-zip-dict-keys-and-values`

Port of Ruff's SIM911 (zip-dict-keys-and-values): `zip(d.keys(), d.values())`, the same plain name on both calls, no arguments to either, no keyword but `strict`, and `zip` the builtin (or `builtins.zip`). Reported on the `zip` call, where `d`'s binding is a dict (Ruff's `is_dict` through `resolves.binding`: a `def`'s `**kwargs`, a dict-typed value or `dict(...)`, a `dict`/`Dict` annotation or return annotation; unpacking carries no value).

### `ruff-w605-invalid-escape-sequence`

Port of Ruff's W605 (invalid-escape-sequence): a backslash in a non-raw string, bytes, f-string or t-string literal part that does not start a valid escape (newline, backslash, quotes, `a b f n r t v`, octal digits `0`-`7`, `x`, `N`, `u`, `U`), one finding per backslash. tree-sitter-python leaves such a backslash as a bare `\` token inside `string_content` (valid escapes are `escape_sequence` nodes); f-string format specifiers keep their text unsplit, so a regex over the specifier walks its escapes and `each` reports every invalid backslash (a specifier with none binds no group and reports nothing).

### `ruff-a005-stdlib-module-shadowing`

Port of Ruff's A005 (stdlib-module-shadowing): a `.py` file whose module, or a package's `__init__.py` whose package, is named like a standard-library module of the target version (`is_known_standard_library`, one `when`-gated check per range of versions; `_private` names are allowed), reported at the start of the file. Its options are Ruff's, with Ruff's defaults: without `flake8-builtins.strict-checking` the scope takes only top-level modules (files directly under the project root or a `src` root, and `x/__init__.py` there, never a root's own `__init__.py`); `flake8-builtins.allowed-modules` leaves files and packages so named out of scope.

### `ruff-inp001-implicit-namespace-package`

Port of Ruff's INP001 (implicit-namespace-package): a `.py` file outside any package (the host sent no module path), reported at byte 0 (`file.start`) as Ruff reports it. Files directly under the project root or `src/` (Ruff's default `src`), files whose first comment starts at byte 0 with `#!` (a shebang: M12a `layout` `start`) and PEP 723 scripts (a `# /// script` comment block closed by `# ///`) are exempt.
