# Data-layer checks, group 2: what `.rr` lacks

Oracle: the scripts at 2b1504ad. Line cites are `script.py:LINE`. When a script has both a suite plugin and a CLI,
the suite plugin is the hard-fail path, so it is the one followed here. Shared conventions:

- *Event order*: the fact engine visits the CPython AST depth-first, in pre-order, in `ast.iter_child_nodes`
  order (`lint_lib/fact_engine.py:829-842`). That is source order except for `FunctionDef` (body before
  decorators and return annotation), `IfExp` (test first) and `Dict` (all keys, then all values).
- *Source triggers*: an `AstCollectorSpec`/`FactExtractorSpec` with `source_triggers` runs on a file only if the file's text
  contains one of them (`lint_lib/fact_engine.py:1184-1190`). Usually this is only a speed-up, but some checks
  depend on it, and those are noted. Writing it as a gate is `EXISTS modnode, text, contains` everywhere.
- *Test exclusion*: `FileScopeSpec(include_tests=False)` drops any path with a part named `tests` or `test`
  (`lint_lib/fact_engine.py:1358`). That is `.scope tests false`.
- *Def extent*: the visitors push a def or class on their stack *before* visiting its decorators, bases, defaults
  and annotations. Code in those positions therefore counts as inside that def or class. In tree-sitter,
  decorators sit outside `function_definition`, in `decorated_definition`, so `within`/`nearest` alone put them
  outside. This is the `def_extent` need below.

### rust_sql_grammar_boundary (hard-fail, 84 lines)

- **Computes:** one finding for each (line, pattern) pair in which one of 8 regexes matches a line of a Rust source file. The
  patterns are `pyo3`, `pythonize`, `Python<'_>`, `PyResult`, `Py[A-Z]…`, and three generated-code markers.
- **Inputs:** the raw text of `rust/nera_compiler_sql/src/**/*.rs`. The second root, `src/generated_grammar`, lies
  inside the first. No AST is used; comments and strings are matched too.
- **Needs:**
  - `NATIVE text documents for non-Python files`. A policy has one `.language`, and a document is a parsed source
    file. This check reads `.rs` files as plain text (`lint_rust_sql_grammar_boundary.py:47-51, 57`). The binary
    needs one of two things: a text reader giving `doc(d, path, "")` plus one root node whose `text` is the file,
    or a `.language rust` grammar. Datalog cannot do this, because it is IO. The text reader is the cheaper of
    the two and serves every text-only check.
  - `LIB source_line(d: number, l: number, s: span)`: one row per physical line. Body:
    `source_line(d, l, x) :- node(d, r, _, 0, _), !within(r, _), match_all("(?m)^(?<x>.*)$", r), line(x, l).`
    The check is per line (`:58-61`). A whole-file `match_all` is not the same: `\s*` in `Python\s*<\s*'_?\s*>`
    would cross newlines. Built only from `match_all` and `line`, so it is LIB, not NATIVE.
  - `EXISTS match, .scope include`: `R(x, p) :- source_line(_, _, x), pat(p, re), match(cat(".*", cat(re, ".*")), x)`
    (Python `re.search` is a substring search; core's `match` matches the whole string). One finding per (line,
    pattern), which is what `lines.first` in the candidate JSON means.
- **Hard parts:** Python `re` and the engine's regex differ slightly. `\b` and `\s` are Unicode in both, but the
  definitions of `\w` can differ on rare scripts. `str.splitlines()` (`:58`) also splits on `\r`, `\x0b`, `\x0c`,
  `\x1c-\x1e`, `\x85`, ` ` and ` `. A `(?m)^…$` split breaks only on `\n` (and `\r\n`), so the line numbers
  differ when a file contains those characters.

### grammar_callable_constructors (hard-fail, 565 lines)

- **Computes:** callable escape hatches in grammar classes, under 7 codes:
  - `CALLABLE_INIT_PARAM`: an `__init__` parameter of a class with `render()` whose annotation mentions `Callable`;
  - `CALLABLE_FIELD`: such a parameter stored as `self.f = p`;
  - `CONSTRUCTOR_FIELD_CALLED_IN_RENDER`, `UNTYPED_…` and `ANY_…`: render calls `self.f()` where `self.f = p`;
  - `CALLABLE_CONSTRUCTOR_ARG`: a lambda, or a name that resolves to a function, passed to a call of a renderable class;
  - `SIMPLE_NAMESPACE_RENDER_HOOK`: `SimpleNamespace(render=…)`;
  - `DYNAMIC_RENDER_ASSIGNMENT`: assigning to `….render`.

  The set of renderable classes is project-wide: a top-level class with `def render` in *any* scanned file.
- **Inputs:** Python AST of `impl/common/l2100_sql_grammar/**/*.py`, minus any path with a `tests` part and minus
  files named `fragments.py` (`lint_grammar_callable_constructors.py:407-408`). Files that fail to parse are skipped
  silently (`:418`). There is also a signed allowlist of finding keys, held in the script and empty (`:96-98`).
- **Needs:**
  - `EXISTS def_name, kind, field, child, is_async, docof`: `renderable(x)` holds when a module-level class x has a direct, non-async
    `def render` (`:159-160, :219-224`). It is joined across documents by name only (`:423-427`); a symbol join
    needs nothing new.
  - `LIB expr_name / leaf_name` (see serverplan_steps), with the oracle's `_name` extras: a subscript gives its
    value's name, and a call gives its function's name (`:135-144`). Every class and constructor match is by this
    unresolved leaf text (`:289-292`).
  - `EXISTS within, text, kind`: an annotation "contains Callable" when it is a name or attribute with leaf
    `Callable`, or has one below it (`:147-152`). This is not names.rr `annotation`, because string annotations
    are not parsed.
  - `LIB sig_param(f, p, x, kind, ann, dflt)` (see the summary table): the constructor parameters are `args` +
    `kwonlyargs` only, never posonly or splats (`:370-374`). names.rr `param` cannot tell those apart.
  - `LIB last_asg`-like "last wins" (see serverplan_steps): `_init_field_assignments` is a dict, so for a field
    assigned twice, the *last in `ast.walk` order* wins (`:245-256`). `ast.walk` is breadth-first, so this is
    (depth, position) order, not source order.
  - `LIB fn_symbol(n, x)`: the oracle's own name lookup (`:395-401`). It walks the enclosing *sync* `def`
    scopes, innermost first, then the module. In each scope, a name that is a direct nested `def` (not
    `async def`) gives True, and a local binding gives False. The local bindings are parameters, plus
    assign/augassign/annassign/for/with-as targets found by `ast.walk` (`:185-212`). Two quirks: `ast.walk` still
    enters nested defs (the `continue` only skips the def node itself, `:199-201`), and `async def`, lambdas and
    classes push no scope (`:278-286`). This is recursive Datalog over `nearest(s, s2, sync_def)`. names.rr
    `ref_site` + `site_form(…, "def")` is LEGB-correct and gives different answers.
  - `LIB def_extent(n, f)` (see the summary table): the scope is pushed before the decorators and defaults are visited.
  - `EXISTS call_arg, kind`: a lambda or name in positional or keyword arguments; `**kw` has no value and is skipped (`:293-297`).
  - `EXISTS signed_list`: the allowlist is keyed by `path:line:code:detail` (`:117, :499`). The suite plugin
    filters by it but *never verifies its signature*; only the CLI does (`:542-548`).
- **Hard parts:**
  - CALLABLE_FIELD and CONSTRUCTOR_FIELD_CALLED_IN_RENDER come from one loop over (field, param), so a param
    stored in two fields gives two identical keys. These collapse under the loose contract.
  - Line anchors are the def lines of `__init__`/`render`, not the assignment or the call (`:331, :338`).
  - `_has_render_method` and `_find_method` see only direct `FunctionDef` children, so a decorated `render` *is*
    seen (a `decorated_definition` in tree-sitter) and an `async def render` is not.

### grammar_str_params (hard-fail, 629 lines)

- **Computes:** three codes for grammar classes not on a 14-name class allowlist:
  - `STR_PARAM`: an `__init__` positional parameter annotated with bare `str` as one `|`-alternative, unless the
    (class, param) pair is in a 28-entry allowlist;
  - `STR_CALL_IN_RENDER`: `str(self.<slot>)` inside the class's `render`;
  - `STR_CONCAT_IN_RENDER`: the same inside a `+`.

  The slots are `_<param>` for every STR_PARAM param. A class with no `__init__` counts every `self.x` as a slot.
- **Inputs:** Python AST of `impl/common/l2100_sql_grammar/**` (recursive; suite plugin
  `lint_grammar_str_params.py:539`). The CLI only globs top-level files (`:554`). Both allowlists and the zero
  baselines are in the script.
- **Needs:**
  - `NATIVE unparse(n: node, s: symbol)`: CPython's `ast.unparse` text of an expression. `_annotation_contains_str`
    splits the *unparsed* annotation text on `|` and strips each part (`:220-227`). Datalog cannot rebuild
    canonical text: that needs re-printing the whole subtree with Python's precedence, quoting and spacing rules.
    The source text is not the same thing: `(str | None)` unparses to `str | None` and matches, while the raw text
    gives `(str`. A structural "union alternative" LIB relation is not exact either: `X[int | str | float]` gives the
    part `str` and is reported, while `X[int | str]` gives `str]` and is not. The finding's message also prints the
    unparsed annotation.
  - `LIB base_name(c, s)` (see serverplan_steps), restricted to Name/Attribute bases (`:230-238`): a grammar class
    has a base starting `Sql`/`_Sql`, or a direct `def render` (`:241-252`).
  - `LIB sig_param`: only `args.args`, not kwonly or posonly (`:355`).
  - `EXISTS nearest, negation`: the render context. `render` must be a sync def whose innermost *class* ancestor is
    the grammar class. Any def counts, even one nested in a method (`:292-298`). Calls inside nested defs or classes
    in render still count, and are attributed to the outer render's class (`:300-310`).
  - `EXISTS kind, field, op, text`: `str(...)` must be a bare name (`:304`). For CONCAT, one operand of `+` is that
    call (`:323-331`). The same call then also yields STR_CALL_IN_RENDER.
  - `EXISTS .budget * by code`: the baselines are all 0 (`:185-189`). The reporter emits *every* finding of a code
    whose count exceeds its baseline (`:501-505, :522`). With zeros this equals plain blocking.
