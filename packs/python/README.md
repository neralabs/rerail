# Rerail rule pack: Python

136 code rules for Python, ported from Ruff 0.16.9 (98 rules) and Bandit 1.9.4 (38 tests). Each is scored against the
tool's own findings on the tool's test fixtures, through the engine's production workspace path, with every
disagreement declared in `divergences.json` (one today: ASYNC221 on a probe). Every port honours its tool's
suppression comment, as the tool does: Ruff's `# noqa` (`suppress.noqa`) and Bandit's `# nosec` (`suppress.nosec`).
24 Ruff rules that Ruff's default selection leaves off ship off (`"on": false` in `pack.json`).

This is the reference pack for the [pack structure and validation contract](../../docs/pack-format.md): it is
checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and whether it is on; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (django) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), the cases Rerail wrote for it (`fixtures/rerail/`, listed in `UPSTREAM.md`: every shipped rule on lines carrying `# noqa` or `# nosec`, and probes of inputs the ports once misread), its recordings (`fixtures.expected.json`, `corpus-django.expected.json`) and the recorder (`generate.py`, sharing `upstream/common.py`) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |
| `candidates/rules/` | Ports still being evaluated (98 Ruff and Bandit ports): never shipped or listed in `pack.json`. 96 have a `conformance.json` port and are scored against the recordings without gating; A005 (GPL-derived fixtures only) and INP001 (tested only with a `namespace-packages` option) are not recorded |

Re-recording runs offline against the pinned tool versions, under CPython 3.14 (Bandit itself runs from its own
install, on CPython 3.13):

```sh
python3.14 upstream/ruff/generate.py fixtures <ruff checkout at 0.16.9> <ruff 0.16.9 executable>
python3.14 upstream/ruff/generate.py corpus <ruff 0.16.9 executable> django=<django checkout>
python3.14 upstream/bandit/generate.py fixtures <bandit checkout at 1.9.4> <bandit 1.9.4 executable>
python3.14 upstream/bandit/generate.py corpus <bandit 1.9.4 executable> django=<django checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack python --rule ruff-f823-undefined-local --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `bandit-b101-assert-used`

Port of Bandit 1.9.4's B101 assert_used with its default configuration, which skips no file (tests included; scope is every file).

### `bandit-b102-exec-used`

As in Bandit, the callee is a bare name whose qualified name through the file's import aliases is `exec`: `exec` itself unless an import aliases it to another name, or a name imported as the module `exec` (`import exec as x`). `from m import exec as y` does not count.

### `bandit-b307-eval`

Same alias resolution as B102: a bare `eval` unless an import rebinds it, or a name bound by `import eval as x`.

### `bandit-b113-request-without-timeout`

Port of Bandit 1.9.4's B113 request_without_timeout: a `requests.get/options/head/post/put/patch/delete` call without `timeout`, or a `requests`/`httpx` call (httpx also covers `request`, `stream`, `Client`, `AsyncClient`) whose `timeout` keyword is `None`. Bandit reads `timeout` as a keyword only. httpx calls without `timeout` are not flagged (httpx has a default). Port of the kernel test b113_requests_without_a_timeout_or_with_none (rerail/native/src/code_rule_dsl/value/rules.rs).

### `bandit-b104-hardcoded-bind-all-interfaces`

Matches any Python string constant whose decoded value is exactly `0.0.0.0`, regardless of where it is used.

### `bandit-b108-hardcoded-tmp-directory`

Matches string constants (and implicitly concatenated strings as a whole) whose decoded value starts with `/tmp`, `/var/tmp` or `/dev/shm`; strings that are bare expression statements (docstrings) are skipped.

### `bandit-b103-set-bad-file-permissions`

Any call whose callee name contains `chmod`, with a literal mode. World-write (0o002) is reported by its own clause (higher severity in Bandit); group-write, group-execute or world-execute (0o031 bits) by the other.

### `bandit-b202-tarfile-unsafe-members`

Old description claimed: allow `filter="data"`, otherwise classify missing members, members supplied by a call, and other members expressions separately. The current clause does none of that — it flags every `extractall` on a value bound from `tarfile.open`.

### `bandit-b303-md5`

Covers PyCrypto/PyCryptodome `MD2/MD4/MD5/SHA.new` and cryptography `hashes.MD5/SHA1`. `hashlib` is Bandit's B324 (candidate rule `bandit-b324-hashlib-insecure-functions`).

### `bandit-b324-hashlib-insecure-functions`

hashlib `md4/md5/sha/sha1` and `hashlib.new("md4"|"md5"|"sha"|"sha1")` unless `usedforsecurity=False`; `crypt.crypt`/`crypt.mksalt` with `METHOD_CRYPT`, `METHOD_MD5`, `METHOD_BLOWFISH`, or with no salt/method at all.

### `bandit-b317-xml-bad-sax`

Only matches `xml.sax.parse/parseString/make_parser` calls that are direct children of an expression statement.

### `bandit-b406-import-xml-sax`

Port of Bandit 1.9.4's B406 import_xml_sax. Divergence: an import of only xml.sax.saxutils (`from xml.sax.saxutils import ...`, `from xml.sax import saxutils`, aliased or parenthesised) is not reported. saxutils holds XML writing and escaping helpers (escape, quoteattr, unescape, XMLGenerator) and no parser, so defusedxml has nothing to replace it with. `import xml.sax.saxutils` is still reported, since it also binds xml.sax and its parse functions (NER-1827).

### `bandit-b501-request-with-no-cert-validation`

Covers requests HTTP-verb calls (get, options, head, post, put, patch, delete) and httpx request/stream/Client/AsyncClient calls whose `verify` keyword is the literal False or the string 'False'. The callee root is read as Bandit's call qualname does: through the head's import alias, with relative imports named by their module text and the dots ignored, otherwise by the head's own text.

### `bandit-b505-weak-cryptographic-key`

Port of Bandit 1.9.4's B505 weak_cryptographic_key with Bandit's medium thresholds: DSA/RSA below 2048 bits or an EC curve below 224 bits (SECP192R1, SECT163K1, SECT163R2), by keyword or position, across cryptography, Crypto and Cryptodome. Port of the kernel test b505_weak_key_sizes_by_keyword_or_position. `# nosec` honoured.

### `bandit-b506-yaml-load`

Port of Bandit 1.9.4's B506 yaml_load: `yaml.load` without `Loader`, or with a `Loader` (by keyword or second position) other than SafeLoader or CSafeLoader. Port of the kernel test b506_yaml_load_without_a_safe_loader. `# nosec` honoured.

### `bandit-b507-ssh-no-host-key-verification`

Only fires once an import whose name contains "paramiko" has been walked (Bandit's substring test, in Bandit's source-order walk). Flags a call whose function name is set_missing_host_key_policy (attribute name, bare name, or a name aliased by an earlier import to one ending in set_missing_host_key_policy) and whose first positional argument is a name, attribute or call naming AutoAddPolicy or WarningPolicy.

### `bandit-b604-any-other-function-with-shell-equals-true`

A `shell=` keyword counts as truthy per Bandit's has_shell: anything but a falsy constant (False, None, a zero number, an empty string or bytes), an empty list or an empty dict. Calls to subprocess.Popen/call/check_call/check_output/run are excluded (covered by B602).

### `bandit-b610-django-extra-used`

Port of Bandit 1.9.4's B610 django_extra_used: a call of `extra` whose `where` or `tables` is not a list of plain strings, or whose `select` is not a dict of plain strings, bound to QuerySet.extra's six positional slots. Port of the kernel test b610_extra_with_a_non_literal_where_tables_or_select. `# nosec` honoured. Divergence: Bandit reports a call of any function or method named `extra`; the port reports only a method call whose receiver looks like a Django QuerySet or Manager: a name or attribute named `objects`, `qs`, `QS`, `queryset`, `querysets`, `manager` or `managers`, alone or after an underscore-joined prefix (`User.objects`, `self.queryset`, `base_qs`, `_default_manager`); a call of `get_queryset`; a method call on such a receiver (`User.objects.filter(...)`); or `self` or `super()` inside a class with a base whose name ends in `QuerySet` or is `Manager`, `BaseManager` or `BaseUserManager`, optionally dotted (`models.QuerySet`); a base that only ends in `Manager` (`AbstractContextManager`, `PluginManager`, `TreeManager`) does not count. A bare `extra(...)` call and `extra` on any other receiver are not reported (NER-1827).

