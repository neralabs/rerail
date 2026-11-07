# Group 5: what .rr lacks for twenty data-layer checks

Oracle: scripts at revision 2b1504ad. Each check lists what it computes, its inputs, and its needs, tagged EXISTS / LIB / NATIVE / LANG / POLICY. Line citations are `script.py:LINE` in that revision.

### auto_api_plan_boundaries (hard-fail, 350 lines)
- **Computes:** In five named route modules of `l9900_auto_api/`, calls whose last name segment is a persisted-naming
  loader inside one of five guarded functions (AAPI001); in `router.py`'s `_execute_graphql_naming_dispatch_query`, calls
  named `plan` (AAPI004) or `execute_async` (AAPI005); in `terminal_payloads.py`, class-body annotated fields of
  `AutoApi*TerminalPayload` classes whose annotation text contains a runtime-handle token (AAPI006).
- **Inputs:** Python AST of `l9900_auto_api/**/*.py`, tests excluded (`tests`/`test`/`conform` parts). Exact paths
  compared: `l9900_auto_api/{router,rest_router,mcp_router,batch,dynamic_rest_router}.py`,
  `l9900_auto_api/terminal_payloads.py`.
- **Needs:**
  - `EXISTS doc, path_part`: exact repository-relative path equality; `.scope dirs "l9900_auto_api"`, `.scope tests false`.
  - `EXISTS field, attr_of, text`: the call name is the bare `Name` id or the `Attribute`'s last segment, never resolved
    (`_call_name`), so pattern/sym_of resolution must not be used.
  - `EXISTS nearest(c, f, fdef), def_name`: the innermost enclosing def's bare name (methods included, no class prefix;
    lambdas and class bodies are transparent).
  - `EXISTS kind, child, field(a, "type", t), text, match`: AAPI006 is a substring test on the annotation's text,
    `match(".*(AsyncDatabaseConnection|…|Callable|Request|Response|…).*", t)`; class name `match("AutoApi.*TerminalPayload", n)`.
  - `LIB own_def(n: node, f: node)`: the oracle's function stack covers a def's decorators and parameter defaults as
    well as its body (`ast.iter_child_nodes` after `on_FunctionDef` pushes, lint_auto_api_plan_boundaries.py:170,182-186);
    tree-sitter puts decorators outside `function_definition`. Body: `own_def(n, g) :- decorator(g, e), (n = e ; within(n, e)).`
    `own_def(n, f) :- nearest(n, f, fdef), !in_decorator(n).` Low impact here.
- **Hard parts:** AAPI006 tests `ast.unparse(annotation)` (lint_auto_api_plan_boundaries.py:143-146, 217-218): substring,
  so `RequestContext` or `HttpResponse` also match; only direct class-body `AnnAssign` statements (not ones nested in an
  `if` of the class body). Tree-sitter source text differs from `unparse` only in whitespace and quotes, neither of
  which the tokens contain. The `source_triggers` gate (line 231) is redundant with the predicates.

### error_handling (hard-fail, 348 lines)
- **Computes:** (1) In an `except` body, a statement `logger.warning(...)`/`logger.error(...)` directly followed by a bare
  `raise`; reported at the raise line. (2) Statement-level `txn.commit()` under `api/l8000_admin/tools`, with the innermost
  enclosing function's name in the message. One code, EH001.
- **Inputs:** Python AST of `api/`, `impl/`, `services/`, `api/l8000_admin/tools/`, tests excluded. At revision 2b1504ad
  `api/` does not exist (impl 2910 files, services 148), so rule (2) never fires.
- **Needs:**
  - `EXISTS doc, match`: the directory test is a string prefix of the relative path (`rel_path.startswith("api")`,
    lint_error_handling.py:166,180), so `match("(api|impl|services).*", p)`, which also accepts `implx/…`; not path_part.
  - `EXISTS pattern atoms`: `` s ~ `logger.warning($$$A)` `` as an `expression_statement`, receiver the literal name `logger`
    (no resolution); bare raise is `kind(r, "raise_statement"), !child(r, _, _)`.
  - `EXISTS precedes_stop(a, b, code_stmt)` with a derived stop set `code_stmt(n) :- child(blk, n, _), !kind(n, "comment")`:
    "the next statement" in the oracle's `body[index + 1]` skips comments, which tree-sitter keeps as named children, so
    `next_sibling` alone is wrong when a comment sits between.
  - `EXISTS nearest(c, f, fdef), def_name`: function name, `<module>` when none.
- **Hard parts:** only the handler's own body list (not nested blocks) is examined; `except*` groups are handlers too
  (tree-sitter `except_group_clause`). `await txn.commit()` is not reported: the call's parent is `Await`, not `Expr`
  (lint_error_handling.py:183). `logger` must be a bare `Name`; `self.logger.error` is not matched.

### system_session_scope (ratchet, 957 lines)
- **Computes:** (A) every argument-less `SessionId.system()` call not in a comparison context, attributed to its innermost
  function (or "module-level"); (C) `async with` whose item is a call named `schema_transaction`. Counted per category over
  "production" files and compared against a signed baseline; the suite reports only the ratchet outcome.
- **Inputs:** Python AST and raw source lines of `impl/`, `data_layer/`, `_internal/`, top-level `l\d{4}_*` dirs, plus
  `benchmarks/ scripts/ demos/ examples/ data_layer_testing/` (scanned, never counted). Signed baseline
  `.system-session-scope-baseline.json` (`counts`, `_metadata.signature`; Ed25519 key hard-coded,
  lint_system_session_scope.py:158).
- **Needs:**
  - `EXISTS pattern atom`: `` c ~ `SessionId.system()` `` (textual, zero args, no resolution: lines 282-291).
  - `EXISTS field, kind, child`: exemptions: parent is a comparison; or `.unwrap_to_sql()` on it whose call sits in a
    comparison, in a tuple/set/list inside a comparison, or is the value of a plain assignment (lines 293-315). Plain
    `Assign` excludes annotated assignment: `kind(a, "assignment"), !field(a, "type", _)`.
  - `EXISTS op(w, "async")` on `with_statement`, item value a call whose name (Name id or attr) is `schema_transaction`.
  - `LIB line_text(d: number, l: number, s: symbol)`: each physical source line. Body:
    `line_text(d, l, s) :- modnode(d, m), match_all("(?m)^(?<L>[^\n]*)$", m), text(L, s), line(L, l).` Needed for the
    string-context exemption, "the call's line, left-stripped, starts with a quote or `#`" (lines 255-260), and the
    suppression "`lint: system-session-ok` on the same or previous line, anywhere in the line" (lines 245-252). Comment
    nodes approximate the second but not the first.
  - `EXISTS path_part, file_name, match`: classification (lines 227-239): test if a part is `tests`/`test`/`conform` or the
    file name starts with `test_`/`conftest`; non-production if a part is in the five dirs; else production
    (`impl` first part is a separate class "impl", also uncounted). Only production feeds the counts (line 709).
  - `POLICY` ratchet from a repository-held signed baseline: `.budget * by code` whose numbers are read from
    `.system-session-scope-baseline.json`'s `counts` (keys `bare-system-session`, `async-schema-transaction`), verified
    against a canonical message `{"counts":…, "linter":"system-session-scope","version":1}` (sort_keys) with a fixed
    Ed25519 public key (lines 499-528, 714-751). Outcome codes `missing_baseline`, `invalid_baseline_signature`,
    `baseline_increase`, each one finding at the baseline file line 1, the last carrying a per-category diff message
    (lines 609-629). Inline budget numbers (as swallowed-exceptions.rr does) lose the signature check; `signed_list` is a
    list, not counts.
