# Group 8: what `.rr` lacks for 18 data-layer checks

Oracle: scripts at revision 2b1504ad (`scripts/…`). Library names cite `lib/core.rr` and `lib/python/*.rr` on this
branch. "EXISTS" means the meaning can be written today, possibly as several rules; the composition is named.
Notes that apply to more than one check:

- **Statement counts and comments.** `core.child` counts named children, and tree-sitter's `comment` is a named
  node. A check like `len(body) <= 4` or "body is exactly one `raise`" (Python `ast`, which has no comments) needs
  statement children with comments left out. That gives the shared LIB `body_stmt` below.
- **Parameter kinds.** Several checks test `args.kwonlyargs` or `args.args` (no positional-only, no `*args`).
  `names.param` gives an index but not the kind. That gives the shared LIB `param_kind` below.
- **Source-trigger prefilters.** Many oracle plugins run their extractor only on files containing one of a few
  substrings (`source_triggers`, `fact_engine.py:1177`). Usually this only saves time. Where it changes results it is
  noted, and it can be written `modnode(d, m), text(m, t), contains("…", t)`.
- **Document-located findings.** Several checks report "missing X in file F" at line 1. The `.violation`s in this
  branch take `at: node | span`. Reporting at the module node (`modnode(d, m)`) gives line 1 for files that start with
  code. Findings located in *non-Python* files (allowlists, baselines) need a new form (LANG `path_finding`).

---

### readthrough_authority_architecture (hard-fail, 355 lines)
- **Computes:** Nine presence/absence tests on named definitions in `impl/read_through/` (RTA001–RTA009;
  RTA007 is emitted at `:249` but missing from the docstring; RTA000 for a parse failure): forbidden calls by last name, a forbidden string constant, functions that must or must
  not mention names, a class that must declare a field, a method whose string constants must cover a set.
- **Inputs:** Python AST of `impl/read_through/**/*.py`, tests excluded (`lint_readthrough_authority_architecture.py:322`);
  the CLI default is 5 named files (`:54-60`).
- **Needs:**
  - EXISTS `callee` + `text`/`attr_of` for `_call_name` (last identifier); `def_name`; `within` for `ast.walk(node)`;
    `str` for `ast.Constant == "__readthrough_local_overlays"`; `!` for every "must contain" (RTA004–RTA009, RTA007).
  - EXISTS subset test (RTA008, `:193-205`): `missing(k, x) :- step(k), required(x), !has_const(k, x)`, then report if
    `missing(k, _)`. Same for RTA003's intersection.
  - EXISTS `assigned` dict test (RTA009, `:215-226`): `assignment` with `left` identifier `lease_conflict_update`,
    `right` of kind `dictionary`, names inside it via `within`.
  - EXISTS `.scope dirs "impl/read_through"`, `.scope tests false`.
  - POLICY parse failure is a finding: `RTA000 "Cannot parse source"` at the error line (`:295-296`). Rerail needs an
    "unparseable file in scope ⇒ finding" setting (shared with authentication_plan_purity, below).
- **Hard parts:** `_names` (`:90-97`) collects `Name.id` and `Attribute.attr` but not keyword names or strings, and
  includes nested defs/lambdas (whole `ast.walk`). `_class_fields` takes only `AnnAssign` with a bare name directly in
  the class body. `"a" "b"` is one `ast.Constant` ("ab") but a `concatenated_string` of two `string` nodes in
  tree-sitter; RTA008's constant set must use the joined value (whether `core.value` gives that for
  `concatenated_string` is not stated). Docstrings count as constants too.

### authentication_plan_purity (hard-fail, 322 lines)
- **Computes:** APP001/APP002: in "semantic plan step" classes, `self.x = …` assignments whose annotation, assigned
  parameter's annotation or call name has a transport/runtime-result type suffix, or whose attribute name is listed.
  APP003: `ReadThroughWriteTransaction*.__enter__`/`__aenter__` transitively (through same-class methods) calls an
  eager authentication name. APP004: `*_sync`/`*_async` functions with auth-ish names not written as legacy lazy forwarders.
- **Inputs:** Python AST of `impl/remote/`, `impl/read_through/`, tests excluded (`lint_authentication_plan_purity.py:41`
  of the plugin block, around line 300).
- **Needs:**
  - EXISTS semantic-class fixpoint (`:101-115`): recursive rule over `class_definition` names in the same file:
    `sem(c) :- def_name(c, n), match(".*PlanStep", n).` and `sem(c) :- basex(c, e), last_name(e, b), (seed(b) ; sem(c2), def_name(c2, b))`.
    The base is matched by its *last identifier* (`_name`, `:73-80`), not resolved; same-file only.
  - EXISTS annotation token test (`:127-129`): `match_all("(?<tok>[^\\[\\]|,\\s]+)", annotation_text)` then
    `match(".*(Client|Connection|…)", tok)`.
  - EXISTS parameter lookup (`:122-124, 138-139`): the assigned value is a bare name equal to a parameter's name
    (same-name lookup, no scope resolution): `params` + `field(p, "type", t)`. Needs `param_kind` (LIB, shared) to drop
    `*args`/`**kw`, which `ast.args.posonlyargs/args/kwonlyargs` exclude.
  - EXISTS APP003 closure (`:197-229`): recursive `reach_m(entry, m)` over calls whose last name equals a method of
    the same class, not entering `_LEGACY_AUTHORITY_HELPERS`; `contains("resolve", n)` and friends for the eager test.
  - EXISTS APP004 (`:147-154`): `decorator` + last name; any call inside whose last name matches `.*_lazy`.
  - POLICY parse failure ⇒ `APP000` finding (shared with readthrough_authority_architecture, about line 281).
- **Hard parts:** `_annotation_text` is `ast.unparse`, so a string annotation `"HttpClient"` becomes the token
  `'HttpClient'` and does *not* end with `Client`. Tree-sitter text keeps the source quotes, so the result matches only
  if quotes are kept in tokens. `_is_legacy_lazy_forwarder` returns True for `legacy_sync`/`legacy_async` even with no
  `_lazy` call; only the `direct_legacy_*` decorators need one. The methods dict keeps the *last* def of a repeated name.
  APP003 reports at the entry method, once per entry.

### pure_lazy_runtime_purity (hard-fail, 3286 lines)
- **Computes:** The plugin path is `scan_paths_cached` (`lint_pure_lazy_runtime_purity.py:2949-2969`), not the
  file-local `scan_source`. It seeds functions decorated `@pure_lazy` or `@lazy`, decorator text up to `(`, last dotted
  token (`:2084-2089, 1704-1711`). Two closures over a project call graph: the pure closure from those seeds, and the
  runtime-owner closure from `*_lazy` functions with `txn`/`conn`/… parameters that call a plan-helper terminal. Both
  stop at `legacy_*` decorated callees (`:2259-2322`). Each colored function's body is analysed for runtime-state
  leaks, PLP-001…PLP-010. Non-decorated colored functions use *strict* mode, where any receiver counts as runtime-bound
  (`:2397, 1234-1235`). Owner-only functions report only PLP-001…005 (`:2425-2441`). Also PLP-007: public `*_lazy`
  functions exported from `data_layer/*.py` (through `__all__` or public names, re-exports followed) that are not
  decorated (`:2510-2572`). Results go through a strict signed burn-down allowlist, 6 fields, with `stable_scope` as
  the key for PLP-009 (`:3051-3076, 2981-3027`).
- **Inputs:** Python AST of `data_layer_production_dirs`, excluding paths containing `/tests/` (`:458-466`). The
  allowlist directory `scripts/allowlists/architecture_grandfathered/pure_lazy_runtime_purity/**.txt`, 60 fragments
  (`:161`). The ceiling file (`:163-166`). Two signatures are script constants.