### `bandit-b613-trojansource`

Closest AST-only approximation of Bandit B613: only bidirectional-control characters inside parsed Python string literals are detected (not comments or other tokens).

### `bandit-b615-huggingface-unsafe-download`

Requires a transformers, datasets or huggingface_hub-like import before the call. Allowed (not flagged): nonliteral revisions, hexadecimal revisions of at least seven characters, and local-path first arguments; all other unpinned downloads are flagged.

### `ruff-async100-cancel-scope-no-checkpoint`

Port of Ruff's ASYNC100: a `with` statement whose last timeout-context item (`asyncio.timeout[_at]`, `anyio.move_on_after`/`fail_after`/`CancelScope`, `trio.move_on_after`/`move_on_at`/`fail_after`/`fail_at`/`CancelScope` calls) is the last item when the statement is `async with`, and whose body has no `yield` (nested scopes included) and no `await`, `async with`, `async for` or async comprehension outside nested functions and classes. Reported on the whole statement. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async105-trio-sync-call`

Port of Ruff's ASYNC105: a call to one of trio's async functions (Ruff's MethodName::is_async list) that is not the operand of an `await` (parentheses ignored), in any context. Reported on the call. Names resolve via `resolves` (M7).

### `ruff-async109-async-function-with-timeout`

Port of Ruff's ASYNC109: a positional-only, positional or keyword-only parameter named `timeout` of an `async def` not decorated with `override` from `typing`, `_typeshed` or `typing_extensions`, reported on the parameter. `asyncio.timeout` exists from 3.11, so below 3.11 a parameter is flagged only after an absolute import of `trio` or `anyio` has been seen (Ruff's seen_module, M10a `resolves.seenBefore`; async_function_with_timeout.rs: `module == AsyncModule::AsyncIo && target_version < PY311`).

### `ruff-async110-async-busy-wait`

Port of Ruff's ASYNC110 (async-busy-wait): a `while` loop whose body is a single statement `await <call>` where the callee resolves to `trio.sleep`/`sleep_until`, `anyio.sleep`/`sleep_until` or `asyncio.sleep`, reported on the loop. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async115-async-zero-sleep`

Port of Ruff's ASYNC115 (async-zero-sleep): a call to `trio.sleep` or `anyio.sleep` with exactly one argument, an int literal whose value is zero (any radix; parentheses allowed) as `seconds=` (trio), `delay=` (anyio) or the first positional argument, in any context, reported on the call. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async116-long-sleep-not-forever`

Port of Ruff's ASYNC116 (long-sleep-not-forever): a call to `trio.sleep` or `anyio.sleep` with exactly one argument, an int or float literal (any spelling; not complex, not signed; parentheses allowed) whose value is greater than 86400 as `seconds=` (trio), `delay=` (anyio) or the first positional argument, in any context, reported on the call. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async210-blocking-http-call-in-async-function`

Port of Ruff's ASYNC210 (blocking-http-call-in-async-function): a call to `urllib.request.urlopen`, `urllib3.request` or an `httpx`/`requests` module-level HTTP method inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async212-blocking-http-call-httpx-in-async-function`

Port of Ruff's ASYNC212 (blocking-http-call-httpx-in-async-function): a blocking method call on a name whose binding holds an `httpx.Client(...)` call or is annotated with `httpx.Client` (directly or in `|`/`Optional`/`Union`: Ruff's check_type through `resolves.binding`) inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7). The binding is Ruff's only_binding (`resolves.binding` with `only`: it shadows no earlier binding of the name in its scope).

### `ruff-async220-create-subprocess-in-async-function`

Port of Ruff's ASYNC220 (create-subprocess-in-async-function): a call to `subprocess.Popen`, `os.popen`, or an `os.spawn*` function whose mode (Ruff's find_argument_value("mode", 0): `arguments` in syntactic mode) is given and is not `os.P_WAIT` (directly, or through a name whose value, read as find_assigned_value's lookup_symbol does (`resolves.binding` with `stopAtGlobal`), is `os.P_WAIT`) inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async221-run-process-in-async-function`

Port of Ruff's ASYNC221 (run-process-in-async-function): a call to `os.system`, `os.posix_spawn[p]`, `subprocess.run`/`call`/`check_call`/`check_output`/`getoutput`/`getstatusoutput`, or an `os.spawn*` function whose mode (Ruff's find_argument_value("mode", 0): `arguments` in syntactic mode) is absent or `os.P_WAIT` (directly, or through a name whose value, read as find_assigned_value's lookup_symbol does (`resolves.binding` with `stopAtGlobal`), is `os.P_WAIT`) inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async222-wait-for-process-in-async-function`

Port of Ruff's ASYNC222 (wait-for-process-in-async-function): a call to `os.wait`, `os.wait3`, `os.wait4`, `os.waitid` or `os.waitpid` inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async230-blocking-open-call-in-async-function`

Port of Ruff's ASYNC230 (blocking-open-call-in-async-function): a call to the builtin `open` (`builtins.open`, `io.open`, `io.open_code`), or `.open()` on a `pathlib.Path(...)` call or on a name whose binding holds one (Ruff's find_binding_value through `resolves.binding`: a plain name assignment, annotated assignment or walrus value, an unpacked name's element of a tuple or list display, or a `with` item's context expression; and, as Ruff's match_value, an unpacked name paired with a display element by index where the engine gives no value: a set display, unequal arities, a position after a starred value), inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7).

### `ruff-async240-blocking-path-method-in-async-function`

Port of Ruff's ASYNC240 (blocking-path-method-in-async-function): a call to an `os.path` function, or a method on a `pathlib` path (a `Path(...)`-family call, or a name whose binding holds such a call or is annotated with a path class, directly or in `|`/`Optional`/`Union`: Ruff's check_type through `resolves.binding`), except Ruff's list of pure methods, inside an `async def` body whose function is the nearest enclosing function, lambda body or class, reported on the callee. Names resolve as in Ruff's resolve_qualified_name, through the engine's `resolves` (M7). A bare name imported from `os.path` resolves through `prefix`. The binding is Ruff's only_binding (`resolves.binding` with `only`: it shadows no earlier binding of the name in its scope).

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

### `ruff-b006-mutable-argument-default`

Port of Ruff's B006 (mutable-argument-default), stable behaviour: a function parameter default that is a list, dict or set display or comprehension, or a call of `dict`/`list`/`set` or `collections.Counter`/`OrderedDict`/`defaultdict`/`deque` (a subscripted callee judged by its value; parentheses transparent), unless the parameter's annotation is immutable by Ruff's `is_immutable_annotation`: an immutable builtin, `typing` or `collections.abc` type (generic ones also subscripted), `None`, `X | Y`, `Union[...]` of immutables, `Optional[X]`, `Annotated[X, ...]`. Names resolve with `resolves` (imports, their scope and order, and unshadowed builtins, as Ruff's semantic model sees them); `builtins.X` counts only where Ruff's table lists it. Stub files are skipped. Lambdas are not checked (Ruff runs B006 on `def` only). Reported on the default. Ruff's option `flake8-bugbear.extend-immutable-calls` adds immutable annotations.

### `ruff-b007-unused-loop-control-variable`

