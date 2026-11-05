# Group 1: what .rr lacks for 16 data-layer checks

Oracle: data-layer scripts at revision 2b1504ad. Cited as `script.py:LINE`; a bare `:LINE` refers to the section's own script.

### linter_enforcement (hard-fail, 521 lines)
- **Computes:** Audits the lint suite's own configuration: every `LinterConfig` in `DL_SUITE` must be enabled and blocking in CI, unless a signed allowlist exempts it. Each must also be classified and backed by a registered plugin and an existing script, and must not ship a placeholder `*SIGNATURE`. Every `scripts/lint_*.py` must be registered, and allowlist entries must not be stale. Codes LE000–LE011. One finding per (code, linter).
- **Inputs:** Python AST used as data, read across files:
  - `scripts/lint_all.py`: the `CheckSuite(linters=[LinterConfig(...)…])` literal; the `linter_plugin_factories=[_plugin("x", …), _rootless_plugin("x", …), _plugin_group(("a","b"), …)]` list; the appended `LinterPluginFactory(names=("linter_enforcement",))`.
  - `scripts/lint_lib/architectural_rules.py`: the `LINTER_CLASSIFICATIONS` dict literal, where positional arg 2 is the classification.
  - `scripts/lint_linter_enforcement.py`: the `ALLOWLISTED_*` literals and `_ALLOWLISTS_SIGNATURE`.
  - `scripts/lint_standalone_linter_entrypoints_plugin.py`: the `STANDALONE_LINTER_COMMANDS` tuple literal.
  - Every `scripts/<config.script>`: its module-level `*SIGNATURE = "…"` lines and its allowlist values.
  - File tree: `scripts/lint_*.py`, and whether each config's script exists.
  - Signing: the Ed25519 public key in `scripts/lint_lib/signing.py:33`.
- **Needs:**
  - `EXISTS call_arg, value, text, within, field, child`: read `LinterConfig(name=…, script=…, enforcement=…, in_ci=…, enabled=…, plugin_name=…)` as rows. Keep only calls within the `linters=` list of the `CheckSuite` call assigned to `DL_SUITE`; the `LinterPlugin(name=name…)` helpers at lint_all.py:629 and :663 are not configs. A keyword left out takes its default through `!call_arg(c, -1, "enabled", _)`.
  - `EXISTS call_arg, child`: plugin names are the first argument of `_plugin`/`_rootless_plugin`, the tuple elements of `_plugin_group`, and the `names=` of `LinterPluginFactory`. The helper names are hard-coded in the rule.
  - `EXISTS field(pair, "key"/"value")`: the dict literals `LINTER_CLASSIFICATIONS` and `ALLOWLISTED_NON_BLOCKING_LINTER_REASONS`. LE010's "architectur" test is `match("(?i).*architectur.*", reason)`.
  - `EXISTS file, match`: script existence (LE008, LE010), and the LE011 glob `scripts/lint_*.py` minus `lint_all.py`, `*_plugin.py` and `*_plugins.py` (lint_linter_enforcement.py:126-131).
  - `EXISTS doc, cat`: joins a config's `script` to the document at `"scripts/" + script`, to read its module-level assignments whose target ends in `SIGNATURE` (LE003).
  - `LIB` `static_nonempty(e: node)`: expression e is a literal set, list, tuple or dict with at least one element, possibly wrapped in `frozenset(…)`, `set(…)` or `sorted(…)`, or a name bound to one. Plus `active_allowlist(d: number)`, which holds if one of these is static_nonempty:
    - the dict that `_get_allowlists_for_signing` returns, in any of its values;
    - `_load_allowlist`'s return;
    - a module-level `ALLOWLIST`, `ALLOWLISTED_CALLSITES`, `ALLOWLISTED_CATEGORY_C_TARGETS`, `ALLOWLISTED_FILES` or `ALLOWED_FILES`.

    This approximates lint_linter_enforcement.py:178-195, which *calls* those functions at run time. When the module is not in `sys.modules`, the oracle falls back to "has any SIGNATURE variable" (lint_linter_enforcement.py:269-270).
  - `NATIVE` `@ed25519_verify(pubkey_hex: symbol, message: symbol, sig_b64: symbol): number`. LE000 verifies an Ed25519 signature over the canonical JSON of three in-source lists, `disabled`, `non_blocking` ("name: reason") and `standalone_entrypoints` (lint_linter_enforcement.py:106-111, 398-414; signing.py:86-106, 159-200). `signed_list` covers only signed policy *files*, and this signature lives in a Python string constant. Cryptography cannot be derived in Datalog.
  - `LANG` ordered string aggregation, `cat : { body } order by key sep ","`, to build the canonical message that is signed: `json.dumps(sort_keys=True, separators=(",", ":"))` of sorted lists (signing.py:100-105). Rules plus `count` can test membership but cannot build one string from many rows.
    ```
    msg = cat : { nb_entry(n, r), e = cat(n, cat(": ", r)) } order by e sep "\",\"".
    sig_failed() :- sig(s), @ed25519_verify(KEY, cat(PREFIX, cat(msg, SUFFIX)), s) = 0.
    ```
  - `POLICY` (alternative to the two items above): move the three lists into `signed_list` files. The check's meaning stays the same and its inputs change. Recommended over NATIVE plus LANG unless the in-source format must be kept.
  - `EXISTS !`: LE000 gates every other code. When the signature fails, only LE000 is reported (lint_linter_enforcement.py:398-414), so every other violation carries `!sig_failed()`.
- **Hard parts:**
  - `_finalize_fact_first_suite` (lint_all.py:2221-2226) sets `plugin_name = name` on every config without one *after* the literal. In the oracle, `config.plugin_name` is therefore always truthy, so LE009 and LE010 (lint_linter_enforcement.py:229-245) apply to every enabled linter. A literal-only reading would miss this.
  - Codes LE009 and LE010 each have two unrelated meanings (lint_linter_enforcement.py:233/319 and 241/334). The second LE009 can never fire, because the allowlist set is built from the reasons dict.
  - Signature extraction is line-based, not AST-based (lint_linter_enforcement.py:140-168). It takes only lines at column 0, cuts at the first `#` (even inside a string), and allows chained `A = B = "…"` and an annotation. An AST reading agrees except on those edge cases.
  - LE003 depends on run-time module state. Which linter modules are in `sys.modules` depends on what `lint_all.py` imported. Treat it as approximate.
  - Findings are anchored at `scripts/<script>` line 1, or at `scripts/lint_all.py` line 1 for stale entries (lint_linter_enforcement.py:373). That is a module node, never the offending literal.

### standalone_linter_entrypoints (hard-fail, 161 lines)
- **Computes:** Runs each script in `STANDALONE_LINTER_COMMANDS` minus `DELEGATED_LINTER_SCRIPTS` as a subprocess, with its args and a 120 s timeout. Reports `STANDALONE_LINTER_FAILED` at `scripts/<script>` line 1 for each non-zero exit, with the child's output as the message.
- **Inputs:** The two literals in `scripts/lint_standalone_linter_entrypoints_plugin.py`, and the exit status and output of 28 other programs. No source analysis.
- **Needs:**
  - `POLICY` composite policy (a suite manifest): "this policy fails for each named member policy that has blocking findings". A finding names the member, and its message carries the member's findings (lint_standalone_linter_entrypoints_plugin.py:127-152). In Rerail each listed script is a policy of its own. This check is then a list of required policies, plus a finding when one of them is missing or fails.
    ```
    .requires "data-layer:async-def-naming", "data-layer:backend-mentions", …   // minus the delegated set
    ```
  - `POLICY` timeouts and budget overruns become findings. A member that times out (exit 124 after 120 s, lint_standalone_linter_entrypoints_plugin.py:104-114) counts as failed, not skipped.
  - `EXISTS` (not needed if the manifest is written in the policy): `child` and `value` read the tuple literal (lint_standalone_linter_entrypoints_plugin.py:21-67).
- **Hard parts:** Exit-code semantics differ per member. Six scripts run without `--check` (lines 27, 39, 42, 48), so they fail only through their own exit logic. The delegated scripts are registered but never run here. This check cannot be expressed as rules over facts at all: its meaning is "other checks pass".

### transaction_closures (hard-fail, 863 lines)
- **Computes:** for each call `….m(…)` with `m` in {write, read, versioned_write, versioned_read, write_version,
  read_version}, takes its callback (keyword `fn=`, else the second positional argument). If the callback is a lambda,
  or a name that is a direct inner `def` of the call's own function defined on an earlier line, it reports writes
  to outer state inside the callback: TXN001 `nonlocal x`, TXN002 `global x`, TXN003 `outer.append(…)` and other
  mutating methods, TXN004 `outer[k] = …`. Scope: production Python files, tests excluded.
- **Inputs:** Python AST only. Files under `impl/`, `data_layer/`, `_internal/` and `l####_*/`, with no
  `test`/`tests` path part (`lint_lib/discovery.py:28-41`, `fact_engine.py:1358`).