- **Needs:**
  - EXISTS seeds and closure: `decorator` + last dotted name; `colored(g) :- colored(f), calls(f, g), !legacy(g)` with
    `calls.calls`/`call_target`. The oracle's own resolution (`:2140-2257`) is narrower. Calls whose root name is a local
    or parameter (other than `self`/`cls`) are skipped. Self-calls are dropped. `self.m()` resolves through same-module
    class bases depth-first (`:2191-2230`), not C3. `_resolve_fqname` takes the longest known dotted prefix (`:2123-2131`).
  - EXISTS flow-ordered taint (`:1315-1338`): locals become tainted, in visit order, when assigned from a value with a
    protected attribute access on a parameter or tainted receiver, a tainted name, a ContextVar `.get()` or a
    runtime-handle helper call. Taint is never removed. This is positive recursion over node start offsets:
    `tainted(f, x, s) :- assign_in(f, x, v, s), (protected(v) ; uses(v, y), tainted(f, y, s0), s0 < s)`.
  - EXISTS visitor pruning (`return` without `generic_visit`, e.g. `:1484, 1492, 1532, 1538, 1558`): a node is not
    examined when a strict ancestor in the same function fired a check or is an allowed helper call. Written as
    `pruned(n) :- within(n, a), trigger(a)`, where `trigger` is the check's condition, *not* "was reported". This is
    stratifiable, and it is equivalent because any trigger ancestor either reported (and pruned) or was itself pruned
    from higher up.
  - EXISTS "all sub-expressions immutable" (`_is_immutable_global_expr`, `:596-613`; `_expr_is_static_mutable_lookup`):
    the positive complement `mutable_expr(e)` by recursion, then `!mutable_expr(e)`. Writing "all children" with `count`
    in the recursion is not stratifiable in Soufflé.
  - EXISTS module-level state classification (`:729-890`): top-level `assignment` lefts (`_extract_bound_names`:
    names inside tuple/list targets), mutation sites anywhere in the file (subscript/attribute store on the name,
    mutating method calls), ContextVar globals (`:1002-1024`), and a fixpoint for runtime-handle helper functions
    (`:1124-1186`, positive recursion: a helper returns a ContextVar get or calls another helper).
  - EXISTS function-local bindings minus `global`/`nonlocal` names, not entering nested defs/lambdas (`:891-949`):
    `binding` + `within_stop(…, fscope)`.
  - LIB `expr_ctx(n, c)` (shared): `ast.Load` tests in `_global_state_name_in_expr` (`:1288-1294`) and `visit_Name` (`:1540`).
  - LIB `param_kind` (shared): parameters include posonly, args, kwonly, `*args`, `**kw` (`:1215-1219`).
    `RUNTIME_OWNER_PARAM_NAMES` minus `self`/`cls` (`:2096-2099`).
  - EXISTS PLP-007 export walk: `dunder_all`/`public_name` and `resolve_export` from python/imports, restricted to
    `data_layer/*.py` (not recursive, `:2522`), unprefixed top-level functions only.
  - NATIVE `strict_allowlist(dir, fragment, line, entry)` (shared with route_guardrail_policy) and the multiset
    reconciliation (EXISTS, as there). The PLP-009 key carries `stable_scope` instead of the line, so moving a line
    does not break the entry (`:3071-3076`).
  - LANG `path_finding` (shared): `PLP-ALLOWLIST` findings at the allowlist directory, line 1 (`:3017-3026`).
  - POLICY burn-down signing and signature-invalid-as-finding (shared with route_guardrail_policy).
- **Hard parts:** visit order is the order of `ast.NodeVisitor` fields: for `Assign`, the targets come before the
  value, and the value subtree is visited after taint is recorded (`:1336-1338`). A `return` before the tainting
  assignment is clean. `cast(T, x)` is unwrapped everywhere (`:544-552`). `__pure_lazy_surface__ = True` in a class
  body relaxes `self`/`self._txn` receivers (`:1238-1247`); it is computed per enclosing class only. Annotation
  mention (`:486-505`) recurses through `X | Y`, subscripts (except `type[...]`), tuples and lists, by last name.
  Module names are path-derived with `__init__.py` dropped (`:1672-1685`). Duplicates are merged on
  `(code, file, line, message)` (`:2724-2733`). PLP-000 fires only for explicit CLI paths that do not exist
  (`:2597-2612`; the `exists()` test is inverted), so it is CLI-only.

### cached_pure_lazy_allowlist (hard-fail, 9 lines → `verify_cached_pure_lazy_allowlist.py`, 915 lines)
- **Computes:** An exact, human-signed contract between the policy module `l2055_lazy_result/cached_pure_lazy_policy.py`
  and every `@cached_pure_lazy(cache_args=…)` and `@memoized_pure_lazy(max_entries=…)` function in production. For
  each function: per-function errors (async, variadic, generator, cache argument use other than `cache.member`
  reads/calls, aliasing, subscripting, `getattr` on it, escape by return or closure, mutable global reads, `global`).
  Plus decorator misuse (non-decorator use, unresolved aliases, escaped `memoized_pure_lazy`, private storage access,
  aliased imports). Plus differences from the policy: unsigned/stale entries, cache args/types/annotations, member-access
  set, approved owner scope/modes/policies, non-empty rationale, and **body and signature hashes**. Plus a hash of
  named runtime symbols in three files. Plus a signature over the canonical JSON text of all entries.
- **Inputs:** Python AST of fixed roots plus top-level `l####_*` dirs, skipping `tests` parts and `test_*` files
  (`verify_cached_pure_lazy_allowlist.py:513-535`). The policy module itself, read by *import* (`:16-27`) as data:
  `CachedPureLazyPolicyEntry(...)`/`MemoizedPureLazyPolicyEntry(...)` literals, `ALLOWLIST_SIGNATURE`,
  `CACHED_PURE_LAZY_RUNTIME_SOURCE_HASH`. `l2055_lazy_result/purity.py`, `l2070_batching/optimizer_run_cache.py`.
  Output is an exit code and stderr lines, with no file or line (`:844-885`).
