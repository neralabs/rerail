# Group 3: what `.rr` lacks for twenty data-layer checks

Oracle: scripts at revision 2b1504ad. Citations are `script.py:LINE` in `scripts/` (helpers in `scripts/lint_lib/`).
The suite runs each check's *plugin* (`lint_all.py` `linter_plugin_factories`), not its CLI. Where the two differ, the
plugin is the reference and the CLI difference is noted.

Messages are not part of a finding's key (loose findings contract), so building message strings (sorted joins,
`{name!r}`) is not listed as a need anywhere below.

## Shared needs (defined once, referenced by name below)

These come up in several checks. Each check's list names the ones it needs and gives its own oracle citation.

- **file_text** (NATIVE) `file_text(path: symbol, root: node)`: the raw text of any file the policy scopes, as one
  root node. `text`, `line`, `contains`, `match` and `match_all` then work on it unchanged. Two uses:
  (a) non-Python files (`.rst`, `.md`), for which core has only `file(path)` (existence) and no text;
  (b) Python files read as the raw string the oracle tests (`token in source`, `source.splitlines()`), with comments
  and blank lines included. For (b), `text(m)` of `modnode` would do if the module node is guaranteed to span byte 0
  to EOF; core does not say so. Native because it is IO. A scope form is needed too: `.scope text "docs/**/*.rst"`
  reads files outside `.language` as text documents.
- **source triggers** (EXISTS with file_text): many plugins set `source_triggers`. A file whose source contains none
  of the strings gets no facts at all (`lint_lib/fact_engine.py:1184`). Usually every finding contains a trigger
  anyway. Where one does not, the gate changes results, and the rule must say
  `count : { trig(t), contains(t, S) } > 0` with `S` the file text. Each check below says whether the gate matters.
- **syntax_error** (NATIVE) `syntax_error(d: number, line: number, msg: symbol)`: document `d` is not valid Python
  for CPython's `ast.parse`. Native because it is parser output. Tree-sitter recovers with ERROR/MISSING nodes and
  does not reject the same inputs as CPython (version-specific syntax), so `kind(n, "ERROR")` is not the same test.
- **extraction-failure finding** (POLICY): a file that does not parse gives exactly one finding at line 1, with a
  code of its own (`SPG-SYNTAX`, `SAP000`, `GPO000`, `LAO000`), and no other findings for that file.
- **name_expr** (LIB) `name_expr(n: node, x: symbol)`: n is a Python `ast.Name`, an identifier in an expression
  position (load, store or del). Sketch: `kind(n,"identifier"), text(n,x)`, excluding the `attribute` field of
  `attribute`, the `name` field of def/class/keyword_argument, identifiers under `parameters` (bare, typed, default
  and splat forms) and `lambda_parameters`, `dotted_name`/`aliased_import` under import statements,
  `global`/`nonlocal` names, `except … as x` targets, and match-pattern capture names. `names.rr` `nameref` excludes
  only the `attribute` and `name` fields, so it also matches parameters and import names, which Python does not
  treat as `Name`.
- **chain** (LIB) `attr_chain(e: node, p: node)` and `call_chain(e: node, p: node)`: p is reached from e by stepping
  down `attribute.object` only (`attr_chain`), or also through `call.function` and `subscript.value`
  (`call_chain`). Both are reflexive and recursive. Each oracle uses one of these step sets, and an extra stopping
  step changes its answer, so the two cannot be merged. Built on them:
  `chain_root(e, x) :- attr_chain(e, p), kind(p, "identifier"), text(p, x)`, and
  `chain_token(e, t)`, the identifier root plus every `attribute` name on the chain (`_receiver_tokens`).
- **param_annot** (LIB) `param_annot(f: node, x: symbol, p: node, t: node, k: symbol)` and
  `param_bare(f, x, p, k)`: parameter x of f is the node p, its annotation is t (or it has none), and k is one of
  "pos" (posonly or regular), "kwonly", "vararg", "kwarg". Sketch: walk `parameters`. Children after a bare `*`
  (`keyword_separator`) or after a `list_splat_pattern` are kwonly. `typed_parameter`/`typed_default_parameter` carry
  field `type`, and the splat kind comes from the wrapped `list_splat_pattern`/`dictionary_splat_pattern`.
  `names.rr` `param(f, x, i)` has neither the annotation nor the kind.
- **str_const** (LIB) `str_const(n: node, v: symbol)`: n is an `ast.Constant` whose value is a `str`, meaning a
  `string` or `concatenated_string` with no `interpolation` child and no `b`/`B` prefix; v is its value. Sketch:
  `str` restricted by `kind`, `child` and a prefix regex. Also `docstring(s: node)`: s is the first
  statement of a class/def/module body and is an `expression_statement` of a `str_const`. `module_doc` exists for
  modules only.
- **py_frame** (LIB) `py_frame(n: node, f: node)`: f is the innermost function whose *Python-ast* subtree holds n.
  In CPython's ast a function's decorators, parameter defaults and annotations sit under the `FunctionDef`. In
  tree-sitter the decorators sit on the enclosing `decorated_definition`, so `nearest(n, f, fdef)` gives the
  decorators of a nested `g` to the outer f. Sketch: `nearest(n, s, frame_stmt)`, where `frame_stmt` is a
  `function_definition` or a `decorated_definition` wrapping one, mapped to the definition. `defstmt(f, st),
  within(n, st)` (EXISTS) already gives "anywhere in f's subtree including decorators".
- **traversal order** (Hard part, shared): "first" and "last" in these collectors mean pre-order over
  `ast.iter_child_nodes` (`lint_lib/fact_engine.py:829`), which is field order, not byte order. A FunctionDef's
  `decorator_list` comes after its `body`, and a ClassDef's `bases` come before its `body`. Node offsets (`node(...)`
  start, or node ids, which names.rr notes grow in source order) match except where decorators or `IfExp`
  (`test` before `body`) are involved.