- **Needs:**
  - `EXISTS call_arg, kind, text, field`: the callee is an attribute with a listed name. The callback is
    `call_arg(c, -1, "fn", a)`, else `call_arg(c, 1, "", a)`.
  - `LIB callback_arg(c: node, a: node)`: keyword `fn` wins over the second positional argument.
    `callback_arg(c,a) :- txn_call(c), call_arg(c,-1,"fn",a).` and
    `callback_arg(c,a) :- txn_call(c), !call_arg(c,-1,"fn",_), call_arg(c,1,"",a).`
    (lint_transaction_closures.py:205-211). The same relation serves transaction_callback_wrappers.
  - `LIB param_kind(f: node, x: symbol, i: number, k: symbol, ann: node)`: a parameter's kind ("posonly", "pos",
    "kwonly", "vararg", "kwarg") and its annotation (`unset` when there is none). The kind comes from the child index
    relative to the `positional_separator` / `keyword_separator` / `list_splat_pattern` children, using `min : {…}`.
    The annotation is the `type` field of `typed_parameter` or `typed_default_parameter`. Today `names.param` folds
    every kind together and misses `*a: T`. `names.site_annotation` misses `typed_default_parameter`. The oracle
    uses `args.args` plus vararg and kwarg in one place and `args.args` alone in another (:184, :214-220).
    Shared with all four checks in this file.
  - `LIB outer_name(call: node, x: symbol)`: the names the oracle treats as outer for an inline lambda. These are
    the module's direct `Assign`/`AnnAssign` name targets, def/class names and import locals (`import a.b` gives
    "a"), plus, for every enclosing `function_definition` of the call, its pos/vararg/kwarg parameters and every
    `Assign`/`AnnAssign`/walrus name target anywhere in its subtree, nested functions included (:169-180, :415-440).
    These are not LEGB scopes: class bodies push nothing, and nested scopes leak upward. So `names.bound_in` and
    `look` do not fit. Body: union of `within(t, f)` target rules and `import_alias`.
  - `LIB direct_outer(f: node, x: symbol)`: for a named callback, only f's `args.args` plus direct body statements
    `x = …` / `x: T = …`, chained targets included (:183-192). Body: `stmt(st, f)` plus a recursion through
    `assignment.right` for `a = b = …`.
  - `LIB inner_def(f: node, x: symbol, g: node)`: g is a direct body statement of f (bare or decorated) named x. The
    last one wins, as in a dict: `max s : { … }` over start offsets (:195-202). The call reports only when
    `line(g) < line(call)` (:482).
  - `LIB local_before(cl: node, x: symbol, n: node)`: inside the closure being checked, x became local before n in
    the detector's pre-order walk. Sources are the closure's parameters (pos, vararg, kwarg), the parameters of
    nested defs, and `Assign`/`AnnAssign`/walrus name targets. The oracle registers the target before it visits the
    right-hand side (:282-296), so `x = x.pop()` is not reported. Body: compare start offsets from
    `node(_, a, _, sa, _)` with `sa <= sn`. Pre-order equals offset order for these node kinds.
  - `LIB closure_body(cl: node, n: node)`: the nodes the detector visits. For a lambda, its body. For a def, the
    statements of its body, nested defs' bodies included. Lambda subtrees are skipped (:259-260), and so are the
    decorators, defaults and annotations of the closure and of every nested def (:249-254 visits `node.body` only).
    Body: `within(n, body)` with `!within_stop(n, _, lambda_k)` and an exclusion for nested-def headers.
  - `EXISTS names.tgt, kind`: TXN004 stores. In the AST, a `Subscript` in Store context is
    `tgt(t), kind(t, "subscript"), field(t, "value", v), kind(v, "identifier")`. `tgt` covers `x[k] = …`,
    `x[k] += …`, annotated stores, for-targets and destructuring (:318-330).
  - `EXISTS kind`: TXN001 and TXN002 are `nonlocal_statement` / `global_statement` in closure_body, one finding per
    listed name (`child(g, i, _)`) (:262-280).
  - `POLICY`: codes are taken from the message text (`_violation_code`, :764-768). The same closure passed to two
    calls is reported twice, and the oracle keeps both rows (:404-413). Under loose keys `(path, line, code)` they
    collapse. Say in the policy whether the multiplicity is part of the key.
- **Hard parts:**
  - Two different "outer" sets. Inline lambdas use the union of every enclosing function's whole-subtree
    assignments and the module bindings. Named closures use only the immediate function's positional parameters and
    direct-body assignments. Neither is name resolution.
  - The order is flow-sensitive inside the closure but not branch-aware: a local assigned in one `if` arm before the
    mutation makes the name local for the rest of the walk.
  - Named lookup is textual and dict-based: the last `def x` in the body wins, then `def_line < call_line` is
    tested on it alone.
  - The call's owner is the AST-walk owner. `on_FunctionDef` fires before the decorators are visited (:370-371,
    :431), so a call in g's decorator counts as inside g. Tree-sitter's `decorated_definition` puts the decorator
    outside g. A shared LIB `walk_owner(c, f)` fixes this (`nearest(c, f, fdef)`, or the definition whose
    `decorated_definition` holds c in a decorator).
  - A `FunctionDef` callback branch (:461) is dead code, since Python cannot pass a def inline. A lambda can only
    trigger TXN003, with walrus-local tracking.
  - Attribute stores (`obj.attr = …`) are listed in the docstring but not checked. `del x[k]` (Del context) is not
    reported.
  - The source pre-filter `.write(`-style text (:584) is redundant with the call test, except for `x.write (…)`
    with a space.

### transaction_patterns (ratchet, 1275 lines)
- **Computes:** for every def, takes the first positional parameter (not `self`) whose annotation names a
  connection. Each such function is categorized by a priority chain:
  1. `__init__` of a connection-like class is LEGITIMATE.
  2. Else a name listed in the built-in set or the signed baseline gets that category.
  3. Else the category is inferred from name and path substrings.
  4. If the function calls `p.write/read/schema_transaction(…)` on that parameter and the category is not in an
     allowed set, it becomes INTERNAL_TXN_VIOLATION.

  The check reports NEEDS_REVIEW, TS_BRIDGE and INTERNAL_TXN_VIOLATION at the def line. Scope: production dirs,
  tests excluded.
- **Inputs:** Python AST. Repository file `.transaction-patterns-baseline.json`: JSON
  `{"_metadata": {signature,…}, "categories": {CAT: [names]}}`, at this pin BACKGROUND 13, BOOTSTRAP 23,
  LEGITIMATE 47. Its Ed25519 signature is over canonical JSON of `categories` (:211-240, :282-292). Same scan scope
  as above.
- **Needs:**
  - `LIB param_kind` (as above): only `args.args`, so no posonly, kwonly or `*a` (:601-605). Skip the name
    `self` only; `cls` is not skipped.
  - `LIB anno_head(t: node, name: symbol)`: the oracle's `_get_annotation_name` (:641-659):
    - Name gives its id.
    - String or other constant gives its whole value (`str(None)` = "None").
    - Subscript gives the head of its value: `Optional[Connection]` gives "Optional", so it is missed.
    - Attribute gives its last attribute.
    - A BinOp (any operator) gives left if its head contains "Connection", else right if that does.

    Over the tree-sitter type kinds `identifier`, `attribute`, `member_type`, `subscript`, `generic_type`,
    `binary_operator`, `union_type`, `string` and `none`, with `str` and `contains`. Then
    `conn_anno(t) :- anno_head(t, n), n != "ConnectionId", (contains("Connection", n) ; n = "conn").`
    (:661-665).
  - `LIB first_conn_param(f, x)`: the lowest index i with a conn annotation, via `min i : {…}`. The oracle stops
    after the first (`break`, :639).
  - `EXISTS nearest(f, k, "class_definition"), def_name, contains`: "innermost class" is `class_stack[-1]`, the
    nearest enclosing class through functions (:570-574, :609). The LEGITIMATE `__init__` rule tests the class name
    for the substrings Connection, Transaction, TransactionGenerator, SyncConnectionPool (:771-789).
  - `EXISTS facts`: CORE_LEGITIMATE_FUNCTIONS (:171-186) as `.decl` facts.
  - `NATIVE signed_json_list(path: symbol, group: symbol, entry: symbol)`: rows of a JSON object of string arrays
    (`categories.CAT[]`), accepted only if the repository's own Ed25519 signature verifies. The signature sits in
    `_metadata.signature`, over `json.dumps(sort_keys, separators=(",",":"))` of the sorted lists, with the
    repository public key (:154, :211-240). `signed_list` has no group column and its signature scheme is Rerail's,
    so it cannot read this file as-is. The alternative is to migrate the baseline into one `signed_list` per
    category (then `EXISTS signed_list`). Datalog cannot verify a signature.
  - `LIB infer_category(f: node, cat: symbol)`: the ordered if-chain of `_infer_category` (:425-546). It is a
    first-match cascade of substring tests on function name and path, each rule guarded by the negation of every
    earlier one. That is expressible as stratified rules: `cat_k(f) :- test_k(f), !hit_1(f), …, !hit_{k-1}(f).`
    `top_level_root` is the first path part in {api, impl, data_layer, _internal} or matching `l\d{4}_.+`
    (:443-447). `path_part` is an unordered set, so it cannot say "first".
  - `EXISTS match` captures: the first matching path part is a leftmost lazy capture on the path,
    `match("(?:[^/]*/)*?(?<r>api|impl|data_layer|_internal|l[0-9]{4}_[^/]+)(?:/.*)?", p)`. This relies on core's
    "first match" capture semantics.
  - `LIB category(f, cat)`: the precedence chain `__init__` rule, then the categorized set, then TS_BRIDGE (an empty
    pattern list, :201-203, so it never fires), then `infer_category` (:671-679). A name in several baseline
    categories takes the first in the oracle's dict order (:346-371). All baseline categories are non-failing
    except through the next rule.
  - `LIB internal_txn(f, p)`: inside f's own body (innermost def, so calls in nested defs belong to them; lambdas
    belong to f), a call `p.m(…)` where receiver identifier `p` has the parameter's text and
    m ∈ {write, read, schema_transaction} (:588-597). Textual, not `ref_site`. Then
    `INTERNAL_TXN_VIOLATION(f) :- internal_txn(f, p), first_conn_param(f, p), category(f, c), !allowed_txn_cat(c).`
    (:619-628).
  - `POLICY`: "ratchet" here behaves as block. `StructuredLinterResult.passed` fails on any violation for both
    hard-fail and ratchet (`linter_plugin.py:184`). The ratchet lives in the signed baseline of exempt names, not in
    a count. The finding code is the category name (:964-969). The plugin loads the baseline without checking the
    signature; only `--ci` verifies it (:1137-1157, :869-874). Rerail's `signed_list` would make the plugin path
    stricter than the oracle.
- **Hard parts:**
  - `_get_annotation_name` is a head-name heuristic, not a type test. `Optional[SyncDatabaseConnection]` and
    `list[Connection]` are not connections. `Connection | None` is (BinOp left). `"Foo | Connection"` as a string is
    (whole-string substring). `ConnectionId | None` is excluded (left "ConnectionId" is returned, then rejected).
  - Tree-sitter parses annotations with the type grammar (`generic_type`, `union_type`, `member_type`), not
    expression kinds, so the LIB must map both shapes.
  - `_infer_category` tests substrings of `str(file_path)` as the framework passes it (`fact_engine.py:935-937`; it
    looks absolute). `"/tools/"`, `"validation"`, `"bulk"` and backend names can then match directories above the
    repo root. Parity needs the same path form, or the rules should be written on the repo-relative path with a
    documented difference.
  - Only the first connection parameter is categorized, and internal calls are counted on that parameter only.
  - Decorator calls belong to the decorated def (AST walk order); see `walk_owner` above.