- **Needs:**
  - NATIVE structural digest `@tree_digest(n: node): symbol`: a hash of a subtree's structure and literal values,
    ignoring positions, comments and formatting. The oracle hashes `ast.dump(node, include_attributes=False)` of the
    body, of `args|returns|type_params`, and of each runtime symbol (`:159-170, 743-777`). Datalog cannot hash, and
    `text` hashing would break on reformatting, which `ast.dump` ignores. The existing hashes are Python-`ast` specific,
    so every policy entry would have to be re-signed once under the new digest.
  - NATIVE `@sha256` (shared with rtt_counting_workarounds) is subsumed if `@tree_digest` exists. Hashes of
    *text* are needed only by rtt_counting_workarounds.
  - NATIVE `@json_str(s: symbol): symbol`: JSON string quoting with `ensure_ascii` escapes (`\uXXXX`) for the canonical
    signed text (`cached_pure_lazy_policy.py:67-79, 96-111`). `replace_all` cannot produce `\u` escapes for arbitrary
    code points.
  - NATIVE signature over derived rows: `signed_rows(name: symbol, version: number, sig: symbol, rows: relation)`.
    The signed payload is the *sorted canonical text* of all entries (`cached_pure_lazy_policy.py:222-238`), built
    by rules, not a file. `signed_list` verifies a file's lines. A relation argument is needed, as for
    `within_stop`'s stop set, because Datalog has no ordered string aggregation.
  - EXISTS policy entries as data: `call(c)`, callee name `CachedPureLazyPolicyEntry`, `call_arg(c, -1, "body_hash", v)`
    + `str`. Tuples of pairs (`allowed_cache_members`) need `child` indexes.
  - LIB `py_qualname(f: node, q: symbol)`: Python `__qualname__` with `<locals>` after a function segment, defined
    only for defs reached through class/def *bodies*, not under `if`/`try` (`:290-359, 538-611`). `names.qualname`
    has no `<locals>` and no reachability restriction.
  - LIB `param_kind` (shared): keyword-only cache args, defaults per parameter (`:205-218`).
  - LIB `expr_ctx` (shared): `Load`/`Store`/`Del` on attributes and subscripts (`:228-266`).
  - EXISTS parent tests (`parents[item]`, `:220-270`): `field(p, "object", n)`, `field(p, "function", n)`, parent
    kind `assignment`/`named_expression`.
  - EXISTS decorator alias fixpoint (`:361-388`): positive recursion over top-level `x = cached_pure_lazy` assignments.
  - EXISTS exact set comparison of member accesses: two `!` joins.
  - LANG `path_finding`, or a finding with no location: every message is policy-level (`:865-867`).
  - NATIVE (minor) Python identifier normalisation: the oracle tests `"cached_pure_lazy"` in the NFKC-normalised
    source (`:430-432, 622`). CPython NFKC-normalises identifiers, so `ast.Name.id` already matches fullwidth
    spellings. Tree-sitter `text` does not.
- **Hard parts:** the policy is Python *code*. Reading it as AST facts gives the same values only while the entries
  stay literal tuples. `ast.unparse` of a cache argument's annotation is compared with the last segment of the approved
  type (`:328-332, 806-813`). `defaults_by_name` pairs positional defaults from the right (`:206-209`).
  `CachedPureLazyPolicyEntry(...)` calls are exempt from the "cache_args keyword" scan (`:460`). Escaped
  `memoized_pure_lazy` references inside `purity.py` are skipped by a `break` (`:640`).

### rtt_assertion_integrity (hard-fail, 435 lines)
- **Computes:** In test files, `assert` comparisons that mention an RTT/request-count measurement and use `<`/`<=`,
  or `>`/`>=` with no exact `==` anchor in the same function (RTA001); `assert_max_flushes` with no exact `== 1`
  anchor in RTT-budget contexts (RTA002); `assert_single_transaction_group(max_flushes≠1)` (RTA003); `if <measurement>:`
  whose body returns or skips (RTA004); backend-dependent conditionals around or inside RTT asserts (RTA005); the
  observed count compared with its own minimum (RTA006); `_assert_rtt_limit` calls (RTA007).
- **Inputs:** Python AST of test files across the repository: `test_*`, any `tests` part, or `conftest.py`, minus
  `scripts/tests/test_lint_rtt_assertion_integrity.py` and three `conform/l209x_*` prefixes
  (`lint_rtt_assertion_integrity.py:83-88`); the CLI skips `.git`, `.venv`, `__pycache__`, `tmp` (`:63`).
- **Needs:**
  - EXISTS measurement-name test (`:76-78, 103-113`): `match("(?i)(actual_db_statements_executed|…)|.*(_rtts|rtt_count|round_trip|roundtrips).*", s)`
    on `Attribute.attr`, `Name.id` (except exactly `query_count`, compared case-insensitively) and string constants.
    `(?i)` stands in for `.lower()`; core has no lowercase functor, and `@convert(s, "lowerCase", "")` also re-splits on
    separators, so it is not a plain `lower`.
  - EXISTS operator tests (`:146, 186-193`): `op(cmp, "<")`/`"<="`/`">"`/`">="`; "all operators are `==`" as
    `!non_eq_op(cmp)`.
  - EXISTS owner function (`:96-98`, nearest `FunctionDef`, lambdas ignored): `nearest(n, f, fdef)`.
  - EXISTS per-owner existence ("this function has an exact `== 1` assert", `:134-153`) as a unary relation over
    function nodes plus `!`.
  - EXISTS RTA006 (`:176-177`): `text(cmp, t), contains("actual_db_statements_executed", t), contains("backend_attainable_min_db_statements", t)`.
  - EXISTS context string (`:239-242`): file stem + `/` + owner name, tested case-insensitively for the
    `_BUDGET_CONTEXT_TOKENS`: two `match("(?i).*(rtt|roundtrip|…).*", …)` on `file_name` and `def_name`.
  - EXISTS path filter: `path_part(d, "tests")`, `match("test_.*", f)`, `doc(d, p, _)` with prefix tests.
- **Hard parts:** `ast.unparse(comparison)` (`:176`) is not the source text. Comments and line breaks inside the parens
  are gone, so the substring test on `text` can differ only in comments. `elif` is a nested `ast.If` in `orelse`, so
  every `elif_clause` is also an `If` for RTA004/RTA005. An outer `if`'s `orelse` (RTA005, `:224-226`) is *all* of its
  `elif`/`else` clauses, nested ones included. `_constant_int` excludes `bool`, so `True == 1` does not anchor, and
  `-1` is a `UnaryOp`, not a constant. Descendants include the node itself.

### rtt_counting_workarounds (hard-fail, 585 lines)
- **Computes:** A line-by-line regex scan of production files for banned RTT/flush tracking names
  (`\bsuspend_flush_tracking\b`, …). Blank lines, comment-only lines and lines inside multi-line string tokens are
  skipped. Each hit is keyed `pattern::enclosing_symbol` and one finding per key is reported, minus a signed allowlist
  of keys. Before that, three gates each *replace* all findings: the allowlist signature is invalid; the
  GraphQL RTT measurement file contains a forbidden substring; the source digest of three locked symbols in that file
  no longer matches its signature.
- **Inputs:** Python source text of `data_layer_production_dirs` (base dirs plus top-level `l####_*`;
  `lint_lib/discovery.py:32-41`), skipping `conform/`, `benchmarks/`, `scripts/`, any `tests` or `test_*` path part
  (`lint_rtt_counting_workarounds.py:144-148`), and 4 allowed definition paths (`:90-97`). Also the allowlist
  `scripts/allowlists/lint_rtt_counting_workarounds_allowlist.txt` (signature in the script constant `:62`), and
  `conform/l9750_graphql/suite/test_rtt_efficiency.py` (a test file, read outside the scope, `:64, 246-282`).
- **Needs:**
  - EXISTS per-line text: `modnode(d, m), match_all("(?m)^(?<ln>[^\\n]*)$", m)` gives one span per line with `line`
    and `text`. This assumes the module node covers every non-blank line; trailing blank lines are skipped anyway.
    Comment-only lines: `match("\\s*#.*", ln)`.
  - EXISTS multi-line string lines (`lint_lib/source_lines.py:9-22`): a `string` node and its line count from
    `count : { match_all("\\n", s) }`, then `l >= line(s), l <= line(s) + k`. A native `end_line(n, l)` would be
    cheaper, but it is not required.
  - EXISTS enclosing symbol (`:158-201`): the innermost def (deepest class/def nesting, then shortest) whose line
    range holds the line. The range comes from `line` plus the def text's newline count, and the name from
    `qualname`. Only defs reached through `body` chains count: defs under `if`/`try` at any level are invisible to
    the oracle, so a rule must use `nearest(f, s, scope)` chains with no non-scope block between them.
    `<module>` is the fallback, and the module name is the dotted *path* (`:141-143`), not `doc.module`.
  - EXISTS dedupe by key (`:283-295`): findings keyed `(pattern, symbol)`, at the minimum line (`min : {…}`).
  - EXISTS `signed_list("scripts/allowlists/lint_rtt_counting_workarounds_allowlist.txt", key)` for the allowlist,
    with entries stripped and `#` comments dropped (`:202-210`).
  - NATIVE `@sha256(s: symbol): symbol` (`:246-262`). The measurement lock hashes a canonical JSON of three
    top-level symbols' source. Datalog has no hash. With it, a rule builds the payload with `cat` and tests
    `signed_list("…/rtt_measurement.lock", digest)`.
  - POLICY a signed list whose signature fails is a *finding* that suppresses everything else (`:368-378`), not
    only a refusal. The gate order itself is EXISTS: nullary relations and `!` (`!bad_signature(), !escape_hatch()`).
  - EXISTS measurement-file substring patterns (`:279-282`): module text `contains`. The file must be in the program's
    documents even though it is a test file (scope: `.scope include` that one path).