- **Hard parts:** `SCOPE_MARKERS` is empty (line 113) and `_module_docstring_end` is never set, so "enclosing scope
  marker" is dead code: every non-exempt call counts. The `system_grant.py`/`_grants.py` exemption and source-trigger gate
  live only in the CLI path (`check_file_content`, line 451); the suite plugin extracts every file; no such file exists at
  this revision anyway. A lambda is not a function for attribution. Multiple counts per line are kept (no dedupe).

### system_grant_bypass (hard-fail, 288 lines)
- **Computes:** Test-mode bypasses of system-grant enforcement: SGB001 defs named `_enable_test_mode`/
  `_reset_used_nonces`/`clear_verification_cache`, names/attributes/imports of `_enable_test_mode`, `setattr(_, "_enable_test_mode", …)`;
  SGB002 the same for `_ENFORCEMENT_MODE`; SGB003 string constant `"RUNNING_TESTS"` in a file named `system_grant.py`;
  SGB004 an autouse fixture that opens system grants or takes a `system_session_id` parameter.
- **Inputs:** Python AST of `l1300_session l2000_connection l2050_transaction l4000_permissions data_layer
  data_layer_testing conform impl/common/l2070_server_plan benchmarks scripts`, tests included.
- **Needs:**
  - `EXISTS def_name, attr_of, import_alias, call_arg, str, file_name, decorator, params`.
  - `LIB ast_name(n: node, x: symbol)`: an identifier in a Python-ast `Name` position (load or store). `nameref` also
    yields parameter names, import-statement identifiers, `global`/`nonlocal` names, `except … as e` and match-capture
    names, none of which is an `ast.Name` (lint_no_system_grant_bypass.py:126-130). Body: `nameref(n, x)` minus
    identifiers under `parameters`, `import_statement`/`import_from_statement`, `global_statement`/`nonlocal_statement`,
    the `alias` of an except clause, and pattern captures.
  - `EXISTS import_alias(i, a, m, x, local)` for `on_alias` (lines 138-142): `x` (or `m` for `import m`) in the banned set,
    or an explicit `as _enable_test_mode`. `local = "_enable_test_mode"` alone over-reports `import _enable_test_mode.sub`.
  - `EXISTS within, kind`: SGB004 "opens system grants": any call anywhere inside the def, nested defs included (every
    function on the stack is marked, line 147), named `system_session_scope` or matching `grant_.*system.*` (Name id or
    attr). Decorator: a `Call` whose func is an `Attribute` with attr `fixture` (bare `@fixture(...)` does not count) and
    keyword `autouse=True` literal (lines 86-98). Parameters: positional-only, regular and keyword-only, not `*a`/`**k`
    (line 106); `params` includes splats, so filter `list_splat_pattern`/`dictionary_splat_pattern`.
  - `EXISTS min : { … }`: the oracle dedupes per file by `(line, code, message)` (line 163), so two hits on one line are
    one finding: key the finding on a representative `min n : { …, line(n, l) }`.