### explicit_semantic_ids (hard-fail, 304 lines)
- **Computes:** For each registered static model whose class name does not start with `_`, runs `validate_static_naming_contract(require_explicit_semantic_ids=True)`. Reports `missing_explicit_semantic_id` at the model's file, line 1, if any of these holds:
  - a naming declaration is present without `participates_in_static_naming_metadata=True`;
  - two of the model's (model, field, relationship) entries share an effective semantic id;
  - two fields or relationships share an effective API name;
  - the model, a non-exempt field or a relationship has no explicit `semantic_id`.
- **Inputs:** At run time this is the Python model registry: an import of `l1000_schema.table_registry`, pydantic `model_fields`, and `dir()` descriptors. Statically, it is the Python AST of every module with `@register_table(...)`:
  - `register_table(...)` keywords;
  - class-body annotated fields with `Column(semantic_id=…, api_name=…, api_aliases=…)`;
  - relationship descriptors `BelongsTo`/`HasMany`/`Through`/`BelongsToThrough` (from `l2100_include.belongs_to`, `l2080_lazy_orm.has_many`, `l2080_lazy_orm.through`, `l2100_include.belongs_to_through`) and their `semantic_id`/`api_name` keywords;
  - `SEMANTIC_ID_EXEMPT_FIELDS` (`l1000_schema/table_registry.py:533`).
- **Needs:**
  - `EXISTS decorator, sym_of, call_arg, value`: a model is a class with a decorator call whose callee has sym_of `l1000_schema.table_registry.register_table`. Its declared id is the `semantic_id=` string, and `participates_in_static_naming_metadata=` is read the same way.
  - `LIB` `class_base` through generic subscripts: `class_base(c, q) :- …, kind(e, "subscript"), field(e, "value", v), sym_of(v, q).` Models are declared `class EncryptedBlob(ApplicationObjectRow[SysId])` (`l1000_schema/encrypted_blob.py:87`). Today's `class_base` calls `sym_of` on the subscript, so `subclass` loses every generic base and inherited fields are lost.
  - `LIB` `model_field(cq: symbol, x: symbol, site: node)`: pydantic fields of class cq. They are annotated class-body assignments (`x: T` or `x: T = …`; `binding` has no row for a bare `x: T`), excluding `ClassVar[...]` and `_`-prefixed names, unioned with the fields of workspace ancestors (`subclass`) that cq does not redeclare. Mirrors `model_cls.model_fields` (table_registry.py:557-562, 597).
  - `LIB` `field_meta(cq: symbol, x: symbol, key: symbol, v: symbol)`: the `semantic_id`/`api_name` keyword of the field's `Column(...)` call, read from the declaring class (schema_def.py:1672-1681, 2045, 2209).
  - `LIB` `relationship(cq: symbol, x: symbol, site: node)`: a class attribute, own or inherited (`dir()`), assigned a call whose sym_of is one of the four descriptor classes; not `_`-prefixed (table_registry.py:1544-1561).
  - `EXISTS qualname, cat, count`:
    - the effective ids: declared, else `module.qualname` for the model and `<model id>.<name>` for members (table_registry.py:1476-1491, 1577-1586; schema_def.py:1685-1696);
    - the effective API names: declared, else the name, with `sys_id` → `id` when there is no `id` field (schema_def.py:1713-1730);
    - the duplicates, as `count : { … } > 1`.
- **Hard parts:**
  - Which models exist is a run-time fact: the registry holds whatever modules the process imported. A static port covers every `@register_table` class, including ones the oracle never sees (benchmarks, conform tests) and ones registered by a call rather than a decorator (`register_table(...)(cls)`). Scope it to production dirs.
  - `QueryableModelMetaclass` (schema_def.py:878) and FTS or "static_naming_generated_field" columns add fields at class creation. Those are not in the source.
  - The oracle reports only the *first* `ValueError` per model, in pydantic field order. The finding is keyed by model, so the order matters only for the message text.

### transaction_callback_wrappers (hard-fail, 458 lines)
- **Computes:** two findings per function, both at the def line, with code `transaction_callback_wrapper`:
  1. A function has a parameter whose annotation text contains both "Callable" and "Transaction", and its own body
     calls a transaction entrypoint `….read/write/…_async/versioned_*/…version*(…)`.
  2. An entrypoint call in the function takes a callback that re-enters a versioned API. Either the callback is a
     lambda containing a `.versioned_read*/write*(…)` or `getattr(x, "versioned_…")` call, or it is a name equal to
     a directly nested def that makes such a call.

  Exempt: two files, and entrypoint-named methods of SyncDatabaseConnection / SyncVersionedDatabaseConnection.
  Scope: production dirs except `impl/`, tests excluded.
- **Inputs:** Python AST. Paths `l2000_connection/connection.py` and `l9900_auto_api/_versioning_helpers.py`,
  relative to the repo root (:85-90, :127-131). Scope `data_layer_production_dirs` minus `impl` (:423).