- **Hard parts:** the scan is on *raw lines*. A banned name in a single-line string or after code on the same line
  counts, and a `#` mid-line does not hide it. Multi-line f-strings are not `STRING` tokens on Python 3.12+, so they
  are scanned. Symbol extraction for the lock (`:226-245`) is line-based (`def`/`class` at column 0, cut at the next
  `@` line, trailing blanks trimmed). `text(def)` differs: own decorators are excluded either way, but trailing
  comments differ, so the stored signature would have to be re-signed under the new extraction. The allowlist's
  signature lives in the script itself (`ALLOWLIST_SIGNATURE`), not beside the list.

### route_guardrail_policy (hard-fail, 575 lines)
- **Computes:** Three finding kinds: `route_guardrails_mode="warn"` keyword in test/conform files;
  any `assert` holding a `"warn"` string constant in test/conform files (one per assert); every `@poorly_implemented_route`
  definition. Findings are matched as a *multiset* against a strict, signed, burn-down allowlist (fragments per source
  file, with an exception-ID field ignored for matching). Excess detections are findings. Stale entries, parse errors of
  the fragments, an invalid signature and burn-down-ceiling errors are allowlist findings.
- **Inputs:** Python AST of `api/`, `conform/`, `data_layer/`, `impl/`, `l9900_auto_api/`, tests included
  (`lint_route_guardrail_policy.py:83-89`). Allowlist directory
  `scripts/allowlists/architecture_grandfathered/route_guardrail_policy/**/*.txt`, 5 tab-separated fields
  (`:79, 113-114`; at this revision only `.gitkeep`). Ceiling `scripts/allowlists/architecture_burn_down_ceilings/route_guardrail_policy.txt`,
  3 fields (`:81, 117-126`). Two signatures are script constants (`:80, 82`).
- **Needs:**
  - EXISTS the three detections: `call_arg(c, -1, "route_guardrails_mode", v), str(v, "warn")`;
    `str(s, "warn"), nearest(s, a, "assert_statement")`; `decorator` + last name. Scope column = nearest def
    name or `"<module>"` (`:289-293`). Path test: first path part `conform` or any part `tests` (`:294-295`).
  - NATIVE fragment-directory reader `strict_allowlist(dir: symbol, fragment: symbol, line: number, entry: symbol)`,
    signed (`lint_lib/strict_allowlist.py:32-80`). It needs a directory of files (each fragment named after its source
    path), line numbers, and blank lines kept so that "blank lines are not permitted" can be reported. `signed_list` is
    one file of entries, with no line numbers.
  - EXISTS multiset reconciliation (`strict_allowlist.py:188-225`, `lint_route_guardrail_policy.py:376-388`): split
    fields with `match("(?<p>[^\\t]*)\\t(?<id>[^\\t]*)\\t(?<rest>.*)", e)`, count approved per payload and detected
    per payload with `count : {…}`, rank detections by `(line, message)` with a `count` of smaller ones, report rank
    `< detected − approved`. Stale = approved beyond the detected count.
  - LANG `path_finding`: findings located at a non-source file and line (`path=ALLOWLIST_DIR, line=1`, `:398-406`).
    Today `at` must be a node or span of a parsed document. For example:
    `.violation STALE_ENTRY(path: symbol, line: number, entry: symbol) "spurious allowlist entry: {entry}"`
    `STALE_ENTRY(f, l, e) :- strict_allowlist(dir, f, l, e), stale(e).`
  - POLICY burn-down signing: the list may only shrink, IDs are permanent, and only fields 0 and 3 may change at
    signing (`:209-221`, `strict_allowlist.py:102-165`). This is a rule about *signing*, not checking. At check time only
    the ceiling's signature and duplicates are verified (`:117-126`), and those are findings.
  - POLICY invalid signature is a finding that does not suppress the others (`:394-397`), unlike rtt_counting_workarounds.
- **Hard parts:** matching is per payload `path\tcode\tscope\tmessage`, so two violations in one function with the same
  message need two allowlist entries. Which occurrence is reported as excess follows sort order. The cache key mixes in
  the allowlist digest (`:132-149`), which is plumbing only. A `b"warn"` bytes constant is not `"warn"`, so `str`
  must not equate them. The source-trigger prefilter (`:143-148`) needs the literal `"warn"`/`'warn'` in the file text.
  `"""warn"""` and `f"warn"` both contain that substring, so the prefilter never hides them.

### connection_structural_boundaries (hard-fail, 874 lines)
- **Computes:** Acceptance gates over a fixed set of connection files: file length caps, the number of top-level
  def/class blocks in `sql_connection.py`, residual top-level symbols, forbidden or too many direct bases of named core
  classes, and lifecycle rules (context-manager overrides, `connect_async` writing sync state, `self.connect()` outside
  allowed methods).
- **Inputs:** Python AST and line count of 9 explicit files: `l2000_connection/connection.py`, `connection_lifecycle.py`,
  `impl/common/l2200_sql_dialect/sql_connection.py`, `impl/{sqlite,postgres,mariadb,mongodb,neo4j}/connection.py`
  (`lint_connection_structural_boundaries.py:62-80`). File existence decides which lifecycle file is read (`:367`).
- **Needs:**
  - NATIVE `line_count(d: number, n: number)`: `len(source.splitlines())` (`:157-158, 516`). Datalog only sees node
    start lines. The count includes trailing blank and comment lines after the last node, and `splitlines` drops one
    trailing newline. A span trick (`match_all` on the module node) covers only the module node's range, which ends
    at its last token.
  - EXISTS thresholds as `option("connection_max_lines", …)` or constants; document finding at `modnode` (line 1).
  - EXISTS top-level blocks (`:159-160`): `def`/`class` whose parent is the module (`decorated_definition` unwrapped),
    `count : {…} > 90`.
  - EXISTS bases by last identifier (`_base_name`, `:181-186`, Name or Attribute only; calls/subscripts dropped):
    `basex` + kind filter; `count` for CSB013.
  - EXISTS method facts: methods of *top-level* classes only (`:447-459`). Calls and assignments inside nested defs and
    nested classes count for the outer method (`:460-461`), which is `within`. `self.connect()`/`self.connect_async()`
    by `attr_of` + `text(o, "self")`.
  - EXISTS sync-state targets (`:210-219`): `tgt` and its pattern descent give attribute targets inside tuple/list
    targets; also `augmented_assignment` and annotated assignment left sides.
  - EXISTS `file("l2000_connection/connection_lifecycle.py")` for the fallback.
- **Hard parts:** `classes[name]` is a dict, so a second top-level class with the same name replaces the first, and the
  first one's methods are lost (`:425-429`). Starred targets in tuples are ignored. CSB031 tests class names ending in
  `Connection` or `ConnectionCore`, CSB030 one class in one file. Facts mode and the direct AST mode (`:236-370`)
  agree. Severity is `"error"` (`:552`).