- **test paths** (POLICY): `FileScopeSpec(include_tests=False)` drops any path with a part `tests` *or* `test`
  (`lint_lib/fact_engine.py:1358`). Directory walks always skip `.git .venv venv __pycache__ tmp`
  (`fact_engine.py:1029`). `.scope tests false` must be defined to mean exactly this.
- **finding at a path** (LANG): a finding located at a repository path that has no parsed document: a required file
  that is missing, or a script outside the policy's scope. Today `.violation` takes `at: node`. Rules plus
  aggregates cannot make an anchor where no document exists, so this is a reporting construct:
  ```
  .violation BCGP001(at: path, line: number) "required architecture file is missing"
  BCGP001(p, 1) :- required(p), !file(p).
  ```
- **signature-failure finding** (POLICY): the oracle checks a signature over allowlists embedded in the script. If
  it fails, the plugin returns *one* finding (`LEG-000`, `PHA000`) at `scripts/<script>.py:1` *in place of* all
  others. `signed_list` exists for the entries, but "refused if the signature fails" is not this reporting shape.
  It needs a policy setting such as `.signed "allow/x.txt" else LEG000`, and the finding-at-a-path construct for its
  location.
- **subprocess collapse** (POLICY): the suite wraps some scripts as subprocesses (`lint_all.py:638-669`). Any failure
  becomes one `SCRIPT_CHECK_FAILED` finding at `scripts/<script>.py:1`, and its message is the first output line.
  The port has to pick which shape to reproduce: the suite's single finding, or the script's detailed findings. If
  the suite's: `count : { X(_, …) } > 0`, plus the finding-at-a-path construct.

---

### hot_path_reflection (hard-fail, 306 lines)
- **Computes:** in 9 fixed hot-path files, five codes: `get_type_hints` calls; `classify_payload` in the executor;
  `getattr/hasattr(x, "model_fields")`; `get_db_column_name` calls in 3 files; any `.model_fields` attribute.
  Per-code file allowlists.
- **Inputs:** Python AST of the 9 `HOT_PATHS` (`lint_hot_path_reflection.py:54-64`); test paths dropped.
- **Needs:**
  - EXISTS `.scope include` (the explicit file list), `doc` path equality for the allowlists, `call_arg` +
    `str` for `_string_arg`, `attr_of` for `.model_fields`.
  - LIB `dotted_expr(e: node, s: symbol)`: `_call_name`. An identifier gives its text. An attribute whose object has
    a dotted_expr gives `@dot(base, attr)`. An attribute whose object does not gives the bare attr. Recursive.
    `lint_hot_path_reflection.py:95-101`. `qn` needs an import binding and does not fall back to the bare attr, so it
    does not fit.
  - LIB str_const for the second argument (`lint_hot_path_reflection.py:108`). An f-string is not a match.
  - POLICY test paths (`lint_hot_path_reflection.py:289`).
- **Hard parts:** `a().get_type_hints()` yields the name `get_type_hints`, so it is reported. `t.get_type_hints()`
  yields `t.get_type_hints`, which is in neither set, so it is not. The match is syntactic: imports are not resolved.

### graph_lazy_sql_plan (hard-fail, 862 lines)
- **Computes:** a contract over 4 fixed files. Banned top-level symbols must stay deleted. Required classes,
  methods and symbols must exist, and are reported at line 1 or at the class line when missing. Methods must carry
  `@pure_lazy`, `@legacy_sync` or `@legacy_async`. Delegate shells must mention a delegate name. One method must
  mention six symbols (GLSP-001…014).
- **Inputs:** Python AST of `l7200_versioned_transactions/{versioned_lazy_mixin,versioned_transactions,graph_lazy_sql}.py`
  and `impl/common/l2070_server_plan/steps/intent_graph.py` (`lint_graph_lazy_sql_plan.py:65-68`).
- **Needs:**
  - EXISTS: absence through `!`/`count = 0` over a required-name relation. Line 1 is `modnode` (its `line` is 1).
    Direct methods via `stmt`/`defstmt`. Decorator names via `decorator` + `attr_of` (`@x.pure_lazy` counts as
    `pure_lazy`, `lint_graph_lazy_sql_plan.py:174-182`). Class-body `Assign` targets via `asg`.
  - LIB name_expr for "mentions" (`on_Name`, `lint_graph_lazy_sql_plan.py:354-360`). Mentions are `Name` ids plus
    `Attribute` attrs anywhere under the method, decorators and parameter defaults included (`defstmt` + `within`).
    Parameter names are not mentions.
  - EXISTS node offsets for "last binding wins". `top_symbols[name]` is overwritten by each later module-level def,
    class, Assign or TypeAlias, and the *kind* is overwritten too (`lint_graph_lazy_sql_plan.py:296,337,352,370`).
    `methods[name]` keeps the last def (`:390`).
- **Hard parts:** top-level means not inside any def or class. Assignments under a module-level `if`/`try` count.
  If a banned function is later reassigned with `X = …`, its kind becomes "assign" and the function is no longer
  reported in the mixin check. `classes.setdefault` (`:297`) keeps the *first* class of a name and its line, but
  methods are attached by class name, so two top-level classes with the same name merge their methods. `_MethodSummary`
  exists only for direct methods of top-level classes (`:379`). The codes GLSP-006/007/010 each mean different
  things in different files. Source triggers: none on this extractor.

### bounded_composed_graph_paths (hard-fail, 387 lines)
- **Computes:** a completion gate written as regexes over source text, not AST. Across 13 Python files, 6 docs,
  `scripts/lint_all.py` and one conformance test it checks: required files exist (BCGP001/011/013); banned defs are
  absent; decorated methods exist inside regex-delimited class blocks; class fields have the required shape;
  banned identifiers appear nowhere (`\bname\b`, comments included); docs do not mention `graph_query_builder(`;
  lint_all registers the check (BCGP012).
- **Inputs:** source text of the files listed at `lint_bounded_composed_graph_paths.py:14-33`, including `.rst` and
  `.md` docs. File existence. No AST.