- **Hard parts:** `str_slots is None` (no `__init__`) means *every* slot counts (`:380-384`). An allowlisted class
  pushes `is_grammar_class=False`, which also turns its render checks off (`:284`).

### generated_index_sources (hard-fail, 275 lines)

- **Computes:** in the generator file, every `Index(…)` call (the bare name `Index`, a from-import alias of
  `l1000_schema.schema_def.Index`, or `<schema_def alias>.Index`) is checked. If its innermost function is not
  `_generated_index`, it is reported. If it is, it must pass `source=source`. The `_generated_index` def itself
  must have a `source` parameter annotated `IndexSource` whose kw-only default is not `MODEL_DECLARED`. All
  findings share one code, `missing_source`, and differ by message.
- **Inputs:** Python AST of the one file `impl/common/l7245_versioned_backends/generic_sparse.py`
  (`lint_generated_index_sources.py:61, 241`).
- **Needs:**
  - `EXISTS .scope include`, `import_alias`, `node` offsets: the alias sets grow as the visitor reaches each import
    (`:77-89`). So an `Index` alias counts only for calls *after* its import in event order, and file-wide, not
    per scope. This is a join with a `start` comparison and no recursion.
  - `EXISTS nearest(c, f, fdef), def_name`: `function_stack[-1]` is the innermost def or async def; at module level
    it is `""` and the call is reported (`:107`). Use `def_extent` for decorators (summary table).
  - `EXISTS call_arg`: the first `source=` keyword must be the bare name `source` (`:124-129`).
  - `LIB sig_param`: `source` is searched in `args` + `kwonlyargs`, and only kw-only defaults count (`:132, :143`).
  - `EXISTS kind, text`: the annotation must unparse to exactly `IndexSource`, and the default must not unparse to
    `IndexSource.MODEL_DECLARED` or `MODEL_DECLARED` (`:138-149`). Both are simple enough to test by structure
    (an identifier, or an attribute of an identifier, with parentheses looked through), so `unparse` is not needed.
    A string annotation `"IndexSource"` unparses with quotes and is a violation, which matches the structural test.
- **Hard parts:** the source trigger matters: a file that calls only an alias `I(…)` and contains none of `Index(`,
  `_generated_index`, `IndexSource` is never scanned (`:67`). With one fixed file this is unlikely.

### serverplan_bypass (hard-fail, 675 lines)

- **Computes:** every call whose callee leaf is `_execute_raw_sql`, `_query_raw_sql_async` or
  `_execute_raw_sql_async` is reported, unless its callsite is on the signed allowlist. The callsite is
  `module::Class.func`, `module::*` or `module::Class.*`. The code is the method name. Findings are deduplicated
  by (file, line, method).
- **Inputs:** Python AST of `data_layer_production_dirs` (`api`, `impl`, `facade`, `data_layer`, `_internal`, `l####_*`),
  tests excluded (`lint_serverplan_bypass.py:518-521`). The allowlist, its Ed25519 signature and public key are in
  the script; every category is empty (`:112-129`).
- **Needs:**
  - `EXISTS leaf_name` (summary), `.scope dirs`, `.scope tests false`.
  - `LIB local_qualname` variant `ctx(n, s)`: `"{innermost class}.{innermost def}"`, `"{def}"` or `"<module>"`
    (`:291-299`). The two stacks are independent: a call in a class body nested in a def gets `Inner.outer`, and a
    class with no def around the call gives `<module>`. names.rr `scope_name` takes the class *around the def*
    instead.
  - `LIB def_extent(n, f)` (summary): the stacks are pushed before decorators, bases and defaults are visited, so a call
    in a decorator takes the def's context.
  - `EXISTS match, path_part, cat`: the module name for allowlist keys is the path from its first `impl`/`api` part,
    or else from the first `l\d{4}…` part (`:275-289`). It is not `doc`'s module.
  - `EXISTS signed_list`: the three key forms, once the in-script dict becomes a signed list file.
  - `POLICY signature failure is one gate finding`: when verification fails, the plugin returns *only*
    `invalid_allowlist_signature` at the script, line 1, and drops every real finding (`:474-483`). core's
    `signed_list` "is refused", with no defined outcome.
  - `EXISTS min : { … }`: findings are keyed by (line, method) (`:391-400`). Pick one call node per key with a
    `min` over start offsets.
- **Hard parts:** the `FETCH_METHODS` set is dead (`:233-240`). The CLI also skips only `/tests/` substrings and
  accepts explicit paths. That is CLI-only.

### mandatory_imperative_tripwire (hard-fail, 8 lines → lint_mandatory_imperative_tripwire.py, 659 lines)

- **Computes:** the plugin is a marker. The suite runs `lint_mandatory_imperative_tripwire.py --check` as a
  subprocess (`lint_all.py:1864-1871`), which calls `violations()` only. The consumer scan (DLC-*) runs only with
  `--consumer-root`, which the suite never passes, so it is out of scope. `violations()` asserts the shape of
  one function, `assert_imperative_execution_allowed` in purity.py:
  - MIT-001: it exists at module top;
  - MIT-002/003: no forbidden opt-out name or string anywhere in the file;
  - MIT-009: its parameters are exactly {sink, is_pending};
  - MIT-010: every `return` in it sits directly under an `if` whose test unparses to one of 4 approved guards;
  - MIT-004: no `getenv`/`get_config`/`getenv_bool` calls;
  - MIT-005: it calls `_find_external_imperative_caller`;
  - MIT-006: it raises `LazyPurityViolation(...)`.

  It also checks three raw-text facts in other files:
  - MIT-007: purity_signature.py contains `"assert_imperative_execution_allowed"`, in double quotes;
  - MIT-008: lazy_result.py and lazy_seq.py each contain the substring `assert_imperative_execution_allowed(` at
    least twice, and base_server_plan.py at least once;
  - MIT-017/018: two of the script's own constant sets are subsets of others.
- **Inputs:** `l2055_lazy_result/purity.py` (AST and text), plus the text of `l2055_lazy_result/purity_signature.py`,
  `l2055_lazy_result/lazy_result.py`, `l2055_lazy_result/lazy_seq.py` and `impl/common/l2070_server_plan/base_server_plan.py`
  (`lint_mandatory_imperative_tripwire.py:19-23`).
- **Needs:**
  - `EXISTS .scope include` (five paths), `modnode` as the anchor of the file-level findings (MIT-001/005/006/007/008
    have no line, `:351, :381, :386, :391, :401`), and `child`/`def_name`/`is_async` for "a module-level def or
    async def" (`:315-319`).
  - `LIB py_name_node(n, x)` (see post_op_side_effects): MIT-002 is `ast.Name` only (`:355`), so not parameters,
    import aliases, `global` names or attribute leaves. `nameref` includes parameters and import names.
  - `NATIVE py_str_const(n, v)` (summary): MIT-003 is any `ast.Constant` string (`:357`). That includes docstrings,
    implicit concatenations (one constant) and f-string literal pieces.
  - `LIB sig_param`: MIT-009 is `args` + `kwonlyargs` only. Posonly and splats do not count (`:360-365`). The set
    equality is two negations.
  - `LIB py_if_guard(r, t)`: the CPython parent `If` of a `return` (`:367-373`). A return directly in an if body
    gets the `if`'s test. In an `elif` body it gets the elif's test. In the final `else:` it gets the test of the
    *last* `if`/`elif` of the chain (the else is that nested If's `orelse`). Anywhere else (in a function body, a
    `for`, `try`, `with`) it is `<unconditional>`. In tree-sitter, `else_clause` and `elif_clause` hang off the
    first `if_statement`, so this needs a small LIB mapping. The walk is all of `ast.walk(guard)`, nested defs included.
  - `NATIVE unparse` (see grammar_str_params): the guard test is compared as `ast.unparse` text against
    `not is_pending`, `_approved_outer_execution_boundary()()`, `caller is None` and
    `module_name in _TRIPWIRE_ALLOWED_MODULES` (`:305-312, :372`). The four shapes are simple, so a structural
    match is a fair approximation. Exact equality, though (comments, parentheses, line breaks inside the test),
    needs unparse.
  - `EXISTS leaf_name, within`: MIT-004/005/006 use `_call_name` (bare name or last attribute) over `ast.walk(guard)`
    (`:376-386`).
  - `EXISTS match, count : { match_all(…) }` on the module text (`text(m, t)`), for MIT-007/008. These are raw
    substrings, comments included (`:388-401`).
  - `EXISTS facts + negation`: MIT-017/018 compare constants of the linter itself (`:402-408`). In `.rr` both
    sets are facts in the rule file, so the check is a self-test of the policy.
  - `POLICY required input missing or unparsable`: the oracle raises when any of the five files is missing or
    purity.py does not parse (`:347-348, :388, :399`). The subprocess then exits non-zero, which is a hard
    failure. `.rr` needs a declared outcome: a finding, or an engine error that blocks.
- **Hard parts:**
  - MIT-002 and MIT-003 apply to the *whole* purity.py, not just the guard.
  - MIT-010's "parent" is the immediate AST parent, so a `return` inside `if a: for …: return` is unconditional.
  - `EAGER_SINKS` and the behavioural-eager discovery (`:185-302`) feed only the consumer scan.

### execution_authority_bypass (hard-fail, 590 lines)

- **Computes:**
  - AUTH001: an `ast.Name`/`ast.Attribute` named `_explicit_authority_var` or `_legacy_boundary_var`, outside
    `l2050_transaction/execution_boundary.py`.
  - AUTH002: a call named `explicit_execution_authority` whose (module, qualname) is not a registry entry.
  - AUTH003: `*.setattr(...)` monkeypatching authority internals.
  - AUTH004/005: the registry tuple `EXPLICIT_EXECUTION_AUTHORITY_CALLSITES` and a curated set of 11 pairs in the
    script are not equal; each direction is one code.

  Two (path, qualname) test exceptions apply.
- **Inputs:** Python AST of `l<digit>*/`, `impl/`, `data_layer/`, `conform/` and `tests/`, tests *included*,
  `.git`/`.venv`/`__pycache__`/`tmp` excluded (`lint_execution_authority_bypass.py:554-569`). The oracle *imports*
  `l2050_transaction.execution_authority_tokens` at load time (`:69`). That file is a literal tuple of
  `ExecutionAuthorityCallsite(module=…, function=…, signature_b64=…)` calls.