### bootstrap_engine_invariants (hard-fail, 115 lines → `lint_bootstrap_engine_invariants.py`, 2163 lines)
- **Computes:** About 45 codes (BEI001–BEI047) over the lazy bootstrap engine. Most are "function/class/attribute named
  in set S is defined or referenced", "inside function F of file G, no call named in S", "function F must have
  decorator `pure_lazy` / return `LazyResult[…]` / keyword-only params", "call C must pass keywords K". It adds
  raw-text searches for deleted surfaces, and one cross-file coverage test (every `BootstrapDataPrimitiveKind` member is
  referenced by some install function).
- **Inputs:** Python AST and raw text of the scan roots: `l2055_lazy_result/purity.py`, `l3200_metadata/**`,
  `l2000_connection/schema_transaction.py`, six `impl/common/l2200_sql_dialect/*.py` files,
  `impl/common/l2070_server_plan/steps/bootstrap_interrogation.py`, two `l7245_versioned_backends` files,
  `impl/read_through/connection.py`, `data_layer_testing/helpers/bootstrap{,_manifest}.py`, tests *included*
  (`lint_bootstrap_engine_invariants.py:25-41`; plugin `:30-36`). Cross-file:
  `l3200_metadata/bootstrap_data_plan.py`, `l3200_metadata/bootstrap_coordinator.py` (`:2004-2047`).
- **Needs:**
  - EXISTS name-set tests: `def_name`, last-name of calls (`_call_name` = `Name.id` or `Attribute.attr`, `:430-450`),
    `decorator` + last name for `pure_lazy`, `file_name(d, "schema_transaction.py")`, `doc(d, path, _)` for the two
    engine files, `within` for `_walk(node)` (nested defs included).
  - EXISTS raw substring search with first-occurrence line (BEI035, BEI046; `:1801-1838`): on the module node,
    `match("(?s)(?<pre>.*?)SchemaSidecarProvider.*", m)`, then `line` of a capture placed at the match. Comments and
    strings count, and so do substrings of longer identifiers (`schema_sidecar` inside `has_schema_sidecar`). Each
    surface gives its own finding even at the same offset.
  - EXISTS file prefilter that changes results (`:1963-1977`): files whose path is not listed and whose text lacks
    `Bootstrap`/`bootstrap`/`schema_transaction`/… give *no* findings, BEI035 included.
  - EXISTS ancestor-`if` guard (`:722-740`): `within(c, i)`, `i` an `if_statement` *or `elif_clause`*, condition a
    comparison with an operand `….ZERO_APPLICATION_TERMINALS`. The guard counts from the test and the else branch too.
  - EXISTS counts: `zero_fields >= 4` (`count : {…}`, `:700-708`); `len(steps) <= 1` (`:742-760`);
    BEI012 `max(publish_lines) > max(tail_lines)` with `max : {…}`, reported at the max tail line (`:833-870`).
  - LIB `param_kind(f: node, x: symbol, k: symbol)`, with k in posonly, pos, vararg, kwonly, kwarg. The body comes from
    the parameter list order relative to `positional_separator`, `keyword_separator` and `list_splat_pattern`
    (`count` of separators before index i). Used by BEI024 (`:558-560`), BEI039 (`:1155-1167`) and BEI017
    (`node.args.args` only, `:984-993`).
  - EXISTS return annotation `startswith("LazyResult[")` (BEI029/033/039; `:1145, 1588, 1630`): `field(f, "return_type", t), text(t, s), match("LazyResult\\[.*", s)`.
  - EXISTS cross-file coverage (BEI038, `:2004-2047`): class `BootstrapDataPrimitiveKind`'s body assignments in one
    file, minus attributes `BootstrapDataPrimitiveKind.X` inside five named functions of another. Reported at line 1
    of the coordinator (module node). Both files must be in the program's documents.
  - EXISTS BEI047's counterpart name (`:2118-2124`): `cat(@slice(n, 0, -6), "_lazy")` when `match(".*_async", n)`.
  - EXISTS `.scope` as explicit files plus directories, tests included; production-only sub-test via `path_part`
    (`:2127-2131`).
- **Hard parts:** messages list sorted missing names (BEI027, BEI034, BEI017). Under the loose findings contract one
  finding per missing name, or per function, is enough; no string aggregation is needed. `ast.unparse` of a return
  annotation normalises spacing (`LazyResult [X]` → `LazyResult[X]`), and text does not. Decorated defs report the `def`
  line, which tree-sitter's `function_definition` also starts on. `_source_may_need…` is checked on the path *and* the
  text. BEI003 exempts `.resolve()` inside functions named `run`/`run_async` (nearest def).

### connection_service_architecture (hard-fail, 83 lines → `lint_connection_service_architecture.py`, 923 lines)
- **Computes:** Connection-layer invariants. Slot probes (`getattr(x, "_current_schema_transaction")`, through
  function-local aliases), loads of slot attributes, forbidden helper imports/defs/calls, scattered
  `load_python_module` calls, sync/async substantial twins, connection-level column probes, operational schema methods
  owned by several non-service classes (in a file and across files), legacy-delegate triples (stub body, lazy
  counterpart, `@pure_lazy`). Plus fixed-file assertions: required service methods, forbidden core methods, backend
  core assembly, schema-transaction facades.
- **Inputs:** Python AST of `l2000_connection/**`, `impl/common/l2200_sql_dialect/**` (own test filter: any `tests`
  part, `test_*` or `*_test.py`, `lint_connection_service_architecture.py:150-154`), four extra backend/FTS files,
  and `impl/{remote,read_through}/connection.py` for CONN103 only (`:17-27, 880-886`).
- **Needs:**
  - LIB `expr_ctx(n: node, c: symbol)`, with c = load, store or del: Python's `ast.Load` for CONN101 (`:384-395`).
    Body: `store` for `tgt` attributes and for annotated or plain assignment lefts; `del` for children of
    `delete_statement`; `load` otherwise. `names.tgt` gives stores, but nothing gives `del`, and no relation names the
    complement.
  - EXISTS alias tracking (`:350-377, 432-466`): per function (nearest def; module level has no aliases), names
    assigned from an expression holding a slot-name string, *before* the probe in visit order. Source order is
    `node(d, n, _, s, _)` start offsets. Nested functions start with an empty set, so use the alias's own function.
  - LIB `body_stmt(b: node, s: node, i: number)`: statements of a block without comments, re-indexed. The body is
    `child(b, s, j), !kind(s, "comment")`, with i = count of earlier non-comment children. Needed for
    `_body_without_docstring`, `len(body)`, "exactly one `raise`/call" (`:215-219, 236-263, 303-332`), and in
    bootstrap (BEI015 `child.body`) and others.
  - EXISTS cross-file duplicate ownership (`:776-818`): `count : {owner classes}` per method name over all documents
    in scope, `> 1`, report non-`*Service` owners.
  - EXISTS fixed-file assertions (`:820-920`): `doc(d, "impl/common/l2200_sql_dialect/sql_connection.py", _)` plus
    `!` over top-level classes and methods. "missing class" is reported at the module node, since the oracle's line is
    `None`, which the plugin turns into 1 (`:43`).
  - EXISTS legacy lazy name (`:196-208`): the decorator keyword `lazy_name="…"` (`call_arg` + `str`), else
    `cat(@slice(n, 0, -6), "_lazy")` for `legacy_async` names ending `_async`, else `cat(n, "_lazy")`.