- **Needs:**
  - NATIVE file_text (`lint_bounded_composed_graph_paths.py:80-82`), for the `.rst`/`.md` docs and the raw Python
    text.
  - LANG finding at a path: BCGP001 for each missing required file at its own path, line 1 (`:85-89`), plus
    BCGP011 for missing docs (`:330-333`) and BCGP013 for the missing conformance test (`:352-353`).
  - EXISTS `match` with captures on spans for region-limited searches. `_class_block` (`:96-102`) is the text from
    `^class X…:` up to the next column-0 `class |def |async def ` or EOF. It can be captured without lookahead,
    `(?ms).*?^class X[^\n]*:(?<body>.*?)(?:^(?:class |def |async def )|\z).*`, and then searched with `match` on the
    `body` span. The line comes from `line` of the span.
  - POLICY subprocess collapse: the suite sees one `SCRIPT_CHECK_FAILED` (`lint_all.py:1919-1925`).
- **Hard parts:** the regex dialect must support `(?m)`, `(?s)`, lazy quantifiers and `\b` (no lookaround or
  backreferences are used). `_method_has_decorator` (`:115-121`) needs the decorator written as a bare `@name` or
  `@name(...)` with exactly 4-space indent, possibly followed by other decorators, so `@mod.pure_lazy` does not
  count. A class block ends at a column-0 `def`, not at a column-0 `@`, so the decorators of the next top-level def
  fall inside it. The `graph_type` test (`:287`) is the substring `"None" in annotation`, so `Optional[X]` passes
  but `X | None` fails. `_module_symbols` matches column-0 `class`/`def` only.

### serverplan_grammar_surface (hard-fail, 142 lines; candidate JSON exists)
- **Computes:** in every Python file of the repository: defs named as forbidden grammar/direct helpers;
  `.attr` uses of those names; `Name` uses of two DDL escape classes; `from … import` of them. Also a syntax-error
  finding.
- **Inputs:** Python AST, broad repo scope with tests included (`lint_lib/serverplan_facts.py:336-350`).
- **Needs:**
  - EXISTS `def_name`, `attr_of`, `import_alias` (the imported name `x`).
  - LIB name_expr for `ast.Name` (`lint_lib/serverplan_facts.py:734`). With plain identifiers, the `from m import
    SqlPrerenderedDdl` name would also count as a `ddl_escape` hit, a second finding the oracle does not make.
  - NATIVE syntax_error + POLICY extraction-failure finding: `SPG-SYNTAX` (`lint_serverplan_grammar_surface.py:85-94`,
    `lint_lib/serverplan_facts.py:505-510`).
- **Hard parts:** each node gives at most one hit, in elif order: grammar name, then direct name
  (`serverplan_facts.py:708-740`); the two sets do not overlap. The import hit is per alias, at the statement's
  line. Source triggers do not change results.

### lazy_sql_surface (hard-fail, 446 lines)
- **Computes:** one code, LSS-001, with five patterns: connection `._grammar_*_lazy()` calls; `SyncSqlConnection`
  defining them; `.execute()/.execute_async()` on a plan-like receiver, outside an allowlist of files and outside
  `with explicit_execution_authority(...)`; `ctx.<attr>` reads inside a `compile` function; `LazyResult[<bad>]` in
  some path prefixes.
- **Inputs:** Python AST under `impl/`, `data_layer/`, `_internal/` and top-level `l####_*` directories, tests
  dropped (`lint_lazy_sql_surface.py:421`, `lint_lib/discovery.py:32-41`).
- **Needs:**
  - EXISTS: path allowlists and prefixes; `within` a `with_statement` whose item is a call of the bare identifier
    `explicit_execution_authority`, the with-items themselves included (`lint_lazy_sql_surface.py:314`); `nearest`
    class for `SyncSqlConnection` (`:307`).
  - LIB chain: the receiver's terminal name, through `call.function` and `attribute.object` (`_call_chain`,
    `:195-205`), tested with `match("(?i)_*(.*plan|plan_executor)_*", t)` (`:208-213`).
  - LIB py_frame: `compile` is the innermost def, and its decorators and defaults count as inside it (`:281`).
  - LIB `unparse(n: node, s: symbol)`: `ast.unparse` text of the subscript slice, compared exactly against
    `"DatabaseRow | None"` and the other bad types (`:296`). A recursive printer over expression kinds that
    normalises spacing, parentheses and quotes. Approximation: `@replace_all(text, "\\s+", " ")` with outer
    parentheses stripped. It differs for `'DatabaseRow'` versus `"DatabaseRow"` (both are outside the bad set anyway).
  - NATIVE file_text, for source triggers. The gate *matters* here: `x._grammar_effect_lazy()` contains none of the
    triggers (`:135-146`), so a file whose only hit is that call gives no findings.
  - POLICY test paths.
- **Hard parts:** the authority depth counts with-blocks across function boundaries. A def nested inside the `with`
  is still exempt. The CLI `_should_scan` (`:164-173`) uses another scope (it also skips `conform`, `tmp` and
  `docs`, and accepts `l####` without the underscore). Use the plugin's scope.

### bootstrap_lazy_sql (info, 390 lines)
- **Computes:** in `l3200_metadata/bootstrap_state.py`: `query_lazy(...).execute_lazy()` (BLB-001), eager
  schema-probe calls (BLB-004), `.model_dump/.model_copy` calls (BLB-003), and `for` loops over row-collection names
  (BLB-002). Report-only.
- **Inputs:** Python AST of one explicit file (`lint_bootstrap_lazy_sql.py:341`).
- **Needs:**
  - EXISTS `.mode log` (enforcement "report", `:349`); elif precedence 001 > 004 > 003 with `!`; `field` chains for
    `_is_query_lazy_execute_lazy` (`:116-121`).
  - LIB `iter_name(e: node, x: symbol)`: an identifier gives its text; a call of the bare identifier
    `list/tuple/set/sorted` gives the iter_name of its first positional argument. Recursive
    (`lint_bootstrap_lazy_sql.py:123-130`). Composable from core, but recursive, so it is a small library relation.