Port of Ruff's B007 (unused-loop-control-variable) with the default `dummy-variable-rgx`: a name bound by a `for` loop's target (itself, or inside tuple/list/starred targets) that does not match `^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$` and never occurs as a name expression in the loop body (the `else` clause does not count; attribute members, keyword-argument names, def/class names, parameters, import and global/nonlocal names and except-handler names are not name expressions). The message says `may not be used` when the body calls `locals`/`globals`/`vars`/`exec`/`eval` (by spelling; Ruff checks they are builtins). Match-statement capture, keyword-pattern, star and `as` names are not name expressions either. Reported on the target name.

### `ruff-b008-function-call-in-default-argument`

Port of Ruff's B008 (function-call-in-default-argument): every call inside a parameter default (of a `def` or a `lambda`), not looking inside lambdas, unless the parameter's annotation is immutable (Ruff's `is_immutable_annotation`, as in B006) or the callee is a known mutable constructor (`dict`/`list`/`set`, `collections.Counter`/`OrderedDict`/`defaultdict`/`deque`) or immutable-returning callable (`datetime` types, `decimal.Decimal`, `fractions.Fraction`, `operator` getters, `pathlib.Path`, `types.MappingProxyType`, `re.compile`, immutable builtins; a subscripted callee judged by its value). Names resolve with `resolves` (imports, their scope and order, and unshadowed builtins, as Ruff's semantic model sees them); `builtins.X` counts only where Ruff's table lists it. A bare-name callee whose binding (`resolves.binding`) is `X = NewType(name, <immutable annotation>)` (Ruff's `is_immutable_newtype_call`: a plain assignment, the type as `tp=` or the second positional argument) is exempt where that binding shadows no earlier binding of the name in its scope (Ruff's only_binding: `resolves.binding` with `only`) and is not an unpacking. Reported on each call. Ruff's option `flake8-bugbear.extend-immutable-calls` adds immutable calls and annotations.

### `ruff-b009-get-attr-with-constant`

Port of Ruff's B009 (getattr-with-constant): `getattr(obj, "name")` with exactly two positional arguments and no keyword arguments, `obj` not starred, the second a str literal (not bytes, an f- or t-string; escapes decoded, implicit concatenations joined) whose value is a Python identifier (Unicode XID, not a keyword) and not a mangled private name (`__x` without a trailing `__`). The callee (parentheses transparent) resolves to the builtin `getattr` (Ruff's `match_builtin_expr`): the unshadowed name, or `getattr` of the `builtins` module through the file's imports. Reported on the call.

### `ruff-b010-set-attr-with-constant`

Port of Ruff's B010 (set-attr-with-constant): `setattr(obj, "name", value)` used as an expression statement, with exactly three positional arguments and no keyword arguments, `obj` not starred, the name a str literal (not bytes, an f- or t-string; escapes decoded, implicit concatenations joined) whose value is a Python identifier (Unicode XID, not a keyword), not `__debug__` and not a mangled private name. The callee (parentheses transparent) resolves to the builtin `setattr` (Ruff's `match_builtin_expr`): the unshadowed name, or `setattr` of the `builtins` module through the file's imports. Reported on the call.

### `ruff-b011-assert-false`

Port of Ruff's B011 (assert-false): an `assert` statement whose test is the literal `False` (Ruff's `is_const_false`; parentheses transparent), reported on the literal. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/B011.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-b012-jump-statement-in-finally`

Port of Ruff's B012 (jump-statement-in-finally): a `return`, `break` or `continue` statement in a `finally` block, reached through the bodies of nested `if` (not `elif`/`else`), `try` (not handlers, `else` or `finally`), `with` and `match` cases; `return` also through `while`/`for` bodies (not their `else`), where `break`/`continue` stop. Function and class bodies stop the walk. Reported on the statement.

### `ruff-b013-redundant-tuple-in-exception-handler`

Port of Ruff's B013 (redundant-tuple-in-exception-handler): the type of an `except` (or `except*`) handler is a parenthesized tuple with exactly one element that is not starred. Parentheses around the tuple are transparent, and an `as` name does not matter. Reported on the tuple.

### `ruff-b014-duplicate-handler-exception`

Port of Ruff's B014 (duplicate-handler-exception): the type of an `except` (or `except*`) handler is a tuple in which the same dotted name (a name or an attribute chain of names, Ruff's `UnqualifiedName`) occurs more than once (parentheses around an element are transparent). Reported once on the tuple, whatever the number of duplicates.

### `ruff-b015-useless-comparison`

Port of Ruff's B015 (useless-comparison): an expression statement whose value is a comparison (parentheses transparent). When the statement is the last one of a function body the message asks whether the result was meant to be returned. Reported on the comparison.

### `ruff-b016-raise-literal`

Port of Ruff's B016 (raise-literal): the exception of a `raise` statement is a literal (Ruff's `is_literal_expr`: string, bytes, number, `True`/`False`, `None`, `...`; not an f-string or t-string, including an implicit concatenation with one; parentheses transparent). Reported on the literal.

### `ruff-b017-assert-raises-exception`

Port of Ruff's B017 (assert-raises-exception): `<x>.assertRaises(...)` or `pytest.raises(...)` whose exception (first positional argument, or `exception=` / `expected_exception=`) resolves to the builtin `Exception` or `BaseException` (the unshadowed name, or through an import of `builtins`; parentheses transparent). `pytest.raises` is exempt with `match=` or a string/bytes literal second positional argument. Reported on a `with` item that has no `as` target, and on any such call with at least two positional arguments or a `func=` keyword (the call form). `pytest.raises` and the exception names resolve with `resolves` (imports, their scope and order, and shadowing, as Ruff's semantic model sees them).

### `ruff-b018-useless-expression`

Port of Ruff's B018 (useless-expression): an expression statement (parentheses transparent; a bare tuple counts) that is not a comparison, a string or f-string literal, or `...`, and has no effect in Ruff's `contains_effect` sense: no call other than an argument-less `set()`/`list()`/`tuple()`/`dict()`/`frozenset()`, no `await`, `yield`, walrus, comprehension, generator or subscript, and no binary operation with an operand that is not a literal or display, anywhere inside (lambda bodies included). An attribute access with an effect is reported as a useless attribute access. The builtin names must resolve to the unshadowed builtins (`resolves`, Ruff's `has_builtin_binding`). Reported on the expression.

### `ruff-b019-cached-instance-method`

Port of Ruff's B019 (cached-instance-method): a decorator `functools.cache` / `functools.lru_cache` (called or not) on an instance method: a function defined directly in a class body, not decorated with `staticmethod`/`classmethod` (or `abc.abstractstaticmethod`/`abc.abstractclassmethod`), not `__new__`, `__init_subclass__` or `__class_getitem__`, in a class that does not list an `enum` base (`Enum`, `Flag`, `IntEnum`, `IntFlag`, `StrEnum`, `ReprEnum`, `CheckEnum`). Names resolve with `resolves` (imports, their scope and order, and shadowing, as Ruff's semantic model sees them); a base naming a class defined in the file is followed to that class's bases (`resolves.binding` on its definition, cycles fail). Reported on the decorator.

### `ruff-b020-loop-variable-overrides-iterator`

Port of Ruff's B020 (loop-variable-overrides-iterator): a name in a `for` loop's target that also occurs as a name in its iterable, where comprehensions in the iterable contribute only the iterables of their `for` clauses and lambdas the children of their body (Ruff's `NameFinder` walks the body's children, so `lambda: x` adds nothing). A name that is a (non-variadic) parameter of a lambda in the iterable is exempt unless it occurs again after that lambda, as Ruff's walk removes it and a later occurrence adds it back. Reported on each such target name (Ruff reports only the last occurrence of a name repeated in the target).

### `ruff-b021-f-string-docstring`

Port of Ruff's B021 (f-string-docstring): the first statement of a module, function or class body is an expression statement whose value is an f-string (an `f`-prefixed literal, or an implicit concatenation containing one; parentheses transparent). Comments before it are not statements. Reported on the statement.

### `ruff-b022-useless-contextlib-suppress`

Port of Ruff's B022 (useless-contextlib-suppress): a call to `contextlib.suppress` with no positional arguments, reported on the call. The callee (parentheses transparent) resolves to `contextlib.suppress` with `resolves` (imports and shadowing as Ruff's semantic model sees them).

### `ruff-b023-function-uses-loop-variable`

Port of Ruff's B023 (function-uses-loop-variable): a name loaded in the body of a function or lambda defined inside a loop (for, while, or comprehension), when the function neither takes it as a parameter nor stores it anywhere in its body, and the loop assigns it (assignment, augmented or annotated assignment, for or comprehension target; not inside nested functions or lambdas). The function is the first def or unsafe lambda on the way down from the loop; a lambda is safe when it is returned directly, called immediately, a positional argument of `filter`/`reduce`/`map`/`functools.reduce`, or the value of a `key=` keyword. Reported on the name.

### `ruff-b024-abstract-base-class-without-abstract-method`

Port of Ruff's B024 (abstract-base-class-without-abstract-method): a class with exactly one base or keyword, which is `abc.ABC` as a base or `abc.ABCMeta` as `metaclass=`, whose body has no method decorated with an `abc` abstract decorator and no annotated attribute without a value. Names resolve with `resolves` (imports and shadowing as Ruff's semantic model sees them). Not in stubs. Reported on the class name.

### `ruff-b025-duplicate-try-block-exception`

Port of Ruff's B025 (duplicate-try-block-exception): an exception name (a name or dotted attribute chain) in an `except`/`except*` clause, as the type or a direct element of a tuple type, that an earlier handler of the same `try` already names; within one tuple only its first occurrence counts. Names compare structurally. Reported on the repeated name.

### `ruff-b026-star-arg-unpacking-after-keyword-arg`

Port of Ruff's B026 (star-arg-unpacking-after-keyword-arg): a `*` unpacking argument of a call that comes after the call's first keyword argument (`name=value` or `**mapping`). Reported on the starred argument.

### `ruff-b027-empty-method-without-abstract-decorator`

Port of Ruff's B027 (empty-method-without-abstract-decorator): in a class whose only base or keyword is `abc.ABC` or `metaclass=abc.ABCMeta`, a method directly in the class body whose body is only `pass`, string literals and `...`, with no `abc` abstract decorator and no `typing.overload` / `typing_extensions.overload` decorator. Names resolve with `resolves` (imports and shadowing as Ruff's semantic model sees them). Not in stubs. Reported on the whole definition, decorators included.

### `ruff-b028-no-explicit-stacklevel`

Port of Ruff's B028 (no-explicit-stacklevel): a call to `warnings.warn` with no `stacklevel` (keyword or third positional argument: find_argument_value, through `arguments` in syntactic mode), no non-empty `skip_file_prefixes`, and no `*`/`**` unpacking. The callee resolves with `resolves` (imports and shadowing as Ruff's semantic model sees them). Reported on the callee, inside any parentheses. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/B028.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-b029-except-with-empty-tuple`