- **Hard parts:** `_is_substantial_method`'s node-count branch (`sum(ast.walk) >= 90`, `:257-261`) is dead code. Not
  thin means at least 5 statements, so a node count (which would differ between `ast` and tree-sitter) is never
  needed. `[decorator_name] = sorted(legacy_decorators)` raises when a method has both `legacy_sync` and
  `legacy_async`, which fails the whole run (`:686`). `SyntaxError` is swallowed in the broad scan (`:526-529`) but
  crashes the fixed-file checks. `_decorator_name` here handles `Name` and `Call` only, not `Attribute`
  (`:175-180`), unlike the other checks.

### ignore_counts (ratchet, 678 lines)
- **Computes:** Nine repository-wide counts over raw source text: `# type: ignore[...]`, `# pyright: ignore[...]`,
  `# mypy: ignore[...]`, `# noqa[: codes]`. Each has a total and an "unqualified" count (no codes), plus the combined
  total. The counts are compared with a signed baseline. Any increase is one `count_increase` finding at the baseline
  file. A missing baseline, a missing signature or an invalid signature each replace that with a single finding
  (`check_ignore_counts.py:465-511`).
- **Inputs:** source text of `.py` files whose first path part is `api`, `impl`, `conform`, `tests`,
  `data_layer_testing`, `scripts` or `benchmarks`, tests included, minus six named files
  (`collect_ignore_statistics.py:125-145, 147-170`). Baseline `.ignore-counts.json` (JSON, `_metadata.signature`),
  Ed25519 public key in the script (`check_ignore_counts.py:111`).
- **Needs:**
  - EXISTS counting: `modnode(d, m), match_all("#\\s*type:\\s*ignore(?:\\[(?<codes>[^\\]]+)\\])?", m)`. The regex runs
    over the *whole text*, strings included, not only comments. Unqualified = the `codes` group did not take part,
    or it holds only commas/blanks (`collect_ignore_statistics.py:111-123`).
  - EXISTS publishing the counts: `.output ignore_count(kind, measure, n)`.
  - NATIVE JSON reader `json_value(path: symbol, key: symbol, v: symbol)` for the baseline
    (`check_ignore_counts.py:232-241`). The only file reader in core is Soufflé `.input` (delimited rows), and
    `signed_list` (lines).
  - POLICY signed numeric baseline ratchet: per-metric ceilings read from a signed baseline file. Decreases are
    allowed. Updates are human-signed (Ed25519 over canonical JSON of the nine counts, with a legacy five-field format
    also accepted, `:120-175`). Today `.budget` holds the numbers inside the policy and counts *findings*. Here the
    budgets are on derived counts (a match counts toward its total, its "unqualified" and the combined total), so
    `.budget` would need to apply to `.output` relations, e.g. `.budget ignore_count by measure from ".ignore-counts.json" signed`.
  - POLICY gate findings: missing/unsigned/invalid baseline is one finding, and nothing else is reported (`:465-497`).
  - LANG `path_finding` (shared): the finding sits at `.ignore-counts.json` line 1.
- **Hard parts:** `NOQA_PATTERN` codes are `[A-Za-z0-9_,\- ]+`, so `# noqa: E501 trailing words` captures
  `E501 trailing words` as one code. `finditer` is non-overlapping, so `match_all` matches it. The baseline compares
  only the nine scalar fields; `by_code` and `by_directory` are verbose output only (`:322-338`).

### test_budget (ratchet, 818 lines)
- **Computes:** The number of conformance tests under `conform/` compared with a signed ceiling. The plugin first
  computes a static lower bound: top-level `test*` functions plus `test*` methods of `Test*` classes without
  `__init__`/`__new__` (`check_test_budget.py:577-590`). If that bound already exceeds the ceiling, it reports
  `budget_exceeded`. Otherwise it runs `pytest --collect-only` in 4 sharded subprocesses for the exact count
  (`:619-639, 234-301`). It also fails on a drop of more than 10% against the baseline's per-directory sum
  ("SUSPICIOUS", `:385-440`), and on fewer than 1000 collected tests.
- **Inputs:** Python AST of `conform/**/test_*.py`. A pytest subprocess with `--backend-config=conform/backends.sqlite.yaml`.
  Baseline `.test-budget-baseline.json` (`total_tests`, `total_skipped`, `by_directory`, `_metadata.signature`;
  the signature covers the first two only, `:96-105`).
- **Needs:**
  - EXISTS static lower bound: `count : { def_name(f, n), match("test.*", n), top-level or method of a "Test.*" class without __init__/__new__ }`.
  - NATIVE external measurement `test_collection(path: symbol, tests: number, skipped: number)`: the exact count needs
    pytest's runtime collection (parametrize, fixtures, plugins, conftest hooks). Datalog over source cannot
    reproduce it. A rule can use the static bound as a sound one-sided check. The exact count is runtime input.
  - NATIVE JSON reader (shared) for the baseline.
  - POLICY signed baseline ratchet (shared), with a **drop tolerance**: a decrease of more than 10% of the observed
    baseline is a failure, not an improvement (`:417-431`). The ceiling and the observed sum differ, since the signed
    ceiling may be higher than `sum(by_directory)`.
  - POLICY gate findings (missing/unsigned/invalid baseline) and LANG `path_finding` (shared).
- **Hard parts:** the static bound counts names, not parametrize expansions, and ignores nested classes and
  inherited test methods. That is why it is only used to fail early. Collection failure raises `CollectionError`,
  which is not a finding.

### migration_progress (metric, 834 lines)
- **Computes:** Regex counts over raw text of `api/`, `impl/`, `conform/` files, in three categories (production,
  conform tests, impl tests). The counts are t-string literals, `string.templatelib` imports, `SqlFragment`,
  `SafeSql(`, `sql(t"`, f-strings with SQL keywords, and string concatenation with SQL keywords. There is also a fixed
  count of the hard-coded `ALLOWED_STATIC_MODULES` constant, and a ratio good/(good+bad). The plugin reports nothing
  and declares no facts (`migration_progress.py:756-768`). The baseline comparison is CLI-only and unsigned
  (`.migration-progress-baseline.json`, `:688-744`).
- **Inputs:** source text of `api/**`, `impl/**`, `conform/**` `.py` files (`:336-363, 453-488`), git HEAD for the
  report (`:436-451`).
- **Needs:**
  - EXISTS every count: `match_all` on the module node with the oracle's regexes, `(?i)` for the IGNORECASE ones,
    `(?m)` for the import pattern. Category from `path_part`.
  - EXISTS publishing: `.output migration_count(category, metric, n)`; the ratio by Soufflé float arithmetic.
  - NATIVE (minor) git HEAD for the report header (`:436-451`). It is display only and can be dropped under the loose
    contract.
- **Hard parts:** `T_STRING_PATTERN` uses a lookbehind (`(?<!["'\\(])\bt["']\w`), so the regex engine must support
  lookbehind. Rust's `regex` crate does not; `fancy-regex` does. A file is counted only when a cheap substring
  pre-test passes (`:377-410`); this never changes a count. The fixed allowlist count is a constant, not a fact.

### grammar_class_survey (metric, 12 lines → `survey_grammar_classes.py`, 380 lines)
- **Computes:** An inventory of classes in `impl/common/l2100_sql_grammar/`: totals, renderable vs support, unique
  names, duplicate-name groups, dead candidates, `_Deferred*`/`_Gated*`, small wrappers, per-bucket counts. The dead
  candidates are classes defined once and referenced zero times across 24 scan roots
  (`find_dead_grammar_classes.py:25-49, 78-125`). The CLI `--check` compares the counts with a signed per-key ratchet
  baseline `.ratchet-baselines/survey_grammar_classes.json` (`survey_grammar_classes.py:304-375`,
  `lint_lib/ratchet.py:138-200`). The suite plugin reports nothing and declares no facts (`:310-321`).