- **Hard parts:** `_should_scan` (`:132`) tests `"bootstrap" in path.as_posix()` on the *absolute* path, so it is
  always true for the scoped file. `sorted(rows, key=…)` unwraps to `rows`. Builtins are matched by name, not
  resolved.

### schema_bootstrap_contracts (hard-fail, 8 lines → lint_schema_bootstrap_contracts.py, 387 lines)
- **Computes:** the plugin file is a marker. The suite runs `lint_schema_bootstrap_contracts.py --check` as a
  subprocess (`lint_all.py:2025-2031`). Over 8 fixed files it reports: `*_lazy` defs without `@pure_lazy`
  (SBC001); missing or broad parameter annotations (SBC002); `getattr/hasattr/setattr/delattr` calls (SBC003);
  `isinstance/issubclass` calls (SBC005); a function taking both a connection-like and a transaction-like parameter
  (SBC004); sync/async types inside lazy functions, or on schema-transaction parameters (SBC006).
- **Inputs:** Python AST of `DEFAULT_TARGETS` (`lint_schema_bootstrap_contracts.py:14-23`).
- **Needs:**
  - POLICY subprocess collapse (`lint_all.py:2025-2031`): the suite sees one `SCRIPT_CHECK_FAILED`.
  - LIB param_annot: params in posonly, args, kwonly, vararg, kwarg order, skipping `self`/`cls` by name
    (`lint_schema_bootstrap_contracts.py:198-215`).
  - LIB name_expr, for `_annotation_names` (`:300-312`): every `Name` id and `Attribute` attr in the annotation.
  - EXISTS core `parsed(s, "annotation", root)`, applied to an annotation that *is* a string constant. On a parse
    failure, the raw string is the name (`!parsed(s,_,_)`, `:302-306`). Strings nested deeper (`list["X"]`) are not
    parsed.
  - EXISTS set equality for `_is_public_readonly_domain_sequence` (`:258-275`): all names of the subscript value
    are `Sequence`, and the slice names are non-empty and disjoint from a set. Use `count : {…} = 0` twice.
  - EXISTS `within` for `_function_annotations` (`:315-328`): `AnnAssign` annotations anywhere under the function,
    nested defs included.
- **Hard parts:** an annotation inside a nested function is reported once for each enclosing `*_lazy` function, at
  the same line. SBC003/SBC005 use the bare name (`Name.id` or `Attribute.attr`), so `obj.getattr()` counts.

### schema_architecture (hard-fail, 232 lines; suite uses lint_schema_architecture_plugin.py)
- **Computes (plugin):** in `l2000_connection`, `l3200_metadata` and `impl/common/l7245_versioned_backends`:
  SAP001, a callable field in a class that has a `SchemaOperationKind` field. For functions with
  `*SchemaTransaction`-annotated parameters: SAP002, mutating the plan (directly or through an alias) when the
  function also executes; SAP005, a `*_lazy` call on the transaction when it executes; SAP004, recursion by its own
  name when it executes; SAP003, a lambda argument to a transaction method. SAP000 on a parse failure.
  **CLI script (232 lines):** `planning_callback` AnnAssign fields, callback-queue calls, and plan mutation inside
  functions named `*materialize_schema_operation_records*`.
- **Inputs:** Python AST (`lint_schema_architecture_plugin.py:229`; CLI `lint_schema_architecture.py:25-32`).
- **Needs:**
  - LIB param_annot (posonly, args, kwonly only; `lint_schema_architecture_plugin.py:52-58`).
  - LIB name_expr, for `_annotation_names` (Name ids and Attribute attrs; `:28-35` of the plugin).
  - LIB chain `attr_chain`/`chain_root` (`_root_name`, `:61-65`): attribute steps only, so a call in the chain stops
    the climb.
  - EXISTS: `within` over the whole function, nested defs and decorators included (via `defstmt`; `ast.walk(node)`,
    `:153`); plan aliases as a one-step flow-insensitive join over `asg` (`:85-92`); the class's direct `AnnAssign`
    statements (`stmt`).
  - NATIVE syntax_error + POLICY extraction-failure finding (`SAP000`, `:201`).
  - CLI only: LIB py_frame, for "innermost function name contains …" (`lint_schema_architecture.py:160`).
- **Hard parts:** facts for an outer function include calls made in nested functions, so one call can be reported
  under both the outer and the inner function (same key). Aliases ignore scope and order. Source triggers do not
  change results.

### bootstrap_eager_resolve (info, 12 lines → lint_bootstrap_lazy_sql.py)
- **Computes:** the same facts and the same findings as bootstrap_lazy_sql, under a second linter name
  (`lint_bootstrap_lazy_sql.py:355-364`).
- **Inputs:** as bootstrap_lazy_sql.
- **Needs:**
  - EXISTS `#include` of one shared rule file into two policies, plus `.mode log`. Nothing new.
- **Hard parts:** none beyond bootstrap_lazy_sql.

### graphql_plan_ownership (hard-fail, 446 lines)
- **Computes:** in `l9750_graphql` and `l9900_auto_api`. Per function, "authority" names start as parameters
  annotated with plan or transaction types and grow by assignment to a fixpoint. It reports reads of `_private`
  attributes on authority roots (GPO001), `getattr(authority, …)` (GPO002) and `object.__getattribute__(authority,
  …)` (GPO003). Also authority-typed fields of `@reusable_template` classes (GPO004), and parse failures (GPO000).
- **Inputs:** Python AST (`lint_graphql_plan_ownership.py:394`).
- **Needs:**
  - LIB param_annot (posonly, args, kwonly; `:52-58`).
  - EXISTS `match_all("[A-Za-z_]\\w*", t)` over the annotation's text for `_annotation_type_names` (`:44-45`). The
    oracle runs it over `ast.unparse`, which also tokenises string annotations' contents, and so does source text.
  - LIB chain `call_chain` + `chain_root` for `_expression_uses_authority` (`:69-78`): through `attribute.object`
    and `call.function`, not subscript.
  - EXISTS recursion for the authority fixpoint: `auth(f, x) :- auth(f, y), assign in f of Name x from an
    expression rooted at y` (`:255-264`). It is monotone, so it stratifies.
  - LIB py_frame: assignments and accesses belong to the *innermost* function frame (`:208,216`), with decorators
    and defaults counted as the function's own.
  - EXISTS `stmt` for "AnnAssign directly in a class body" plus `nearest` class decorated `reusable_template`
    (`:221`).
  - NATIVE syntax_error + POLICY extraction-failure finding (`GPO000`, `:344`).