- **Needs:**
  - `LIB param_kind` (as above): all kinds, posonly, kwonly, `*a`, `**kw` (:190-198).
  - `LIB anno_mentions(t: node, w: symbol)`: the annotation's text with comments and layout removed, as
    `ast.unparse` gives it (:178-184), contains w. Body: some identifier, attribute name or string contents at or
    below t contains w (`within`, `text`, `str`, `contains`). It must not use the raw `text(t, …)` of the
    annotation, which includes comments (the candidate hit exactly this; batch-three notes). A word cannot span two
    tokens, so per-token `contains` is exact. Then `cb_param(f) :- param_kind(f,_,_,_,t), anno_mentions(t,"Callable"),
    anno_mentions(t,"Transaction").` (:199-205).
  - `LIB callback_arg` (as in transaction_closures; :208-214).
  - `LIB walk_owner(c, f)`: the innermost def by AST walk, with lambdas transparent and decorators belonging to the
    decorated def (:268-273, :287-301).
  - `EXISTS within, kind`: rule 2a is `reentrant(r), within(r, l), kind(l, "lambda"), callback_arg(c, l),
    entry_call(c), walk_owner(c, f)`. The oracle walks every AST ancestor lambda of r (:279-285), so r may sit in a
    nested def inside the lambda.
  - `EXISTS call_arg, str, text`: `getattr` is textual (`func.id == "getattr"`, not resolved). Any
    positional argument after the first (`args[1:]`) that is a plain string in the versioned set counts (:304-311).
  - `LIB nested_reentrant(f, x)`: g has f as its walk_owner parent (`_function_stack[-1]` at g's exit), g's own
    walk-owned calls include a reentrant one, and `def_name(g, x)` (:297-301). Rule 2b:
    `callback_arg(c, n), kind(n, "identifier"), text(n, x), walk_owner(c, f), nested_reentrant(f, x)` (:233-236).
    Textual names, no order test.
  - `EXISTS nearest(f, k, "class_definition"), def_name`: API boundary is f's name in TRANSACTION_ENTRYPOINTS and the
    innermost enclosing class (through functions) named one of the two connection classes (:167-175). This skips
    both findings (:219-220).
  - `LIB class_qualname(f: node, q: symbol)`: every enclosing class's name joined with ".", functions omitted
    ("Outer.Inner.m", or "m" in a function nested in a method of C gives "C.m") (:162-165). It is in the message,
    not the key. Body: recursion over `nearest(·, k, "class_definition")` with `@dot`. `names.qualname` and
    `scope_name` differ: they add the module, add function segments, or keep one class only.
  - `EXISTS doc, .scope`: path exemption by equality on the repo-relative path; scope minus `impl`.
  - `POLICY`: the two messages share one code. Under loose keys `(path, line, code)`, a function with both collapses
    to one finding. Either keep the message kind in the key or split the code (`…_helper` / `…_inline`).
- **Hard parts:**
  - "calls a transaction entrypoint" is any attribute call with a listed name in the function's own walk-owned body.
    No receiver test is made, so `fh.read()` with a listed name also counts when the function has a
    Callable[…Transaction…] parameter.
  - The `check_inline_reentrant_callbacks` gate (source contains "getattr" or a versioned name, :344) and the source
    triggers (:82) are implied by the predicates, so they need no translation.
  - A nested def's reentrant flag counts only calls whose innermost walk owner is that def. Calls in its own nested
    defs propagate one level only.

### plan_scope_ownership (hard-fail, 596 lines)
- **Computes:** Fourteen ownership rules (PSO001-PSO013) between base and versioned plan scopes, each a finding with a
  code and a line. Their inputs: runner signatures and bodies, workflow classes, versioned-scope methods, calls in
  listed files, stores to `_callback_versioned_txn`, and three literal substrings of the raw source.
- **Inputs:** Python AST and raw source text of the production dirs plus `conform/` and `data_layer_testing/`, tests
  included, minus `.git`, `.venv`, `__pycache__`, `tmp` and the cache directories (`lint_plan_scope_ownership.py:58-59`,
  `:154-169`, `:574-577`). Seven named files (`:91-101`, `:349`, `:385`).
- **Needs:**
  - `EXISTS def_name, field(p,"type",t), text, within, nearest(n,f,fdef), callee, attr_of, cname-style rules`.
    Covers PSO002 and PSO003 (calls anywhere under a runner, nested defs included, by identifier or attribute name,
    `:309-331`); PSO005 (direct defs in the two versioned scope classes, `:334-344`); PSO004, PSO006, PSO007 and
    PSO009 (doc path in a set, innermost def name contains "nested_versioned", and call text contains a lifecycle
    name or "Versioned", `:345-393`); and PSO013 on a call (`:394-402`).
  - `EXISTS match captures + cat`. The PSO008 expected type is `node.name.removesuffix("PlanWorkflow") + "Transaction"`
    (`:283`). Messages interpolate names.
  - `EXISTS tgt (python/names)` for PSO010's `ctx is Store` (`:403-415`). It covers assignment, annotated-assignment,
    augmented, `for`, `with … as` and pattern targets, and leaves out `del`. It is internal today: export it as an
    interface relation (`store_target`). Owner `""` at module level is reported.
  - `EXISTS modnode, text(m,t), match_all, min` for PSO011, PSO012 and the file-level PSO013 (`:417-455`): literal
    substrings with exact newlines and indents, reported at the line of the first occurrence, and for PSO013 a second
    substring "else None" anywhere in the file. A `match_all` capture gives a span with `line`. `min` picks the first.
  - `EXISTS .scope dirs …, "conform", "data_layer_testing"`, `.scope tests true`.
  - `LIB positional_params(f: node, p: node, i: number)`: CPython's `args.args`, i.e. the parameters after a `/`,
    before `*` or `*args`, with splats left out, renumbered. PSO001 and PSO008 read `args.args[0]` and `args.args[1]`
    (`:236-249`). `names.param` counts every child, positional-only ones and separators included.
  - `NATIVE parse_failed(d: number)`, the same need as async_sync_delegation. The CLI reports PSO000 at line 1 with
    the parser's message (`:487-489`). The suite drops syntax errors silently (`fact_engine.py:1349`).
  - `POLICY` extraction failures as findings. A non-syntax extractor error becomes `FACT-EXTRACTION-FAILED` at line 1
    of the file (`fact_engine.py:1336-1350`). The CLI and the suite differ on syntax errors (PSO000 in the CLI,
    nothing in the suite). The port must pick the suite's behaviour.
- **Hard parts:**
  - Duplicate diagnostics: one parameter node can get PSO001 twice, as "must accept exactly …" and as "broader than
    …" (`:240-267`). The loose key needs a reason column, or two violation relations, to keep both. PSO009 fires on
    every call whose text contains "Versioned", so nested calls each count.
  - The annotation checks compare `ast.unparse(...)` with `!=` (`:240`) and substrings (`:250`, `:259`, `:284`).
    `text` agrees for plain names and subscripts. It differs on whitespace inside brackets and on comments in
    multi-line annotations. `ast.unparse(call)` (`:375`, `:385`, `:394`) drops comments and requotes strings, so
    `text` can over-report when "Versioned" appears only in a comment inside a call.
  - Trigger gate: the suite extracts only files containing one of eight substrings (`:131-141`). Files with
    `SyncReadPlanWorkflow` or `execute_read_plan_workflow_async(` but no trigger produce no PSO008 or PSO012. The
    single-file CLI path does not gate. Guard the rules with a `contains` over the module text.
  - "Owner" is the innermost `def`/`async def` and never a lambda (`ast_collectors.py:67-76`). This is `nearest(…,fdef)`.
  - PSO011 and PSO012 report only the first occurrence of each pattern in a file. Each of the six PSO012 patterns
    reports separately.

### embedding_vector_orchestration (hard-fail, 596 lines)
- **Computes:** In the embedding and vector modules it reports these codes:
  - EVO001: connection-owned `read`/`write`/`read_async`/`write_async` calls, unless the enclosing
    `path::qualname` is grandfathered.
  - EVO106, `embedding_worker.py` only: `conn.*_transaction` entrypoints.
  - EVO101, worker only: `txn.query(...)…<terminal>()` chains.
  - EVO105, worker only: `txn.query(...)`.
  - EVO102, worker only: `.resolve()` or `.resolve_async()` inside any enclosing `*_lazy` function.
  - EVO104, worker only: hybrid `*_sync_lazy`-style function names.
  - EVO107 and EVO108 are computed but not reported by the suite plugin.
- **Inputs:** Python AST of `l5200_vector_index/**`, `impl/common/l2070_server_plan/**` and the file
  `data_layer/vector_index.py`, tests excluded (`:138-142`, `:536-541`). Repository-relative paths are compared
  with `l5200_vector_index/embedding_worker.py` (`:152`). The grandfather list is a constant (`:171-185`). The
  CLI flag `--warn-query-terminals` (`:575-577`) affects the CLI exit code only.
- **Needs:**
  - `EXISTS doc(d, path, _)` with a `match` on the path, for the two directory prefixes plus one file. `.scope dirs`
    as used elsewhere names directories, not a nested prefix or a single file. Also `.scope tests false`.
  - `LIB class_path_name(f: node, q: symbol)`: the oracle's qualname (`:222-231`). It is every enclosing **class**
    name joined with "." and then the function's own name; enclosing functions are skipped. So it differs from
    names' `qualname` (which includes functions and the module) and from `scope_name` (nearest class only).
    Sketch:
    - `cpath(c, x) :- def_name(c, x), !nearest(c, _, "class_definition")`
    - `cpath(c, @dot(p, x)) :- nearest(c, k, "class_definition"), cpath(k, p), def_name(c, x)`
    - `class_path_name(f, x)` for a function with no enclosing class
    - `class_path_name(f, @dot(p, x)) :- nearest(f, k, "class_definition"), cpath(k, p), def_name(f, x)`
  - `EXISTS cat` plus a fact list for grandfather membership: `cat(path, "::", q)` with q from
    `class_path_name(F)` of the innermost `function_definition` (`nearest(c, F, fdef)`), or "<module>" when there
    is none (`:227-231`, `:295`). Lambdas do not count as functions.
  - `EXISTS` recursion over `callee`/`attr_of` for `_is_conn_receiver` (`:233-243`):
    - the identifier `conn` or `connection`;
    - an attribute ending `.conn`, `.connection` or `._conn`;
    - `R.session(...)` where R is itself a conn receiver, recursively.
  - `EXISTS` recursion for EVO101 (`:245-252`). Walk inward from the terminal's receiver through calls whose
    function is an attribute. The **first** `.query` reached decides: it fires iff that call's receiver is
    txn-like. A txn-like receiver is an identifier ending in `txn`, or an attribute whose name ends in `txn`
    (`:254-259`). A `.query_lazy(...)` link is walked through, not stopped at.
  - `EXISTS nearest(c, F, lazy_fn)` with a derived unary node relation, `lazy_fn(F)` iff the def name ends in
    `_lazy`, for EVO102 (`:375-389`). It is the nearest lazy ancestor at any depth, through non-lazy functions.
    The reported qualname is that function's bare name.
  - `EXISTS .mode log EVO107, EVO108` (or no violation at all). The plugin drops codes outside `FAILING_CODES`
    (`:127-135`, `:520`). The CLI-only `--warn-query-terminals` would be an `option` gating EVO101's mode, and is
    not part of the suite meaning.
- **Hard parts:**
  - The script doc says nested `*_lazy` definitions are "scanned separately". The code instead reports a `.resolve()`
    under **any** enclosing lazy function, even through a non-lazy inner def, and names the nearest lazy one
    (`:378-389`).
  - The grandfather key uses the qualname of the *innermost* function only, built from classes alone. In
    `def f(): def g(): conn.read()`, the key is `path::g`, not `path::f.g`. The entry
    `l5200_vector_index_task_payloads.py::…` (no `/`) can never match an in-scope path: it is dead.
  - EVO101 and EVO105 both fire on `txn.query(q).execute()`, at the same line, one per code. There is no
    deduplication: two qualifying calls on one line are two findings each.
  - `_is_txn_receiver` uses `endswith("txn")`, so `self.my_txn` and `wtxn` qualify. `_conn` qualifies only as an
    attribute, not as a bare name.
  - The source-trigger gate (`:104-120`) cannot remove a failing finding: each failing shape contains one of the
    trigger substrings.

### hnsw_architecture (hard-fail, 263 lines)
- **Computes:** Over `impl/common/**`, `l5200_vector_index/**` and `vector_types.py` (tests excluded, and `rust_bridge/generated_*` skipped), reports:
  - HNSW001: forbidden names (Name, Attribute or def/class name);
  - HNSW002: forbidden substrings in str constants;
  - HNSW003–005, in files named `algorithm.py` only: backend imports, `def render`, and `SqlFragment.x(...)` calls;
  - HNSW007, in HNSW modules: a JSON token in any import, name, attribute or non-docstring string.

  HNSW006 ("forbidden module exists") is produced only by the CLI path.
- **Inputs:** Python AST and raw source text: the substring trigger gate, applied to the whole file. Paths.
- **Needs:**
  - `EXISTS modnode, text, contains`: the extractor runs only when the source contains one of `hnsw`, `Hnsw`, `HNSW`, `json`, `Json`, `JSON` (lint_hnsw_architecture.py:88; fact_engine.py:1184-1190). The gate matters: `scored_pairs_query` (HNSW001), the HNSW002 texts, and HNSW004/005 can occur in a file without a trigger. This assumes the module node's text is the whole file. A `doc_text(d, s)` would make that explicit.
  - `LIB` `expr_name(n: node, x: symbol)`: an identifier in expression position, as `ast.Name`. It is `nameref` minus parameter identifiers, the identifiers inside import `dotted_name`/aliases, and `global`/`nonlocal` names (lint_hnsw_architecture.py:134-135, 142). `nameref` includes those, and ast does not see them as Names.
  - `LIB` `docstring(s: node)`: the first statement of a module, class or def body is an expression statement holding a string (lint_hnsw_architecture.py:122-128). Today only `module_doc` exists, for modules.
  - `LIB` `str_constant(n: loc, v: symbol)`: the `ast.Constant(str)` values that `ast.walk` visits.
    - A plain or implicitly concatenated string gives one value: `"a" "b"` is the single Constant "ab".
    - Each literal part of an f-string is its own Constant; `ast.walk` visits these (lint_hnsw_architecture.py:138, 149).
    - Bytes are excluded.

    Datalog over `kind`/`value` covers this only if `value` is defined for `concatenated_string` and for f-string `string_content` parts. If it is not, this becomes a `NATIVE` extension of `value`.
  - `EXISTS attr_of, def_name, match, line, file_name, path_part, import_alias`:
    - HNSW001 and the attribute side of HNSW007;
    - the case-insensitive JSON regex `(?:sqljson\w*|\bjsonb?(?:_\w+)?\b)`, used as a search, so wrap it in `.*…`;
    - HNSW003 on the module text *as written* (`node.module`): relative dots are dropped and not resolved, so `field(i, "module_name", …)` gives the dotted part and `module_text` must not be used;
    - HNSW004: a plain `def render`, not `async def`; HNSW005: `SqlFragment.<x>(…)` with a bare-name receiver.
- **Hard parts:**
  - HNSW006 never fires in the suite, because the plugin reporter only reads extractor facts (lint_hnsw_architecture.py:186-187 vs 200-211). The CLI does report it, through `file(path)`.
  - `hnsw_module` is `"hnsw" in stem.lower()` or `sql_table_hnsw` in the path parts (line 120). The `algorithm.py` test is by file name anywhere in scope, not by the `_ALGORITHM` path.
  - An `Import` symbol joins the module and the alias names with "," and " ". Those are non-word separators, so testing each piece on its own is equivalent.

### conn_or_txn_union (hard-fail, 579 lines)
- **Computes:** for every def not on the allowlist (`InnermostClass.name` or bare name), over its pos, posonly and
  kwonly parameters except `self` that have annotations:
  - UNION: the first parameter whose annotation's type-name set meets both the CONNECTION_TYPES and
    TRANSACTION_TYPES sets. One finding, and the function is not tested further.
  - Else SEPARATE_PARAMS, when one parameter has a connection type and another a transaction type.

  The finding is at the def line, with code UNION or SEPARATE_PARAMS. If the in-script allowlist signature fails,
  the result is one `unsigned_allowlist` finding instead of everything else.
- **Inputs:** Python AST. Files under production dirs plus `conform/` (existing only), excluding `test`/`tests` path
  parts and `test_*` file names (:424, :454-461). The signed allowlist is inline in
  `scripts/lint_conn_or_txn_union.py`: the set literal `ALLOWLIST` (:86-118) and `_ALLOWLIST_SIGNATURE` (:122),
  verified by `lint_lib/signing.py:65-157` over canonical JSON `{"allowlist", "linter", "version": 1}`.
- **Needs:**
  - `LIB param_kind` (as above): `args`, `posonlyargs` and `kwonlyargs`; not `*a` or `**kw` (:256).
  - `LIB type_names(t: node, x: symbol)`: the oracle's `_get_all_type_names` (:188-220):
    - Name gives its id.
    - Attribute gives its last attribute (no descent into the object).
    - Subscript gives its value's names plus each slice element's names.
    - `|` gives both sides.
    - A string constant gives its whole value, unparsed: `"Connection"` matches, `"Connection | None"` does not.
    - Anything else gives nothing.

    Recursive over `identifier`, `attribute`, `member_type`, `subscript`, `generic_type`, `type_parameter`,
    `binary_operator` with op `|`, `union_type`, `string`. Then `conn_t(x)` and `txn_t(x)` are exact-membership
    facts (:125-146).
  - `EXISTS min aggregate`: UNION is reported once, at the first both-typed parameter in the oracle's order
    (args, then posonly, then kwonly, :256). Only the details text depends on which one; the key is the def line.
  - `EXISTS nearest(f, k, "class_definition"), def_name`: the allowlist key uses the innermost enclosing class through
    functions (:176-186). The test is `q ∈ A ∨ name ∈ A`.
  - `NATIVE signed_source_list(path: symbol, var: symbol, entry: symbol)`, or migrate. The entries are a Python set
    literal in a script with an Ed25519 signature over `{"allowlist": sorted, "linter": name, "version": 1}`
    (`signing.py:65-83`). Datalog could read the literal if the script were parsed, but it cannot verify the
    signature. `signed_list` covers it only if the list moves to a Rerail-signed file (then `EXISTS signed_list`).
    This is the same need as transaction_patterns' `signed_json_list`: one native "foreign signed allowlist" reader
    with format variants.
  - `POLICY`: a signature failure is a finding, not an engine error. If the list fails, the reporter returns exactly
    one finding `unsigned_allowlist` at `scripts/lint_conn_or_txn_union.py:1` and suppresses every other finding
    (:412-421). core.rr says only that `signed_list` "refuses" a bad list. The policy needs a directive (e.g.
    `.on_signature_failure finding CODE at "path"`) or a nullary relation `signed_list_invalid(path)` so a rule can
    report it and the other rules can be guarded by `!signed_list_invalid(…)`.
- **Hard parts:**
  - Exact set membership on extracted names, not substrings: `SyncWriteTransaction` and `SyncReadTransaction` are
    not in TRANSACTION_TYPES, so they never trigger.
  - A string annotation counts only as a whole: `"Connection"` matches, `"Connection | None"` does not.
  - `Annotated[Connection, …]` contributes "Annotated" and "Connection"; a call element contributes nothing.
  - Nested functions and methods are all checked. The class context is the innermost enclosing class even across
    functions.
  - Type-grammar node kinds in tree-sitter differ from expression kinds (`generic_type`/`union_type`), so the LIB
    needs both shapes.
  - SEPARATE_PARAMS does not require Optional, despite the docstring: any conn-typed parameter plus any txn-typed
    one triggers it.

### connection_mode_split (hard-fail, 390 lines)
- **Computes:** In the `SCAN_ROOTS` dirs (tests excluded), reports:
  - old mixed connection class names: as a class definition, an import alias name or `asname`, a Name, or an attribute;
  - private connection cores outside the allowed files and roots;
  - public `async def` methods directly on `Sync*Connection`/`SessionHandle`/`VersionHandle` classes;
  - public sync methods on the matching `Async*` classes, unless the name ends in `_async` or is in the allowlist;
  - `read`/`write`/`read_async`/`write_async` methods on `_SessionHandleCore`/`_VersionHandleCore`.
- **Inputs:** Python AST and paths.
- **Needs:**
  - `LIB` `expr_name(n, x)`, as in hnsw_architecture: `ast.Name` visits only expression identifiers (lint_connection_mode_split.py:198-199). Import names are checked separately (lines 186-196), and parameters are not Names.
  - `EXISTS import_alias`: `ImportFrom` checks both `name` and `asname` (lines 187-190). `Import` checks the last dotted segment and the asname, through `split_dot`/`@head` (lines 193-196). Both report at the statement's line.
  - `EXISTS def_name, is_async, defstmt, field, child, match`: only *direct* class-body items (`node.body`). A def counts directly or under a decorator (`defstmt`), but not one nested in an `if` inside the class body. That is why `nearest(f, c, scope)`/`method_of` is slightly too wide (lines 221, 232, 244).
  - `EXISTS doc, match`: private core allowed (lines 213-216), through an exact path set or the prefixes `impl/` and `l2070_batching/`.
- **Hard parts:**
  - Names inside string annotations are not checked; ast does not parse them. So `parsed`/`annotation` must not be used.
  - The source-trigger gate (line 152) is a no-op: every reported name contains one of the triggers.
  - Repeats on one line with the same message collapse under set semantics. The oracle counts each.

### async_sync_delegation (hard-fail, 2708 lines)
- **Computes:** Per Python file in the data-layer production dirs (tests excluded), six detectors feed three categories.
  A: an `async def` with no await that calls its sync twin. B: an `async def` with no await at all.
  C: a lazy/sync name mix, context-manager or public sync/async pairs not thin over one helper family, legacy-decorator
  triples with missing parts, and `to_thread`/`run_in_executor` escape hatches. The suite does not report the
  violations themselves. It compares the counts per category (A, B, C, total) with a signed baseline file and reports
  one finding per count that went up.
- **Inputs:** Python AST of `impl/`, `data_layer/`, `_internal/` and every top-level `l\d{4}_*` dir
  (`lint_lib/discovery.py:28-41`), tests excluded (`lint_async_sync_delegation.py:2273-2274`). A signed JSON
  baseline `.async-sync-delegation-baseline.json` (`:2285`, Ed25519 public key hard-coded at `:2288`). The signed
  in-script Category C allowlist `ALLOWLISTED_CATEGORY_C_TARGETS` (`:158`, empty at the pin). Raw file text, for the
  extractor's trigger gate (`:161-178`, `lint_lib/fact_engine.py:1177-1203`).
- **Needs:**
  - `EXISTS is_async, op(n,"async"), nearest(n,f,fdef), def_name, decorator, call_arg/posarg, callee, attr_of,
    match captures, cat, count/min/max, contains`. A/B: "has await" is an `await`, or a `with`/`for` with
    op "async", whose nearest fdef is f (`:342-356`). The sync-twin call is a call whose callee identifier or attribute
    text equals the f name minus `_async` (`:358-370`). The path and `(suffix, name)` allowlists (`:434-735`) are
    `match` on `doc` path plus `def_name` facts.
  - `EXISTS signed_list` for the Category C allowlist (`:2201-2215`). Its key is `"{file}::{Class}.{method}"`
    (`:1503`, `:1570`), built with `cat`.
  - `EXISTS .scope dirs "impl","data_layer","_internal","l[0-9][0-9][0-9][0-9]_*"`, `.scope tests false`.
  - `LIB body_stmt(b: node, st: node, i: number)` and `body_len(b: node, k: number)`: the statements of a body
    block, without comment nodes and without a leading docstring statement, renumbered from 0. Body:
    `body_stmt(b,st,i) :- child(b,st,_), !kind(st,"comment"), !docstring(b,st), i = count:{earlier such siblings}`.
    The oracle tests `len(body)==1`, `body[-1]` and `body[:-1]` after `_strip_method_docstring` everywhere:
    `:738-744`, `:845-870`, `:905-914`, `:946-971`, `:1361-1419`. Plain `child` indexes count comments, and
    tree-sitter makes them named children.
  - `LIB callable_name(e: node, s: symbol)`: the oracle's `_extract_callable_name` (`:756-765`). An identifier gives
    its text. `o.a` gives `@dot(callable_name(o), a)` when `o` is a pure name chain, and just `a` when it is not
    (`foo().bar` gives "bar"). To keep it stratified, use a positive `name_chain(o)` and then
    `callable_name(n,a) :- attr_of(n,o,a), !name_chain(o)`. Helper-family equality (`:747-753`, `:768-781`)
    compares these strings after a `_async`/`_sync` suffix is stripped by a capture.
  - `LIB decorator_name(f: node, s: symbol, called: number)`: `_decorator_name` (`:1039-1046`). Identifier text,
    else the attribute name, and for a `@d(...)` call the same applied to its function. `called` = 1 marks the call
    form, because `_decorator_names` skips call decorators (`:1060-1068`) and `_has_decorator_named` does not
    (`:1027-1036`).
  - `LIB direct_def(s: node, x: symbol, f: node)`: f is the last `def` named x that is a direct statement of class or
    module body s, decorated or not. The oracle builds `{item.name: item for item in node.body if FunctionDef}`
    (`:1487`, `:1544`, `:1664-1671`), so a later def wins, defs under `if`/`try` are invisible, and assignments do
    not shadow. Body: `stmt(st,s), defstmt(f,st), def_name(f,x), st = max:{same-name direct def statements}`.
    `names.method`/`live_def` do not match: the first includes nested `if` bodies, the second treats assignments as
    shadowing.
  - `LIB final_rebinding(s: node, x: symbol, y: symbol)`: `_resolve_final_bound_function` (`:1807-1838`). Take the
    latest direct `x = <identifier y>` assignment in s's body (later assignments of other values do not reset it,
    `:1834-1837`), and follow it until no rebinding is left. `max` picks the latest. Recursive.
  - `LIB not_benign_stub(st: node)`: the complement of `_is_benign_stub_setup` (`:887-902`), written as positive
    recursion so it stays stratified. Base: a statement that is not `pass`, `del`, an assignment (with or without an
    annotation, but not `+=`), or `...`. Recursive step: an `if` whose clause blocks contain a statement that is not a
    raise of NotImplementedError/RuntimeError (`:872-884`) and is not_benign_stub, or that has an empty body once its
    docstring is removed. `_is_unsupported_or_tombstone_async_body` (`:905-914`) then needs a last statement that is
    such a raise and `count:{earlier body_stmt, not_benign_stub} = 0`.
  - `LIB non_static_value(e: node)` and `non_local_target(t: node)`: complements of `_is_static_return_value`
    (`:927-943`) and `_is_local_state_target` (`:917-924`), recursive through tuple, list, set and dict. They are
    used for the `start`/`stop`/`_on_last_unsubscribe` lifecycle exemption (`:946-985`).
  - `NATIVE parse_failed(d: number)`: CPython's `ast.parse` rejected the document. The suite drops such files with no
    finding (`fact_engine.py:1105-1108`, `:1349`; `analyze_file` `:1855-1857`). Tree-sitter recovers from errors, so
    `kind(_, "ERROR")` misses MISSING nodes and errors only CPython reports (e.g. `await` outside an async function,
    invalid targets). Datalog cannot derive what the CPython parser accepts.
  - `NATIVE signed_counts(path: symbol, key: symbol, n: number)`: reads a JSON count baseline and checks its Ed25519
    signature over the canonical message `json.dumps({category_a_count, category_b_count, category_c_count,
    total_violations}, sort_keys=True)` (`:2296-2323`). It refuses the file when the signature fails. `signed_list`
    holds allowlist entries under its own signing scheme, not a signed map of counts.
  - `POLICY` ratchet on grouped counts from a signed file. The budget comes from the repository file rather than the
    policy text. Codes are grouped into categories: A = sync-delegation; B = no-await; C = mixed-name,
    context-manager pair, public pair, eager pair, legacy missing-lazy, legacy missing-pair and escape-hatch (`:2063-2073`).
    A total is also checked. A breach is one finding per increased group, at the baseline path, line 1, with the
    message "{label} increased by {d} ({base} -> {cur})" (`:2393-2437`, `:2253-2266`). Today's
    `.budget * by code {…}` is inline and keyed by code, not by groups of codes or by a signed file.
  - `POLICY` failure semantics of the signed inputs. A missing baseline, an unsigned baseline and a bad signature
    each give one blocking finding (`missing_baseline`, `unsigned_baseline`, `invalid_baseline_signature`), and so
    does an allowlist that fails verification (`unsigned_allowlist`). Each replaces every other finding
    (`:2201-2251`).