- **Inputs:** Python AST of the grammar directory, tests excluded, and of the reference scan roots (tests excluded by
  default, `find_dead_grammar_classes.py:146-160`). The signed ratchet baseline (JSON).
- **Needs:**
  - EXISTS definitions, buckets (`path_part` after `l2100_sql_grammar`), bases by last name (subscript bases by their
    value, `find_dead_grammar_classes.py:186-205`), constructor parameters (`param_kind`, shared LIB; `args.args[1:]`
    drops `self`, `survey_grammar_classes.py:104-117`), `render` method present.
  - EXISTS reference counting (`find_dead_grammar_classes.py:78-125`): `Load` names (`expr_ctx`, shared LIB),
    attribute names, `from … import` alias names, any string constant equal to a class name. References from inside
    the class's own body to its own name do not count. `count : {…} = 0`.
  - EXISTS `.output grammar_survey(key, n)`.
  - POLICY signed per-key ratchet baseline (shared with ignore_counts). For keys missing from the baseline, the
    previous value is 0 (`ratchet.py:198`).
  - NATIVE JSON reader (shared).
- **Hard parts:** the reference scan is *name-based*: any string `"SelectStmt"` anywhere keeps the class alive, and
  test files do not count. Constructor-shape groups are keyed by ordered tuples of bases and parameters, but they
  are printed only, not ratcheted, so no ordered string aggregation is required.

### tstring_syntax (metric, 167 lines)
- **Computes:** Intended to count PEP 750 t-string literals (`ast.TemplateStr`) in every `.py` file outside
  `browser_build/`. As written, it reports **nothing**. `count_tstrings_in_file` raises `RuntimeError` right after
  reading the file, and the bare `except Exception: pass` swallows it (`lint_tstring_syntax.py:42-51`). Under
  Python < 3.14 it exits 2 before scanning (`:24-28`). It has no plugin: the registry lists it as `metric-only`
  (`lint_lib/architectural_rules.py:1006-1008`).
- **Inputs:** every `.py` file in the repository (`:81-95`).
- **Needs:**
  - EXISTS the intended check: `kind(s, "string")`, its `string_start` token text matching `(?i)[rbuf]*t["'].*`,
    `.output tstring(d, s)`. This assumes the pinned tree-sitter-python grammar parses the `t` prefix. If it does not,
    it is a grammar update, not a language need.
- **Hard parts:** matching the oracle exactly means reporting zero. A port should record that the oracle is broken, not
  copy that behaviour.

### lazy_result_usage (hard-fail, 527 lines; candidate JSON exists)
- **Computes:** Public methods named `execute`, `first`, `one`, `one_or_none`, `select_one`, … inside classes whose
  name contains `Transaction`/`Query`/`Table`. Excluded: skip-listed classes, names starting (after `_`) with
  `Async`/`Lazy`, and methods whose return annotation text lacks `LazyResult`/`LazySeq`/`DeferredFuture`. Also exempt
  are compliant wrappers: a `*_lazy` sibling, a `legacy_sync`/`legacy_async` decorator, a reference to the lazy name
  anywhere in the def, a protocol/ABC class, `@abstractmethod`, or a `...`/`pass` body (`lint_lazy_result_usage.py:160-238`).
  **All** violations are reported only when their number exceeds the baseline total (`:356-375`).
- **Inputs:** Python AST of `data_layer_production_dirs`, skipping paths containing `tests/`, `test_`, `__pycache__`,
  `l2055_lazy_result/`, `l2070_batching/` (`:119-125, 271-274`). Unsigned baseline `scripts/lazy_result_baseline.json`
  (`{"total": N, "files": {...}}`, `:61, 287-293`).
- **Needs:**
  - EXISTS the detector: `def_name`, `nearest(f, c, "class_definition")`, which is the innermost class at *any* depth,
    as the oracle's class stack at function exit (`:148-159, 203-206`); `decorator` + last name; `within` for the lazy
    reference; return annotation `text` + `contains`.
  - LIB `body_stmt` (shared): placeholder body = exactly one statement, `...` or `pass` (`:231-238`).
  - EXISTS `.budget * { "total": N }`: all-or-nothing reporting past a total, which is the oracle's rule.
    The candidate JSON is `mode: block` with no budget, so it reports every violation even when the total is
    within budget. That is its main gap.
  - POLICY (minor) the budget comes from an unsigned JSON file the CLI rewrites (`--update-baseline`, `:294-303`), not
    from the policy text. With `.budget` in the policy, updating it is a policy edit. This is acceptable, but it is a
    change of workflow.
- **Hard parts:** skip patterns are substring tests on the relative path string. `test_` matches anywhere, e.g.
  `latest_tools/`. `ast.unparse` of the return annotation normalises quotes: `"LazyResult[int]"` as a string
  annotation still contains `LazyResult`, so `text` gives the same answer. A method with no annotation reports
  return type `None`.

### imperative_private_functions (metric, 365 lines; candidate JSON exists)
- **Computes:** Taint over the shared symbol graph (`lint_lib/symbol_graph.py`). The seeds are functions with
  execution hits (calls named in `EXECUTION_NAMES` or starting `execute_grammar`/`grammar_fetch`), reason
  `direct_execution`. Other seeds are private non-abstract, non-property, non-legacy functions with no lazy signature
  in an "architectural scope" (path hints, qualname hints, lazy/connection annotation names), reason `eager_signature`.
  Taint then propagates to every caller through the graph's *declared-reference* edges, reason `calls_imperative`
  (`lint_imperative_private_functions.py:111-180`). The finding code is the reason, and severity is metric.
- **Inputs:** Python AST of `api/` plus `data_layer_production_dirs`, tests excluded (`lint_symbol_graph_plugins.py:63-65`).
- **Needs:**
  - LIB `declared_ref(f: node, g: node)`: the symbol graph's call/reference edges (`symbol_graph.py:1908-2260`,
    `_resolve_edges`, `_resolve_ref`, `_resolve_self_attr`, `_resolve_super_attr`, `_resolve_via_param_type`,
    `_resolve_module_attr`, `_add_legacy_decorator_edges`). These are *declaration* semantics: a function referenced as
    a callback or returned counts, as does a method reached through a parameter's annotated type, a `self.x`
    attribute's annotated or `__init__`-assigned type, `super()`, breadth-first bases, and re-exports. Bodies are
    composed from `ref_site`, `sym_of`, `resolve_export`, `mro_method`, `assigned_attr`, `annotation` and `params` +
    annotation types. `calls.call_target` is value-flow based and does not follow parameter annotations.
  - LIB `param_type(f: node, x: symbol, cq: symbol)`: the class an annotated parameter names
    (`_resolve_via_param_type`, `symbol_graph.py:2190-2232`). It is part of `declared_ref` but reusable.
  - EXISTS seeds and propagation: `tainted(g) :- seed(g). tainted(f) :- declared_ref(f, g), tainted(g).`. Reason is
    seed reason, else `calls_imperative`. `is_private` (`symbol_graph.py:236-241`): dunder names are not private; a
    method of a `_Class` is. Decorator text endings are matched with `match(".*(property|\\.setter|\\.deleter)", …)`.
  - EXISTS annotation-name lowercase tests with `(?i)` regexes; path hints with `doc` path `contains`.