- **Hard parts:** the module-level `_function_findings` (`:51-108`) uses `ast.walk`, so nested functions count.
  The collector the plugin runs (`:206-268`) uses innermost frames only. The two differ; follow the collector.
  Authority propagates by name with no scope or order: a later assignment taints an earlier read. Source triggers do
  not change results.

### lazy_adapter_ownership (hard-fail, 529 lines)
- **Computes:** for functions at module level and direct methods of module-level classes, in six directories:
  LAO001, a legacy adapter whose body is not a stub. LAO002/003, the adapter's derived `*_lazy` target is missing
  (module level) or not `@pure_lazy`. LAO004, a `@pure_lazy` body calls an eager terminal attribute. LAO005, a
  transaction-facing function calls eager methods on a transaction parameter. LAO006, an async transaction-facing
  function never awaits. LAO000 on a parse failure.
- **Inputs:** Python AST under `data_layer`, `impl/common`, `l9750_graphql`, `l9900_auto_api`, `l3200_metadata`,
  `l2000_connection` (`lint_lazy_adapter_ownership.py:462`).
- **Needs:**
  - EXISTS `stmt`/`defstmt` for the owning functions (`owns_fact`, `:236`). A def under a module-level `if` is not
    an owner.
  - EXISTS `nearest(c, f, owner)` with a derived stop set, which gives each call and await to the nearest owner
    frame (`:284-300`). Combined with LIB py_frame, a decorator call is the decorated function's own.
  - EXISTS `@replace_all(@replace_all(n, "_async$", ""), "_sync$", "")` and `cat(…, "_lazy")`, plus `call_arg`
    keyword `lazy_name` with LIB str_const (`_legacy_target`, `:81-95`). The *first* legacy decorator is taken
    (`legacy_nodes[0]`, `:242`): `min` over child index.
  - EXISTS node offsets, for the target lookup `by_owner_and_name` (`:312`), keyed by (class *name*, function
    name), where the last fact wins.
  - LIB str_const for `_is_adapter_stub`/`_is_abstract_stub` (`:166-186`): a statement is a stub if it is a str or
    `...` expression, `raise`, `pass`, `del`, or a call of `_legacy_delegate_stub[_async]`. Write it as
    `count : { child(body, st, _), !stub(st) } = 0`.
  - LIB param_annot (posonly, args, kwonly). `_is_database_transaction_annotation` (`:104-113`) can be computed on
    the source text: `match_all("[^\\[\\]|]+", t)`, then strip, then take the part after the last dot with a capture.
  - NATIVE syntax_error + POLICY extraction-failure finding (`LAO000`, `:417,431`).
  - NATIVE file_text for source triggers (`:451`). The gate matters for LAO004: `@mod.lazy` sets decorator name
    `lazy`, but the source need not contain the trigger `@lazy`.
- **Hard parts:** the annotation is split on `ast.unparse` text, so a string annotation `'ReadTransaction'` keeps its
  quotes and does not match. Source text behaves the same way, since quotes stay on both sides. Two top-level
  classes with the same name share one lookup key. The owner key uses the class's simple name. Methods of nested
  classes outside any owner frame are dropped. `has_await` counts `await` only (not `async for`/`async with`),
  nested defs included.

### legacy_execution_surfaces (hard-fail, 1724 lines; candidate JSON exists)
- **Computes (plugin, `lint_legacy_execution_surfaces.py:1496-1557`):** in 14 scope directories, one file at a time:
  - LEG-010: legacy-wrapper imports outside an allowlist and without a valid public boundary.
  - LEG-020/021: `_legacy_delegate_stub` defs and calls outside the inventory, with calls inside all-stub boundary
    wrappers exempt.
  - LEG-030: compatibility-wrapper defs.
  - LEG-040/050: raw transaction and raw cursor method calls, per scope with allowlists.
  - LEG-080/081/082: on internal execution paths, eager transaction/plan calls outside legacy surfaces, and
    boundary-helper calls.
  - LEG-083: transitive eager `.resolve()` through name-resolved intra-file calls.
  - LEG-084/085: raw `legacy_execution_boundary` imports and calls.
  - LEG-060/061: graph fallback guards and per-line fallback tokens in the graph scope.
  - LEG-000: the embedded allowlist signature fails.

  CLI only (`scan_repo`, `:1560-1617`): LEG-001 file-name inventory over every file, LEG-070 removed files,
  LEG-071 per-line tokens in `.py/.rst/.md` files, and LEG-002 syntax errors.
- **Inputs:** Python AST and raw text; the embedded signed allowlist constants (`:158-347`). CLI: every repository
  file path, and the text of `.rst`/`.md` files.