- **Hard parts:**
  - Category A/B use the innermost function only. An await in a nested `def` marks the nested def, not the outer
    one (`:342-346`). A comprehension's `async for` is not an `AsyncFor` node and does not count. A function with any
    await is never A or B.
  - The family comparisons are string equality on `callable_name` text, `self.` included, so `self._x` and `_x`
    differ. The eager-pair check compares `(family, post_helper)` tuples, where the second slot can be the sentinel
    `"__exception_translation__"` (`:1374-1388`). `_resolve_legacy_result_*` takes the lambda from positional
    `args[2]` or from the keyword `build_lazy` (`:1243-1257`). The posarg index works here only because keywords
    follow positionals.
  - In the public-pair checker, the order of early returns decides what is reported (`:1555-1649`): callback
    entrypoints go one way, and txn-bound eager entrypoints are reported only when they "participate" in a lazy pair.
    The lazy-pipeline-triple exemption comes before the signature comparison.
  - Legacy triples look only at module-level functions and top-level classes (nested classes are skipped,
    `:1666-1671`). Detection needs the bare decorator, but `lazy_name=` is read from the first decorator with that
    name, called or not (`:1071-1079`). The counterpart class is named by swapping the `Sync`/`Async` prefix
    (`:1760-1781`). A cycle in `_resolve_final_bound_function` stops at whichever name repeats first, an order a
    fixpoint does not have.
  - Escape hatches give one finding per source line (`_reported_lines`, `:1902`, `:1921`, `:1937`). The first one in
    traversal order wins, and an `await` comes before its call, so its "await …" text is kept. Two escapes on one line
    count once. This matters because the result is a count: key escape findings by `(doc, line)` and choose the
    description with `min` over node start. The escape check has no path exemptions: `_should_scan_escape_hatches`
    (`:1999`) is never called, so `impl/browser/` is reported. Only the function allowlist applies (`:1962-1969`).
  - Mixed-name lowercases the name and needs "lazy" and "sync" in it ("async" contains "sync"), e.g.
    `match("(?i).*lazy.*", x), match("(?i).*sync.*", x)`. It covers sync and async defs, nested ones included
    (`:263-266`, `:1451-1466`).
  - Trigger gate: the suite runs the extractor only on files whose raw text has one of 12 substrings (`:163-177`).
    A file whose only finding is a mixed name like `def foo_lazy_sync` with no trigger produces nothing. A faithful
    port guards every rule with `modnode(d,m), text(m,t), contains(<trigger>, t)` over the list.
  - Path tests run on `str(file_path)`, the absolute path in suite mode. `"/tests/" in path` (`:440`) also matches
    directories above the repository root. `endswith(suffix)` is not anchored at a `/`, so "logged_table.py"
    (`:654`) matches any `*logged_table.py`.
  - `ast.unparse(annotation)` substring tests ("Callable", "Transaction", "LazyResult", "Query") read the annotation's
    `text`. The normalised spelling differs only in quotes and whitespace, and a string annotation's text includes
    its quotes in both.
  - A docstring is an `Expr` of a `str` constant. A tree-sitter `string` with interpolations (an f-string) is
    `JoinedStr` in CPython and is not a docstring there. A `concatenated_string` is a docstring.