Port of Ruff's B029 (except-with-empty-tuple): an `except`/`except*` clause whose type is the empty tuple `()` (inside any parentheses, with or without `as`). Reported on the whole clause.

### `ruff-b030-except-with-non-exception-classes`

Port of Ruff's B030 (except-with-non-exception-classes): flattens an `except`/`except*` type as Ruff's `flatten_iterables` does (a top-level tuple's elements; `*` unpacking of tuples and lists; both sides of `+`, splicing tuple sides and unpacking starred sides) and reports each resulting expression that is not a name, attribute, subscript or call. Parentheses and an `as` binding (`except E as e`, tree-sitter's as_pattern) are transparent.

### `ruff-b031-reuse-of-groupby-generator`

Partial port of Ruff's B031 (reuse-of-groupby-generator): in the body of `for k, g in itertools.groupby(...)`, a use of `g` inside a nested loop body or list/set/dict comprehension, or after an earlier statement (or the enclosing `if` test or `match` subject) that uses it. Ruff's per-branch use counting (maximum over if/match branches), resets at break/continue/return and reassignment of `g` are not modelled.

### `ruff-b032-unintentional-type-annotation`

Port of Ruff's B032 (unintentional-type-annotation): an annotated statement without a value whose target is a subscript of a name (`d[k]: v`) or an attribute of a name other than `self` (`o.a: v`), inside any parentheses (tree-sitter reads a parenthesized target as a one-element `tuple_pattern`). Reported on the statement.

### `ruff-b033-duplicate-value`

Port of Ruff's B033 (duplicate-value): a literal element of a set display (string, bytes, number, boolean, `None`, `...`, not an f- or t-string; parentheses are transparent) equal to an earlier literal element. Equality is Ruff's HashableExpr (`compare` `text: "structure-hash"`): numbers fold by value (`True == 1 == 1.0 == 0x1`), strings by decoded value (`'a' == "\x61"`, implicit concatenation joined), and an int of 2**64 or more by its token.

### `ruff-b034-re-sub-positional-args`

Port of Ruff's B034 (re-sub-positional-args): a call to `re.sub` or `re.subn` with more than three positional arguments, or to `re.split` with more than two (starred arguments count). The callee resolves with `resolves` (imports and shadowing as Ruff's semantic model sees them). Reported on the call.

### `ruff-b035-static-key-dict-comprehension`

Port of Ruff's B035 (static-key-dict-comprehension): the key of a dict comprehension is constant: a literal, a name the comprehension's generators never store (targets and walrus targets, nested ones included), or an attribute, subscript, operator or tuple built only from such parts. Reported on the key, inside any parentheses.

### `ruff-b039-mutable-contextvar-default`

Port of Ruff's B039 (mutable-contextvar-default): the `default=` of a `contextvars.ContextVar(...)` call (the callee may be subscripted) is a list, dict or set display or comprehension, or a call to anything other than a function Ruff knows returns an immutable value (`is_immutable_return_type`: `datetime`, `decimal`, `fractions`, `operator`, `pathlib.Path`, `types.MappingProxyType`, `re.compile`, immutable builtins, and Ruff's option `flake8-bugbear.extend-immutable-calls`). Names resolve with `resolves` (imports, and builtins only when unshadowed). Reported on the default, inside any parentheses.

### `ruff-b904-raise-without-from-inside-except`

Port of Ruff's B904 (raise-without-from-inside-except): a `raise X` without `from` in an `except`/`except*` body, reached only through `if`/`elif`/`else`, `with`, `for` and `while` bodies (not their `else`) and `match` cases; nested `try`, `def` and `class` are not entered. `raise name` re-raising the handler's own `as` name is exempt. Reported on the raise statement.

### `ruff-b905-zip-without-explicit-strict`

Port of Ruff's B905 (zip-without-explicit-strict), from target version 3.10 (`when`; `expression.rs:792`): a call to builtin `zip` (bare `zip`, or `zip` from `builtins`) with no `strict=` keyword (find_keyword, through `arguments` in syntactic mode), at least two positional arguments or a `*` argument, and no infinite iterator argument (`itertools.cycle`/`count`, `repeat` without a finite `times`). `zip` and the itertools names resolve with `resolves`, so a shadowed `zip` is not reported. Reported on the call. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/B905.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-b911-batched-without-explicit-strict`

Port of Ruff's B911 (batched-without-explicit-strict), from target version 3.13 (`when`; `batched_without_explicit_strict.rs:72`): a call to `itertools.batched` with no `strict=` keyword (find_keyword, through `arguments` in syntactic mode) whose first positional argument exists, is not `*` unpacking and is not an infinite iterator. The callee resolves with `resolves` (imports, their scope and order as Ruff's semantic model sees them). Reported on the call.

### `ruff-b912-map-without-explicit-strict`