- **Needs:**
  - EXISTS `signed_list` for the allowlist entries, moved out of the script into signed list files. POLICY
    signature-failure finding plus LANG finding at a path: LEG-000 replaces all findings (`:1498-1509`).
  - LIB `from_module_raw(i: node, m: symbol)`: `ImportFrom.module` as written, dots dropped (`on_ImportFrom`,
    `:416-433`). `from ..l2050_transaction.legacy_wrappers import x` matches, and `from .legacy_wrappers import x`
    does not. `module_text`/`import_alias` resolve relative imports, so they do not give this. Sketch: the
    `dotted_name` text of `module_name`, or of `relative_import`'s child 1.
  - LIB chain `chain_token` with the `cast` step: `cast(T, x)` with 2 or more arguments descends into `x`, and
    subscript descends into its value (`_receiver_tokens`, `:598-609`). The token tests become
    `match("(?i)…", t)`, with `(?i).*cursor.*` for cursors (`:619-683`).
  - LIB `class_path(n: node, s: symbol)`: the enclosing class names joined by "." (functions skipped), giving the
    symbol `Class.method` (`:516`). Recursive with `cat`.
  - LIB `name_call_target(c: node, g: node)` for LEG-083 (`_resolved_function_calls`, `:1024-1040`). The candidate
    is the callee's attr or id, plus `Class.attr` for `self./cls.` calls. A candidate resolves only when exactly one
    function symbol in the file has that simple name or symbol, and that symbol is not the caller.
    `count : {…} = 1`. Then reach-eager is a recursion over it, cut at legacy surfaces, and the reported line is the
    first call (`min`) whose target reaches. `python/calls.rr` `call_target` resolves by scope, not this way, so it
    gives different answers.
  - LIB py_frame: calls belong to the innermost function. `_legacy_surface_depth` is any enclosing legacy-decorated
    function (`within`, `:499`).
  - EXISTS node offsets for "boundary wrapper" (`:533`): a function is one if some function *at or before it* in
    traversal order with the same name has a legacy decorator. The same applies to boundary-name aliases added at
    the import (`:424`): a call before the import does not see the alias.
  - EXISTS `cmp(c, "is", l, r)` with `r` = None and `chain_token(l)` for `_is_graph_fallback_guard` (`:741-753`).
  - NATIVE file_text, for LEG-061: per-line substring tests over raw lines
    (`match_all("(?m)^.*(grammar_rows?_lazy|fetch(one|all)_via_plan).*$", S)`, `:1331-1347`). Also for source
    triggers (`:86-100`), and the gate matters: LEG-030 names, the LEG-060 guard and the `grammar_rows_lazy` token
    contain no trigger.
  - EXISTS path tests with a capture for `removeprefix("api/")` (`:384`), `_is_internal_execution_path` (`:998`).
  - CLI only: NATIVE file_text for `.rst/.md` (LEG-071, `:1121-1144`), and `file(path)` for LEG-001/070
    (`:1084-1118`).
- **Hard parts:** LEG-083's DFS (`:1043-1081`) has a shared memo, a `visiting` set that returns None on a cycle, and
  roots taken in `sorted(symbol)` order (`:1236`). A function whose only path to an eager call goes through a cycle
  member visited earlier gets memo None, and is not reported. A Datalog least fixpoint would report it. Exact parity
  needs a procedural DFS; recommend the fixpoint and document the difference. `functions[symbol]` is overwritten
  when a symbol is redefined, but calls already recorded stay on the old object. "First eager line" follows
  traversal order, where decorators come after the body. A decorator *call* (`@legacy_sync(...)`) is one of the
  wrapper's calls, so such a wrapper is never "all stub" (`:542`).

### removed_compat_surfaces (hard-fail, 277 lines; candidate JSON exists)
- **Computes:** any def or class name, `Name` id or `.attr` from a set of 36 removed names (RCS001), in 8 directories.
- **Inputs:** Python AST (`lint_removed_compat_surfaces.py:251`).
- **Needs:**
  - EXISTS `def_name`, `attr_of`.
  - LIB name_expr (`on_node`, `:185-200`): import names, parameters and keyword names are not `Name`, so they must
    not count.
  - POLICY test paths.
- **Hard parts:** the plugin keeps duplicates. The CLI dedupes by (line, name) (`:164`), which makes no difference
  under set-of-keys. Source triggers are the names themselves, so the gate changes nothing.

### legacy_callback_quarantine (hard-fail, 515 lines)
- **Computes:** callback APIs (`read/write/…` taking `fn`) not marked legacy (LCB-010). Calls of callback methods on
  session- or connection-like receivers outside legacy functions (LCB-020). Connection classes defining
  `commit_branch` (LCB-030). Calls of it on connection receivers (LCB-031). Protocol classes are exempt.
- **Inputs:** Python AST under `impl`, `data_layer`, `services` and every top-level directory starting with `l`
  (`lint_legacy_callback_quarantine.py:472`).
- **Needs:**
  - LIB chain `chain_token` (no `cast` step; `:158-167`), tested with `match("(?i)(conn|…)|.*(session|connection|version)", t)`.
  - LIB param_annot kind, for "a parameter named `fn` among posonly, args or kwonly", where vararg/kwarg do not
    count (`:181-182`).
  - EXISTS `nearest(f, c, nonproto_class)` with a derived set (`:306`): the innermost *non-Protocol* class at any
    depth, defs in between ignored. Plus `within` a Protocol class, and `within` a legacy-marked function for depth.
  - EXISTS "the call has at least one argument" (`:265-268`; `_call_supplies_callback` is subsumed).
- **Hard parts:** `_is_protocol_class` (`:186`) takes the base's name through Name, Attribute or Call only, so
  `Protocol[T]` (a Subscript) is *not* a protocol. The scope glob is `l*/**`, which includes `llm_runtime`. Test
  parts are `tests`/`test` (plugin); the CLI also drops `conform` (`:69`).

### pending_pipeline_effect_kinds (hard-fail, 246 lines)
- **Computes:** any def or class name, `Name` or `.attr` from 4 deleted names (PPK001), in `l2070_batching`,
  `impl` and `scripts`, excluding the script itself.
- **Inputs:** Python AST (`lint_pending_pipeline_effect_kinds.py:213-222`).
- **Needs:**
  - LIB name_expr (`:98-117`); EXISTS `def_name`, `attr_of`.
  - EXISTS `.scope exclude "scripts/lint_pending_pipeline_effect_kinds.py"` (the oracle uses an empty collector
    for its own file, `:121-142`).
- **Hard parts:** none beyond name_expr.

### versioned_lifecycle_legacy_surfaces (hard-fail, 283 lines; candidate JSON exists)
- **Computes:** in `impl/`, classes named `*VersionedWriteTransaction` or `*LazyVersionedWriteTransactionSparse`
  that directly define `commit/rollback/commit_branch` and their `_async` forms (VLL001), reported at the def line.