- **Needs:**
  - `EXISTS call_arg, str, kind, docof`: read the registry statically, from the keyword arguments of the calls in
    the tuple assigned to `EXPLICIT_EXECUTION_AUTHORITY_CALLSITES`. That file is in scope.
  - `LIB local_qualname` variant with `<locals>` (`:306-322`): class names are appended plainly. Each def that
    has any enclosing *def* gets `<locals>.` before it, and a class inside a def does not. So `f.C.m` comes out
    as `f.C.<locals>.m`, which is not Python's `f.<locals>.C.m`. Use `"<module>"` when the stack is empty. names.rr
    `qualname` has the module prefix and no `<locals>`.
  - `EXISTS replace_all(path)`: the module is the path parts joined with dots, `__init__` kept (`:141-142`).
  - `LIB py_name_node` for AUTH001's Name case. `field(_, "attribute", n)` gives the Attribute case (`:154-159`).
    The Attribute is reported at the start line of the whole attribute expression.
  - `EXISTS leaf_name, call_arg, str, contains`: AUTH003's two argument forms (`:170-187`). The first
    argument is a string containing a marker. Or the second argument is a string literal in a 4-name set and the
    first is the name or attribute leaf `execution_boundary`.
  - `EXISTS facts + negation, modnode`: AUTH004/005 are set differences. The oracle anchors them at line 0 of the
    tokens file (`:194-219`); `modnode` gives line 1.
  - `LIB def_extent`: calls in decorators and defaults take the def's qualname (the visitor pushes on entry).
- **Hard parts:** the oracle reads the *runtime* value of the registry. A static read matches only while the file
  stays a literal tuple of keyword-argument calls. Anything computed would need `POLICY` "unreadable input". The
  registry's signatures are not checked here; the runtime checks them. AUTH004 compares on (module, function) only.

### serverplan_steps (hard-fail, 3564 lines)

- **Computes:** about 40 per-file AST sub-checks over the ServerPlan step modules, reported under 30 codes
  (STEP-016…050; STEP-033 and STEP-034 each cover several distinct sub-checks). Each sub-check is keyed on
  `ServerPlanStep` subclasses (by base *name*) and their methods (`compile`, `level`, `can_batch`, `step_tags`,
  `tables_read/written`, `execution_role`, `execute_runtime`, `cte_output_columns`, `cte_column_type_hints`, …),
  or on calls by callee *text* (`CompiledStep(...)`, `RuntimeBuilderSpec`, `runtime_builder_post_op`,
  `ResultMaterializationSpec`, `ResultAssertionSpec`, `SqlColumnTypeHints`, …) and their keyword arguments.
  The sub-checks cover: purity of `compile()` (no branching on resolved `self._x` data, no `model_dump` and similar
  calls, no raise in L0 steps); post-op callbacks (no mutation of captured `self._` aliases, no recovery of txn/conn
  authority, no user callables); runtime-builder escape hatches; runtime-ref post-ops (four per-file
  function-summary fixed points); role contracts (VIRTUAL/SQL against tables, tags, grammar and publication
  methods); and two signed allowlists (`can_batch` False, runtime state on `self`).
- **Inputs:** Python AST only. Plugin scope is `impl/common/l2070_server_plan/steps`,
  `impl/common/l7300_sql_versioned_transactions/steps` and `impl/common/l9700_sql_graph_query/steps`, without tests
  (lint_serverplan_steps.py:90, 3459). The CLI also scans `l2070_batching/post_op.py`
  (lint_serverplan_steps.py:95, 3538), but the suite plugin does not, and the candidate JSON includes it. One
  constant comes from another script: `POST_OP_FATE_OVERRIDES` (lint_post_op_side_effects.py:203). The signed
  allowlists `CAN_BATCH_FALSE_ALLOWLIST` and `LIFECYCLE_RUNTIME_STATE_ALLOWLIST` (both empty) and their signature are
  constants in the script itself (lint_serverplan_steps.py:115–117, 220). The CLI-only `--scan-legacy-postops` mode
  also reads `impl/common/l2070_server_plan`, `l2055_lazy_result`, `l2080_lazy_orm` and `l2070_batching`
  (lint_serverplan_steps.py:190).