Port of Ruff's B912 (map-without-explicit-strict), from target version 3.14 (`when`; `expression.rs:797`): a call to builtin `map` (bare `map`, or `map` from `builtins`) with no `strict=` keyword (find_keyword, through `arguments` in syntactic mode), at least three positional arguments (a function and two iterables) or a `*` argument, and no infinite iterator among the arguments after the first. `map` and the itertools names resolve with `resolves`, so a shadowed `map` is not reported. Reported on the call.

### `ruff-ble001-blind-except`

Port of Ruff's BLE001 (blind-except): the first builtin `Exception` or `BaseException` (`resolve_builtin_symbol`: the unshadowed name or `builtins.`) in an `except` clause's type, searching nested tuples, reported on it, unless the handler body (not entering nested `try`, `def` or `class`) re-raises (`raise`, `raise e`, `raise ... from e` with the handler's name) or logs the exception: an expression-statement call of `exception`, or of another logger method with `exc_info` not known falsy, on a logger candidate (a value resolving to `logging` or `flask.current_app.logger`, an inline `logging.getLogger(...)`/`Logger(...)`, or a name whose last part starts with `log`/`LOG` or ends with `logger`/`logging` and that has no qualified name: `resolves.import: true`, Ruff's `resolve_qualified_name(..).is_some()`), or a name resolving to `logging.<method>`.

### `ruff-e402-module-import-not-at-top-of-file`

Port of Ruff's E402 (module-import-not-at-top-of-file): an `import` or `from ... import` statement directly at module level after an import boundary, reported on the statement. A boundary is any earlier module-level statement other than an import, the module docstring (a plain string literal as the first statement), a single-target assignment to a dunder name, and the allowed path/environment setup calls (`sys.path` mutation, `os.environ` edits, `os.putenv`/`unsetenv`, `matplotlib.use`, `pytest.importorskip`, `site.addsitedir`); `if`, `try`, `with` and `match` blocks and everything inside them are never boundaries. Qualified names are resolved through the file's imports as Ruff resolves them at that point of the module.

### `ruff-e711-none-comparison`

Port of Ruff's E711 (none-comparison) with default settings: `None` compared with `==` or `!=`, reported on the `None` literal (inside any parentheses). The first operand is checked when the second is not a non-singleton constant; each later `None` when the operand before its operator is not one (Ruff's `is_constant_non_singleton`: string, bytes and number literals, and tuples of constants). Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/E711.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-e712-true-false-comparison`

Port of Ruff's E712 (true-false-comparison) with default settings: `True` or `False` compared with `==` or `!=`, reported on the whole comparison. Operands are checked as for E711; `True == True` (same literal on both sides of `==`) is not reported.

### `ruff-e713-not-in-test`

Port of Ruff's E713 (not-in-test): `not` applied to a comparison whose only operator is `in` (the comparison may be parenthesized), reported on the comparison inside any parentheses; also inside a string Ruff parses as a type definition (M11a `parse` with `annotation`, nested strings included), reported on each such comparison inside the string.

### `ruff-e714-not-is-test`

Port of Ruff's E714 (not-is-test): `not` applied to a comparison whose only operator is `is` (the comparison may be parenthesized), reported on the comparison inside any parentheses; also inside a string Ruff parses as a type definition (M11a `parse` with `annotation`, nested strings included), reported on each such comparison inside the string.

### `ruff-e721-type-comparison`

Port of Ruff's E721 (type-comparison): a comparison with an `==` or `!=` operator whose operand on either side is a type (a call whose callee resolves to the builtin `type` or `builtins.type`, or a name from Ruff's builtin type and exception list that resolves to the builtin) while neither side is a dtype (`x.dtype`, or a call of a callee resolving to `numpy.dtype`), reported on the whole comparison once per offending operator (a chained comparison can report twice, as in Ruff).

### `ruff-e722-bare-except`

Port of Ruff's E722 (bare-except) with default settings: an `except` clause that names no exception type, unless its body re-raises with a bare `raise` statement. Reported on the `except` keyword, as Ruff does. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/E722.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

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

Port of Ruff's F523 (string-dot-format-extra-positional-arguments): a `.format` call on a valid format string with a positional argument (not starred; counted by syntactic index, past a starred one too) that no automatic or numbered field reads. `formatArgs` reads the format string receiving `.format` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it), and the call's arguments as Ruff's strings.rs does. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F523.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f524-string-dot-format-missing-arguments`

Port of Ruff's F524 (string-dot-format-missing-arguments): a `.format` call without `*` or `**` on a valid format string with a field (top level or nested in a spec) whose index is past the positional arguments or whose name no keyword argument passes. `formatArgs` reads the format string receiving `.format` (a str literal with no f/t/b prefix, implicit concatenations joined, escapes decoded, parentheses transparent) with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it), and the call's arguments as Ruff's strings.rs does. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F524.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f525-string-dot-format-mixing-automatic`

Port of Ruff's F525 (string-dot-format-mixing-automatic): a `.format` call on a valid format string with both an automatically numbered field (`{}`) and a manually numbered one (`{0}`), nested fields in specs included. The format string is a str literal (no f/t/b prefix; implicit concatenations joined, escapes decoded, parentheses transparent) receiving `.format`, read with the `braceFormat` grammar (a port of ruff_python_literal's format parser as pyflakes' FormatSummary reads it).

### `ruff-f541-f-string-missing-placeholders`

Port of Ruff's F541 (f-string-missing-placeholders) with default settings: every f-string part of a string expression (an implicit concatenation counts as one expression) when no f-string part of that expression has a replacement field. `{{`/`}}` escapes are not placeholders; nested f-strings inside a replacement field or format spec are their own expressions.

### `ruff-f601-multi-value-repeated-key-literal`

Port of Ruff's F601 (multi-value-repeated-key-literal) with default settings: a literal dictionary key (string, bytes, number, boolean, None, Ellipsis, tuple, f-string, signed number, real +/- imaginary; Ruff's is_literal_key) equal to an earlier key of the same dict display, reported on each repeat (parentheses are transparent). Equality is Ruff's HashableExpr (`compare` `text: "structure-hash"`): numbers fold by value (`1 == 1.0 == True`, `1 + 0j == 1`), strings by decoded value, tuples of constants element by element, anything else structurally as ComparableExpr (names without NFKC normalization). Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F601.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f602-multi-value-repeated-key-variable`

Port of Ruff's F602 (multi-value-repeated-key-variable) with default settings: a dictionary key that is a bare name equal to an earlier key of the same dict display, reported on each repeat (parentheses transparent).

### `ruff-f622-multiple-starred-expressions`

Port of Ruff's F622 (multiple-starred-expressions, a semantic syntax error) with default settings: a tuple or list assignment target (assignment, `for`, comprehension `for`, `with ... as`), at any nesting, with two or more starred elements of its own. Reported on the target tuple or list, parentheses included.

### `ruff-f631-assert-tuple`

Port of Ruff's F631 (assert-tuple) with default settings: an `assert` whose test (not its message) is a non-empty tuple, parenthesized any number of times. Reported on the whole statement.

### `ruff-f632-is-literal`

Port of Ruff's F632 (is-literal) with default settings: `is` or `is not` with an operand that is a non-singleton constant (string, bytes or number literal, tuple of constants) or a list, set or dict display or comprehension, one finding per offending operator reported on the whole comparison, as in Ruff; also a comparison inside a string Ruff parses as a type definition (a string annotation, `cast`/`TypeVar` arguments, `List["..."]`), reported on the comparison inside the string, once per offending operator; strings nested in one (`List['1 is x']` inside a string annotation) are read too. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F632.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f633-invalid-print-syntax`

Port of Ruff's F633 (invalid-print-syntax) with default settings: the left operand of `>>` when it resolves to the builtin `print` (or `builtins.print`), reported on `print`. tree-sitter-python parses a statement `print >> f, x` as a Python 2 print statement whose `print` is a keyword token, not a name reference, so name resolution cannot be asked there; that form falls back to a file-wide test: no binding of `print` anywhere in the file (assignment, loop, with/except and walrus targets, def/class names, parameters, imports, global/nonlocal).