- **Inputs:** Python AST (`lint_versioned_lifecycle_legacy_surfaces.py:252`).
- **Needs:**
  - EXISTS `stmt`/`defstmt` for direct methods, decorated ones included (`:129-131`). A def under an `if` in the
    class body is excluded. `def_name`, plus `match` on the class name.
- **Hard parts:** the line is the `def` line, not the decorator's (CPython ≥3.8 `lineno`). The tree-sitter
  `function_definition` starts at `def`/`async`, which agrees.

### plan_helpers_allowlist (hard-fail, 607 lines)
- **Computes:** every import, alias assignment or call of `*_via_plan` helpers and lazy terminals (PHA001), unless
  its call-site string `module::Context.path` is in a signed allowlist. Private helpers are exempt inside
  `plan_helpers` itself. PHA000 when the signature fails.
- **Inputs:** Python AST under `api`, `impl` and top-level `l####_*` directories (`lint_plan_helpers_allowlist.py:532-541`);
  the signed constant `ALLOWLISTED_CALLSITES` (`:101-117` of the script body).
- **Needs:**
  - EXISTS `signed_list` for the entries. POLICY signature-failure finding plus LANG finding at a path (PHA000,
    `:490-503`).
  - LIB `ctx_path(n: node, s: symbol)`: *all* enclosing class names, then *all* enclosing function names, each
    outer to inner, joined by "." (`_context`, `:249-251`). `def f(): class C: def g` gives `C.f.g`. Built
    recursively with `cat`. `qualname` uses nesting order, so it differs.
  - EXISTS module name from the path, with captures (`_path_to_module`, `:234-247`): from the first `impl` part, or
    after `api`, or from the first `l####_` part.
  - LIB from_module_raw (`node.module == …`, `:271,277`).
  - LANG position-ordered recursion, for the alias map `imported_names` (`:288,339`). It is one file-wide dict keyed
    by local name, scopes ignored. It is written in traversal order, and only when the value resolves to a helper.
    It is read at each use as of that point. The value of `b = a` depends on the alias of `a` *before* that
    assignment, which depends on earlier assignments. So "latest resolving alias before position p" is recursion
    through `max` or `!`. It is locally stratified by source position, and Soufflé rejects it. Construct: an
    aggregate or negation allowed inside recursion when every recursive dependency points to a strictly earlier
    position.
    ```
    .pragma "ordered-by" "helper_at" "u"    // strata ordered by u's position
    helper_at(u, h) :- use(u, x), s = max s2 : { alias_site(x, s2, _), s2 < u }, alias_site(x, s, h).
    ```
    `alias_site(x, s, h) :- assign_name(s, x, v), helper_at(v, h).` closes the cycle. The flow-insensitive
    approximation (any alias anywhere) differs only when a name is rebound.
  - EXISTS `match("(?i).*(execute|fetch|scalar|ddl).*", local)` for `_is_eager_alias` (`:213`).
- **Hard parts:** `_TargetHelperSet.__contains__` (`:133-136`) makes `in TARGET_HELPERS` true for the
  `ALLOWED_LAZY_TERMINALS` and legacy private names too. So a *call* of `grammar_rows_lazy(...)` is reported as
  "old or private … call", even though importing it under a lazy alias is fine. In `import …plan_helpers` without
  `as`, the alias is recorded as `plan_helpers` (`:305`), although Python binds `impl`. Tuple and list targets are
  unpacked. The CLI scope adds skips (`docs`, `services`, `conform`, …), which the plugin does not have.

### serverref_purity (hard-fail, 253 lines)
- **Computes:** in `l2070_batching/server_ref.py`: `_plan_from_lazy_value` defs; forbidden strings in a
  `__slots__ = (…)` tuple; assignments to forbidden attributes (plain and annotated); annotated fields in
  `*ServerRef` classes whose AnnAssign mentions `Any`/`object` (SRP001).
- **Inputs:** Python AST of one file (`lint_serverref_purity.py:232`).
- **Needs:**
  - LIB str_const for the slot strings (`:114-117`), with the tuple only; a list does not count.
  - LIB name_expr for `Name` `Any`/`object` (`:139-141`); EXISTS `attr_of` for `.Any`.
  - EXISTS `within(n, s)` for an AnnAssign s, and `nearest` class (innermost at any depth, through functions,
    `:125`).
- **Hard parts:** `_mark_annotation_ancestors` (`:146-157`) climbs to the *first* AnnAssign above the name, so `Any`
  in the target or the value, not only in the annotation, flags the field. String annotations (`"Any"`) are not
  seen. Only direct targets of `Assign` count; tuple-unpacking elements and augmented assignment do not.

### serverref_sql_lowering (hard-fail, 377 lines)
- **Computes:** in the production directories (as lazy_sql_surface): `.bind_cte` attributes; `render` methods on
  ServerRef classes; `cast(SqlRenderable…, value|session_id_rhs)`; SRL001 per raw line containing
  `getattr(type(value), "render", None)`; NLB001, native-list classes without a docstring; NLB002, `.driver_value()`
  outside two files; NLB003, except handlers touching `.PORTABLE_VARIADIC`.
- **Inputs:** Python AST plus raw source lines (`lint_serverref_sql_lowering.py:143-148, 253`).
- **Needs:**
  - NATIVE file_text for the per-line substring scan, comments and strings included (`:143-148`), via
    `match_all("(?m)^.*getattr\\(type\\(value\\), \"render\", None\\).*$", S)`.
  - LIB str_const / `docstring` for `ast.get_docstring(clean=False) is None` (`:184`).
  - EXISTS `stmt` (direct method of a ServerRef class, `:220-224`); `within` an `except_clause` for any attribute
    `.PORTABLE_VARIADIC`, the exception type included (`:205-215`); `file_name` for NLB002 (`:190-192`); a
    recursive `subscript.value` descent for `_is_sqlrenderable_expr` (`:132-139`).
  - POLICY test paths.
- **Hard parts:** `except*` clauses are `except_group_clause` in tree-sitter and must be included. Source triggers
  do not change results.