- **Needs:**
  - `EXISTS within, within_stop(n, f, scope)`. The oracle uses two walks. `_walk_compile_body` and
    `_iter_non_nested_returns` really stop at nested def, class and lambda, so they are `within_stop`. Every
    `for node in _walk_nodes(fn): if <nested def>: continue` loop skips only the def node itself and still visits
    its body, so it is plain `within` (see Hard parts).
  - `EXISTS method_of, def_name, retval, call_arg, callee, nameref, str, kind/field/child`, and `min` over
    `node` start offsets for "first": `_get_method` takes the first body item with the name, `_get_class_level` the
    first `return <int>`, and STEP-026 `break`s at the first offending node (lint_serverplan_steps.py:361, 2474, 508).
  - `EXISTS count : {…}` for "exactly one non-docstring statement, `return ()`" (STEP-034b,
    lint_serverplan_steps.py:428) and for "no args and no keywords" (STEP-038, lint_serverplan_steps.py:1678).
  - `EXISTS match("(?i).*(callback|callable|…).*", x)` for every lowercase-substring token test
    (lint_serverplan_steps.py:611, 616, 656, 2209).
  - `EXISTS` rule-local recursion for the expression-shape predicates `_expr_has_user_callable_shape`,
    `_expr_is_direct_callable`, `_literal_kwargs_has_mutable_literal`, `_literal_payload_values`,
    `_expr_derives_from_output_columns`, `_expr_consumes_runtime_refs` and `_expr_uses_refless_runtime_refs_post_op`
    (lint_serverplan_steps.py:2214, 637, 772, 1896, 1931, 1153, 1255). The oracle's `seen_names` guard against cycles
    gives the same answer as a Datalog least fixpoint: reachable along some path equals reachable along some simple
    path.
  - `EXISTS` recursion for the four per-file "function summary" fixed points (lint_serverplan_steps.py:1226, 1328,
    1760, 2119): a recursive relation keyed by (document, def name) and joined to calls by exact callee text.
  - `EXISTS text(modnode, s), contains(s, "closed_post_op")` for the whole-source text gates in front of
    STEP-033/034/035 (lint_serverplan_steps.py:3297–3301). The gate on STEP-035 is not redundant: a file with
    `source_refs=` and no `runtime_ref` substring is skipped. Cost: one interned symbol per file.
  - `EXISTS signed_list` for `can_batch_false` and `lifecycle_runtime_state` (STEP-020/021,
    lint_serverplan_steps.py:2611, 2657), once the in-script constants become signed list files.
  - `EXISTS` a message or reason column for the codes that several sub-checks share (STEP-033 has three, STEP-034
    two, STEP-029 eight messages).
  - `LIB expr_name(e: node, s: symbol)`: the *syntactic* dotted name of a Name/Attribute chain, with no resolution
    and nothing for other roots (`_node_name`, lint_serverplan_steps.py:602). Body:
    `expr_name(e,x) :- kind(e,"identifier"), text(e,x).` and
    `expr_name(e,@dot(p,a)) :- attr_of(e,o,a), expr_name(o,p).` Add `leaf_name(e,x)` for the last segment
    (`_runtime_builder_call_name` :668; the `rsplit(".")[-1]` at :1786 and :2105). `qn`/`sym_of` resolve through
    imports, and `dotted` covers import items only. Nearly every sub-check compares this unresolved text
    (`== "CompiledStep"`, `.endswith(".ResultMaterializationSpec")`).
  - `LIB base_name(c: node, s: symbol)`: the textual name of each positional base: an identifier's text, an
    attribute's attr, or recursively a subscript's value (`_extract_base_name`, lint_serverplan_steps.py:332). Then
    `step_class(c) :- base_name(c, "ServerPlanStep")`. `class_base` is resolved and would differ: it misses
    unresolvable bases and needs imports.
  - `LIB ann_mentions(a: node, x: symbol)`: some identifier text, or an attribute's attr, under annotation `a`
    equals `x`; a variant also matches string constants that *contain* x (`_annotation_mentions_name` :413,
    `_annotation_mentions_any` :1703; used with `Hydrator`, `RawResult` and AUTHORITY_TYPE_NAMES :131). This is not
    `annotation`/`parsed`: the strings are tested by substring, not parsed.
  - `LIB last_asg(f: node, x: symbol, v: node)`: the value of the last `x = v` or `x: T = v` with a bare-name target
    anywhere under f, nested defs included, last by source order. It is flow-insensitive and scope-blind
    (`_collect_simple_assignments` :1122, `_collect_method_assignments` :1917). Body: `max` start over Assign and
    AnnAssign sites under `within(a, f)`, joined back. `binding_value` plus `ref_site` is scope-aware and may-precise,
    so it gives other rows.
  - `LIB arg_or_kw(c: node, i: number, k: symbol, v: node)`: positional argument i if the call has one, else keyword
    k (`_runtime_builder_builder_name_expr` :683, `_is_scalar_value_post_op_call` :1530, `assert_result` spec
    :2106). Body: `call_arg(c,i,"",v).` and `call_arg(c,-1,k,v), !call_arg(c,i,"",_).` The oracle counts starred
    arguments in `len(call.args)`, and `posarg` does not.
  - `LIB empty_refs(v: node)`: `None`, `""`, or an empty tuple, list or set (`_runtime_ref_ids_may_be_non_empty`
    :1085). It is shared by STEP-027/033/035/046/049. Parentheses and concatenated strings need care (the candidate
    JSON's `empty-refs` util).
  - `LIB post_op_fate(symbol, fate)`: shared facts from lint_post_op_side_effects.py:203, with `AUDIT_REQUIRED` as the
    default (lint_serverplan_steps.py:1479, 1500). They should live in one `#include` shared with post_op_side_effects,
    which signs them. Today the table has no `RuntimeBuilderKind.*` or `RuntimeBuilderName.*` keys, so STEP-033(b)
    reduces to "report when the closed_post_op has no runtime-builder symbol".
  - `POLICY` signature failure becomes one finding: if the multi-allowlist signature fails, the plugin returns a
    single `invalid_allowlist_signature` finding at the script, line 1, *instead of* all other findings
    (lint_serverplan_steps.py:3419–3430). `signed_list` is "refused if the signature fails", which is a refusal, not
    a reported finding that replaces the others. Both lists are signed by one signature (:220).
  - `POLICY` extraction-failure findings: a parse error becomes a `PARSE` finding at the line taken from the error
    text (lint_serverplan_steps.py:3229–3263).
  - `POLICY` a second, report-only mode: `--scan-legacy-postops` emits `LEGACY-POSTOP` category findings over a
    wider scope that excludes any path containing `/tests/`. It prints per-category counts and fails only when the
    total exceeds `--legacy-postop-ratchet-max N` (lint_serverplan_steps.py:3064, 3142–3176, 3518–3527). It is not
    part of the suite plugin. As a separate policy, `.budget * { "total": N }` and `.output` cover it.
- **Hard parts:**
  - **Fake "skip nested" loops.** `_walk_nodes` returns a flat pre-order tuple, so `continue` on a nested
    def/lambda/class skips only that node. Nested bodies *are* scanned in raw-row shaping (:1734), result-builder
    raises (STEP-041 :2021), static-exception summaries (:2140, whose `Return`s include nested defs' returns), method
    assignments (:1917) and `_method_calls_raw_row_shaping_function` (:1797). Only `_walk_compile_body` and
    `_iter_non_nested_returns` truly stop.
  - **Summaries by name, not by node.** A summary is keyed by `fn.name` in the file, so STEP-034a reports *every*
    def with a summarized name, including ones that do not qualify (:1352). Calls match summaries by full dotted text,
    so `self.helper()` ("self.helper") never matches the summary "helper". STEP-041 alone matches by leaf name
    (:2105). Callback defs are found by name: a dict over all defs (last wins) for STEP-028 (:2251), and over nested
    defs of compile for STEP-026 (:448).
  - **Multiplicity.** STEP-033/035 scan calls with `_walk_nodes(fn)` for *every* function, so a call in a nested
    function is reported once per enclosing function, each time with that function's `last_asg` bindings
    (:1375, 1414); module-level calls are never checked. STEP-019 checks defs nested in non-def statements twice
    (:2579). STEP-021 walks nested classes, so a nested step class is reported twice. STEP-023 keys by `path:Class`,
    so same-named classes in one file are reported together (:2765). Under the loose findings contract the finding
    key (code, node, reason) collapses most of these.
  - **First, last and pre-order.** "First" and "last" follow Python-AST field order, which is source order except for
    `Dict` (all keys, then all values), `IfExp` (test, then body) and `FunctionDef` (body before decorators and
    returns). Start-offset `min`/`max` matches the oracle except on those shapes (STEP-026 first offending node,
    `last_asg` with dict-literal values).
  - **Python ast against tree-sitter.** `elif` is a nested `If` in Python, so STEP-016 must also take `elif_clause`
    conditions (:2431). `a = b = v` is one Assign with two targets in Python and nested assignments in tree-sitter.
    Python has no parenthesized or decorated nodes: a `decorated_definition` wraps a method, and
    `parenthesized_expression` must be seen through in every shape predicate. A "Name" is `nameref`, not any
    identifier: `_expr_references_names` and the `self._x` alias tests must not match attribute or keyword
    identifiers. `args[1]` of a callback means positional parameters only (posonly and args), which `param`'s child
    index does not give when `/`, `*` or splats come first (:471).
  - **Small quirks.** `_get_class_level` accepts `bool`, so `return False` makes a step L0 and `return True` gives
    level 1 (:2474). STEP-018 looks at *top-level* body statements only (:2485). `_returns_false` (STEP-020) does
    too (:2607). `_returns_empty_frozenset` scans every return, nested ones included (:2702).
    `_method_returns_nonempty_concrete_tables` tests `tables_read` first and `tables_written` only when the first
    gives nothing (:2946). `_execute_runtime_contains_python_shaping` (:1579) is dead code. The kind/name pair check
    (STEP-032 slot, :1012) returns nothing.
  - **Messages.** Some messages join sorted sets (`sorted(tags)` :3003, `sorted(forbidden)` :2202,
    `sorted(symbols)` :1500). Under the loose contract these are display text, so no string-aggregation construct is
    needed.
### post_op_side_effects (hard-fail, 1329 lines)

- **Computes:** an inventory check over four fixed files. It reports:
  - forbidden runtime-ref tokens in the post-op implementation (POSTOP200);
  - a bypass of the runtime signature verifier (POSTOP203);
  - category-level entries in the signed runtime registry (POSTOP204);
  - set differences between the signed registry and the symbols the interpreter can reach (POSTOP205/206);
  - three Ed25519 signatures over sets computed from the code (POSTOP201/202, and the fate signature, which gates
    POSTOP001/002/000);
  - stale or debt-fated enum members (POSTOP001/002).

  Every finding is reported at `l2070_batching/post_op.py` line 1, with the message `"{symbol}: {message}"`
  (`lint_post_op_side_effects.py:1243-1251`).
- **Inputs:** the Python AST of exactly these files (`explicit_files`, `:1259`):
  - `l2070_batching/post_op.py`;
  - `l2070_batching/post_op_specs.py`;
  - `l2070_batching/post_op_domain_decoders.py`, read only for POSTOP200;
  - `l2070_batching/post_op_allowlist.py`.

  Runtime config held in the lint script itself:
  - the fate override map `POST_OP_FATE_OVERRIDES` (`:203`);
  - the signatures `POST_OP_FATES_SIGNATURE` (`:230`) and `POST_OP_SURFACE_SIGNATURE` (`:102`);
  - the Ed25519 public key `PUBLIC_KEY_HEX` in `scripts/lint_lib/signing.py:33`.

  The runtime registry's signature is a string literal in `post_op_allowlist.py` (`ALLOWLIST_SIGNATURE`).
  No git, no baseline file.
- **Needs:**
  - `EXISTS doc, modnode, child, field, kind, text, str`: limit to the four files by path. The anchor of
    every finding is `modnode` of post_op.py, since the oracle anchors all findings there at line 1.
  - `EXISTS kind/field/child/text, cat`: the enum members of top-level classes `PostOpKind`, `PostOpBehavior` (post_op.py,
    post_op_specs.py) and `DomainPostOpDecoderKind` (post_op_specs.py). A member is a direct class-body
    `Assign`/`AnnAssign` with a bare-name target, rendered as `"Class.NAME"` (`:538-552`, `:576-588`). This is composable
    from the tape. A small reusable form would be the `LIB` relation below.
  - `LIB class_body_name(c: node, x: symbol)`: x is a bare-name target of an assignment or annotated assignment that is a
    direct statement of class c's body, with or without a value. The body sketch: `field(c,"body",b), child(b,es,_),
    kind(es,"expression_statement"), child(es,a,0), assign_name(a,x)`, where `assign_name` recurses into a chained
    `right` assignment. `binding(c, x, site, "assign")` is not the same: it includes tuple-unpacking targets and
    statements nested under `if`/`for` in the class, and it omits the bare `X: int`. (`:576-588`)
  - `EXISTS child, field, kind, is_async, text, match, decorator`: the surface entries.
    - `post_op_factory.<f>` is a top-level non-async `def` in post_op.py whose name ends in `_post_op`.
    - `post_op_spec.<C>` is a top-level class in post_op_specs.py whose name ends in `Spec`, decorated by the bare
      name `dataclass` or by a call of it; `dataclasses.dataclass` does not count (`:591-628`).
    - "Top-level" means a direct child of `module`, or the `definition` of a `decorated_definition` that is one.
      `rdefn` is not that: it accepts defs nested under a module-level `if`.
  - `EXISTS str, kind`: the entries of the runtime registry are the string elements of the set, list or
    tuple literal assigned to `SIGNED_POST_OP_SYMBOLS` at module top. The literal may be wrapped in a call
    (`frozenset({...})` → its first argument) (`:632-652`). The signature is the `ALLOWLIST_SIGNATURE` value from
    an **annotated** assignment only. It is a string literal, or the bare name `NEEDS_SIGNING`/`UNSIGNED`, which
    becomes that symbol (`:664-675`).
  - `NATIVE signed_set(linter: symbol, set: relation, sig: symbol)`: true when `sig` is a valid Ed25519 signature,
    under the fixed maintainer key, of the canonical JSON `{"allowlist":[sorted entries],"linter":linter,"version":1}`.
    The JSON is built over **all rows of a derived unary relation** `set`. It is false for `NEEDS_SIGNING`, `UNSIGNED` and `""`.
    It is used three times:
    - POSTOP201: `post_op_surface` over the surface set (`:735-753`);
    - POSTOP202: `post_op_runtime_allowlist` over the registry entries (`:914-933`);
    - the fate signature: `post_op_side_effects` over the `"Member=fate"` strings (`:940-952`).

    Why native: Ed25519 cannot be written in Datalog. Datalog also cannot serialise a whole relation in sorted
    order: there is no ordered string aggregate. core's `signed_list` does not fit. It reads entries from a signed
    file and *refuses* when the signature fails. Here the entries are computed by rules, the signature is a value
    (a policy constant or a literal in source), and a failure is a finding or flips other rules. The `set`
    argument reuses the relation-as-argument form of `within_stop`/`nearest`. The engine must evaluate `set`
    completely in an earlier stratum.
  - `EXISTS rules + facts + negation`: the fate of each member is its override, or `audit_required` when it has none
    (`:936-942`). The override facts and the fate signature move from the lint script into the rule file.
    The **trusted** overrides are those facts gated on `signed_set(...)` holding (`:1207-1208`). POSTOP001 is a
    trusted override with no member (`:1019-1027`). POSTOP002 is a member whose fate is a debt fate
    (`:1029-1043`). An invalid signature therefore makes every member POSTOP002.
  - `EXISTS rules`: POSTOP200 tests four token positions against the set of 18 tokens (`:115-133`, `:678-687`).
    - an `ast.Name` identifier;
    - an `ast.arg` parameter name;
    - the attribute part of `x.attr`;
    - a string constant equal to the token (docstrings included).

    The attribute part is `field(_, "attribute", n)` and the string case is `str`. One finding per node,
    symbol `"path:line"`.
  - `LIB py_name_node(n: node, x: symbol)`: identifier n stands where CPython's AST has an `ast.Name` or an `ast.arg`.
    The sketch is `nameref(n,x)` minus several identifier positions, plus every parameter identifier
    (`*args`, `**kw`, lambda parameters, typed and default parameters). The excluded positions are:
    - the dotted names of `import`/`from` statements;
    - `global`/`nonlocal` names;
    - `except … as x`;
    - match-pattern capture names.

    `nameref` keeps the excluded positions, which in CPython are bare `str` fields (`:678-687`).
  - `EXISTS min : { … }, line, within, negation`: POSTOP203 has four cases, mutually exclusive in this order
    (`:756-809`):
    1. any top-level `Assign`/`AnnAssign` to the bare name `ENFORCE_POST_OP_RUNTIME_ALLOWLIST`;
    2. else no top-level `def`/`async def assert_post_op_symbol_signed`. If there are several, the first in source
       order is used (`next(...)`);
    3. else no call named `_post_op_allowlist_signature_valid` or `verify_allowlist` anywhere inside it (`_call_name`:
       a bare name or the last attribute);
    4. else a `return` whose line is below the first verification call's line.

    Case 4 is `min l : { … line(c, l) }` and a comparison.
  - `EXISTS match`: POSTOP204 is an entry that has none of the prefixes `assertion.`, `decoder.DomainPostOpDecoderKind.`,
    `materializer.`, or is one of five broad names (`:818-842`).
  - `EXISTS within, str, match, negation`: the reachable set for POSTOP205/206 has two parts (`:845-872`).
    - String constants under the first top-level `def post_op_runtime_symbol` in post_op.py that start with
      `assertion.` or `materializer.`. "Under" means anywhere: nested functions and conditional expressions count.
    - `"decoder." + member` for each `DomainPostOpDecoderKind` member.

    POSTOP205 is reachable but not registered; POSTOP206 is registered but not reachable (`:875-896`).
  - `EXISTS negation over finding relations`: POSTOP000 fires only when the fate signature is invalid and no other
    POSTOP finding exists (`:1235-1241`). This uses stratified negation of the other `.violation` relations. It can
    fire only when there are no enum members, because an invalid signature otherwise yields POSTOP002 for every
    member.
  - `POLICY extraction failure`: the oracle raises, and the check crashes rather than reporting, in these cases:
    - `SIGNED_POST_OP_SYMBOLS` is absent, its value is not a set/list/tuple literal, or it has a non-string element
      (`:648`, `:651-652`);
    - `ALLOWLIST_SIGNATURE` is absent or not annotated (`:674-675`);
    - post_op.py or post_op_specs.py is missing or does not parse (`:567-573`, `:1209` re-reads them from disk).

    In the plugin path, a missing allowlist file instead gives empty entries and `NEEDS_SIGNING` (`:1226-1230`).
    That yields POSTOP202, plus POSTOP205 for every reachable symbol. A hard-fail policy needs a declared outcome
    for "a required input file or literal is missing or malformed", reported as a finding or as a check error.
  - `POLICY hard-fail`: no budget. Any finding blocks (`.mode block`).
- **Hard parts:**
  - **The taint analysis is dead code.** `_RuntimeOpBehaviorVisitor` (`:290-535`, codes POSTOP103–113: helper-call
    taint propagation, mutation, comprehension filtering) is never called by `_collect_violations` or the plugin.
    Only `tests/test_lint_post_op_side_effects.py` uses it. A port must **not** implement value-flow taint. It is
    the main over-claim risk in reading this file.
  - **Fate signature, fates and trust.** The fate signature is computed over the *full* override map. The trusted
    map that drives POSTOP001 is that map, or `{}` when the signature fails. This is not recursive:
    `signed_set` reads only the policy's facts and the members, never the trusted relation.
  - **Mapping CPython AST to tree-sitter.**
    - `ast.Name` vs identifier positions: see `py_name_node`. The keyword name in `f(runtime_ref=1)` and
      `def runtime_ref():` do **not** match.
    - `ast.Constant` merges implicitly concatenated strings into one value; `str` must too.
    - The literal fragments of an f-string are `Constant` nodes in CPython, so `f"runtime_ref"` matches.
    - For enum members, `A = B = 1` gives both names, `A, B = …` gives none, and `X: int` (no value) gives X.
  - **Line numbers.** POSTOP200's symbol is `path:lineno` of the node's start line. Several tokens on one line give
    duplicate rows, which collapse under the loose set-of-keys contract. POSTOP203 compares start lines, not
    offsets: a `return` on the same line as the first verification call is not a bypass.
  - **Base64.** Python's `b64decode` is permissive: it drops characters outside the alphabet. The candidate imitates
    this with `rx_replace(sig,'[^A-Za-z0-9+/=]','')`. `signed_set` should decode the same way.
  - **Plugin vs CLI paths.** The plugin builds POSTOP203's message from a pre-rendered string, which doubles the
    prefix. It also re-reads the surface entries from disk instead of using its facts (`:1209`). The finding
    keys are equal under the loose contract.
### executor_publication_repairs (hard-fail, 147 lines)

- **Computes:** one finding for each (line, token) where a line of the executor contains one of four producer-owned
  tokens, as a raw substring, comments and strings included. A line holding `_cte_publication_contract_cache`
  also holds `_publication_contract_cache`, so it gives two findings.
- **Inputs:** the text of `l2070_batching/executor.py` (`lint_executor_publication_repairs.py:126`).
- **Needs:**
  - `EXISTS .scope include, contains`, plus `LIB source_line` (see rust_sql_grammar_boundary):
    `R(x, t) :- source_line(_, _, x), token(t), text(x, s), contains(t, s)` (`:78-86`). The file is Python, so
    `modnode` is the root.
- **Hard parts:** the `splitlines()` line numbering (see rust_sql_grammar_boundary).

### serverplan_name_inference (hard-fail, 177 lines)

- **Computes:** a per-line regex check, skipping lines whose first non-blank character is `#`. Four codes:
  - SPNI001: a quoted control-column literal *and* a column-shape comparison on the same line;
  - SPNI002: `ref….split/endswith/…("…suffix")`;
  - SPNI003: `refs.get(f"…{…ref…}…suffix")`;
  - SPNI004: `column….<text method>(` in `sql_column_type_hints.py` only.
- **Inputs:** the text of `l2060_query_builder/`, `l2070_batching/` and `impl/common/l2070_server_plan/`, tests
  excluded (`lint_serverplan_name_inference.py:25-29, 156`).
- **Needs:**
  - `LIB source_line` plus `EXISTS match` with `(?i)` and `.*` around each pattern (`:56-73, :85-96`), `file_name`
    for SPNI004, and `!match("\\s*#.*", s)` for the comment skip.
- **Hard parts:**
  - The patterns use lazy `*?` and bounded classes, which both regex engines treat alike. `_COLUMN_SHAPE_RE` has two
    alternatives with greedy and lazy `.*`; whole-line matching does not depend on that.
  - The trigger (`column`/`ref`, case-sensitive, `:79`) is implied by every pattern except under IGNORECASE: a line
    reading `REF.split("x.raw")` matches, but a file whose only `ref` is upper-case is never scanned. Gate on
    `contains` over the module text to match.
  - Line splitting is as in rust_sql_grammar_boundary.

### versioned_table_name_manipulation (hard-fail, 423 lines)

- **Computes:** construction of physical versioned-table names outside the two owner modules, under four codes:
  - VTN001: `+`, `%`, `.format`, `.join` or an f-string that combines a versioned suffix (`_sparse`, `_tip`,
    `_staged`, `_log`, `_field_deltas`);
  - VTN002: `bounded_identifier_with_suffix(…, suffix)`;
  - VTN003: `.removesuffix`/`.replace` with a suffix, or `x[:-4|-7|-13]` inside `derive_versioned_table_names(…)`;
  - VTN004: any `VersionedTableNames(…)` call.

  A "suffix value" is a literal suffix, or a name *currently* bound to one. It can also be an if-expression of
  either, or a call of a function *currently* known to return one.
- **Inputs:** Python AST of the whole repository (`broad_repo_scope`, tests excluded,
  `lint_versioned_table_name_manipulation.py:380`), minus paths containing `/docs/`, `/impl/doltdb/`,
  `/impl/mongodb/` or `/impl/neo4j/` (`:139-145`), and the owners `l1000_schema/table_utils.py` and
  `l7000_versioned_schema/versioned_table_info.py` (`:56-59, :133-137`).
- **Needs:**
  - `LANG positional recursion` (traversal-ordered state). `_string_constants` and `_suffix_producers` are
    file-global maps, updated as the visitor reaches each event:
    - `x = v` sets or clears x, depending on whether v is a suffix value (`:188-198`);
    - a `for` target clears it (`:208-212`);
    - each def clears its parameter names *file-wide* and, if a direct return yields a suffix value, adds the def
      to the producers (`:214-231`);
    - `y = f`, with f a producer, makes y a producer.

    Each test (`_suffix_value`, `:155-163`) reads the state *as of that node*. A name's state at n is the value
    of the last event on that name before n, and that value is itself a suffix-value test at the earlier event.
    So `state(n, x) :- ev(e, x, v), before(e, n), !later_ev(e, n, x), suffix_value_at(e, v)` recurses through
    negation and `max`. Soufflé rejects that as unstratifiable, even though it is well-founded: every step goes
    strictly back in event order. The construct needed is either recursion that is stratified by a declared order
    (event position), or a fold over a document's nodes in order:
    ```
    .decl st(n: node, x: symbol, v: symbol) ordered by pre(n)    // may read st only at strictly earlier positions
    st(n, x, v) :- assign_ev(n, x, e), suffix_at(e, v).          // suffix_at reads st(m, …) with pre(m) < pre(n)
    ```
    Rules plus aggregates cannot say it. A flow-insensitive "some assignment of x has a suffix" gives other
    findings: a later rebinding to a non-suffix still counts, and a use before the assignment counts.
  - `NATIVE py_order(n, pre, depth)`: the CPython walk keys (see the summary table). Start offsets give the right
    order except for the field-order cases in the header. Those matter here: a def's parameter clearing and
    producer registration run when the def is entered, *before* its body's assignments are visited (`:233-241`).
    So `def f(): x = "_tip"; return x` is not a producer unless x already held `_tip`. A decorator is visited
    *after* the body. Datalog could only rebuild the order by comparing the fields under every lowest common
    ancestor, which needs the ancestor pairs that core's tree relations exist to avoid. Start offsets are a close
    approximation.
  - `NATIVE py_str_const(n, v)` (summary): f-string literal pieces are `ast.Constant` nodes, compared exactly
    against the suffixes, for the JoinedStr exception (`:298-305`) and in `_contains_versioned_suffix_value`
    (`:173-174`). Escape decoding and CPython's merging of adjacent pieces cannot be done in Datalog.
  - `LIB expr_name`: `context.call_name` is the *dotted* name (`lint_lib/ast_collectors.py:100-103, 138-146`).
    So `x.VersionedTableNames(…)` is `"x.VersionedTableNames"`, and VTN002/VTN004 and the
    `derive_versioned_table_names` ancestor test match only bare calls (`:290, :313, :332`). `.removesuffix`, `.format`
    and `.join` test the attribute leaf (`:317-331`).
  - `EXISTS within, field, value`: the slice bound is `-4|-7|-13` (`UnaryOp(USub, Constant)`, so `- 4` counts),
    tested against any `derive_versioned_table_names` call ancestor at any depth, crossing defs (`:278-293`). For
    the comprehension case, a `Name` whose comprehension ancestor binds that name and iterates a literal tuple or
    list holding a suffix value (`:260-271`).
  - `EXISTS path_part, match, doc`: the owner and docs/backends exclusions.
- **Hard parts:**
  - The JoinedStr exception: literal suffixes only in {`_log`, `_field_deltas`} with exactly one placeholder are
    allowed (`:304-305`). `f"{a}_log"` passes. `f"{a}{b}_log"` and `f"{a}_tip"` fail.
  - `_contains_versioned_suffix_fragment` (substring) is used only for `%` and `.format` receivers, which are
    literals or suffix-bound names (`:256-258`).
  - Every BinOp node is tested, so `a + "_tip" + b` reports the inner `+` only. Both sides of the inner one are
    tested, and the outer one's left is a BinOp, which is not a suffix value.

### history_serverplan_projection (hard-fail, 380 lines)

- **Computes:**
  - HSP-001: production code imports `impl.common.l2070_server_plan.steps.history_runtime_projection`
    (`from` with that exact module, or `import …history_runtime_projection`).
  - In `steps/*history*.py` files:
    - HSP-002: a `return` whose value is a str literal, a set/tuple/list display, or `frozenset(<args>)`, and which
      holds string constants `"history"` and `"runtime"` anywhere below;
    - HSP-003: such a return holding `"__history_runtime__"`;
    - HSP-004: `CompiledStep(…, post_op_with_refs=…)`.
- **Inputs:** Python AST of `impl/` and `l7200_versioned_transactions/` (`lint_history_serverplan_projection.py:64,
  353-355`). Production means not a `tests` part and not a `test_*` file name (`:94-102`). History files are
  under `impl/common/l2070_server_plan/steps/` with `history` in the file name (`:105-108`).
- **Needs:**
  - `EXISTS import_alias / module_text, path_part, file_name, match`: HSP-001 by exact module text. A relative
    `from .history_runtime_projection import x` has `node.module == "history_runtime_projection"` and is *not*
    reported (`:134`).
  - `EXISTS kind, within, call_arg, leaf_name`: the tracked return and HSP-004 (`:201, :233-238`).
  - `NATIVE py_str_const(n, v)`: the tags are every `ast.Constant` str visited below the return (`:181-192`): plain
    strings, implicit concatenations as *one* value (`"his" "tory"` is `history`), and f-string literal pieces. Bytes
    do not count.
  - `EXISTS` two existentials per return, for HSP-002, and one for HSP-003 (`:211-230`).
- **Hard parts:**
  - The return stack tags the innermost *tracked* return, but returns cannot nest, so "below the return" is exact.
  - The plugin's paths are absolute, so `is_relative_to(ROOT/…)` holds (`lint_lib/fact_engine.py:1037`).

### revision_ancestry_invariants (hard-fail, 201 lines)

- **Computes:** over whole-file text, three codes:
  - `forbidden_descriptor`: the first `^class\s+<Name>\b` for each of 7 removed class names;
  - `removed_surface`: the first occurrence of `stmt_ancestry_chain`;
  - `stored_revision_text_cast`: every non-overlapping match of two case-insensitive text-cast regexes over a
    revision column, which may span lines (`[\s\S]{0,80}`).

  Each is reported at the line of the match start.
- **Inputs:** the text of `impl/common/l2100_sql_grammar/domain/versioning/` and
  `impl/common/l2070_server_plan/steps/`, test directories excluded (`lint_revision_ancestry_invariants.py:46-49,
  180`). Only the CLI also drops `test_*.py` (`:90`).
- **Needs:**
  - `EXISTS match_all, line, min : { … }` over the module text: `min l : { match_all("(?m)^class\\s+X\\b", t) … line }`
    for the first class match, the same for the substring, and one finding per `match_all` span for the casts
    (`:112-139`). Whole-file `match_all` is the right shape here, because the patterns cross lines.
- **Hard parts:**
  - The source trigger is case-sensitive `revision`/`ancestry`/`stmt_ancestry_chain` (`:80`). A file that has only
    `class SqlNativeRevisionValue` and no lowercase `revision` or `ancestry` is never scanned, so gate on
    `contains` over the text.
  - `finditer` and `match_all` must agree on leftmost-first matching with the lazy bounded `[\s\S]{0,160}?`.
  - IGNORECASE uses Unicode case folding in both engines.

### serverplan_state_tripwires (hard-fail, 4216 lines)

- **Computes:** 33 codes (SPT-001 … SPT-034, no 007) over production Python files. There are two layers:
  1. A per-node visitor (`scan_tree`, :2362–3222) reports forbidden names, calls, attributes and strings inside
     lexical "scopes" (pure/planning, preprocess, compiled-result callback, step/intent class, backend-provider
     helper, compilation-context helper).
  2. A per-file interprocedural pass (:3224–3621) builds name-keyed facts per top-level function or method. It
     runs a flow-insensitive name taint (authority / provider / runtime_value), then reports "anchor reaches a sink
     through file-local calls" (SPT-002/003/005/010/011/012/019/020/021/022/024/026/028) and direct native Python
     work (SPT-027).
  It also has three gate codes (SPT-GENERATED-RETURN-KINDS, SPT-ALLOWLIST, SPT-STALE-ALLOWLIST). Each finding has a
  qualname, the dotted class/def stack plus the call, attribute or `if` name, and a ratchet key built from it.
- **Inputs:**
  - Python AST of `data_layer_production_dirs` (`api`/`impl`/`data_layer`/`_internal` + `l\d{4}_*`). Path parts
    `tests` or `test_*` are excluded (:580).
  - Whole-file source text, for token prefilters (:130, :155, :2376, :3731–3736).
  - `scripts/serverplan_return_kinds.json` (generated; :3739). It is freshness-checked against the tree (:4039).
  - `l2070_batching/production_ir_allowlist.py`: the signed `ALLOWED_PRODUCTION_IR_TYPES` literal, imported as
    Python (:56–61, :95, :4049).
  - The inline signed frozensets in the script itself (:97–120, :530–560).
  - The ratchet baseline `scripts/allowlists/lint_serverplan_state_tripwires.txt` (:94, :469; empty at 2b1504ad).
- **Needs:**
  - `EXISTS` `.scope dirs`/`.scope tests false` for the file set. `cname` (as in plan-scope-side-effects) for
    `_call_name` (:595): a Name's id or an Attribute's attr. Name-set facts for the ~25 frozensets.
    `def_name`, `decorator`, `nearest`, `within`, `call_arg`, `text`, `line`, `contains`, `match`, `cat`.
  - `EXISTS` Source-token prefilters: `modnode(d, m), text(m, t), contains("CompiledStep", t)`. The visitor gate
    (:3731), the function-facts gate (:3225, :3735) and the step-marker gate (:2376) are each a disjunction of
    `contains` over the module text. They gate whole files: a forbidden def in a file that has none of the tokens
    gives no finding.
  - `EXISTS` Module string constants (:762) via `static_value`, for `getattr(ctx, NAME)` and builder-name tokens.
    Caveat: the oracle follows only module-level `NAME = "str"` assignments, last one wins. core's `static_value`
    says "file-local constant bindings", which may be broader.
  - `EXISTS` Annotation name sets (:879): every `identifier` within the annotation node. String annotations are
    not parsed, so do not use names.rr `annotation`, which adds `parsed` strings. Union detection (:831) is
    `binary_operator` with op `|` or a subscript of `Union`/`Optional` anywhere below.
  - `EXISTS` The visitor codes are single-node predicates once the scope relations below exist:
    - SPT-001 :2659/:2758; SPT-002 :2673/:2868/:2881; SPT-004 :3211; SPT-005 :3014/:3034.
    - SPT-006 :2434/:3053/:3072/:3086/:3099; SPT-008 :3117; SPT-009 :3136; SPT-010 :3000; SPT-011 :2919.
    - SPT-012 :2771/:3173; SPT-013 :2578; SPT-014 :2592; SPT-015 :2513; SPT-016 :2530; SPT-019 :2947.
    - SPT-020 :2789/:3193; SPT-023 :2895; SPT-025 :3155; SPT-029 :2719; SPT-030 :2807; SPT-031 :2820.
    - SPT-032 :2460; SPT-033 :2842; SPT-034 :2855.
  - `LIB` `local_qualname(n, q)`: the oracle's `stack` (:608), the class and def names from the module down,
    without the module prefix. Calls are `@dot(stack, callname)`, `if` statements `@dot(stack, "if")`. Body:
    like names.rr `qualname`, with `""` at the module. Needed because every finding's qualname and ratchet key
    use it (:461–466). names.rr `qualname` carries the module and `scope_name` keeps only one class.
  - `LIB` `def_extent(n, f)`: n is inside def f, decorators and the parameter list included. The visitor pushes the
    def before visiting decorators, defaults and annotations (:2669–2697, `generic_visit` after push). In
    tree-sitter, decorators sit in `decorated_definition`, outside `function_definition`, so `within` misses
    them. Body: `within(n, f) ; (decorator(f, e), (n = e ; within(n, e)))`.
  - `LIB` `pure_def(f)`: name ends `_lazy`, or a `@pure_lazy` decorator (last segment, call stripped, :752), or a
    name in {lower, compile, preprocess, pre_op, post_op}, or `pending_pipeline_effects`/`apply_*_effect`
    (:1144, :621, :2486). Also `in_pure(n) :- def_extent(n, f), pure_def(f)`: any enclosing def, not only the
    innermost (`any(purity_stack)`). Also `in_preprocess(n)`: any enclosing class or def named `preprocess`
    (:2745).
  - `LIB` `callback_def(g, k)`: nested def g is passed by bare name as keyword k to `CompiledStep(…)` in its
    immediately enclosing def. The first such keyword wins (:2411–2425, setdefault). `in_result_callback(n)`
    holds when some enclosing def has k ∈ {post_op, post_op_with_refs} (:2746). Body: `nearest(g, f, fdef)`,
    `def_extent(c, f)`, `cname(c, "CompiledStep")`, `call_arg(c, -1, k, v)`, `text(v, x)`, `def_name(g, x)`.
  - `LIB` `chain(e, i, x)` / `chain_tail2(c, a, b)` / `chain_has(c, x)`: `_call_chain` (:625) flattens
    Name/Attribute/Call(func) into a tuple. A chain whose base is anything else restarts at the attribute, so
    `xs[0].execute` gives `("execute",)`. The tests are `chain[-2:]` (query.execute, conn.fetchone, np.array, …)
    and "`graph_query_builder_lazy` anywhere in the chain". Recursive over `field(…, "object"/"function", …)`.
  - `LIB` `sig_param(f, p, x, ann, dflt)`: the oracle's `method_args`, posonly + args + kwonly, *without*
    `*args`/`**kwargs` (:2485). Each has its annotation node and default node. names.rr `param` includes splats
    and has no annotation or default. `site_annotation` misses `typed_default_parameter`. SPT-016 needs defaults
    paired as Python does (:1115).
  - `LIB` `step_class(c)` / `intent_class(c)` / `direct_method(f, c)`: a positional base's text contains
    `ServerPlanStep`/`IntentStep` (:804–818, unparse substring; keyword bases excluded). A direct method is
    `nearest(f, c, scope)` with c a class and no def in between (`not function_stack`, :2483).
  - `LIB` `tombstone(f)` (:1128): drop every `Expr(Constant)` statement, not only the docstring. The last
    remaining statement is `raise NotImplementedError(…)` (a call, not a bare name), and every earlier one is
    an assign, annassign, `del` or `pass`. Negation over the block's children.
  - `LIB` `intent_contract_bad(T)` (:998–1027): the oracle's recursive "allowed intent contract" over same-file
    dataclasses. A class name is looked up in a by-bare-name map that includes nested classes, last one winning
    (:1412). `seen` returns True on a cycle, so "allowed" is a greatest fixpoint with a universal over fields.
    Stratified form: least-fixpoint `bad(T)` (a field whose annotation is not ServerRef/`type[…]`/structural and
    whose local type is missing, not a dataclass, has no fields, or is `bad`), then `allowed = !bad`. No LANG
    needed.
  - `LIB` `shape_attr(cq, a)` / `shape_local(f, x)` for SPT-018 (:2556–2654):
    - `self.a = …` in an intent `__init__` whose value names a non-contract parameter.
    - Then a flow-insensitive fixpoint over Name-target assignments in `lower`.
    - Then every if/while test, IfExp test, `not …` and BoolOp in `lower` that mentions one. One finding per line.
  - `LIB` `fact_fn(f, q)`: the functions that get interprocedural facts. These are module-level defs and methods
    of classes at any nesting depth, never defs nested in defs (:1397). q is the class stack plus the name.
    `fact_body(n, f)` is `within` at any depth: the walk includes nested defs and lambdas (:1925).
  - `LIB` `selected(f)`: an anchor, or a same-file function whose *bare name* equals the last segment of a call
    made anywhere under a selected function (:1828–1898). Anchors: pure, materializer, compilation-context
    helper, provider helper, native work. A name-keyed closure: `selected(g) :- selected(f), fact_body(c, f),
    cname(c, x), def_name(g, x), fact_fn(g, _)`.
  - `LIB` `fcall(f, c, g)`: the oracle's file-local call edges (:2147–2163, :1322).
    - `name(…)` resolves to a module-level function by name, `self.m(…)`/`cls.m(…)` to a method of the same
      class qualname.
    - `CompiledStep(post_op=|post_op_with_refs=|on_commit=|on_rollback= h)` with h a Name or `partial(h, …)`
      resolves to a nested def or a module function. A nested def maps to `class_stack.h`, which has no facts, so
      that edge dead-ends (:1932).
    - calls.rr `calls`/`call_target` resolve by value flow and across imports, so they over-approximate this.
  - `LIB` `ret_kind(name, k)`, keyed by *bare function name* across the project (:1545, :1594, :3756):
    - "authority"/"provider" when the return annotation names AUTHORITY/PROVIDER types.
    - Per file this tests identifiers (:879). Project-wide (the JSON) it is a *substring* test on the unparsed
      annotation text (:3756–3765, :3812).
    - "runtime_authority" is a per-file fixpoint: functions that read `carrier.txn`-like attributes or call a
      runtime_authority function with a carrier argument (:1560–1621).
    - A call `foo()` anywhere then takes the kinds of every def named `foo` in the project, methods included.
  - `LIB` `taint(f, x, k)` and `expr_kind(f, e, k)` (:1650–1750, :1941–2004): parameter seeds come from
    annotation names (`ReadTransaction`…, `ResolvedRefs`, `Hydrator`). Name seeds: `txn`, `conn`, `resolved`,
    `runtime_refs`, `hydrator`. The 2nd and 3rd parameters of nested `post_op`/`post_op_with_refs` callbacks are
    seeds too (:1229–1296). The expression kind recurses through call receivers, attribute objects and subscript
    values. It is a flow-insensitive fixpoint over Name targets of every assignment in `fact_body`. Text-keyed
    names, not binding sites, so it must not use `ref_site`. A second set, `semantic_runtime_value`, is seeded the
    same way plus any expression that *mentions* a runtime name (:1727).
  - `LIB` `direct_sink(f, kind, n)` for 13 sink kinds (:2034–2232): eager_sql, native_python_work,
    runtime_callback, large_column, resolved_refs, nested_execute, ctx_access, materialized_pipeline,
    runtime_authority, runtime_value, runtime_semantic_projection (:1765), runtime_sequence_projection and
    provider_authority (a provider receiver with an authority argument). Each is a composition of the relations
    above.
  - `LIB` `reaches_sink(f, kind, site, g)`: f's call site `site` leads through `fcall*` to g with
    `direct_sink(g, kind, _)`. The finding line is the *call site in the anchor* and the qualname is the anchor's.
    The `g != f` filter applies to SPT-002/003/005/010/011/012/019/020 but not SPT-021/022/026. SPT-024 filters on
    g's leaf name not being post-process (:616, :3505). This is recursive Datalog. See Hard parts for where the
    oracle's DFS differs.
  - `LIB` `semantic_builder(b)` / `semantic_helper(x)` for SPT-028's project branch (:3647–3672, :3679–3713,
    :2289):
    - Builder names compared against `spec.builder_name` in any function that has a direct
      runtime_sequence_projection. This runs project-wide over files whose text mentions `runtime_builder`/
      `RuntimeBuilder`, and over *all* functions there, not only the selected ones.
    - Helpers are bare names of functions that build such a builder (`runtime_builder_post_op(_, NAME)`,
      `RuntimeBuilderSpec(builder_name=…)`).
    - The anchor's sink is a local builder site or a call to a helper by bare name, reported at the smallest line
      (:2311–2320).
  - `LANG` First-witness selection. A finding's line is the *first* sink in the oracle's `ast.walk` order
    (`setdefault`, :2034–2232). `ast.walk` is breadth-first, so the order is lexicographic (Python-AST depth,
    source position), not the minimum line. The anchor's call site is the first entry of `calls` (callback
    targets first, then BFS order) whose callee reaches the sink (:2278). Rules plus `min` give one number, so
    this needs an argmin over a composite key that returns the row:
    ```
    SPT_003(site, q) :- anchor(f, q),
        site = argmin (depth(s), start(s)) : { reaches_sink(f, "eager_sql", s, g), g != f }.
    ```
    The alternative is encoding `depth * 2^32 + start` in a 64-bit `min` and joining back. That needs a
    `LIB depth(n, k)`, which is tree-sitter depth, not Python-AST depth, so it is approximate.
  - `NATIVE` `json_file(path, key_path, value)`: a generic reader for a repository JSON file, here
    `scripts/serverplan_return_kinds.json` (:3742). core reads only tsconfig and package.json. Freshness is then
    `EXISTS`: set inequality between the file's (name, kind) rows and the computed `ret_kind` (:4039–4048).
  - `NATIVE` `signed_py_literal(module_path, const_name, entry)`: the production IR allowlist is a frozenset
    literal in a repository Python module, verified against a signature embedded in the same module (:56–61,
    :95, :4049). `signed_list` reads a list file. Reusing it means either changing where the entries live, or a
    reader that extracts a constant's literal and checks the embedded signature. Datalog cannot verify a
    signature.
  - `POLICY` Gate findings that replace all others. A stale return-kinds file, a failed production-IR signature
    or a failed Python-work signature each yields exactly one finding (at the JSON file or the script, line 1)
    and suppresses every other finding (:4039–4073, CLI :4153–4170). `signed_list`'s "refused" needs to say it
    produces such a finding, not an engine error.
  - `POLICY` Finding-key suppression by signed list. Findings whose `code::qualname::relpath` is in the signed
    sets are dropped, for every code (:555, :3728, :4076). Per-rule `!signed_list(…, cat(…))` can say it but
    must be repeated on 33 codes. A policy-level `suppress by signed_list(key template)` would match the oracle.
  - `POLICY` Counted ratchet baseline keyed by `code::qualname::relpath` with multiplicity (:469–514, :4077–4103):
    - Only the excess over each key's count is reported, and of a key's findings the *last* ones in
      (file, line, code) order are taken.
    - Each key whose count drops yields SPT-STALE-ALLOWLIST.
    - `--strict` ignores the baseline.
    Dedup is on (code, line, qualname, message) (:3624), so messages, which carry `ast.unparse` text (:3047),
    take part in multiplicity.
  - `POLICY` Extraction failure. In the plugin path a collector that raises is stored as a failed fact
    (fact_engine.py:905–907) and the reporter reads `bundle.facts.get(…, [])`, so the file contributes no
    findings silently (:4075). An `.rr` policy should choose: report the file, or match this.