### `ruff-f634-if-tuple`

Port of Ruff's F634 (if-tuple) with default settings: the test of an `if` or `elif` branch (not a conditional expression, not `while`) that is a non-empty tuple, reported on the tuple (inside any extra parentheses).

### `ruff-f701-break-outside-loop`

Port of Ruff's F701 (break-outside-loop, a semantic syntax error) with default settings: `break` with no enclosing `for`/`while` body between it and the nearest function or class definition. A loop's `else` block does not count as inside that loop.

### `ruff-f702-continue-outside-loop`

Port of Ruff's F702 (continue-outside-loop, a semantic syntax error) with default settings: `continue` with no enclosing `for`/`while` body between it and the nearest function or class definition. A loop's `else` block does not count as inside that loop.

### `ruff-f704-yield-outside-function`

Port of Ruff's F704 (yield-outside-function, a semantic syntax error) with default settings: `yield`, `yield from` or `await` whose scope does not allow it. Scopes are found by climbing: a function or lambda body allows both; a class body or the module allows neither; a list, set or dict comprehension rejects `yield` and is transparent for `await`; a generator expression rejects `yield` and allows `await`. The first iterable of a comprehension, parameter defaults, decorators and class bases belong to the enclosing scope. Reported on the expression.

### `ruff-f706-return-outside-function`

Port of Ruff's F706 (return-outside-function, a semantic syntax error) with default settings: `return` whose nearest enclosing function or class body is not a function body (module level, or directly in a class body).

### `ruff-f707-default-except-not-last`

Port of Ruff's F707 (default-except-not-last) with default settings: a bare `except:` handler followed by another handler of the same `try`. Reported on the `except` keyword.

### `ruff-f722-forward-annotation-syntax-error`

Port of Ruff's F722 (forward-annotation-syntax-error) with default settings: a string Ruff parses as a type definition (a string annotation, a `type` value, a `cast`/`TypeVar` argument, a typing generic's `"..."` slice), read with the `annotation` grammar. Three checks: a string Ruff's parser refuses (the parse is one `ERROR` node), reported on the string; each semantic syntax error Ruff's checker reports in a string it parsed (`await` or `yield` outside a function, a walrus rebinding a comprehension variable, duplicate arguments, `__debug__` stored, a name used before its `global`, ...: an `ERROR` node after the expression), reported at the offending node; and Ruff's second error on an `await` evaluated outside any function ("`await` statement outside of a function": the string's host is not in a function body with no class between them, and the `await` is not in a lambda's body or a generator expression's element, conditions or later iterables), at the `await`. Strings nested in one (`List['a b']` inside a string annotation) are read too. Not inside a function decorated with a `no_type_check` that resolves to `typing`, `_typeshed` or `typing_extensions` (Ruff's match_typing_expr): its parameters, defaults, return annotation and body. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F722.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it. Divergence: a string after the first element of a subscript whose generic resolves to a jaxtyping import (`Float[Array, "b n"]`, `jaxtyping.Float[...]`, `jt.Shaped[...]`) is a jaxtyping shape, which jaxtyping parses with its own grammar, not a forward annotation, so it is not reported. Ruff 0.16.9 reports it (NER-1827). A shape inside a string annotation (`"Float[Array, 'b n']"`) is still reported.

### `ruff-f811-redefined-while-unused`

Port of Ruff's F811 (redefined-while-unused) with default settings, on the engine's shadowed-binding pairs (M10a `binds.redefines`): a binding (not a loop variable) whose shadowed binding is unused, on the same fork, where Ruff's Binding::redefines allows the pair (an import redefines an import, a definition or an assignment; a submodule import only the same qualified name, `sameImport`; an assignment only an import or a definition; any other binding an import, a definition or an assignment). Same scope: a dummy name is exempt unless the shadowed binding is an import, and a `def` decorated with an `overload` that resolves to `typing`, `_typeshed` or `typing_extensions` is never redefined. Across scopes: only a shadowed import that is no explicit re-export, and not from a class body. Reported on the redefining name. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F811.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f821-undefined-name`

Port of Ruff's F821 (undefined-name), on the engine's name resolution (M10a `resolves.unresolved`): a reference with no binding in scope (or one an unconditional `del` unbound), no builtin of the target version or of Ruff's option `builtins` and no visible star import, outside a `try` body whose handler catches the builtin `NameError`; and such a name inside a string Ruff parses as a type definition (M11a `parse` with `annotation`: string annotations, `cast`/`TypeVar` arguments, typing generics' string slices, nested strings), reported on each such name inside the string. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F821.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f822-undefined-export`

Port of Ruff's F822 (undefined-export) with default settings, on the engine's name resolution (M10a `resolves.unresolved` on `__all__` entries): a string entry of a static module `__all__` definition (Ruff's extract_dunder_all_names) (an implicit concatenation is one entry) that names no module binding (a bare annotation included) and no builtin, in a module without a star import. `__init__.py` is exempt (Ruff reports it only in preview). Reported on the string.

### `ruff-f823-undefined-local`

Port of Ruff's F823 (undefined-local) with default settings, on the engine's name resolution (M10a `resolves.beforeLocal`, Ruff's undefined_local): a function's first read of a name that resolves to an enclosing function's or the module's binding which the function's own later binding of the name shadows, where that binding is the function's first and latest of the name (not `global`, `nonlocal` or `del`). Reported on the read. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F823.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f841-unused-variable`