### raw_execute_regression (hard-fail, 444 lines; candidate JSON exists)
- **Computes:** in the production directories, RER001 for: calls of removed raw-surface attributes; defs of removed
  names, except `execute_raw` query materialisers and raising stubs in compatibility files; the whole-file
  presence of 5 banned substrings, reported at line 1.
- **Inputs:** Python AST plus raw source text (`lint_raw_execute_regression.py:185-193, 421`).
- **Needs:**
  - NATIVE file_text for `banned in text` over the whole file, comments and strings included, reported at line 1
    (`_check_text`, `:185-193`). Line 1 is `modnode`.
  - LIB py_frame: `raise` and `._raise_*()` calls belong to the innermost function only (`:248-276`), decorators
    included.
  - EXISTS `nearest` class at any depth ending in `Query` or `_CachedQuery` (`:179-182`); a path allowlist for the
    stub files (`:300`).
  - POLICY test paths.
- **Hard parts:** the plugin path does not apply `_is_excluded` (`:160-170`). The plugin scope already leaves out
  `scripts/`, `demos/` and the rest, but `node_modules`/`venv` parts would only be excluded by the CLI. Several
  banned substrings in one file give several findings with the same key, line 1.

---

## Summary table

Distinct non-EXISTS needs, merged across names. Checks are this group's.

| tag | signature or construct | checks that need it |
|---|---|---|
| POLICY | test-path meaning of `.scope tests false`: parts `tests`/`test`; walks skip `.git .venv venv __pycache__ tmp` | hot_path_reflection, lazy_sql_surface, bootstrap_lazy_sql, bootstrap_eager_resolve, schema_architecture, graphql_plan_ownership, lazy_adapter_ownership, legacy_execution_surfaces, removed_compat_surfaces, legacy_callback_quarantine, pending_pipeline_effect_kinds, versioned_lifecycle_legacy_surfaces, plan_helpers_allowlist, serverref_sql_lowering, raw_execute_regression (15) |
| LIB | `name_expr(n, x)`: Python `ast.Name` (identifier in expression position) | graph_lazy_sql_plan, serverplan_grammar_surface, schema_bootstrap_contracts, schema_architecture, removed_compat_surfaces, pending_pipeline_effect_kinds, serverref_purity (7) |
| NATIVE | `file_text(path, root)`: raw text of any scoped file (Python with comments, `.rst`, `.md`) + `.scope text` | bounded_composed_graph_paths, lazy_sql_surface, lazy_adapter_ownership, legacy_execution_surfaces, serverref_sql_lowering, raw_execute_regression (6) |
| LIB | `py_frame(n, f)`: innermost function in Python-ast terms (decorators, defaults, annotations included) | lazy_sql_surface, schema_architecture (CLI), graphql_plan_ownership, lazy_adapter_ownership, legacy_execution_surfaces, raw_execute_regression (6) |
| LIB | `param_annot(f, x, p, t, kind)` / `param_bare(f, x, p, kind)`: parameter, annotation, and pos/kwonly/vararg/kwarg kind | schema_bootstrap_contracts, schema_architecture, graphql_plan_ownership, lazy_adapter_ownership, legacy_callback_quarantine (5) |
| LIB | `attr_chain(e, p)`, `call_chain(e, p)` (± `cast` step), `chain_root(e, x)`, `chain_token(e, t)` | lazy_sql_surface, schema_architecture, graphql_plan_ownership, legacy_execution_surfaces, legacy_callback_quarantine (5) |
| LIB | `str_const(n, v)` (str `ast.Constant`: no f-string, no bytes) and `docstring(s)` | hot_path_reflection, lazy_adapter_ownership, serverref_purity, serverref_sql_lowering (4) |
| NATIVE + POLICY | `syntax_error(d, line, msg)` (CPython parse failure) + extraction-failure finding at line 1 with its own code | serverplan_grammar_surface, schema_architecture, graphql_plan_ownership, lazy_adapter_ownership (4) |
| LANG | finding located at a path with no parsed document: `.violation X(at: path, line: number)` | bounded_composed_graph_paths, legacy_execution_surfaces, plan_helpers_allowlist, schema_bootstrap_contracts (via collapse) (4) |
| POLICY | signature-failure finding: one coded finding at the script path replaces all others | legacy_execution_surfaces, plan_helpers_allowlist (2) |
| POLICY | subprocess collapse: the suite sees one `SCRIPT_CHECK_FAILED` per script; the port must choose a shape | bounded_composed_graph_paths, schema_bootstrap_contracts (2) |
| LIB | `from_module_raw(i, m)`: `ImportFrom.module` as written, relative dots dropped, unresolved | legacy_execution_surfaces, plan_helpers_allowlist (2) |
| LIB | `class_path(n, s)` (enclosing classes) / `ctx_path(n, s)` (all classes, then all functions) | legacy_execution_surfaces, plan_helpers_allowlist (2) |
| LIB | `unparse(n, s)`: `ast.unparse` text (whitespace, parentheses and quotes normalised) | lazy_sql_surface (1; lazy_adapter_ownership can use source text) |
| LIB | `dotted_expr(e, s)`: dotted Name/Attribute text, falling back to the bare attr on a non-name base | hot_path_reflection (1) |
| LIB | `iter_name(e, x)`: identifier through `list/tuple/set/sorted(...)` wrappers | bootstrap_lazy_sql, bootstrap_eager_resolve (2) |
| LIB | `name_call_target(c, g)`: intra-file call resolution by unique simple name / `Class.attr` | legacy_execution_surfaces (1) |
| LANG | position-ordered recursion: aggregate or negation inside recursion when dependencies point to earlier positions | plan_helpers_allowlist (1) |

Composable from existing relations, so not needs: source-trigger gating (`contains` over file_text; it changes
results in lazy_sql_surface, lazy_adapter_ownership and legacy_execution_surfaces); "last definition wins" and
"first in order" (node offsets, `max`/`min`); decorator names (`decorator` + `attr_of`); direct members (`stmt`,
`defstmt`); innermost or non-Protocol class (`nearest` with a derived set); region-limited regex (`match` captures
on spans); report-only (`.mode log`); two linters on one fact set (`#include`).