- **Hard parts:** `ast.Constant` visits f-string literal parts, so `f"RUNNING_TESTS{x}"` matches SGB003 (lines 155-160);
  tree-sitter has `string_content` children there. `on_Name` only checks `_enable_test_mode`/`_ENFORCEMENT_MODE`, so a
  later use of `_reset_used_nonces` is not reported, only its def and import. `setattr` needs `args[1]` positional
  (`call_arg(c, 1, "", v)`; calls.rr's posarg index quirk, OPEN-ISSUES calls #8, applies). No `system_grant.py` exists
  at this revision, so SGB003 is currently dead.

### module_level_caches (hard-fail, 868 lines)
- **Computes:** Module-level mutable caches not in a reviewed allowlist: name bound to `{}`/`dict()` (dict_cache), `set()`
  (set_cache), `[]`/`list()` or a list literal under a cache-like name (list_cache), a call of `OrderedDict`,
  `WeakKeyDictionary` or `*Cache` unless a managed cache constructor with registration enabled (constructor_cache); and
  `@lru_cache`/`@cache`/`@functools.…` on module-level functions or methods of top-level classes (lru_cache). Code is the
  pattern type; allowlist key `path::pattern_type::name`.
- **Inputs:** Python AST of `impl/ data_layer/ _internal/ l\d{4}_*/`, tests excluded; JSON allowlist
  `scripts/allowlists/lint_module_level_caches_allowlist.json` (`entries[]` of path, pattern_type, name, classification,
  reason; unsigned).
- **Needs:**
  - `EXISTS scope_node(a, m), kind(m, "module")`: "module level" is no enclosing class, def or lambda; module-level
    `if`/`try`/`with`/`for` bodies count (line 475).
  - `EXISTS kind, field, child, list_len, call_arg, pattern atoms`: `` v ~ `dict()` ``, `` `set()` ``, `` `list()` `` (bare
    Name callee, no args, unresolved), empty `dictionary`/`list`; annotated assignment only with a value; each plain
    `Name` target of a chained `a = b = {}` separately (lines 415-428).
  - `EXISTS match`: all-caps exemption `name == name.upper() and "_" in name and not name.startswith("_")` (line 214);
    cache-like name `match("(?i).*(cache|holder|memo|pool|registry|token).*", x)`; constructor `match(".*Cache", c)`;
    managed `match("_*(?<b>.*)", c)` then `b` in the four managed names; `register_with_manager=False` literal disables.
    Infrastructure paths `impl/common/l1000_cache/`, `l1000_cache/` exempt assignments only (line 276).
  - `EXISTS decorator, attr_of, def_name, scope_node`: decorator unwrapping (line 286): bare `lru_cache`/`cache`, a call of
    one (recursively), or `functools.lru_cache`/`functools.cache` with the literal receiver `functools`. Methods only
    when exactly one class encloses and no def (lines 478-488); name `Class.method`. One finding per decorator.
  - `NATIVE json_rows reader`: rows from a JSON file's array of objects, one column per named key. `.input r(IO=file, …)`
    exists but defines no mapping from JSON records to columns. e.g.
    `.input cache_allow(path: symbol, pattern_type: symbol, name: symbol, classification: symbol, reason: symbol)
     (IO=json, filename="scripts/allowlists/lint_module_level_caches_allowlist.json", records="entries")`
    (lint_module_level_caches.py:576-608).
  - `POLICY` input-validation failure: a malformed allowlist (not an object, missing key, empty classification or
    reason after strip, duplicate key) yields one `invalid_allowlist` finding at the allowlist file, line 1, and no other
    findings (lines 589-605, 713-725). Empty/duplicate checks are Datalog (`count : {…} > 1`, `match("\\s*", r)`, a
    nullary `bad_allowlist()` negated in every violation); missing keys or bad JSON need the reader's failure to surface
    as a finding rather than abort.
- **Hard parts:** the oracle reports `node.lineno` of the `def` line, not the decorator (tree-sitter `function_definition`
  starts at `def`, which matches). CLI severities (CRITICAL/WARNING) and `benchmarks`/`examples` exclusion
  (`TEST_PARTS`, line 124) apply only to the CLI; the suite plugin is hard-fail for all and excludes only
  `tests`/`test`/`conform`. The cache-name token `token` matches any name containing it (`tokenizer`). Stale allowlist
  entries are not reported. `dict`/`set`/`list` are matched by name even if shadowed.


### notification_lifecycle (hard-fail, 392 lines)
- **Computes:** one finding per call (bare or `await`ed) whose callee's last name segment is one of four
  `create_*notification_channel` factories, unless the call sits in the context expression of an `async with` item,
  or the innermost enclosing def returns a `NotificationChannel` (annotation text) or is itself named like a factory.
  Message names the chain of enclosing function names (`outer.inner`, or `<module>`).
- **Inputs:** Python AST of `api/`, `impl/`, `services/` (tests excluded); path substrings `/tests/`, `/test_`,
  `conftest.py`, file name `factory.py` skip the file.
- **Needs:**
  - `EXISTS kind, field, text, nearest, within, op, def_name, match`: callee last segment (identifier, or an
    attribute's `attribute` field), `nearest(c, it, with_item_k)` + `field(it, "value", v)` + (`v = c` or
    `within(c, v)`) + the with statement's `async` token, innermost def via `nearest(c, f, fdef)`, return annotation
    via `field(f, "return_type", r), match(".*NotificationChannel.*", r)`; path skips as `match` on `doc` path and
    `file_name(d, "factory.py")`.
  - `LIB fn_chain(n: node, s: symbol)`: dotted names of the enclosing *function* definitions only (classes skipped),
    outermost first; `fn_chain(n, s) :- nearest(n, f, fdef), !nearest(f, _, fdef), def_name(f, s).` plus the
    recursive step `@dot(outer, name)`; `"<module>"` when there is no def. `qualname`/`scope_name` include classes
    and the module, so they do not fit (lint_notification_lifecycle.py:146, :215).
- **Hard parts:** the collector fires on both `Await` and its inner `Call` (lint_notification_lifecycle.py:181-192),
  so `await create_…()` is reported twice with the same line and message; as a finding set it collapses to one.
  The withitem test takes the *first* withitem ancestor at any depth, across def boundaries, and returns false if
  its statement is a plain `with` (lint_notification_lifecycle.py:227-237): `nearest` with a with-item stop set is
  exactly that. The factory exemption reads only the innermost def (stack top), and lambdas do not push.
  `ast.unparse(returns)` vs tree-sitter source text differ only in spacing/quotes; a substring test is insensitive.

### unresolved_forward_refs (hard-fail, 594 lines)
- **Computes:** for each attribute `X.annotation` where X is a name containing `field_info`, ending `_info`
  (case-insensitive) or exactly `f`, or `….model_fields[k]`, report when its immediate AST parent is a call to one
  of six type-inspection functions (by last callee segment), an `is`/`is not` comparison against non-None, or an
  `==`/`!=` against a capitalised name. Safe: `str(…)`, any None comparison, `isinstance(…, str|ForwardRef|TypeVar)`,
  and being inside the 2nd positional arg of `R.get(k, …)` where R is one of six names or a name the innermost def
  assigned from `get_type_hints(…)`. Message `{function}(): {problem}`.
- **Inputs:** Python AST of `api/`, `impl/` (tests excluded; CLI also skips `tests` dirs and `test_` in the name).
- **Needs:**
  - `EXISTS kind, field, text, match, call_arg, cmp, within, nearest, def_name`: the receiver test with
    `match("(?i).*field_info.*|.*_info", x)`; parent-is-call as `call_arg(c, i, "", n), i >= 0` (a keyword
    argument's AST parent is `keyword`, so `call_arg` with `""` keyword is the right test); `isinstance` type
    argument via `call_arg(c, 1, "", t)` and tuple children; uppercase heuristic `match("[A-Z].*", s)`;
    `resolved_hint_vars` per def as `within(a, f), nearest(f0, f, fdef)`… (walk includes nested defs, see below).
  - `LIB ast_parent(n: node, p: node)`: the Python-AST parent, skipping tree-sitter-only wrappers
    (`parenthesized_expression`, `argument_list`, `expression_statement`); `ast_parent(n, p) :- child(w, n, _),
    wrapper(w), ast_parent(w, p).` plus the direct case. The oracle's tests are all "parent is X"
    (lint_unresolved_forward_refs.py:250, :263, :274); `(f.annotation) is T` has a parenthesized parent in the tree.
  - `LIB cmp_link(c: node, i: number, op: symbol, rhs: node)`: one row per operator of a (possibly chained)
    comparison, with `left(c, l)`; `cmp` has no row for chains, while the oracle zips `ops` with `comparators`
    (lint_unresolved_forward_refs.py:181-186, :275, :285). Body: the k-th operator token and the k+1-th operand child.
- **Hard parts:** the `is` rule iterates `comparators` only, so in `T is f.annotation` the comparator is the
  annotation itself and the message is "compared with 'is' to a type" (lint_unresolved_forward_refs.py:275-281).
  `resolved_hint_vars` is reset per def but collected by `ast.walk` over the whole def, nested defs included, and
  only from plain `Assign` with `Name` targets (lint_unresolved_forward_refs.py:331-349); at module level the set is
  empty. The fallback test looks at every ancestor call, beyond the current def (lint_unresolved_forward_refs.py:213).
  The function column is the innermost def's bare name (no class).

### permission_error (hard-fail, 349 lines)
- **Computes:** each `raise PermissionError` / `raise PermissionError(…)` (callee a bare name) in a file where no
  earlier (traversal-order) `from …l1100_exceptions… import PermissionError` without alias was seen, unless the
  raise line or the line above contains `lint: allow-builtin-permission-error`.
- **Inputs:** Python AST of `api/`, `impl/`, `l1100_exceptions/` (tests excluded); comments for the suppression.
- **Needs:**
  - `EXISTS kind, field, text, node, line, match, docof`: the raise's exception (identifier, or a call whose
    `function` is an identifier) with text `PermissionError`; the import as `import_from_statement` whose
    `module_name` text matches `.*l1100_exceptions.*` with an un-aliased name `PermissionError`; "earlier" as a
    start-offset comparison from `node(d, n, _, start, _)`; suppression as a `comment` node with `line` l or l-1 whose
    text contains the marker.
  - `LIB line_text(d: number, l: number, s: symbol)` (as system_session_scope): only for exact parity of the
    suppression, which reads raw source lines (lint_permission_error.py:131-141), so a marker inside a string on the
    line also suppresses; comment nodes cover the real cases.
- **Hard parts:** the alias set grows during a pre-order walk (lint_permission_error.py:143-150, :172), so an import
  *after* the raise in source order (e.g. a function-level import later in the file) does not count, and the test
  is file-wide, not scoped: an import inside one function legitimises raises in every later function. An aliased
  import (`as DbPermissionError`) leaves bare `PermissionError` reported. It is text-only: a local class named
  `PermissionError` is still reported; `sym_of(e, "builtins.PermissionError")` would be more precise but differs.

### forwardref (hard-fail, 421 lines)
- **Computes:** a state machine per sync `def` (and the module): after `annotation = X.annotation`, report
  `isinstance(annotation, type)` (FWDREF001), `issubclass(annotation, …)` (FWDREF002) and `callable(annotation)`
  (FWDREF003) unless an `isinstance(annotation, …ForwardRef…)` came between the latest such assignment and the call.
- **Inputs:** Python AST of `api/`, `impl/`, `public/` (tests excluded).
- **Needs:**
  - `EXISTS kind, field, text, child, node, nearest, call_arg, op`: events are plain patterns
    (`a ~ `annotation = $X.annotation``, `c ~ `isinstance(annotation, type)``, the ForwardRef check over the 2nd
    argument: `ForwardRef` name, `….ForwardRef` attribute, or a tuple containing either, one level per tuple);
    "between" by start offsets; the state region as `nearest(n, r, syncdef)` with `syncdef` a derived node set
    (`function_definition` without `op(f, "async")`), or no such ancestor for the module region. "The latest
    assignment had no check after it" is `∃a before c in the region with !∃k: a < k < c`, which needs no ordering
    construct (any earlier assignment with no check after it implies the latest one has none).
- **Hard parts:** only `on_FunctionDef` saves/resets/restores state (lint_forwardref.py:107-123): `async def`
  bodies share their enclosing region's state, and events inside them leak out to later code of the outer def.
  The check flag is set only while in context (lint_forwardref.py:139-150), and a later re-assignment clears it
  (:135). Order is the engine's pre-order walk of `ast.iter_child_nodes` (fact_engine.py:817-842), which equals
  source order except for field orders such as decorators after the body and dict keys before values.
  The variable must be literally `annotation`; the message always says `annotation`.

### sync_async_signatures (hard-fail, 505 lines)
- **Computes:** per file, a map from bare def name (any scope, methods included) to the signature of the last def
  with that name that has a non-`self` positional parameter; for each name `X_async` with an `X` entry, report at the
  async one's line when X's first parameter type is one of six transaction names and X_async's is one of four
  connection names. `async` is not required, only the suffix.
- **Inputs:** Python AST of `api/`, `impl/`, `data_layer/`, `_internal/` and every top-level `l\d{4}_.+` directory
  (tests excluded).
- **Needs:**
  - `EXISTS def_name, match, node, text, str`: the pair by capture `match("(?<x>.*)_async", n)`; "last
    def with that name in the file" by start offsets and negation.
  - `LIB pos_param(f: node, x: symbol, p: node, i: number)`: the regular positional parameters (`args.args`):
    after a `/`, before any `*`, `*args` or `**kw`; `param` also gives `*args` rows (its 2nd rule takes child 0 of a
    splat) and cannot tell positional-only ones apart (lint_sync_async_signatures.py:183-193). Body: `param` rows of
    kinds identifier/typed/default/typed_default, with no `keyword_separator`/`list_splat_pattern` child before i
    and a `positional_separator` before i if there is one.
  - `LIB ann_head(a: node, s: symbol)`: the oracle's `_get_annotation_name` (lint_sync_async_signatures.py:152-174):
    name→id, attribute→attr, subscript→its value's head, string/None constant→its value (whole string, unparsed),
    `A | B`→ the side whose head is in the type sets, else left's head if any, else right's. Recursive on
    `binary_operator` and `subscript`, with `!` for the preference.
- **Hard parts:** the map is keyed by bare name across all classes and nesting levels, so `A.create(txn)` and
  `B.create_async(conn)` pair up across classes, and a later same-named def overwrites an earlier one; a def with
  only `self` (or none) does not overwrite (lint_sync_async_signatures.py:195-211). Only `self` is skipped, not
  `cls`. A string annotation `"WriteTransaction | None"` is taken whole and so never matches; `Optional[T]` gives
  `Optional`. The top-level `l\d{4}_` directory list is computed from the tree (discovery.py:32-41); a scope glob
  `l[0-9][0-9][0-9][0-9]_*/**` says the same.

### tstring_deps (hard-fail, 474 lines)
- **Computes:** two codes over the whole repository (tests included): `tstring_dependency` for `import X` or
  `from M import …` where X/M contains `string.templatelib`, outside two allowed path prefixes; and
  `deprecated_sql_import`, one per alias `sql` in `from l2000_connection.safe_sql import …`, outside eight
  allowed prefixes. The first message rebuilds the import (`from M import a as b, c`).
- **Inputs:** Python AST of every `.py` file under the root (skip `__pycache__`, `.venv*`, `.git`, `tmp`).
- **Needs:**
  - `EXISTS import_alias, field, text, match, doc`: module text with leading dots stripped by capture
    (`match("\\.*(?<m>.*)", t)`, the oracle reads `node.module` raw and unresolved, so `import_alias`'s resolved
    module is the wrong column); prefixes as `match("browser_build/.*|impl/common/…/template_compat.py.*", p)`.
  - `LIB joined(i: node, s: symbol)`: the from-import's names rendered `a as b, c` in order
    (lint_tstring_deps.py:220, :246-248), by recursion over child index: `acc(i, k, cat(s0, ", ", item_k)) :-
    acc(i, k-1, s0), …`. Soufflé has no string aggregate, but index recursion builds it. Only needed for the exact
    message; the statement's `text` is an approximation that differs on wrapping and comments.
- **Hard parts:** the deprecated test is an exact match on `node.module`, so `from .safe_sql import sql` inside the
  package is not reported and `from .l2000_connection.safe_sql import sql` is (dots are not part of `module`).
  The categories and `--summary`/`--by-category` modes are CLI-only and do not affect plugin findings.


### txn_required_methods (hard-fail, 325 lines)
- **Computes:** calls `versioned_backend_registry.get(a, b)` (receiver a bare name or an attribute ending in that
  name) whose 2nd positional argument, and calls `….query_log_table(a, …)` whose 1st positional argument, "looks
  like a connection": a bare name in {conn, connection, db_connection, txn_conn}, or an attribute named `_conn`.
- **Inputs:** Python AST of `api/`, `impl/` (tests excluded).
- **Needs:**
  - `EXISTS kind, field, text, call_arg, match`: callee attribute and receiver tests; the argument by
    `call_arg(c, 1, "", a)`; the message's rendering of the argument (`x`, `x.attr` or `...attr`, else `<expr>`) by
    three rules over `kind`/`field`/`text` and `cat`.
- **Hard parts:** `node.args` counts `*xs` as a positional argument; `call_arg` gives splats no row and (open
  issue 8) its index counts every argument child, so `get(*a, conn)` differs; such calls are not in the corpus.
  `arg_str.endswith("._conn")` (lint_txn_required_methods.py:160) holds for any attribute named `_conn`, since a
  deep receiver renders as `..._conn`.

### grammar_fragment_usage (hard-fail, 690 lines)
- **Computes:** in "grammar classes" (name starting `Sql`/`_Sql`, a `render` method in the body, or a base, mapped
  through earlier import/assignment aliases, that starts `Sql`/`_Sql` or is an earlier grammar class of the same
  file): forbidden wrapper class names; `__init__` parameters annotated with SqlFragment (or a local alias of it,
  including inside string annotations as a word); class-level annotated attributes of SqlFragment type; and, for
  classes with `render`, every `return` in a method reachable from `render` by `self.`/`cls.` calls that returns a
  captured string field or passes through a field set from an untyped/`Any`/`str` constructor parameter. Files
  that do not parse give `python_parse_error`.
- **Inputs:** Python AST of `_internal/`, `api/`, `data_layer/`, `impl/` and every top-level `l[0-9]*` directory
  (tests excluded).
- **Needs:**
  - `EXISTS kind, field, text, node, def_name, method_of, within, match_all, str, max`: "latest earlier
    alias" as `max` over start offsets of earlier from-imports/assignments binding the name; the word test in string
    annotations by `match_all("(?<w>[A-Za-z0-9_]+)", s)` against the alias set (the batch-four root cause:
    words, not type syntax); reachable methods as a positive recursive closure over `self.m(…)`/`cls.m(…)` calls
    whose name is a def in the same class body (lint_grammar_fragment_usage.py:400-429); returns by `within(r, m)`
    (`ast.walk` descends into nested defs and lambdas).
  - `LIB grammar_class(c: node)`: recursive, ordered: a base qualifies only if a grammar class of that bare name
    was defined *earlier* in the same file (lint_grammar_fragment_usage.py:263-270); base names go through the
    alias map (last from-import or module-level `X = SqlY` before the class).
  - `LIB builds_str(e: node)` with a name set: string or f-string literal, list/set/tuple element, dict value,
    `+` operand, conditional branch, `str(…)`, or `recv.m(…)` for 13 string methods with `builds_str(recv)`; a name
    counts when it is in the order-dependent set of names assigned a string-builder earlier at module level (any
    statement outside a class, function bodies included) or, for `__init__`, earlier in the walk
    (lint_grammar_fragment_usage.py:114-148, :311-318, :364-386).
  - `LIB rendered_field(e: node, f: symbol)`: `self.f`/`Cls.f` through `+`, conditional, subscript value, and the
    first argument of `SqlFragment(…)`, `SqlFragment.static/with_params(…)`; nothing through `SqlFragment.param`
    or any other call (lint_grammar_fragment_usage.py:151-169).
  - `LIB param_annotation(f: node, x: symbol, p: node, t: node)`: every parameter of a def with its annotation,
    including `*args: T`, `**kw: T` and keyword-only ones (`_constructor_arguments`,
    lint_grammar_fragment_usage.py:172-175). `site_annotation` covers only `typed_parameter`, and `param`'s second
    rule reads child 0, which for `*args: T` is the splat pattern, not the identifier.
  - `LIB fragment_ann(t: node, names)`: SqlFragment in an annotation through names, attributes (`.SqlFragment`),
    list/tuple elements, `|`, subscript value and slice, and words of string constants
    (lint_grammar_fragment_usage.py:178-207). Small recursive relation.
  - `POLICY extraction-failure finding`: a file that cannot be parsed yields one `python_parse_error` finding at
    line 1, and no other findings for the file (lint_grammar_fragment_usage.py:545-556). Tree-sitter recovers from
    errors, so the engine must expose "this file has a syntax error" (an `ERROR`/`MISSING` node, or a parse-status
    fact) for a rule to report it.
- **Hard parts:** almost every set is built in traversal order: `_import_aliases` (any from-import anywhere, last
  wins), `_grammar_classes`, `_module_string_names`, so a class that subclasses one defined later in the file is
  not a grammar class. In `__init__`, the walk is `ast.walk`, which is breadth-first: a string assigned inside an
  `if` is not yet known to a top-level `self.f = x` that follows it (lint_grammar_fragment_usage.py:365); source
  order is a close but not exact stand-in. Captured class-body fields use the module string names as of the class
  statement. The `__init__` check fires only for sync defs and only when the innermost class on the stack is a
  grammar class; the class-attribute check needs *no* enclosing function at all (method stack empty,
  lint_grammar_fragment_usage.py:324). `SqlFragment` itself and the forbidden classes are never grammar classes for
  these checks (:235). Message `{class}: {message}`.

### grammar_allowlist_burndown (hard-fail, 397 lines)
- **Computes:** over the linter scripts themselves, the size of five named allowlists (`NAME = frozenset({...})`,
  `NAME: T = {...}`, set unions with `|`); reports `allowlist_grew` at line 1 of the owning file when a list's
  number of distinct string entries exceeds its maximum (all five maxima are 0).
- **Inputs:** Python AST of `scripts/` (tests excluded); the CLI reads only `lint_grammar_render_methods.py` and
  `lint_sql_grammar_escape_hatches.py`.
- **Needs:**
  - `EXISTS kind, field, text, str, count, modnode`: entries by a short recursion over `binary_operator`
    `|` and set elements (any call's first positional argument is unwrapped, not only `frozenset`,
    lint_grammar_allowlist_burndown.py:249-276); `count : { entry(a, v) }` per assignment; the threshold and the
    `grew by` arithmetic are plain comparisons; the finding sits on the module node (line 1).
- **Hard parts:** the reporter keeps the *last* fact per variable name across every file in `scripts/`
  (lint_grammar_allowlist_burndown.py:216), so any other script assigning a set literal to, say,
  `ALLOWLISTED_CLASSES` can hide or replace the owner's count; only the CLI checks ownership (:306-320). In Datalog
  "last" needs an order over (path, offset): text order on paths (NEEDS.md assumes `<` on symbols) plus `!` on a
  later fact says it, if the engine's file order is path order. Assignments anywhere count, nested in functions
  included; tuple targets do not. Once those grammar linters are themselves `.rr` policies whose allowlists are
  `signed_list` or `.input` files, this check becomes a count over those inputs instead of Python AST.

### polymorphic_tripwires (hard-fail, 684 lines)
- **Computes:** seven tripwires in nine named files: base-route serialisation without `registry=` (rest router);
  `*by_name*` maps keyed by a raw `type_name` (two files); forbidden metadata-repair calls (three Auto API files);
  sync polymorphic helpers called from an async def; `_run_sync` / `_execute_simple_sql_ir` inside an async def under
  `_execute_dynamic_schema_polymorphic_metadata_ir` (executor); branch-copy `compile` methods of four classes that
  never call `_canonical_type_name_expr_for_copy`. Callees are matched by last name segment and through local
  aliases (`f = helper`).
- **Inputs:** Python AST of the nine `KNOWN_RELATIVE_PATHS` (also under an `api/` prefix), tests excluded.
- **Needs:**
  - `EXISTS kind, field, text, call_arg, nearest, within, is_async, def_name, file paths`: the per-file switches as
    path equalities; positional-then-keyword lookup of the model argument and the `registry` keyword (absent or
    `None`) by `call_arg`; the type_name key test as `within` over the key expression (attribute `type_name`,
    subscript `["type_name"]`, `getattr(x, "type_name")`); "some enclosing def is named …" by `within` and
    `def_name`; "innermost def is async" by `nearest(c, f, fdef), is_async(f)`; the missing-helper case by `!` over
    calls within the `compile` def.
  - `LIB alias_at(n: node, seed: symbol, x: symbol)`: x names the seed (or an alias of it) at node n: an
    assignment `T = X` / `T = o.X` (targets: names, attribute names, tuple elements) whose statement ends before n
    and whose scope (def or class) encloses n; recursive for chains (lint_polymorphic_tripwires.py:293-303,
    :445-492). Copy-on-enter stacks mean an alias made inside one def is invisible after it returns.
  - `LIB stack_qualname(n: node, s: symbol)`: the enclosing class names, *then* the enclosing def names, dotted,
    plus a leaf; `"<module>"` if none (lint_polymorphic_tripwires.py:208-213). Not `qualname` (module prefix,
    interleaved) nor `scope_name`. Built from a class chain and `fn_chain`.
- **Hard parts:** alias sets are updated on *exit* of the assignment (lint_polymorphic_tripwires.py:365-371), so the
  value's own calls do not see the new alias; order is source order within enclosing scopes. The model-argument
  rule ignores keyword `model=` whenever there are two positional arguments (:335-341). The branch-copy flag is set
  on the innermost enclosing *requiring* context (:440-443), i.e. calls in nested defs count for the outer
  `compile`. Only the innermost class must be in the file's class set (:471-476).

### polymorphic_type_id_authority (hard-fail, 488 lines)
- **Computes:** in six named files: per source line, five SQL-guard regexes (trigger file) and three
  case-insensitive collision-message regexes (two files), one finding per (line, pattern message); in the CRUD and
  prepare-instance files, named defs whose source contains given substrings; in the DDL file,
  `SqlColumnName("type_id")` calls whose innermost def is `generate_polymorphic_check_constraint`.
  Message `{qualname}: {message}`, qualname `<module>` for line patterns.
- **Inputs:** Python source text (raw lines, comments and strings included) and AST of the six
  `KNOWN_RELATIVE_PATHS` (also under an `api/` prefix).
- **Needs:**
  - `EXISTS match_all, modnode, line, text, match, nearest, def_name, str`: line patterns as
    `match_all` over the module node's text with `\s` written `[^\S\n]` so a match cannot cross a line, as the
    oracle's per-line search cannot (lint_polymorphic_type_id_authority.py:291-303), and `(?i)` for the collision
    set; the span's `line` is the finding line, and set semantics give the (line, message) dedup of `_record`
    (:206-210). Function-source substrings via `text(f, s), match(".*type_id must match.*", s)`.
  - `LIB stack_qualname` (as polymorphic_tripwires; lint_polymorphic_type_id_authority.py:200-204).
- **Hard parts:** `_function_source` takes whole lines from the `def` line to the end line (decorators excluded,
  :220-224); a node's text starts at `def`, which differs only by the first line's indentation. A substring in a
  nested def also counts for the outer one. The dedup keeps the first qualname for a (line, message) pair; findings
  keyed with the qualname could differ only if two scopes report the same line and message, which these patterns
  cannot.


### async_patterns (ratchet, 1133 lines)
- **Computes:** a pre-order walk with per-function state, reporting calls by their method (attribute) name:
  ASYNC001 any `….to_thread(…)`, ASYNC002 any `….run_in_executor(…)` (both in any context); in an async def:
  ASYNC004 sync write/read methods while inside a `@X.write_async(…)`/`@X.read_async(…)`-decorated async def chain,
  ASYNC005 six internal sync I/O methods, ASYNC006 26 sync transaction methods when not in txn mode and the
  receiver is bare `self`, rooted at a transaction parameter, or a query alias/chain built from one, ASYNC007
  `read`/`write` on `self._conn|conn|connection|_connection` or a session alias, ASYNC008 two polymorphic helpers.
  Findings marked fallback (api file inside an abstract-looking class, or a fallback comment within ±5 lines;
  ASYNC002 also `_wrap_for_executor` within ±15 lines outside impl) are dropped by the plugin.
- **Inputs:** Python AST of `impl/{sqlite,postgres,mariadb,doltdb,mongodb,neo4j,common}`, `api`, `conform`
  (tests included by scope, then excluded by path substrings `tests/`, `test_`, `__pycache__`); raw source lines
  for the fallback windows; CLI only: `.ratchet-baselines/lint_async_patterns.json` (signed, `violations: 89`).
- **Needs:**
  - `EXISTS kind, field, text, nearest, op, decorator, call_arg, node, match, doc`: method name as the callee
    attribute's `attribute` text; "in an async function" as `nearest(c, f, fn_or_lambda), is_async(f)` (a nested
    sync def or lambda resets the state, lint_async_patterns.py:503-543); the function name from `def_name`;
    the abstract class as `nearest(c, k, class_k)` with an `ABC` base (name or attribute) or, in api files, a name
    containing one of five words (lint_async_patterns.py:309-332); in-scope/in-impl/in-api as `match` on the path.
    The fallback exclusion is negation. The CLI ratchet is `.budget * { "total": 89 }`.
  - `LIB txn_mode(n: node, w: number, r: number)`: the write/read flags in force at n (lint_async_patterns.py:
    465-501). The nearest fn-or-lambda must be an async def A; if A has a `….write_async(…)` / `….read_async(…)`
    call decorator, the flags are A's own (decorators override, both can hold); else recurse to A's nearest
    fn-or-lambda if it is async, else (0, 0). Recursive over `nearest(a, a2, fn_or_lambda)`.
  - `LIB param_ann(f: node, x: symbol, p: node, t: node)`: every parameter form (posonly, regular, kwonly,
    `*args`, `**kw`, with or without type or default) with its annotation. `param` misses typed splats
    (`*args: T`: child 0 is a `list_splat_pattern`) and `site_annotation` covers only `typed_parameter`, not
    `typed_default_parameter` (lint_async_patterns.py:399-420).
  - `LIB ann_mentions(a: node, s: symbol)`: annotation a references type name s: name, attribute's last segment,
    subscript value or slice, `|` either side, tuple element, or a string literal *containing* s as a substring
    (lint_async_patterns.py:353-379). Recursive over the annotation tree.
  - `LIB recv_root(e: node, x: symbol)`: the base name of an attribute/call/subscript chain
    (lint_async_patterns.py:422-437); and `LIB txn_query(e: node, f: node)`: e is a call chain `….m(…)` in which
    some link's method is `query|cross_branch|select_at_revision` on a receiver rooted at a transaction name of
    async def f (lint_async_patterns.py:443-455). Both recursive.
  - `LIB alias_from(f: node, kind: symbol, x: symbol, at: number)`: names that become a session or query alias
    in async def f, with the offset where they do: `x = ….session(…)`, `x = <query chain from txn>`, or `x = y`
    with y an alias at an earlier offset; Assign and AnnAssign, `Name` targets only, never removed by rebinding.
    A use at c counts when `at < start(c)` (lint_async_patterns.py:545-576). Positive recursion with an offset
    comparison; no ordering construct is needed.
  - `LIB line_text(d, l, s)` (as in A.md) plus a window join: `line_text(d, l2, s), l2 >= l-5, l2 <= l+5,
    contains("ASYNC_FALLBACK:", s)`; the TODO form needs `TODO: Implement` and `native async` anywhere in the same
    11-line window, possibly on different lines (lint_async_patterns.py:578-584, :615-616, :339-344).
  - `LIB own_def` (as in A.md): the state of a def covers its decorators and defaults, which tree-sitter puts
    outside `function_definition` (pre-order enters the def before its decorator list).
  - `POLICY` ratchet from the signed `.ratchet-baselines/lint_async_patterns.json` (the A.md need): the CLI
    `--check` compares the actionable count with it (lint_async_patterns.py:1114-1126), while the plugin is
    hard-fail on any actionable finding (lint_async_patterns.py:990-997). The two disagree; the constant `.budget`
    expresses the CLI only while the file is unchanged.