### async_executor (hard-fail, 296 lines)
- **Computes:** In files under `impl/` (plugin scope, tests excluded) whose absolute path contains a strict backend dir and matches none of the allowed regexes, reports each source line that matches `run_in_executor\s*\(` and has no `# noqa: async-executor`. This is raw text, so comments and strings count. Code `run_in_executor`; the message carries `line.strip()[:80]`.
- **Inputs:** Raw source text and paths.
- **Needs:**
  - `EXISTS match_all, match, line, @slice, @replace_all` (or `text_rule`): take each line as a span with `match_all("(?m)^(?<L>[^\n]*)$", m)`. Keep those that match `.*run_in_executor\s*\(.*` and not `.*#\s*noqa:\s*async-executor.*`. Report at L, with the message `@slice(trimmed, 0, 80)`. The candidate JSON does this with `forbid.regex.lines` (`first`, `exclude`, `terminators: "lf"`).
  - `EXISTS doc, match`: should_check_file and is_allowed are *substring* and `re.search` tests on the absolute path (lint_async_executor.py:96-120), written as `match(".*impl/sqlite.*", p)` and `match(".*(conform/|/tests?/|impl/common/).*", p)` and so on.
- **Hard parts:**
  - Lines are split on `\n` only (lint_async_executor.py:126); batch-two notes this as "LF-only line splitting". Core `line` must use the same terminator, and here that is LF.
  - Substring path tests also match `impl/sqlite_x/`, any `…conform/` and any `tests/` in the checkout's absolute path.
  - One finding per line, however many calls the line has.

### hasattr_async (hard-fail, 287 lines)
- **Computes:** Over `api/`, `data_layer/`, `impl/{common,sqlite,postgres,mariadb,mongodb,neo4j,doltdb}/` and every top-level `l####_*` dir, on raw text lines:
  - reports `hasattr(…, '<x>_async'` (code `hasattr`) and `getattr(…, '<x>_async', None)` (code `getattr`); both can fire on one line;
  - skips lines whose stripped form starts with `#`, and lines matching any of 21 allow regexes anywhere in the line.
- **Inputs:** Raw source text, paths, and the file tree (the `l\d{4}_.+` dirs that exist).
- **Needs:**
  - `EXISTS match_all, match, line` (or `text_rule` with line excludes, as in the candidate JSON): take each line as a span. Apply `!match("\\s*#.*", L)` and `!match(".*(allow1|allow2|…).*", L)`, then the two regexes with a capture for the method name, used in the message.
  - `EXISTS .scope dirs "l[0-9][0-9][0-9][0-9]_*"`: `data_layer_production_dirs` (discovery.py:32-41). Then exclude `/tests/` and `__pycache__` by path regex (lint_hasattr_async.py:70-74).
  - `NATIVE` (option) `line` numbered as Python `str.splitlines()` counts lines: it also breaks on `\r`, `\v`, `\f`, `\x1c`–`\x1e`, `\x85`, ` ` and ` ` (lint_hasattr_async.py:141). async_executor breaks on LF only. A line terminator option on the native line splitter and `line` (batch-two: "keep Python splitlines behavior as the default; allow LF-only scanning with correct original offsets"). Datalog cannot recount lines without span offsets.
- **Hard parts:**
  - The allow patterns are unanchored substrings of the whole line, so `delete_async` also suppresses `soft_delete_async` and `update_async` suppresses `bulk_update_async`.
  - `[^,]+` and the per-line split mean a call that spans lines is never seen.
  - The source-trigger gate (`hasattr` or `getattr`, and `_async`; lines 52-53) is implied by the regexes.

### sql_grammar_escape_hatches (hard-fail, 1358 lines)
- **Computes:** Over `api/`, `impl/` (tests excluded), it finds classes whose base tail-name is in the "renderable"
  set. That set is a fixpoint over bare class names, then over type aliases, across all scanned files. It reports
  each `__init__` regular parameter whose annotation has a leaf type outside the safe set. It also reports
  classes named in a fixed forbidden set, and any source line that mentions a forbidden class as `Name(`,
  `import Name` or `, Name`. Findings are deduplicated by (file, line, class). The check reports nothing unless
  there are more than 16 findings; above 16 it reports every one. The script's embedded allowlist must carry a
  valid Ed25519 signature first.