Port of Ruff's F841 (unused-variable), on the engine's usedness (M10a `binds`): the latest binding of a name in a function or lambda scope made by `=` (plain or annotated), `+=`, `:=`, `with ... as name`, a match capture or a `type` alias (Ruff's Assignment, NamedExprAssignment and WithItemVar) that nothing reads, not under `global`/`nonlocal`, not an unpacked assignment (Ruff's is_unpacking_assignment: a name in a tuple-like target whose value is no display, or in a chain with a non-tuple target, a walrus in the value of an all-tuple unpacking, a `with` tuple target), not a dummy (Ruff's option `dummy-variable-rgx`) or traceback name; and any `except ... as name` nothing reads, not a dummy name. A `locals()` call in the function leaves its assignments' usedness unknown (Ruff's USES_LOCALS; not the `except` names'). Names in strings Ruff parses as type definitions are reads (M11a). Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/F841.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-f842-unused-annotation`

Port of Ruff's F842 (unused-annotation) with default settings (`dummy-variable-rgx` = `^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$`), on the engine's usedness (M10a `binds` with form `annotation`, Ruff's unused_annotation): the latest binding of a name in a function or lambda scope that is a bare annotation (`x: int`, `(x): int`) nothing reads, not a dummy name. Names in strings Ruff parses as type definitions are reads (M11a).

### `ruff-f901-raise-not-implemented`

Port of Ruff's F901 (raise-not-implemented) with default settings: the exception of a `raise` (not its `from` cause) that is the name `NotImplemented` or a call whose callee is that name, parentheses transparent. Reported on the name. Ruff matches the name itself, not the builtin binding.

### `ruff-n999-invalid-module-name`

Port of Ruff's N999 (invalid-module-name): a `.py` or `.pyi` file in a package (the host sent its module path) whose name is not a valid module name (`is_module_name`: `[a-z_][a-z0-9_]*`, not a keyword), reported at the start of the file. The name is the file's stem (`moduleName`), or for an `__init__` file the package root's directory (`package`: Ruff reads `package.path()`, the top package, not the file's own directory), as on disk. In a `versions` or `migrations` directory a leading digit is allowed (`is_migration_name`). Names matching `pep8-naming.ignore-names` (Ruff's option, with its defaults setUp, tearDown, ...) are exempt.

### `ruff-pie790-unnecessary-placeholder`

Port of Ruff's PIE790 (unnecessary-placeholder): a `pass` statement, or an expression statement that is only `...`, in a body (module, class, function or compound-statement block) of two or more statements. An `...` is kept in a `TYPE_CHECKING` block, a `typing.Protocol` class or an `abc.abstractmethod`-decorated function, and there Ruff stops scanning the body, so a later `pass` is kept too.

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

### `ruff-pie808-unnecessary-range-start`

Port of Ruff's PIE808 (unnecessary-range-start): `range(0, stop)` (an int literal zero, any spelling) with exactly two positional arguments and no keywords, where `range` resolves to the builtin (`match_builtin_expr`). Reported on the `0`.

### `ruff-pie810-multiple-starts-ends-with`

Port of Ruff's PIE810 (multiple-starts-ends-with): an `or` chain with two or more operands `R.startswith(x)` (or two `R.endswith(x)`) on the same name `R`, each with one positional argument and no keywords, where the argument is not a name bound to a tuple (Ruff's is_bound_to_tuple through `resolves.binding`, read as lookup_symbol does with `stopAtGlobal`: a `def`'s `*args`, a tuple-typed value, `tuple(...)`, or a `tuple`/`Tuple` annotation or return annotation). Reported on the whole chain, once per such group.

### `ruff-ret501-unnecessary-return-none`

Port of Ruff's RET501 (unnecessary-return-none): `return None` in a function whose every `return` is bare or returns `None` (nested functions and classes are separate; generators and functions whose body is a lone `return` are skipped), unless the function has a return annotation other than `None` or a property decorator (`property`, `enum.property`, `functools.cached_property`, `abc.abstractproperty`, `types.DynamicClassAttribute`, resolved through the file's bindings and imports). Reported on the `return` statement.

### `ruff-ret502-implicit-return-value`

Port of Ruff's RET502 (implicit-return-value): a bare `return` in a function that also has a `return` with a value other than `None` (nested functions and classes are separate; generators and functions whose body is a lone `return` are skipped), reported on the `return`.

### `ruff-ret503-implicit-return`

Port of Ruff's RET503 (implicit-return): a function that has a `return` with a value other than `None` and whose last statement can fall through (Ruff's has_implicit_return: recursively the last statement of each `if`/`elif`/`else`, `for`/`while` `else`, `match` case and `with` body; an `if` without `else` and a loop without `else` fall through; `while True`, `assert False`, `return`, `raise`, `try` and calls of known no-return functions do not). No-return functions are `exit`/`quit` (builtin, `sys`, `_thread`, `pytest`), `os`/`posix` `_exit`/`abort`, `_winapi.ExitProcess`, `pytest.fail`/`skip`/`xfail`, `assert_never` from `typing`/`_typeshed`/`typing_extensions`, and functions defined before the analysed function with a `NoReturn`/`Never` return annotation; the callee names are resolved through the file's bindings and imports. Reported on the function, from its first decorator.

### `ruff-ret504-unnecessary-assign`

Port of Ruff's RET504 (unnecessary-assign): `return x` whose preceding statement (comments aside) is `x = value` with a single name target, or a `with` block (not using `contextlib.suppress`) whose last statement is such an assignment, reported on the returned name. Skipped when `x` is declared `global`/`nonlocal` or bare-annotated in the function, referenced in a `finally` enclosing the return (a read or `del` resolving to a binding in the function's scope), or when the function scope's latest binding of `x` has a reference from another scope (`binds` with `latest` and `nestedUse`, which inherits the references of the bindings it shadows); generators and functions whose body is a lone `return` are skipped.

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

### `ruff-sim102-collapsible-if`

Port of Ruff's SIM102 (collapsible-if) with default settings: an `if` whose body (or whose last `elif` clause's body, when the last clause is an `elif`) is exactly one `if` without `elif`/`else`, when the `if`'s own body is one statement, the tested condition is not `__name__ == "__main__"` or a boolean literal, and the `if` is not a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Every `if` of a nested chain is reported, as Ruff does (its parent-chain exemption compares the parent with itself and never applies). Reported at the `if` or `elif` keyword.

### `ruff-sim103-needless-bool`

Port of Ruff's SIM103 (needless-bool): an `if`/`else`, a trailing `elif`/`else`, or an `if` followed by a `return` statement, whose two branches are single `return True` / `return False` statements with opposite values, unless the `if` is a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Reported at the `if` (or the last `elif`).

### `ruff-sim105-suppressible-exception`

Port of Ruff's SIM105 (suppressible-exception) at target 3.14: a `try` with one simple statement (expression, assignment, `del`, `assert`, import or `pass`), exactly one `except` handler whose body is only `pass` or `...`, no `else` or `finally`, and whose handled types are all names or attribute chains (or none). Reported on the `try` statement.

### `ruff-sim107-return-in-try-except-finally`

Port of Ruff's SIM107 (return-in-try-except-finally): the first top-level `return` of a `finally` body when the `try` body or an `except` handler body has a top-level `return`.

### `ruff-sim108-if-else-block-instead-of-if-exp`

Partial port of Ruff's SIM108 (if-else-block-instead-of-if-exp): an `if`/`else` whose two bodies are each one plain assignment `N = value` to the same name (no yield/await value), unless it is a `sys.version_info`/`sys.platform` or `TYPE_CHECKING` block. Missing: Ruff reports only when the generated ternary fits the line length.

### `ruff-sim109-compare-with-tuple`

Port of Ruff's SIM109 (compare-with-tuple): an `or` chain with two or more operands `N == name` (one `==`, both sides names) for the same `N`, anywhere in the chain; not reported when a comment lies inside the chain or follows it on its last line with only white space between (Ruff's has_comments). Reported on the whole `or` chain, once per such name.

### `ruff-sim110-reimplemented-builtin`

Partial port of Ruff's SIM110 (reimplemented-builtin): in a function scope, a `for` loop whose body is one `if` (no `elif`/`else`) returning a boolean literal, followed by a `return` of the opposite literal in the loop's `else` or as the next statement; the loop target and the `if` test contain no `await`/`yield`. Missing: Ruff reports only when the generated `return any(...)`/`all(...)` fits the line length.

### `ruff-sim112-uncapitalized-environment-variables`

Port of Ruff's SIM112 (uncapitalized-environment-variables): a str literal (not bytes, an f- or t-string; escapes decoded, implicit concatenations joined) whose value has an ASCII lowercase letter, other than `https_proxy`/`http_proxy`/`no_proxy`, used as `os.environ[...]` or as the first argument of a call resolving to `os.environ.get`/`os.getenv`, after Ruff's traversal has visited an absolute import of `os` (M10a `resolves.seenBefore`). Reported on the string.

### `ruff-sim113-enumerate-for-loop`

Port of Ruff's SIM113 (enumerate-for-loop): `N += 1` as a top-level statement of a non-async `for` body without `continue`, where the binding `N` reads (`resolves.binding`) is a name assignment of `0` (plain, annotated, or an unpacked element of a display, paired by index as Ruff's match_value where the engine gives no value) whose parent statement is the loop's, `N` is bound only once inside the loop (`binds` with `bindings` within the loop: deletions and comprehension walruses count, comprehension targets, nested functions' locals and keyword arguments do not), and no load of `N` outside the loop in its scope, at or after the initial assignment, reads the initial binding (whose references Ruff's rebinding inherits) or a binding inside the loop. The loop and the initial assignment are found again by node identity (`compare` with `text: "node"`), so identical loops in one scope stay apart.

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

### `ruff-sim208-double-negation`

Port of Ruff's SIM208 (double-negation): `not` applied to another `not` expression, through any parentheses, reported on the outer `not` expression (each outer `not` of a longer chain).

### `ruff-sim210-if-expr-with-true-false`

Port of Ruff's SIM210 (if-expr-with-true-false): a conditional expression whose body is the literal `True` and whose `else` arm is `False` (through any parentheses), reported on the whole expression. Ruff's message and fix differ when the test is a comparison or `bool` is shadowed; what is flagged does not.

### `ruff-sim211-if-expr-with-false-true`

Port of Ruff's SIM211 (if-expr-with-false-true): a conditional expression whose body is the literal `False` and whose `else` arm is `True` (through any parentheses), reported on the whole expression.

### `ruff-sim212-if-expr-with-twisted-arms`

Port of Ruff's SIM212 (if-expr-with-twisted-arms): `B if not A else A` where the negated test operand and the `else` arm are the same plain name (through any parentheses), reported on the whole expression.

### `ruff-sim220-expr-and-not-expr`

Port of Ruff's SIM220 (expr-and-not-expr): an `and` expression (one run of the operator; parentheses start a new one) with a value `not A` and a value `A`, the same plain name (through parentheses), and no side effect anywhere in it (Ruff's `contains_effect`). Reported on the whole `and` expression, once per such pair.

### `ruff-sim221-expr-or-not-expr`

Port of Ruff's SIM221 (expr-or-not-expr): an `or` expression (one run of the operator; parentheses start a new one) with a value `not A` and a value `A`, the same plain name (through parentheses), and no side effect anywhere in it (Ruff's `contains_effect`). Reported on the whole `or` expression, once per such pair.

### `ruff-sim222-expr-or-true`

Port of Ruff's SIM222 (expr-or-true), `is_short_circuit` in flake8_simplify/rules/ast_bool_op.rs:736. Over the values of one `or` run (tree-sitter nests them to the left; parentheses start a new run), a value whose truthiness is unknown is skipped when the run is not in a boolean test, or when it has a side effect (`contains_effect`). The first value that is not skipped and is truthy (`True`, a non-empty literal, ...), or whose successor is, triggers; the finding starts at the value after the last skipped value before it (the whole expression when none), as Ruff's `furthest`. Truthiness follows `Truthiness::from_expr`, including f-string emptiness and `list(...)`-style calls. The finding spans that value only; Ruff's range runs to the end of the expression. Ported from the Ruff conformance suite (rerail/conformance/ruff/rules/SIM222.json at 3f102adcb), with Ruff's `# noqa` honoured as the Ruff CLI honours it.

### `ruff-sim223-expr-and-false`

Port of Ruff's SIM223 (expr-and-false), `is_short_circuit` in flake8_simplify/rules/ast_bool_op.rs:736. Over the values of one `and` run (tree-sitter nests them to the left; parentheses start a new run), a value whose truthiness is unknown is skipped when the run is not in a boolean test, or when it has a side effect (`contains_effect`). The first value that is not skipped and is falsy (`False`, `None`, zero, an empty literal, ...), or whose successor is, triggers; the finding starts at the value after the last skipped value before it (the whole expression when none), as Ruff's `furthest`. Truthiness follows `Truthiness::from_expr`, including f-string emptiness and `list(...)`-style calls. The finding spans that value only; Ruff's range runs to the end of the expression.

### `ruff-sim300-yoda-conditions`

Port of Ruff's SIM300 (yoda-conditions): a comparison with one `==`, `!=`, `<`, `<=`, `>` or `>=` whose left operand is more constant-like than its right, on Ruff's three levels (`ConstantLikelihood`, flake8_simplify/rules/yoda_conditions.rs:76-136): literals (not f- or t-strings) are definite; a name or attribute whose name is cased uppercase is probable; tuples, lists and dicts (keys and values, `**` values), binary operations and unary `+`/`-`/`~` take the least of their parts (empty is definite); everything else, sets included, is unlikely. Reported on the comparison.

### `ruff-sim401-if-else-block-instead-of-dict-get`

Port of Ruff's SIM401 (if-else-block-instead-of-dict-get): `if K in D: V = D[K] else: V = DEFAULT` (or the `not in` form with the arms swapped), and the conditional-expression form, where the default has no side effect (`contains_effect`). Operands are compared by source text, where Ruff compares expressions (`ComparableExpr`). The statement form needs `D` to be a name whose binding is a dict (`is_dict` through `resolves.binding` with `only`: Ruff's is_known_to_be_of_type_dict, whose only_binding asks that the binding shadows no earlier one). Partial: Ruff also asks that the suggested `V = D.get(K, DEFAULT)` fits the line length.