- **Hard parts:** alias sets are per async def and order-dependent (Assign is visited before its own value, so
  `s = conn.session(); s.write()` works but a use before the assignment does not); a nested async def starts empty
  sets but *inherits* txn mode. ASYNC006 is suppressed in txn mode, where ASYNC004 takes over with a different
  method set. Path tests are substrings of the whole path string: `api/` also matches `l9900_auto_api/`, so
  `in_api` (and the abstract-class fallback) can hold for impl files; `test_` anywhere excludes the file.
  Messages carry `ast.unparse(receiver)`; source text differs only in spacing. ASYNC003 is declared and never emitted.

### render_fstrings (ratchet, 597 lines)
- **Computes:** every f-string (AST `JoinedStr`) anywhere inside the outermost def named `render` or `render_*`
  (sync or async, nested defs and lambdas included), reported as `{Class.}method() uses f"<preview>"`, where the
  preview is literal parts with `{...}` for each field, cut to 47 chars plus `...` past 50.
- **Inputs:** Python AST of `impl/common/l2100_sql_grammar/` (tests included); CLI only:
  `.ratchet-baselines/lint_render_fstrings.json` (signed, `violations: 0`).
- **Needs:**
  - `EXISTS def_name, match, nearest, within, strlen, substr`: render defs by `match("render|render_.*", n)`;
    "outermost" as no render def above it (lint_render_fstrings.py:179-183); the class is the innermost class
    enclosing the render def at any depth (`nearest(r, k, class_k)`), which is the class stack top at entry;
    the truncation with `strlen`/`substr`. The ratchet total is `.budget * { "total": 0 }`.
  - `LIB fstring(n: node)`: the nodes Python parses as `JoinedStr`: a `string` whose `string_start` prefix
    contains `f`/`F`, unless it is part of a `concatenated_string` (then the concatenation, if any part is an
    f-string, is one node), plus every `format_specifier` of a replacement field (Python makes each format spec a
    nested `JoinedStr`, and the walk visits it: lint_render_fstrings.py:164-177).
  - `LIB fstring_preview(n: node, s: symbol)`: literal parts unescaped (`{{` gives `{`) and `{...}` per
    interpolation, joined in order by index recursion as B.md's `joined` (lint_render_fstrings.py:191-202).
    Only for the message.
  - `LIB own_def` (as in A.md): f-strings in a render def's decorators or defaults count.
  - `POLICY` ratchet from the signed baseline (the A.md need), CLI only (lint_render_fstrings.py:578-590); the plugin
    is `ratchet` enforcement, which the runner fails on any finding (linter_plugin.py:184).