- **Hard parts:**
  - **Two entry points differ.** The suite plugin (:4032) passes empty project builder sets (:4013), so SPT-028's
    project branch never fires. It uses the *generated JSON* as project return kinds. The CLI
    (`_collect_current_violations_from_source_facts`, :3839) computes both live and reads files as `ROOT/path`,
    which is absolute (:3831). The raw `path.as_posix()` tests (:1859, :2498, :3064, :3150) then never match on
    the CLI, so SPT-006 executor dispatch, SPT-019 and SPT-025 depend on which path runs. The plugin path is the
    hard-fail one.
  - **Name-keyed, not resolved.** Return kinds, the reachable-function closure and taint are all keyed by bare
    name text. Names.rr's binding-site resolution (`ref_site`, `sym_of`, `call_target`) is more precise and gives
    different keys: an import-aliased function, a same-named method in another class, a shadowed local. Parity
    needs text-keyed library relations, kept apart from the resolving ones.
  - **Nesting rules differ by layer.** Visitor scope flags look at *every* enclosing def (`any(stack)`). The
    step-method test needs *no* enclosing def. The callback kind looks only at the *immediate* outer def. The
    step-execution marker map covers only top-level functions and methods, so a nested def never counts
    (:2747, :1623). Function facts exist only for top-level functions and methods, but their bodies include
    nested defs.
  - **The DFS is not reachability.** `_transitive_sink` memoises `None` for a node first reached while a cycle is
    open (:2265–2285). With A→B→A and A→sink, B is cached as not reaching even though B→A→sink. Results then
    depend on the iteration order of functions. Datalog's closure reports more; matching the oracle exactly
    would need the DFS order.
  - **Order dependence in SPT-018.** `shape_attrs` is filled when `__init__` is visited, so a `lower` defined above
    `__init__` sees none (:2563 vs :2603). The fix needs `precedes` between the two defs, and decorated defs are
    wrapped in `decorated_definition`.
  - **SPT-029.** The fallback search covers the `orelse` and every *top-level* statement of the innermost
    function after the `if`'s line (:2708–2715), not statements nested in blocks. An `if` outside any function
    checks only its `orelse`.
  - **Text heuristics.** Several tests use `ast.unparse` text: base names split at `[` and `.` (:2430);
    `"resolved" in unparse(receiver)` (:801); annotation substrings in the project map. Source text equals
    unparse only up to whitespace, quotes, parentheses and comments, so text-based ports can differ on odd
    formatting.