### `ruff-sim905-split-static-string`

Port of Ruff's SIM905 (split-static-string): `.split(...)` or `.rsplit(...)` called on a str literal (implicit concatenations included; not bytes, f- or t-strings), where `sep` (keyword, else first positional argument) is absent, `None` or a str literal, and `maxsplit` (keyword, else second positional argument) is absent or an integer literal, optionally negated, that fits in an i32 (`get_maxsplit_value`). Reported on the call.

### `ruff-sim910-dict-get-with-none-default`

Port of Ruff's SIM910 (dict-get-with-none-default): `.get(key, None, ...)` whose second positional argument is `None`, with no keywords, called on a dict display or dict comprehension (through parentheses), or on a name whose binding is a dict (Ruff's `is_dict` through `resolves.binding`: a `def`'s `**kwargs`, a dict-typed value or `dict(...)`, a `dict`/`Dict` annotation or return annotation; unpacking carries no value).

### `ruff-sim911-zip-dict-keys-and-values`

Port of Ruff's SIM911 (zip-dict-keys-and-values): `zip(d.keys(), d.values())`, the same plain name on both calls, no arguments to either, no keyword but `strict`, and `zip` the builtin (or `builtins.zip`). Reported on the `zip` call, where `d`'s binding is a dict (Ruff's `is_dict` through `resolves.binding`: a `def`'s `**kwargs`, a dict-typed value or `dict(...)`, a `dict`/`Dict` annotation or return annotation; unpacking carries no value).

### `ruff-t100-debugger`

Port of Ruff's T100 (debugger): imports of debugger modules (`is_debugger_import`) and of debugger functions (`from M import n` with `M.n` in `is_debugger_call`), once per imported name, matched by module with `resolves.imported`, relative imports by the module's text (`relative: "text"`: Ruff ignores the level). It also flags calls whose callee resolves to a debugger function (`resolve_qualified_name` through the file's imports, or the builtin `breakpoint`).

### `ruff-w605-invalid-escape-sequence`

Port of Ruff's W605 (invalid-escape-sequence): a backslash in a non-raw string, bytes, f-string or t-string literal part that does not start a valid escape (newline, backslash, quotes, `a b f n r t v`, octal digits `0`-`7`, `x`, `N`, `u`, `U`), one finding per backslash. tree-sitter-python leaves such a backslash as a bare `\` token inside `string_content` (valid escapes are `escape_sequence` nodes); f-string format specifiers keep their text unsplit, so a regex over the specifier walks its escapes and `each` reports every invalid backslash (a specifier with none binds no group and reports nothing).

### `ruff-a005-stdlib-module-shadowing`

Port of Ruff's A005 (stdlib-module-shadowing): a `.py` file whose module, or a package's `__init__.py` whose package, is named like a standard-library module of the target version (`is_known_standard_library`, one `when`-gated check per range of versions; `_private` names are allowed), reported at the start of the file. Its options are Ruff's, with Ruff's defaults: without `flake8-builtins.strict-checking` the scope takes only top-level modules (files directly under the project root or a `src` root, and `x/__init__.py` there, never a root's own `__init__.py`); `flake8-builtins.allowed-modules` leaves files and packages so named out of scope.

### `ruff-inp001-implicit-namespace-package`

Port of Ruff's INP001 (implicit-namespace-package): a `.py` file outside any package (the host sent no module path), reported at byte 0 (`file.start`) as Ruff reports it. Files directly under the project root or `src/` (Ruff's default `src`), files whose first comment starts at byte 0 with `#!` (a shebang: M12a `layout` `start`) and PEP 723 scripts (a `# /// script` comment block closed by `# ///`) are exempt.