- **Hard parts:** `f"{x:>10}"` gives two findings on one line (the string and its format spec), with different
  previews; as finding keys without the message they collapse. Implicit concatenation `"a" f"{b}"` is one finding
  at the first part's line. The stats collector (render methods scanned) feeds only the CLI report; `.output`
  covers it if wanted.

### async_sync_boundary (ratchet, 717 lines)
- **Computes:** calls whose dotted callee name is `asyncio.run`/`asyncio.run_coroutine_threadsafe` in a sync
  context (SYNC_WRAPS_ASYNC), `asyncio.to_thread`/`to_thread` in an async def (ASYNC_WRAPS_SYNC), or ends with
  `run_in_executor` in an async def (EXECUTOR_ESCAPE). Context is the innermost def (sync or async), else module
  (sync). Dropped: files under eight informational directory substrings, and 60 (file-suffix, function) allowlist
  pairs with `*` wildcards. Message `{label}: {function}: {call}()`.
- **Inputs:** Python AST of `api/`, `impl/`, `conform/` (tests included by scope; `tests/` etc. are then
  informational); CLI only: `.ratchet-baselines/lint_async_sync_boundary.json` (signed, `actionable: 2`).
- **Needs:**
  - `EXISTS nearest, fdef, is_async, def_name, match, doc`: the context as `nearest(c, f, fdef)` (lambdas and
    classes are transparent, lint_async_sync_boundary.py:329-347); the allowlist as shipped facts
    `allow(suffix, fn)` tested with `match(cat(".*", suffix), path)` and the `*` forms as `contains`
    (lint_async_sync_boundary.py:268-284); informational dirs as `contains`.
  - `LIB dotted_callee(c: node, s: symbol)`: the callee's attribute chain joined by dots, including the base
    name only when the chain ends in a plain name; `loop().run_in_executor` gives `run_in_executor`
    (lint_async_sync_boundary.py:300-311). Recursive with `@dot`; `qn`/`sym_of` resolve imports and would turn
    `from asyncio import run` into a match, which the oracle does not make.
  - `LIB own_def` (as in A.md), for calls in decorators and defaults.
  - `POLICY` ratchet from the signed baseline (the A.md need), CLI only, over the actionable count
    (lint_async_sync_boundary.py:698-704); the plugin fails on any actionable finding.