- **Inputs:** Python AST of `api/**/*.py` and `impl/**/*.py`, minus `tests`/`test`/`conform` dirs and test files
  (`lint_sql_grammar_escape_hatches.py:1196`). It reads only files whose text contains one of the
  `source_triggers` (`:134-149`, gate at `lint_lib/fact_engine.py:1177`). The raw source lines of the same files
  are scanned for usages. The signed allowlist is a constant in the script itself: `ALLOWLISTED_CLASSES`,
  `ALLOWLIST_SIGNATURE` and the public key (`:112, :371, :386`).
- **Needs:**
  - `EXISTS .scope dirs "api", "impl"; .scope tests false`.
  - `EXISTS text(m) + contains` on the module node for the source-trigger gate. A file is used only if its text
    contains `SqlRenderable`, `SqlClause`, … (`:134`). This changes results: a file that defines `class X(SqlExpr)`
    and has no trigger word adds no class edge.
  - `LIB tail_name(e: node, x: symbol)`: `_get_name`/`_base_name`. An identifier gives its text, an attribute
    gives its `attribute` field, and a subscript recurses on `value`. Otherwise there is no row
    (`:669-677`, used for bases at `:842`). Also used by grammar_render_methods.
  - `EXISTS` recursion for the two-stage name closure (`:939-982`). Rule-local:
    `renderable_cls(x)` is seeded with SqlRenderable/SqlClause/SqlFragment/SqlParam, then
    `renderable_cls(c) :- class_def_name(k, c), base tail_name in renderable_cls`. Next, `renderable(a)` holds
    for `renderable_cls`, plus aliases `a = <rhs>` with an alias target in `renderable`. It is a second stratum,
    and alias rows never feed back into `renderable_cls`. Bare names only: python/names' `subclass` (resolved
    qualified names) is **not** the oracle's meaning.
  - `LIB alias_target(a: node, x: symbol)`: `_extract_alias_targets` (`:279-295`). For an `assignment` with no
    `type` field (an `ast.Assign`, not an AnnAssign), every identifier target of any scope (chained `a = b = …`
    included) maps to the RHS. An identifier gives its text, `|` recurses on both sides, an attribute gives its
    `attribute`, and parentheses are transparent.
  - `LIB param_kind(f: node, p: node, k: symbol)`: `"posonly" | "regular" | "vararg" | "kwonly" | "kwarg"`,
    from `child` indexes against the `positional_separator`, `keyword_separator` and `list_splat_pattern`
    children. Body sketch: `regular` when every `/` index is below i and no `*`/`*args` index is.
    The oracle checks `init_node.args.args` only (`:611`, `:870`), so positional-only and keyword-only
    parameters are never checked. python/names' `param` does not separate the kinds.
  - `LIB ann_leaf(t: node, x: symbol)`: `_extract_leaf_types` (`:632-667`). It is recursive:
    - a name outside the containers {list, tuple, dict, set, frozenset, Sequence, Iterable, Mapping, Optional}
      is a leaf;
    - `None` gives "None"; other constants give `str(value)` ("True", "1", "Ellipsis");
    - a string constant gives every `[A-Za-z_][A-Za-z0-9_]*` token outside the containers, by `match_all`;
    - a subscript gives the value's tail name (unless it is a container) plus the leaves of the slice;
    - `A | B` gives both sides;
    - an attribute gives its `attribute` only, even `typing.Optional` used bare;
    - a tuple gives its elements;
    - anything else gives nothing. A list (as in `Callable[[str], int]`) and a call give nothing.
    It must cover both tree-sitter type shapes (`subscript`/`binary_operator` and `generic_type`/`union_type`/
    `member_type` under `type`).
  - `EXISTS count : {…} = 0` for "unsafe = leaves − safe is empty" and for "unsafe == {str}" (`:1027-1033`). The
    second is `count : { unsafe(p, x), x != "str" } = 0`, with DATA_VALUE_PARAMS/OPERATOR_PARAMS empty facts.
  - `LIB source_line(d: number, l: number, s: span)`: one span per physical line of document d, from
    `match_all("(?m)^(?<s>[^\n]*)$", m)` on the module node. Used by the usage scan (`:807-828`): skip a line
    when `match("\\s*#.*", s)`; report each forbidden name F with
    `contains(cat(F, "("), s) ; contains(cat("import ", F), s) ; contains(cat(", ", F), s)`.
    Leading whitespace the module node omits does not change these tests.
  - `EXISTS signed_list` for `ALLOWLISTED_CLASSES` (empty today). The allowlist moves from a script constant to a
    signed list file.
  - `POLICY` signature failure is a finding, not a load error (`:1175-1186`). An unverifiable allowlist yields
    exactly one finding `invalid_allowlist_signature` (at the script, line 1), and detection is skipped even
    above the ceiling. core says only that `signed_list` is "refused if the signature fails". The policy must
    say whether that refusal is one reported code that replaces the other findings.
  - `POLICY` all-or-nothing ceiling (`:1187-1199`, constant `MAX_ESCAPE_HATCH_VIOLATIONS = 16` at `:133`). At
    16 findings or fewer the output is **empty**. Above 16, **every** finding is reported (not only the excess),
    each message carries the total ("count N > ceiling 16"), and the run blocks. The ceiling is a fixed constant,
    not a signed baseline that a ratchet lowers. `.budget * { "total": 16 }` gives the threshold, but its stated
    meaning ("block only past these counts") does not say that findings at or below the budget are dropped.
  - `LANG` ordered string aggregate, for the message only (`:1029-1040`: `", ".join(sorted(unsafe_types))`).
    Soufflé aggregates are numeric, and `cat` joins a fixed number of strings, so a variable-size set cannot be
    turned into one symbol. Example:
    `unsafe_param(p, cls, list) :- …, list = join ", " x : { unsafe(p, x) }.   // sorted by x`
    `.violation escape_hatch(at: loc, cls: symbol, list: symbol) "Parameter uses unsafe type(s): {list}"`
- **Hard parts:**
  - The finding key is (file, line, class), shared by the class findings and the usage findings (`:831-839`).
    Two parameters on one line make one finding; a `class SqlBuiltQuery(` line is both a class finding and a
    usage finding but counts once. The ceiling counts after this deduplication. So both kinds must be one
    `.violation` relation keyed by a per-line representative (`min` node id on that line), or `.budget *` counts
    twice.
  - Renderable membership is global by bare name: a class or alias in any in-scope file with that name counts,
    with no scoping or import resolution. Alias edges come from any `Name = expr` in any scope. A class that
    inherits from an alias is not renderable, because the alias stage runs after the class closure (the batch-four
    root-cause notes confirm this).
  - The forbidden-name check fires on the class name alone, before any base or allowlist test (`:1005-1013`).
  - Only `__init__` defined directly in the class body counts, and only `def`, not `async def` (`:567`, `:866`). In
    tree-sitter, a decorated `__init__` sits under `decorated_definition`, which the oracle sees through.
    `self` is skipped by name; `cls` and others are checked. Unannotated parameters are skipped.
  - `Literal["x"]` yields the leaf "x" from the string tokens, so it is unsafe. A string annotation is
    tokenised, not parsed (`"list[SqlExpr]"` gives SqlExpr only, since "list" is a container).
  - The usage scan is a substring test on raw lines. Strings, docstrings and trailing comments count; whole-line
    comments do not. `, SqlRawSql` also matches `, SqlRawSqlWithParams`, which yields two class names on one line.
  - Files with a syntax error contribute nothing (`:744-753`). A tree-sitter port must drop documents containing
    `ERROR` nodes to agree.

### grammar_render_methods (hard-fail, 1063 lines)
- **Computes:** Inside `render` methods of classes with a base whose tail name contains "Sql", and in every
  function nested in them, it reports f-strings (unless inside a `GrammarException(...)` call), `.join` on a
  literal, any `.format(...)`, `t(...)`/`Template(...)` calls, `+` between two string literals or f-strings, and
  `SqlFragment.static`/`SF.static`/`static` with a non-literal positional argument. Findings are deduplicated by
  (file, line, class, type). Scope: `impl/common/l2100_sql_grammar`, without tests.
- **Inputs:** Python AST of `impl/common/l2100_sql_grammar/**/*.py`, tests excluded (`:897`). A source-trigger gate
  (`:128-134`) cannot remove a finding, since every finding needs `def render(`. The signed allowlist
  `ALLOWLISTED_STATIC_CLASSES` is embedded in the script (empty today). The unsigned `LEGACY_ALLOWLISTED_CLASSES`
  is empty too (`:112-126`).
- **Needs:**
  - `EXISTS .scope dirs "impl/common/l2100_sql_grammar"` (or `match` on `doc` path); `.scope tests false`.
  - `LIB tail_name(e, x)` (as in sql_grammar_escape_hatches) for `_base_name` (`:362-369`). The grammar class
    test is `contains("Sql", x)` on any base's tail name (`:246`). It is a substring test with no resolution.
  - `EXISTS` recursion plus `nearest` for the render context (`:253-354`). Rule-local:
    `ctx(F, cls) :- def_name(F, "render"), nearest(F, k, "class_definition"), sqlish(k), def_name(k, cls)`, and
    `ctx(F, cls) :- !render_own(F), nearest(F, G, fdef), ctx(G, cls)`. A node's context is
    `nearest(n, F, fdef), ctx(F, cls)`. The class is the innermost class *anywhere* above F, not necessarily
    F's direct parent scope, and a nested `def render` in a nested Sql class takes over. `async def` is included.
  - `LIB string_form(n: node, form: symbol)`: `"str" | "fstr" | "bytes" | "tstr"`. It maps tree-sitter `string` and
    `concatenated_string` onto Python's AST: the prefix letters come from the `string_start` child's text.
    A `concatenated_string` with any f part is one `JoinedStr`; with only plain parts it is one `Constant`.
    It is needed for:
    - `_is_string_literal`, a str Constant only, bytes excluded (`:382`);
    - `on_JoinedStr` (`:273`);
    - `_is_string_expr` (`:385`).
    core's `str` does not say which prefix a literal has.
  - `LIB literal_const(n)`: any `ast.Constant`. That is `string_form` "str"/"bytes", or `integer`, `float`, `true`,
    `false`, `none`, `ellipsis`. `.join` counts on any constant receiver (`:388-389`: `isinstance(…, ast.Constant)`),
    so `b"".join(...)` is reported and `f"…".join(...)` is not.
  - `EXISTS` call name, rule-local (`:371-380`): `f(…)` gives "f"; `n.a(…)` with an identifier receiver gives
    "n.a"; `x.y.a(…)` gives "a"; any other receiver gives "". STATIC matches "SqlFragment.static", "SF.static" or
    "static". T_STRING matches "t" or "Template", so `string.Template(...)` is **not** reported.
  - `EXISTS within` plus that call name for the GrammarException exemption (`:356-360`): any `call` ancestor, at
    any depth, even outside render, whose call name is "GrammarException".
  - `EXISTS op(n, "+")` on `binary_operator` for STRING_CONCAT (`:313-319`). Augmented `+=` is not a BinOp and is
    not reported.
  - `EXISTS min` aggregate for line deduplication (`:780-794`). The finding is keyed by (line, class, type), and
    its `at` is the least node of that type on that line:
    `at = min m : { cand(m, cls, ty), line(m, l) }`.
  - `EXISTS signed_list` for `ALLOWLISTED_STATIC_CLASSES`. It exempts only STATIC_RUNTIME_VALUE (`:288`).
  - `POLICY` signature failure as one `invalid_allowlist_signature` finding that replaces detection (`:858-867`).
    The signature is checked **only when the allowlist is non-empty** (`:155-156`, `:858`). An empty list needs no
    signature here, whereas sql_grammar_escape_hatches requires one even for an empty list. The policy needs a
    per-list choice: "empty list is trusted".