- **Hard parts:** the `detail` ("calls X") depends on BFS order (`:173-179`). It is message only and not part of the
  key. The candidate needed a native `reference_edges` table and a SQL `relational` clause
  (Data-layer-rule-expressivity `docs/data-layer-batch-five.md`, "Declared references"). In `.rr` that is plain
  recursive Datalog over `declared_ref`, so no new construct is needed. The batch notes say the graph *deliberately
  ignores* assignment aliases, quoted receiver annotations, positional-only annotations, conditional field annotations
  and indirect module re-exports. `declared_ref` must reproduce those omissions, not resolve more.

### internal_dead_code (metric, 1024 lines; candidate JSON exists)
- **Computes:** Reachability over the same symbol graph. Roots are the exports of `data_layer/*.py`, literal dynamic
  imports `import_module("m").attr` found by regex in source text, and module-level references of every imported
  module. Edges are calls, class uses, public methods and dunders of reached classes, bases and subclasses, subclass
  overrides, protocol-conforming methods, convention-named siblings, and annotation classes
  (`symbol_graph.py:2569-2605`). Then comes a second expansion through annotation names that match exactly one class
  in the project (`lint_internal_dead_code.py:252-275`). Unreached functions and classes are filtered by many
  exemptions (abstract, property, protocol, public API module, dunder, validator, overload, `main`, registered-callback
  names, `_lower_*`, abstract-base implementations, imported anywhere). Only the `high_confidence_*` categories are
  reported, intents/grammar/other split by name and base heuristics (`lint_symbol_graph_plugins.py:110-118`), minus
  `impl/browser/`.
- **Inputs:** Python AST and raw text of `api/` plus `data_layer_production_dirs`, tests excluded.
- **Needs:**
  - LIB `declared_ref` (shared with imperative_private_functions), plus LIB `dl_reach(s: node)` for the edge kinds
    above: `public_methods_of_class`, `_subclass_override_methods`, `_protocol_conforming_methods`,
    `_convention_named_siblings` (`symbol_graph.py:2364-2520`), as recursive rules over `class_def`, `class_base`,
    `subclass`, `method`, `mro_method`.
  - EXISTS roots: `dunder_all`/`public_name` of `data_layer/*.py`. Dynamic-import roots are
    `match_all("(?:\\bimport_module|\\bload_python_module|\\bimportlib\\.import_module)\\(\\s*['\"](?<m>[^'\"]+)['\"]\\s*\\)\\.(?<a>[A-Za-z_][A-Za-z0-9_]*)", text)`
    on module text, then `resolve_export(@dot(m, a), q)` (`lint_internal_dead_code.py:84-150`).
  - EXISTS unique-name annotation expansion: `count : { class_def(_, q), split_dot(q, _, n) } = 1` over a
    non-recursive relation, inside the recursive expansion. This is stratifiable because the count is over class
    definitions only.
  - EXISTS exemption predicates: decorator text endings, name patterns, `is_abstract`, which needs a definition
    (abstractmethod decorator or ABC base, as `SymbolInfo.is_abstract` sets it).
  - EXISTS two categories per class at one span: the finding key includes `category`.
- **Hard parts:** the production plugin adds dynamic-import roots, but standalone `analyze_dead_code` does not
  (batch-five notes, "Dynamic-import probes disagreed"). The `.rr` port must follow the plugin. Module-level
  references are expanded once per module the first time any of its symbols is reached, plus for every module imported
  anywhere. That is monotone, so recursion gives the same fixpoint. "Imported anywhere" is by fully-qualified name
  from `imported_symbols`, so a symbol imported only under `TYPE_CHECKING` still counts.

## Summary table

Needs that are the same thing under different names are merged into one row.

| tag | signature or construct | checks that need it |
|---|---|---|
| LANG | `path_finding`: a finding at (repository path, line) with no node, for allowlist directories, baseline JSON files and policy-level messages | pure_lazy_runtime_purity, cached_pure_lazy_allowlist, route_guardrail_policy, ignore_counts, test_budget |
| LIB | `param_kind(f, x, k)`, with k in posonly, pos, vararg, kwonly, kwarg | authentication_plan_purity, pure_lazy_runtime_purity, cached_pure_lazy_allowlist, bootstrap_engine_invariants, grammar_class_survey |
| POLICY | Gate findings: an invalid signature or a missing/unsigned baseline is a finding. It either replaces all other findings (rtt_counting_workarounds, ignore_counts, test_budget) or is reported alongside them (route_guardrail_policy, pure_lazy_runtime_purity) | rtt_counting_workarounds, route_guardrail_policy, pure_lazy_runtime_purity, ignore_counts, test_budget |
| LIB | `expr_ctx(n, c)`, with c in load, store, del (CPython `ast` context) | pure_lazy_runtime_purity, cached_pure_lazy_allowlist, connection_service_architecture, grammar_class_survey |
| POLICY | Signed numeric baseline ratchet over derived counts, not findings: per-key ceilings from a signed JSON file, decreases allowed, with an optional drop tolerance (test_budget: a drop of more than 10% fails) | ignore_counts, test_budget, grammar_class_survey |
| NATIVE | JSON reader `json_value(path, key, v)` for baseline files | ignore_counts, test_budget, grammar_class_survey |
| LIB | `body_stmt(b, s, i)`: block statements without comments, re-indexed | connection_service_architecture, lazy_result_usage |
| LIB | `declared_ref(f, g)`: the symbol-graph declaration edges (callbacks, returned functions, re-exports, `self`/`super`/base BFS, parameter-annotation receivers via `param_type(f, x, cq)`, field types), with the oracle's omissions kept | imperative_private_functions, internal_dead_code |
| NATIVE | `strict_allowlist(dir, fragment, line, entry)`: signed fragment-directory reader with line numbers (multiset entries) | pure_lazy_runtime_purity, route_guardrail_policy |
| POLICY | Burn-down signing: allowlist may only shrink, IDs are permanent, limited field edits | pure_lazy_runtime_purity, route_guardrail_policy |
| POLICY | Parse failure is a finding (RTA000, APP000) | readthrough_authority_architecture, authentication_plan_purity |
| NATIVE | `@sha256(s)` (subsumed for cached_pure_lazy_allowlist by `@tree_digest`) | rtt_counting_workarounds, cached_pure_lazy_allowlist |
| NATIVE | `@tree_digest(n)`: structural hash of a subtree, the equivalent of `ast.dump` | cached_pure_lazy_allowlist |
| NATIVE | `@json_str(s)`: JSON string quoting with `ensure_ascii` escapes | cached_pure_lazy_allowlist |
| NATIVE | `signed_rows(name, version, sig, rows)`: signature over derived rows | cached_pure_lazy_allowlist |
| NATIVE | NFKC identifier normalisation (minor) | cached_pure_lazy_allowlist |
| LIB | `py_qualname(f, q)`: `__qualname__` with `<locals>`, only for defs reachable through bodies | cached_pure_lazy_allowlist (rtt_counting_workarounds composes the same reachability from `nearest`) |
| LIB | `dl_reach(s)`: dead-code reachability (public methods of reached classes, subclass overrides, protocol-conforming methods, convention siblings, unique-name annotation expansion) | internal_dead_code |
| NATIVE | `line_count(d, n)`: physical lines of a file (`splitlines`) | connection_structural_boundaries |
| NATIVE | `test_collection(path, tests, skipped)`: pytest's runtime collection count | test_budget |
| NATIVE | git HEAD (minor, display only) | migration_progress |
| POLICY | Budget total read from an unsigned, CLI-rewritten baseline file instead of the policy (minor; `.budget * {"total": N}` covers the meaning) | lazy_result_usage |

Checks with no non-EXISTS need: rtt_assertion_integrity, tstring_syntax (its oracle is broken and reports nothing).
migration_progress needs only the minor git HEAD.