- **Hard parts:** names are textual, not resolved: `run_in_executor` is any method of that suffix on any receiver,
  `to_thread` bare matches a local helper of that name. The allowlist is keyed by the bare function name (no
  class), so `close` is allowed for every `close` in `impl/postgres/connection.py`. Informational is a substring
  test anywhere in the path (`tests/` inside `conform/…/tests/`).

### backend_coupling (info, 505 lines)
- **Computes:** per import statement outside an `if TYPE_CHECKING:` (whole statement, `else` included), with the
  module being `from` M (raw, no dots) or the first name of `import a, b`: `direct_backend_import` when the file is
  public (`api/`, `data_layer/`, `_internal/`, or a top-level `l\d{4}_` component outside `impl/`, not
  `__init__.py`) or under `impl/common/`, and M starts with one of six `impl.<backend>` prefixes; else
  `sql_in_nonsql` when the file is under `impl/mongodb|neo4j` and M contains `impl.common.<sql dir>` for eight dirs.
  Three registration files, `_internal/dynamic_import.py`, and `test_*` files under `impl/common/` are skipped.
- **Inputs:** Python AST of `api/`, `data_layer/`, `_internal/`, every top-level `l\d{4}_.+` directory,
  `impl/common`, `impl/mongodb`, `impl/neo4j` (tests excluded).