- **Hard parts:**
  - Decorators, default values and the return annotation of `render` itself are in render's context in the
    oracle: `ast.iter_child_nodes` visits them after the function is entered (`fact_engine.py:829`). In
    tree-sitter, decorators sit outside the `function_definition` node, so `within(n, F)` misses
    `@deco(f"…") def render`. The rule must also take the `decorator` children of F's `decorated_definition`.
  - An f-string's format spec is itself a `JoinedStr` (`f"{x:{w}}"`), and nested f-strings are separate nodes.
    These collapse only through the per-line deduplication. A multi-line f-string reports at its start line.
  - STATIC_RUNTIME_VALUE makes one finding per non-literal positional argument, `*args` included (keywords are
    ignored). All of them share the call's line, so deduplication leaves one finding per call line.
  - t-strings (`t"…"`, `ast.TemplateStr`) are not `JoinedStr`, so they are not reported as F_STRING. Only calls
    named `t`/`Template` give T_STRING.
  - The legacy visitor classes (`RenderMethodVisitor`, `DetailedRenderVisitor`, `RenderBodyAnalyzer`,
    `:410-760`) are dead code. The plugin uses only `GrammarRenderMethodsCollector`.

## Summary table

One row per distinct non-EXISTS need. Needs that the sections give under different names are merged, with the
section names in brackets.

| tag | signature or construct | checks that need it |
|---|---|---|
| LIB | `param_kind(f, p, x, i, kind, ann)`: each parameter's kind (posonly / pos / vararg / kwonly / kwarg) and annotation, typed `*a: T` included; `positional_params` (CPython `args.args`) is its `pos` slice [param_kind, positional_params] | transaction_closures, transaction_patterns, transaction_callback_wrappers, conn_or_txn_union, plan_scope_ownership, sql_grammar_escape_hatches |
| POLICY | a signed input that fails verification (missing, unsigned or bad signature) becomes one named finding at the list or baseline file, line 1, and suppresses every other finding; per list, whether an empty list needs a signature. Alternative: a testable relation `signed_list_invalid(path)` [unsigned_allowlist, invalid_allowlist_signature, invalid_baseline_signature, LE000] | linter_enforcement, conn_or_txn_union, async_sync_delegation, sql_grammar_escape_hatches, grammar_render_methods |
| NATIVE | a reader for foreign signed files: the repository's own Ed25519 scheme over canonical JSON, for a JSON category list, a JSON count baseline, or a set literal plus signature constant in a Python source. One reader with format variants, or `@ed25519_verify(pubkey, message, sig): number`. Becomes `EXISTS signed_list` if the lists move to Rerail-signed files (the POLICY alternative linter_enforcement recommends) [signed_json_list, signed_source_list, signed_counts, ed25519_verify] | linter_enforcement, transaction_patterns, conn_or_txn_union, async_sync_delegation |
| LIB | annotation type names over tree-sitter's type kinds (`generic_type`, `union_type`, `member_type`) and expression kinds, in three precisions the oracles use: first or head name, every leaf name ignoring containers, and words of the annotation without comments, as `ast.unparse` prints it [anno_head, type_names, anno_mentions, ann_leaf] | transaction_patterns, transaction_callback_wrappers, conn_or_txn_union, sql_grammar_escape_hatches |
| LIB | `walk_owner(c, f)`: the function owning a node in the oracle's AST walk, with lambdas transparent and decorators belonging to the decorated def (where `nearest(c, f, fdef)` puts them outside) | transaction_closures, transaction_patterns, transaction_callback_wrappers, grammar_render_methods (render context includes decorators and nested defs) |
| LIB | callee naming: `tail_name(e, x)`, the last identifier of a callee (attribute, subscript value, recursively); `callable_name(e, s)`, the dotted callee text (`foo().bar` gives `bar`) | sql_grammar_escape_hatches, grammar_render_methods, async_sync_delegation |
| POLICY | count-based enforcement besides `.budget`: (a) a ratchet over grouped counts (categories A, B, C and total) read from a signed repository file, one finding per category that went up; (b) an all-or-nothing fixed ceiling, with no findings at or below N and every finding above it, the total shown in messages; (c) "ratchet" that in practice blocks on any finding, with the category as the code | async_sync_delegation, sql_grammar_escape_hatches, transaction_patterns |
| NATIVE | `parse_failed(d)`: CPython's `ast.parse` rejects the file (tree-sitter recovers, so the tape cannot show it) | async_sync_delegation, plan_scope_ownership |
| POLICY | extraction failures as findings: `FACT-EXTRACTION-FAILED` for non-syntax errors; for syntax errors, PSO000 in the CLI and nothing in the suite | async_sync_delegation, plan_scope_ownership |
| LANG | ordered string aggregation: `s = cat : { r(k, x) } order by k sep ","`. Used to build the canonical JSON that was signed (linter_enforcement) and the joined list in a message (sql_grammar_escape_hatches) | linter_enforcement, sql_grammar_escape_hatches |
| LIB | `class_path_name(f, q)`: enclosing class names joined by `.` plus the def's name, with no module and no enclosing functions [class_qualname, class_path_name] | transaction_callback_wrappers, embedding_vector_orchestration |
| LIB | `expr_name(n, x)`: an identifier in expression position, as `ast.Name` (`nameref` minus parameters, import names, global/nonlocal names) | hnsw_architecture, connection_mode_split |
| LIB | `callback_arg(c, a)`: the callback of a transaction call, keyword `fn=` first, else the second positional argument | transaction_closures, transaction_callback_wrappers |
| POLICY | finding multiplicity under loose keys: the same closure passed to two calls is reported twice, and two messages share one code; the key needs a reason or occurrence column | transaction_closures, transaction_callback_wrappers (plan_scope_ownership's double PSO001 is handled by a reason column, EXISTS) |
| LIB | `store_target(n)`: make names' internal `tgt` public (`ctx is Store`) | plan_scope_ownership |
| LIB | definition order within a body: `inner_def(f, x, g)` (last def wins, by `max`), `direct_def(s, x, f)` (the last direct def in a class or module body; defs under `if` and assignments do not count), `final_rebinding(s, x, y)` (the latest `x = y` chain), `local_before(cl, x, n)` (pre-order by offset) | transaction_closures, async_sync_delegation |
| LIB | transaction_closures' "outer" name sets, which are not LEGB: `outer_name`, `direct_outer`, `closure_body` | transaction_closures |
| LIB | transaction_patterns' classification: `first_conn_param` (`min`), `infer_category` (a first-match cascade by stratified negation), `category` (precedence), `internal_txn` | transaction_patterns |
| LIB | `nested_reentrant(f, x)` | transaction_callback_wrappers |
| LIB | body shape: `body_stmt(b, st, i)` / `body_len(b, k)` (statements without comments or a leading docstring, renumbered); `docstring(s)` for modules, classes and functions (today only `module_doc`) | async_sync_delegation, hnsw_architecture |
| LIB | async_sync_delegation exemptions: `decorator_name(f, s, called)`, `not_benign_stub(st)`, `non_static_value(e)`, `non_local_target(t)` | async_sync_delegation |
| LIB | string literal facts: `str_constant(n, v)` (the str Constants `ast.walk` sees, implicit concatenation as one and each f-string literal part separately; NATIVE if `value` does not cover these), `string_form(n, form)` (str / fstr / bytes / tstr), `literal_const(n)` (any `ast.Constant`) | hnsw_architecture, grammar_render_methods |
| LIB | sql_grammar_escape_hatches helpers: `alias_target(a, x)` (type-alias RHS names), `source_line(d, l, s)` (one span per physical line, by `match_all`) | sql_grammar_escape_hatches |
| NATIVE | line splitting option: Python `splitlines()` terminators versus LF only, for `line` and line scans | hasattr_async |
| LIB | pydantic and ORM model facts: `class_base` through generic subscripts (`Base[T]`), `model_field(cq, x, site)` (bare `x: T` and inherited fields), `field_meta(cq, x, key, v)` (`Column(semantic_id=…)`), `relationship(cq, x, site)` (own or inherited BelongsTo / HasMany / Through descriptors) | explicit_semantic_ids |
| LIB | `static_nonempty(e)` and `active_allowlist(d)`: static form of the run-time "has an active allowlist" test | linter_enforcement |
| POLICY | composite policy / suite manifest (`.requires policy …`): fails for each member policy with blocking findings; a member that overruns its timeout or budget is a failure finding, not a skip | standalone_linter_entrypoints |

Cross-cutting notes, not needs. These change results but are expressible today:
- **Trigger gate:** several suites analyse only files whose raw text contains one of a fixed list of words. A
  faithful port guards rules with `contains` on the module text. It matters for hnsw_architecture,
  sql_grammar_escape_hatches, async_sync_delegation and plan_scope_ownership; for connection_mode_split and
  hasattr_async it changes nothing.
- **Source text versus `ast.unparse`:** substring tests on annotation or call text over-report when a word appears
  only in a comment.
- **Run-time inputs:** explicit_semantic_ids reads the live model registry and linter_enforcement calls config
  functions. A static port covers a different set of models and configs.