## Summary table

Needs that are the same thing under different names are merged into one row. Checks are counted by name.

| tag | signature or construct | checks that need it |
|---|---|---|
| LIB | `expr_name(e, s)` / `leaf_name(e, s)`: the unresolved dotted text of a Name/Attribute chain, and its last segment (oracle `_name`, `_call_name`, `_dotted_name`, `_node_name`, `cname`, `_call_chain`). A variant looks through a subscript or call to its value or function. | grammar_callable_constructors, generated_index_sources, serverplan_bypass, mandatory_imperative_tripwire, execution_authority_bypass, serverplan_steps, post_op_side_effects, versioned_table_name_manipulation, history_serverplan_projection, serverplan_state_tripwires (10) |
| LIB | `sig_param(f, p, x, kind, ann, dflt)`: parameters with their kind (posonly / arg / kwonly / vararg / kwarg), annotation node and default node. names.rr `param` includes splats and keeps none of this | grammar_callable_constructors, grammar_str_params, generated_index_sources, mandatory_imperative_tripwire, serverplan_steps, serverplan_state_tripwires (6) |
| POLICY | Outcome when a required input is missing, unparsable or unreadable as a literal (oracle crash, `PARSE` finding, or silent skip): a declared finding or a blocking engine error, per policy | mandatory_imperative_tripwire, execution_authority_bypass, post_op_side_effects, serverplan_steps, serverplan_state_tripwires, grammar_callable_constructors (6) |
| NATIVE | `py_str_const(n, v)`: the string `ast.Constant`s CPython would build. Implicit concatenation is one value, and f-string literal pieces are values (escapes decoded, adjacent pieces merged). It extends `value`/`str` | mandatory_imperative_tripwire, post_op_side_effects, serverplan_steps, versioned_table_name_manipulation, history_serverplan_projection (5) |
| LIB | `def_extent(n, f)`: n is inside def/class f, its decorators, bases, defaults and annotations included. Sketch: `within(n,f) ; (decorator(f,e), (n=e ; within(n,e)))` | grammar_callable_constructors, generated_index_sources, serverplan_bypass, execution_authority_bypass, serverplan_state_tripwires (5) |
| NATIVE | `py_order(n, pre, depth)`: the CPython walk keys. `pre` is the pre-order index in `iter_child_nodes` field order; `depth` is AST depth, for BFS `ast.walk`. Start offsets approximate them | grammar_callable_constructors, serverplan_steps, versioned_table_name_manipulation, serverplan_state_tripwires (4) |
| LIB | `py_name_node(n, x)`: identifier n where CPython has an `ast.Name` (or `ast.arg`). It is `nameref` minus import names, `global`/`nonlocal`, `except … as` and match captures, plus parameters | mandatory_imperative_tripwire, execution_authority_bypass, post_op_side_effects (3) |
| LIB | `local_qualname(n, q)`: the class/def stack without the module prefix. Variants: `<locals>` before each def nested in a def (execution_authority_bypass), and independent innermost class + innermost def (`Class.func`, serverplan_bypass) | serverplan_bypass, execution_authority_bypass, serverplan_state_tripwires (3) |
| LIB | `source_line(d, l, s)`: one span per physical line of a document, from `match_all("(?m)^(?<x>.*)$", root)` and `line` | rust_sql_grammar_boundary, executor_publication_repairs, serverplan_name_inference (3) |
| LIB | `base_name(c, s)`: the textual name of each positional base (identifier, attribute leaf, through a subscript), unresolved, unlike `class_base` | grammar_str_params, serverplan_steps, serverplan_state_tripwires (3) |
| POLICY | Signature failure is one gate finding (`invalid_allowlist_signature` and the like) that replaces every other finding. core's `signed_list` says only "refused" | serverplan_bypass, serverplan_steps, serverplan_state_tripwires (3) |
| NATIVE | `signed_set(linter, set, sig)`: Ed25519 verification of a canonical JSON over all rows of a *derived* relation, with the signature as a value. A failure is a fact rules can use. It also covers allowlists that are literals in Python source (`signed_py_literal`): read the literal statically, then verify | post_op_side_effects, serverplan_state_tripwires (2) |
| NATIVE | `unparse(n, s)`: CPython `ast.unparse` text. The predicates compare or split it, which source text and structure do not reproduce | grammar_str_params, mandatory_imperative_tripwire (2; serverplan_state_tripwires approximates by text) |
| LIB | `last_asg(f, x, v)`: the last `x = v` under f by walk order, flow- and scope-insensitive (BFS order in grammar_callable_constructors) | serverplan_steps, grammar_callable_constructors (2) |
| NATIVE | Text documents for files outside the policy language: `doc` plus one root node whose `text` is the file (or a `.language rust` grammar) | rust_sql_grammar_boundary (1) |
| LANG | Recursion stratified by a declared order (`ordered by pre(n)`), or an in-order fold over a document's nodes: traversal-ordered state whose value at a node depends on the state at earlier nodes | versioned_table_name_manipulation (1) |
| LANG | Argmin with a witness over a composite key (`argmin (depth, start) : { … }`). `min` plus `py_order` encoding is the fallback | serverplan_state_tripwires (1) |
| LIB | `py_if_guard(r, t)`: the test of the CPython parent `If` of a statement (an `elif` is a nested If, and `else` belongs to the last if/elif of the chain) | mandatory_imperative_tripwire (1) |
| LIB | `fn_symbol(n, x)`: the oracle's own sync-def scope stack lookup (a direct nested def wins; a local binding, walked through nested defs, loses) | grammar_callable_constructors (1) |
| LIB | `class_body_name(c, x)`: a bare-name target of an assign/annassign directly in a class body, chained targets included | post_op_side_effects (1) |
| LIB | `arg_or_kw(c, i, k, v)`, `empty_refs(v)`, `ann_mentions(a, x)`, and the `post_op_fate(sym, fate)` facts shared with post_op_side_effects | serverplan_steps (1) |
| LIB | Interprocedural, name-keyed relations used by one check: `pure_def`/`in_pure`/`in_preprocess`, `callback_def`/`in_result_callback`, `chain`/`chain_tail2`/`chain_has`, `step_class`/`intent_class`/`direct_method`, `tombstone`, `intent_contract_bad`, `shape_attr`/`shape_local`, `fact_fn`/`fact_body`, `selected`, `fcall`, `ret_kind`, `taint`/`expr_kind`, `direct_sink`, `reaches_sink`, `semantic_builder`/`semantic_helper` | serverplan_state_tripwires (1) |
| NATIVE | `json_file(path, key_path, value)`: a generic reader for a repository JSON file | serverplan_state_tripwires (1) |
| POLICY | A counted ratchet keyed by a finding-key string with multiplicity. It reports only the excess, the last ones in order, and gives a stale-entry finding when a count drops | serverplan_state_tripwires (1) |
| POLICY | Suppression at the policy level by a signed list of finding keys (`code::qualname::path`), for every code. grammar_callable_constructors and serverplan_bypass can say theirs per rule with `!signed_list` | serverplan_state_tripwires (1) |
| POLICY | A report-only legacy mode with a count limit. A separate policy with `.budget * {"total": N}` and `.output` probably covers it | serverplan_steps (1) |

Not needs: whole-file source-trigger gates (`contains` on the module text), module-anchored findings (`modnode`),
reading a literal registry statically (`call_arg`/`str`), per-line deduplication (`min`), and zero
baselines (`.budget * by code`) are all expressible today.