- **Needs:**
  - `EXISTS import_stmt, field, child, text, within, match, doc, path_part, file_name`: the TYPE_CHECKING test
    as `within(i, s), kind(s, "if_statement"), field(s, "condition", c), text(c, "TYPE_CHECKING")` (a bare name
    only, not `typing.TYPE_CHECKING`; python/names' `type_checking_block` resolves the name and exempts only the
    body, so it is the wrong relation here: lint_backend_coupling.py:358-361); module text with dots stripped by
    capture, the first `dotted_name` child for `import`; path classes as `match` on the path. One finding per
    statement, `direct_backend_import` first (lint_backend_coupling.py:369-383). Severity info as `.mode log`.
- **Hard parts:** prefix tests are string prefixes, so `impl.sqlite_utils` counts as `impl.sqlite`. An import in
  `elif TYPE_CHECKING:` or in any later `elif`/`else` of that chain is exempt in the oracle (it sits in the inner
  AST `If`), which in tree-sitter means the elif clause and the clauses after it. The public test reads path
  components for the layer regex but string prefixes for the rest. The older `BackendCouplingChecker` class
  (lint_backend_coupling.py:174-294) is CLI-dead code with slightly different path tests; the collector is the oracle.


## Summary table

Needs that are the same thing under different names are merged into one row. No check here needs a new LANG
construct. The order-dependent state the oracles build while walking the tree ("seen so far", "latest before")
can be written with start-offset comparisons, `max` and `!`. Strings in a fixed order can be built by recursion
over child indices.

| tag | signature or construct | checks that need it |
|---|---|---|
| LIB | `own_def(n: node, f: node)`: the innermost def whose Python-AST extent (decorators, parameter defaults, body) holds n | auto_api_plan_boundaries, system_grant_bypass, system_session_scope, async_patterns, render_fstrings, async_sync_boundary |
| POLICY | ratchet budget read from a signed counts baseline in the repository (`.ratchet-baselines/<linter>.json`, `.system-session-scope-baseline.json`; Ed25519 over a canonical message); outcomes `missing_baseline` / `invalid_baseline_signature` / `baseline_increase` at the baseline file. The CLIs ratchet, while the plugins fail on any finding | system_session_scope, async_patterns, render_fstrings, async_sync_boundary |
| LIB | `line_text(d: number, l: number, s: symbol)`: physical source lines via `match_all` over the module text, plus ±k line window joins for marker comments | system_session_scope, permission_error (exact suppression only), async_patterns |
| LIB | enclosing-name chains: `fn_chain(n, s)` (defs only, dotted, `<module>`); `stack_qualname(n, s)` (class names, *then* def names, plus leaf), built from fn_chain and a class chain. `qualname`/`scope_name` do not fit | notification_lifecycle, polymorphic_tripwires, polymorphic_type_id_authority |
| LIB | `param_ann(f: node, x: symbol, p: node, t: node)`: every parameter form (posonly, regular, kwonly, `*args: T`, `**kw: T`, typed defaults) with its annotation (`param` misses typed splats; `site_annotation` covers only `typed_parameter`) | grammar_fragment_usage, async_patterns |
| LIB | `pos_param(f, x, p, i)`: only the regular positional parameters (`args.args`: after `/`, before `*`); can be a filter over `param_ann` | sync_async_signatures |
| LIB | annotation walkers: `ann_mentions(a, s)` (s is referenced through name, attribute, subscript value/slice, `\|`, tuple, string literal by word or substring), and `ann_head(a, s)` (the head name, with a preference for the union side) | grammar_fragment_usage, async_patterns, sync_async_signatures |
| LIB | `alias_at(n: node, seed: symbol, x: symbol)` / `alias_from(f, kind, x, at)`: local alias chains `T = X` created at a source offset and visible in enclosing-scope order (recursive, offset-compared) | polymorphic_tripwires, async_patterns |
| LIB | ordered string join over children by index recursion (`joined(i, s)`, `fstring_preview(n, s)`); only for exact messages | tstring_deps, render_fstrings |
| POLICY | an input or parse failure becomes a finding and suppresses the others: an unparsable file gives `python_parse_error` at line 1 (needs a parse-status / ERROR-node fact); a malformed allowlist gives one `invalid_allowlist` | grammar_fragment_usage, module_level_caches |
| NATIVE | JSON record reader: `.input r(…)(IO=json, filename=…, records="entries")`, one column per object key | module_level_caches |
| LIB | `ast_parent(n, p)`: the Python-AST parent, skipping `parenthesized_expression` / `argument_list` / `expression_statement` | unresolved_forward_refs |
| LIB | `cmp_link(c, i, op, rhs)`: one row per operator of a chained comparison (`cmp` has none for chains) | unresolved_forward_refs |
| LIB | `ast_name(n, x)`: identifiers in Python-AST `Name` positions (`nameref` minus params, import names, global/nonlocal, except-as, match captures) | system_grant_bypass |
| LIB | `dotted_callee(c, s)`: the textual dotted callee, unresolved (base name only when the chain ends in a name) | async_sync_boundary |
| LIB | `fstring(n)`: the nodes Python parses as `JoinedStr` (f-prefixed strings, concatenations that contain one, format specifiers) | render_fstrings |
| LIB | `grammar_class(c)`, `builds_str(e)`, `rendered_field(e, f)`: the recursive, file-ordered grammar-class and string-capture predicates | grammar_fragment_usage |
| LIB | `txn_mode(n, w, r)`, `recv_root(e, x)`, `txn_query(e, f)`: transaction-mode flags inherited through async defs, receiver roots, and query chains rooted at a transaction | async_patterns |
