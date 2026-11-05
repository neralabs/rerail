# Group 6: what .rr lacks for ten data-layer checks

Oracle: scripts at revision 2b1504ad. Cites are `script.py:LINE` (a bare `:LINE` means the section's own script).

### matching_architecture (hard-fail, 1268 lines)
- **Computes:** 41 codes (MA001–MA041) over generic-matching code:
  forbidden imports between matching layers, application/facade/conformance files, backend switches, raw SQL,
  dynamic imports, runtime fallbacks, unfrozen/open/Protocol contracts, synthesized counters, and two
  repository-level presence checks (MA033, MA034). Each finding is (code, path, line, symbol, fixed message).
- **Inputs:** Python AST and raw source text of `data_layer/**`, `impl/common/**`, `conform/l5250_matching/**`,
  `l1470_matching_contracts/**`, `l5250_matching/**`, `l[0-9]{4}_*/**` (no `tests` part), plus any
  `--application-root` dirs (`lint_matching_architecture.py:1154-1183`); raw text of
  `data_layer_testing/matching_operator.py` (`:1196-1197`), which is outside that scope. CI runs `--check` only
  (`lint_all.py:1477-1484`); the phase gates run `--check --phase N`.
- **Needs:**
  - `EXISTS doc, path_part, file_name, match` file classes: application (`:166-168`, `"applications" in parts`),
    matching production (`:171-173`), public facade (`:176-177`), conformance prefix (`:258`), lower layer = digits
    of the first `l\d{4}_` path part via a regex on the whole path (`:220-225`, as layer-violations does).
  - `EXISTS text, contains` scan gate: a file is scanned only if it is a matching path, or its raw text contains
    one of 7 markers, or it is in a lower layer and matches `def match(` by regex (`:1141-1146`). `text(modnode)`
    gives the whole file. Rules need a `scanned(d)` relation built this way, because MA034's file must be in
    `.scope` but must not be scanned.
  - `EXISTS import_stmt, child, text` imported modules (`:160-163`): `from X import` gives X without leading dots
    (relative level ignored), `import a, b` gives each name. Do not use `import_alias`'s resolved module.
  - `EXISTS kind, field, within, min` "one finding per file at the min line" codes (MA001, MA002, MA026, MA027,
    MA021) and "first offending node in function" codes (MA039, MA040, MA029, MA022, MA023).
  - `EXISTS nearest, def_name` symbol `Class.func`: `class_stack[-1]` is the nearest enclosing class at any depth,
    through functions (`:254-256`, `:846`). Same as `scope_name`.
  - `LIB name_or_attr(n: node, x: symbol)`: n is a Python `ast.Name` with id x (an identifier in expression
    position: not a def/class/param/keyword/import-alias/global name) or an attribute whose attr is x. Body:
    `nameref` minus identifiers under `import_*_statement`, `parameters`, `global_statement`, `nonlocal_statement`;
    plus `attr_of(n, _, x)`. `nameref` today includes import names and parameters. Used by MA001/MA026
    (`:265-268`, `:647-651`), MA035 (`:415-417`), MA038 (`:431`), MA024 (`:944-947`), contract usage (`:198-204`).
  - `LIB ann_name(n: node, s: symbol)`: the oracle's `_annotation_name` (`:107-119`): Name gives id, Attribute gives
    dotted prefix + attr, `A | B` gives "a|b", Subscript gives its value's name, anything else "". Recursive with
    `cat`. Built from the tree, not `text(n)`, which keeps whitespace, comments and parens. Used by MA004, MA005,
    MA009, MA012–MA014, MA017, MA021, MA030.
  - `LIB unparen(n: node, m: node)`: m is n with any `parenthesized_expression` wrappers removed. The Python AST
    has no paren nodes; tree-sitter does. Needed wherever the oracle tests `isinstance(x, ast.BinOp)` etc. on an
    annotation, a counter argument or a tuple element (`:122-131`, `:578-584`, `:912-916`).
  - `LIB static_str(n: node, s: symbol)`: the oracle's `_static_string` (`:228-238`): a str constant (implicit
    concatenation included), `a + b` of two static strings (`cat`), an f-string with no interpolation. No name
    following (so not `static_value`). Used by MA002 (`:288-292`) and MA011 (`:624-632`).
  - `LIB ast_tuple_elt(t: node, i: number, e: node)`: Python `ast.Tuple` elements: `tuple`, bare `expression_list`
    (`return a, b`), `pattern_list`, and the multi-index of a subscript (`x[a, b]`), which tree-sitter does not give
    a tuple node. MA032 (`:442-445`).
  - `LIB assign_target(a: node, t: node)`, `assign_value(a: node, v: node)`: Python's `Assign.targets` for chained
    `a = b = v` (tree-sitter nests a second assignment as the right side), and `AnnAssign` with an optional value.
    MA031/MA037 (`:313-373`), MA014's "exactly one target" (`:145`), `predicted_counter_names` (`:298-308`).
  - `LIB class_field(c: node, name: symbol, ann: node, st: node)`: a direct body statement of class c that is an
    annotated assignment with an identifier target. MA009, MA020, MA030 (`:763-845`).
  - `LIB fstr_literal(n: node, s: symbol)`: n is an f-string (or an implicit concatenation containing one); s is
    its literal parts concatenated in order, interpolations dropped. Recursive `cat` over `child` index. MA007
    (`:635-648`) then `match("(?is)\\s*(SELECT|INSERT|UPDATE|DELETE|CREATE|ALTER).*", s)`.
  - `NATIVE value(piece, v)` for f-string literal pieces (`string_content` / escape children of an interpolated
    string). Core's `value` is defined for literals; whether it covers pieces of an f-string is unstated. Unescaping
    is lexing, not logic. Needed so that `f"\nSELECT {x}"` lstrips to SELECT (`:637-640`).
  - `EXISTS call_arg` (positional) / `field(…, "arguments")` first positional argument for MA031 (`:575`) and
    MA011's second positional argument (`:620-623`). Mind open issue 8: `posarg` counts keyword arguments.
  - `LIB match_delegate(fname: symbol, i: number)`: a function in the same file whose call `match*(…, p, …)` passes
    its own i-th positional parameter `p` as the 2nd argument (`:241-252`); then a call to `fname(…)` with a static
    string at position i is MA011 (`:622-625`). Keyed by bare function name, file-local, any nesting.
  - `EXISTS match` uppercase test `id.upper() == id` (`:582`): `match("[^\\p{Ll}]*", id)`.
  - `LANG finding at a path`: MA033 is reported at `l5250_matching/maintenance.py:1` (`:1185-1194`), a fixed path
    the rule names, which may not be a parsed document (it does not exist at this revision). `.violation` takes a
    node or span. A rule needs `at` to be (path, line):
    `.violation ma033(at: file_line) "…"`
    `ma033(@at_file("l5250_matching/maintenance.py", 1)) :- owner_doc(_), count : { owner_doc(d), has_policy(d) } = 0.`
  - `EXISTS text(modnode), contains` MA034 (`:1195-1205`): put the testing file in `.scope` (excluded from
    `scanned`), anchor at its module node (line 1).
  - `POLICY option-defined scope`: `--application-root` (repeatable) adds every non-test `*.py` under each root to
    the scan and marks them application files (`:1162-1172`); a missing root is a hard error, not a finding
    (`:1167-1169`). `option` can carry the roots, but `.scope` is static; the scope must be extendable from an option.
  - `POLICY code subset by option`: `--phase N` reports only the codes of phase N (`:1243-1251`), used by the
    `run_matching_phase*_gate.py` scripts. Expressible as `phase_code(p, code)` facts if a rule can test a
    finding's code, which `.violation` does not expose; otherwise a selector over violation names.
- **Hard parts:**
  - Every per-function code (MA028, MA023, MA025, MA024, MA039, MA040, MA029, MA022) walks the whole function
    including nested defs, so a call in an inner function is reported once per enclosing function, each with that
    function's symbol (`:850-937`). A rule keys findings on (call, enclosing function), not on the call.
  - "First" selections follow Python AST pre-order (`_walk_nodes`, field order), not source order: decorators are
    visited after the body, `a if c else b` visits c first. MA018's `.get(x, d)` (`:999-1012`) and MA017's first
    `str` parameter (`:986-990`) report that node's line; min source offset is the near-equivalent.
  - MA031's `predicted_counter_names` is file-wide (any scope) names assigned from a value containing a binary
    operator and a name `pair_evaluations`/`expression_node_count` (`:298-308`).
  - MA014 decides on the *first* direct `model_config = ConfigDict(...)` with exactly one target; `frozen` must be the
    constant `True` (`:142-154`); an annotated `model_config: ConfigDict = …` does not count.
  - MA009's "unsafe" is Name-only for the bare set (an Attribute's dotted name never matches), recursive through
    `|`, with a `None` side unsafe (`:122-132`). Subscripts (`list[int]`) and string annotations are safe.
  - MA004 matches handlers syntactically (`ann_name`, no tuples, no resolution), so `names.rr`'s `catches` is the
    wrong relation (`:652-668`).
  - MA027's first argument is any `ast.Constant` (numbers, None, bytes) or f-string (`:661-664`).
  - The text-marker gate is raw text: a marker in a comment makes a file scanned.

### impl_direct_execution (hard-fail, 659 lines)
- **Computes:** one `direct_execution` finding per source line that calls `._execute_direct_internal(`,
  `._execute_direct_internal_async(`, `._fetchone(`, `._fetchall(`, `.fetchone(`, `.fetchall(`,
  `.execute_grammar_fetchone(` or `.execute_grammar_fetchall(` in a production file that is not allowlisted.
  Files under `impl/*/fts/` are allowlisted, but they are still checked for the two `_execute_direct_internal*`
  names. Ratchet count `violations` (baseline 0, so in effect a zero gate).
- **Inputs:** Python AST and raw line text of `impl/`, `data_layer/`, `_internal/` and every top-level
  `l\d{4}_.+` directory (`lint_lib/discovery.py:28-41`). The CLI scan (`scan_directories`,
  `lint_impl_direct_execution.py:429`) includes tests, but the allowlist regexes exempt `impl/*/tests/` and
  `impl/**/test_*.py`. Allowlist `ALLOWED_FILES` with its signature in the script (`:196-248`), checked by
  `lint_signing.verify_allowlist` (`:580`). Baseline `.ratchet-baselines/lint_impl_direct_execution.json`.
- **Needs:**
  - `EXISTS` ``c ~ `$O.$M($$$A)` ``, M in a fact set of the eight names; `doc`, `match` for the
    exact-path, `endswith("/" + allowed)` and `(^|/)pattern` path tests (`:279-293`).
  - `EXISTS signed_list`: `ALLOWED_FILES` moves to a signed file. Today the signature sits in the script (`:248`).
  - `EXISTS` `.scope include` globs: `l[0-9][0-9][0-9][0-9]_*/**/*.py` plus the three base directories
    (`discovery.py:29`), provided `.scope` globs support character classes.
  - `EXISTS` `.budget * { "total": 0 }` for the ratchet (`:645-651`).
  - `EXISTS min` aggregate, keyed by line. The oracle reports at most one finding per line: `_reported_lines`
    (`:389-395`). If two calls sit on one line, the violation keeps the call with the smallest node id:
    `c = min c2 : { dexec(c2), line(c2, l) }`.
- **Hard parts:** two detectors share one per-line dedup set. (1) A text scan (`:372-381`) runs over each line
  with string and comment tokens blanked out (`:111-145`), and skips lines whose first non-blank character is
  `#`. (2) An AST pass (`:364-370`) reports at `Call.lineno`, which is the line where the *call expression*
  starts. When a call is split across lines (`cursor\n    .fetchall()`), the text scan reports the line of
  `.fetchall(` and the AST pass reports the line of `cursor`, so the oracle gives **two** findings for one call.
  To match, the rule needs two violations: one at the call node's line, and one at the line of the attribute
  name when the `(` follows the name with no space between them (`end` of the attribute equals `start` of the
  argument list). The text scan never fires for `x.fetchone (` with a space. The AST pass does. The
  message contains the *regex source* (`\._fetchone\(`) of the first pattern in list order that matched
  (`:148-157`). That does not matter under the loose key.

### render_raw_strings (ratchet, 749 lines)
- **Computes:** inside an outermost `render`/`render_*` function in `impl/common/l2100_sql_grammar/**`, it reports
  three cases. `template_call`: `Template("…")` with a str literal as the first positional argument.
  `static_call`: `SqlFragment.static("…")`. `string_literal`: every other str constant whose decoded value is
  longer than 1, except docstrings, strings anywhere under `raise`/`assert`, and direct positional arguments of
  15 allowed callee names. Ratchet `violations` (baseline 0).
- **Inputs:** Python AST of `impl/common/l2100_sql_grammar/**/*.py`, tests included (`lint_render_raw_strings.py:427`,
  `:558-559`), skipping `__pycache__`. Baseline `.ratchet-baselines/lint_render_raw_strings.json`.
- **Needs:**
  - `EXISTS` `def_name`, `nearest(n, f, fdef)`, `within`, `kind`: render context is "n is within a render def
    that has no render def above it" (`:213-223`: the stack only pushes when it is empty). Nested non-render
    functions inside a render body are still scanned. The reported class is the innermost class enclosing that
    outermost render def (`:220`).
  - `EXISTS call_arg` (python/calls) for the Template/static tests and the allowed-argument test, plus
    `min` over positional indices for "first positional argument" (`:297`, `:306`).
  - `NATIVE py_str_part(lit: node, i: number, v: symbol, at: loc, in_spec: number)`: the str constants that
    CPython's `ast` makes for a string expression. That means one value for a plain or implicitly concatenated
    literal (adjacent pieces merged); each literal run between interpolations of an f-string (merged across
    adjacent pieces of a concatenation); and each literal part of a format spec (`f"{x:>10}"` gives a constant
    `">10"`), with `in_spec = 1`. Bytes literals give no row. `v` is decoded, as `value` decodes.
    `on_Constant` (`:196-211`) visits exactly these constants and filters on `len(node.value) <= 1` (`:200`).
    Datalog cannot derive this: tree-sitter splits `string_content` around escape sequences and interpolations,
    and nothing in the tape merges adjacent pieces or decodes partial contents. The batch-four root-cause note
    (`pythonFString`) found the same gap.
  - `LIB ast_parent(n: node, p: node)`: the CPython-AST parent, skipping tree-sitter-only wrappers
    (`parenthesized_expression`, `argument_list`, `expression_statement` around an assignment, …).
    The body is a rule per wrapper kind, then `nearest(n, p, ast_node)` over a `!wrapper(kind)` set.
    `_is_allowed_string_arg` tests `context.parent(node)` is a Call (`:258-259`); in tree-sitter the parent of an
    argument is the `argument_list`.
  - `LIB doc_string(f: node, s: node)`: s is the first statement of f's body and is a bare string expression.
    Body: `field(f,"body",b), child(b,st,0), kind(st,"expression_statement"), child(st,s,0), kind(s,"string")`.
    Only the render def's own docstring is exempt (`:237-242`). The docstring of a nested function is reported.
  - `EXISTS .budget * { "total": 0 }` (`:735-741`).
- **Hard parts:** the keyword branch of the allowed-argument test is dead code. `context.parent` follows
  `ast.iter_child_nodes`, so the parent of a keyword's value is the `ast.keyword`, not the Call
  (`ast_collectors.py:29-32`). The `isinstance(parent, ast.Call)` test (`:259`) therefore rejects it, and
  `x.replace(old="a")` *is* reported. Only positional arguments are allowed. Strings under a raise/assert are
  exempt at any depth (`:250`). Template/static calls are reported even inside raise/assert (`on_Call` has no
  such test), and their first argument is excluded from `string_literal` only when it is a plain Constant
  (`:188-194`). `Template(f"…")` is not a `template_call` (JoinedStr), but its literal parts are each
  `string_literal`. Async defs count as render functions. For multi-line implicit concatenations, the
  finding line is the first piece's line, as CPython reports it.

### rowid_comparisons (ratchet, 793 lines)
- **Computes:** a `rowid_comparison` finding is one comparison operator pair in a `Compare` (chains give one per
  pair). `==`/`!=` and `<`/`<=`/`>`/`>=` need both operands to "look like a RowId". `in`/`not in` needs only the
  left one. `is` is ignored. Ratchet `issues` (baseline 507).
- **Inputs:** Python AST of the production directories. The CLI ratchet run
  (`lint_rowid_comparisons.py:736-740`) includes tests (`include_tests=not args.exclude_tests`), but the plugin
  excludes them (`:659-661`). Baseline `.ratchet-baselines/lint_rowid_comparisons.json` (signed).
- **Needs:**
  - `LIB base_simple(c: node, x: symbol)`: unqualified base names: Name id, Attribute attr, the value of a
    Subscript, recursively (`:391-399`). Body: three rules over `field(c,"superclasses",…)`, `kind`, `text`.
    `class_base` resolves to qualified names, which the oracle does not do.
  - `LIB rowid_type(d: number, x: symbol)`: recursive closure **per file**. The seed is the 11 names plus
    `NodeId`. A class of the same file whose simple base is a RowId type is added, unless the class is
    `CommitTriggerId`/`HandlerId`. `NodeId` itself is removed at the end, but its subclasses are kept
    (`:402-417`, `:485`). The body is a two-rule recursion plus a final filter. Datalog recursion is enough, but
    note the scope: only classes of the same document count (`finish_file` uses `self.class_bases`).
  - `NATIVE op_tok(n: node, s: symbol, start: number)`: anonymous operator tokens with their byte offsets.
    `op(n, s)` has no position, so in `a == b < c` it cannot pair each operator with its operands, and
    `cmp` covers single-operator comparisons only (names.rr:199-203). `check_compare` walks
    `zip(ops, comparators)` (`:285-350`). With offsets, a `LIB cmp_pair(c, i, op, l, r)` takes operands
    `child(c,l,i)`, `child(c,r,i+1)` and the token between them.
  - `LIB type_name(e: node, t: symbol)`: `_type_name` (`:359-368`). Name gives its id. Attribute gives its attr.
    Call and Subscript give the result for their func/value, recursively. Annotation events use it with an empty
    map (`:471`, `:477`), so `Optional[RowId]` gives "Optional" and `RowId[Session]` gives "RowId".
  - `EXISTS max` aggregate, plus `NATIVE py_ast_order(n: node, i: number)` for exact parity only.
    The annotation map is one **flat dict for the whole file**, filled in traversal order with last write wins:
    the events list is replayed in order in `finish_file` (`:486-491`). A comparison sees the latest
    `x: T` / parameter `x: T` event before it anywhere in the file, in any function. Rule:
    `p = max p2 : { ann(d, x, _, p2), p2 < pc }, ann(d, x, t, p)`. Start offsets match the traversal order
    except where CPython's field order differs from text order. Decorators and return annotations are visited
    after the body (FunctionDef `_fields`), and defaults after all parameters. A pre-order index would make
    that exact.
  - `LIB rowid_expr(e: node, d: number)`: `_looks_like_rowid_expr` (`:228-256`). It holds for a Name whose
    current annotation is a RowId type (not the name itself); for an expression whose `type_name` is a RowId type
    (so `RowId(...)` and `x.GraphId` hold, and through Subscript `x[0]` when x is annotated); for a Call whose
    function is an attribute ending in `_id`; for `.new()` on a RowId type name or, recursively, on a RowId
    expression; and for any attribute ending in `_id`. The body is about 7 rules over `kind`, `field`, `text`
    and `match(".*_id", a)`.
  - `EXISTS .budget * { "total": 507 }`. The key is per pair, so the violation sits at the right-hand operand of
    the pair. One `Compare` can give several findings on the same line, and the ratchet counts each one.
- **Hard parts:** the annotation map is not scoped. `def f(x: RowId)` early in a file makes `x == y` in an
  unrelated later function a RowId operand. A later `x: int` (any non-None type name) overwrites it. A string
  annotation (`x: "RowId"`) gives `None`, so it records no event and leaves the old entry in place (`:472`).
  Only `ast.AnnAssign` with a Name target and `ast.arg` record events. Plain assignments never do. For a
  `Name` operand the oracle consults only the map (`:230-232`), but for a `Call` it falls back to the bare
  name (`_type_name` default, `:361`). The RowId closure is per file, so a subclass defined in another module
  is not a RowId type there.

### security_patterns (ratchet, 815 lines)
- **Computes:** SEC001 (`*.import_module(...)` always, and `__import__(non-literal)`); SEC002
  (`SessionId.system()` in a path containing `api/`, unless one of seven exemptions holds); SEC003
  (`.extractall(...)`); SEC004 (bare `open(...)` with a write/append mode, severity WARNING); SEC005
  (`SqlLiteral(non-literal)`); SEC007 (line regex `exec(`/`eval(` outside comment lines and test trees); SEC008
  (line contains `pickle.load`, WARNING); SEC009 (line regex `yaml\.load\s*\([^)]*\)` without
  `SafeLoader`/`safe_load` on the line). Each AST rule honours a `# noqa: SEC00N` or `# noqa: SEC` marker on the
  call's start line. Ratchet counts `errors` and `warnings` separately (both 0).
- **Inputs:** Python AST plus raw physical lines of every `*.py` under the default scan directories, tests
  included (`iter_python_files`, `lint_security_patterns.py:711-721`). Four path-substring allowlists with one
  combined signature in the script (`:123-186`). Baseline `.ratchet-baselines/lint_security_patterns.json`.
- **Needs:**
  - `EXISTS` pattern atoms, `call_arg`, `str`, `contains`, `min` for SEC001/003/004/005. SEC004 takes
    the first str constant among positional arguments 1 and later, and a `mode=` keyword overrides it
    (`:470-480`). SEC001's `import_module` is any attribute call, not only `importlib.` (`:358-359`).
  - `EXISTS signed_list` for the four lists (one signed file per list, or one file with a list-name column).
    Path matching is **substring** on the repo-relative path (`_path_matches`, `:329-334`), so `"tests/"` also
    matches `integration_tests/`. Use `contains(allowed, path)`.
  - `LIB ast_parent(n, p)` (as in render_raw_strings) for SEC002's immediate-parent exemptions:
    `or`/`IfExp` parent (`:243-249`), `yield` parent (`:279-283`), keyword-argument parent whose name contains
    "session" (`:292-297`), attribute parent `.unwrap_to_sql` (`:217-218`). The oracle stops after the first
    ancestor, so `(a or SessionId.system())` in tree-sitter has a `parenthesized_expression` in between that
    ast does not.
  - `LIB ast_stmt(n: node, s: node)`: the nearest enclosing *CPython statement*, with an
    `expression_statement` around an `assignment` counted as the Assign/AnnAssign itself. Body: a `py_stmt(kind)`
    set over the tree-sitter statement kinds, then `nearest(n, s, py_stmt)` and one remapping rule. Used by
    `_first_statement_ancestor` (`:299-303`) for the "comparison variable" and "entry-point assignment"
    exemptions (`:220-275`), which read `targets[0]`, the *leftmost* target of a chained `a = b = …`.
  - `LIB ast_constant(n: node)`: CPython `ast.Constant`. That covers string/concatenated strings with no
    f-string piece, integer, float, true/false/none, ellipsis; not negative numbers (UnaryOp) or f-strings.
    Body: a kind set plus a `!` test for an `interpolation` child. Used by every "non-literal argument" test
    (`:361`, `:378`, `:492`).
  - `NATIVE doc_text(d: number, s: span)` (the whole file's bytes as a span; the serverplan_quality need), then
    `LIB source_line(d: number, l: number, s: span)`: `match_all("(?m)^(?<s>[^\\n]*)$", t)` over it gives one span
    per physical line, with `line`. SEC007-009 are per-line tests over raw text (`check_raw_patterns_source`,
    `:548-609`), and they include docstring lines and lines outside any node. The module node's text cannot stand in,
    because it need not start at byte 0. SEC007's "first non-blank character is `#`" test (`:565`) needs the whole
    line. `str.splitlines` also breaks on `\v \f \x1c-\x1e \x85 \u2028 \u2029`, which the regex should list.
  - `EXISTS` comment nodes + `line` + `contains` for the `# noqa: SEC…` suppression (`:313-317`). The oracle
    tests the raw line, so a marker inside a string literal on that line also suppresses. With comment
    nodes, that case is lost.
  - `POLICY` per-code severity (`SEC004`, `SEC008` are WARNING, the rest ERROR, `:310`, `:486`, `:591`). The
    ratchet budget is per severity bucket (`:802-805`), so `.budget` needs a grouping of codes into buckets:
    `.budget * by severity { "error": 0, "warning": 0 }`. While both are 0, `{"total": 0}` gives the same result.
  - `POLICY` extraction-failure finding: an unreadable file yields `SEC000` at line 0 (`:510-522`).
- **Hard parts:** SEC007's regex uses look-behind (`(?<!\.)(?<!\w)\bexec\s*\(`, `:567`), which RE2/Rust
  `regex` lack. It can be rewritten as `(^|[^.\w])(exec|eval)\s*\(`. SEC007 skips files whose path parts
  include `tests` or `conform` (`:558-560`); SEC008/009 do not, and none of the three skip strings or
  docstrings. SEC008 is a plain substring (`cpickle.loads`, `pickle.loader` match). SEC002 tests `"api/" in
  str(path)` (`:408`), also a substring. `_is_in_comparison` climbs every ancestor up to the module
  (`:203-209`), through lambdas and defs. SEC005 uses `node.args[0]`, which in ast includes a `Starred`, so
  `SqlLiteral(*xs)` is reported. `posarg` drops splats, so the rule needs a separate splat case. The noqa marker
  string is assembled at run time (`:315`) only so that the script does not count itself.

### serverplan_dispatch_architecture (hard-fail, 875 lines)
- **Computes:** In generic ServerPlan directories plus `base_server_plan.py`: SPD001 backend-family attribute or
  backend-identity comparison; SPD002 `isinstance`/`type(x) ==`/`case Cls()` on concrete step types; SPD003
  `type(x).__name__ == "FooStep"`; SPD004 dialect presence (`dialect is None`, truthiness in conditions);
  SPD005 (only in `base_server_plan.py`) any use of an executor-owned value outside a closed API; SPD000 if the
  signed allowlist is non-empty or its signature fails. Zero gate: every finding fails.
- **Inputs:** Python AST of `impl/common/l2065_plan_core`, `l2066_plan_optimizer`, `l2070_server_plan/{optimizers,
  steps,strategies}`, `l7300_sql_versioned_transactions`, `l9700_sql_graph_query`, `l2070_batching`, and
  `impl/common/l2070_server_plan/base_server_plan.py`, tests excluded (`lint_serverplan_dispatch_architecture.py:84-93,
  769-779`); minus `backend_profiles.py`, `backend_traits.py` (`:94-99, 786-787`). Signed allowlist
  `scripts/allowlists/lint_serverplan_dispatch_architecture.txt` with the signature embedded in the script
  (`:59-60`, `:827-836`).
- **Needs:**
  - `EXISTS doc, match` scope and late-boundary exclusions.
  - `EXISTS attr_of` SPD001 family attributes (`:681-682`).
  - `LIB backend_source(n)`, `backend_literal(n)` (`:182-187`, `:235-238`): `backend_type` name, any attribute chain
    ending in or passing through `.backend_type` (recursive on the object), a call of `get_backend_type*` /
    `backend_type_for`; `BackendType.X`, or a string whose casefold is in the set (`match("(?i)(doltdb|…)", v)`).
    SPD001 compare: both found anywhere in the comparison's subtree (`:241-244`, `ast.walk`). Plain rules.
  - `LIB type_names(n, x)` (`:166-175`): terminal names through tuples and `|` (recursive), plus `unparen`;
    `step_type(x)` by suffix/allow/deny sets (`:178-181`). SPD002 for `isinstance` second arg, `type(..) ==` operand,
    `case C():` class pattern (`:263-283`, `:749-751`). SPD003 with `type(x).__name__`/`x.__class__.__name__` operand
    and any string constant in any operand (`:273-282`).
  - `LIB dialect_source(n)`, `dialect_truth(n)` (`:190-226`): `dialect` name/attribute, `getattr(x, "dialect")`;
    truthiness = source, `not source`, `bool(source)`, or a boolean operator with a truthy operand (recursive through
    `boolean_operator` only). Tested on `if`/`while`/ternary/`assert`/`case` guard conditions (`:672-677, 744-760`).
    Compare with a `None` operand anywhere in any context (`:208-214`, `:740-741`).
  - `EXISTS kind "comparison_operator"` for comparisons: Python `Compare` includes chains, so `cmp` (single operator
    only) is the wrong relation.
  - `LIB unparen(n, m)` (see matching_architecture): `if (dialect):` and `isinstance(x, (A))`.
  - `LIB executor_owner(n: node)`: SPD005's provenance fixpoint over one file (`:298-353`, `:457-519`). Seeds: calls
    of `_create_backend_executor`, any callee containing `backend_executor`, executor type names (`BackendExecutor`,
    any `*BackendExecutor` imported, subclasses of one, aliases `X = BackendExecutor`, factories whose return
    annotation is an executor), `cast(<executor annotation>, v)`, parameters annotated as executor or unannotated
    and named `backend`/`executor`, `self.attr` where the attr was ever assigned an owner or is a class-level
    annotated executor field. Propagation: attribute chains, method calls unless the method is in the API set,
    `getattr`/`attrgetter`/`__getattribute__` on an owner, `await`/`*`/unary/walrus/ternary/bool-op/collection
    literals, constant index into tuple/list literals (negative indices) and constant key into dict literals,
    for-loop/comprehension/match targets over owners, nested functions inherit outer owner names. All monotone
    rules over the tree, with `count` for the store count and `match_all("[^|]+", @replace_all(s, " ", ""))` for
    string annotations `"A | B"` (`:302-306`). No new construct.
  - `LIB latest_store(n: node, st: node)`: for a name read n in function f, the textually latest store to the same
    name in f at or before n (`max` over node start). The oracle's `runtime_owner_aliases` stack is updated in
    traversal order, and a non-owner reassignment removes the alias (`:521-533`). The alias set is only consulted
    for names stored more than once in the function (`:316-321`), so `count` of stores picks the flow-sensitive or
    the flow-insensitive answer. Stratifies: `latest_store` sits below the recursive `executor_owner`.
  - `EXISTS within, nearest` SPD005 report sites (`:535-571`, `:679-742`): attribute/subscript stores of an owner,
    returns and yields of an owner name (not inside `_create_backend_executor`), attribute access off an owner not in
    the API, reflective calls on an owner, an owner passed to a non-API plain-name callee, comparisons with an owner
    operand, subscripts of an owner.
  - `EXISTS qualname, nearest` finding scope `".".join(scope)`: the chain of all enclosing class *and* function
    names (`:575-578`); qualname minus the module prefix. Not `scope_name` (class + function only).
  - `EXISTS signed_list, count` SPD000 "allowlist must be empty" (`:827-829`).
  - `POLICY signature failure is a finding`: SPD000 is a hard-fail finding at the allowlist path, line 1, when the
    list is non-empty or `verify_allowlist` fails (`:829-836`), including an "unsigned" signature
    (`signing.py:127-134`). Core says `signed_list` is "refused if the signature fails", which reads as a run
    error, not a reportable finding. The expected signature lives in the linter source (`:60`), so in .rr it must be
    the policy's own data (a signed empty list).
  - `LANG finding at a path` (same as matching_architecture MA033): SPD000 is anchored on a `.txt` file that is
    not a parsed document.
- **Hard parts:**
  - SPD005's alias tracking mixes flow-insensitive facts (computed by the module-wide fixpoint in `on_Module`) with a
    traversal-order alias set: `on_Assign` updates aliases before the assignment's children are visited, so in
    `x = x.y` the read of `x` already sees the new state (`:535-538`). Branches are ignored: textual order only.
  - `_pipeline_runner_args` tuple unpacking marks the second target as owner (`:541-549`); `match` subjects that are
    owners mark capture names (`:745-749`).
  - Finding identity is (code, scope, whitespace-normalised source segment, occurrence), occurrence counted in
    (line, code, expression) order (`:631-647`, `:151-154`). One Assign with two attribute targets adds the same node
    twice (`:537-540`): the findings key must carry the target, or set semantics merges them.
  - SPD002 and SPD003 can both fire on one comparison; each code once per node (`dict.fromkeys`, `:283`).
  - Python `BoolOp` is flat and tree-sitter's `boolean_operator` is binary; the recursive rule gives the same answer.

### serverplan_quality (hard-fail, 4557 lines)

Oracle: `scripts/lint_serverplan_quality.py`, plus `scripts/conditional_architecture_facts.py`, which computes SPQ-029.
The plugin (`serverplan_quality_plugin`, :4428) is the hard-fail path. The CLI (`cli_main`, :4501) is a ratchet over
per-code counts, and it does not compute SPQ-029.

- **Computes:** about 50 independent syntactic tripwires (SPQ-001…053, without 009 and 019). Each reports `(code, path,
  line)` for a forbidden shape in a fixed set of ServerPlan files. Most are name tests on calls, attributes, defs,
  keywords and imports, under file-path gates. Three are project-level:
  - SPQ-049: a function becomes a "payload taxonomy" candidate through a colour count. When the repository has ≥2
    candidates, every candidate is reported.
  - SPQ-029: inheritance closure over `ConditionalStep`, plus "required canonical owner" contracts, which report when
    an owner file or definition is absent.
  - SPQ-053: occurrences of `.ORDERED_SEMANTIC` per (path, class, function), measured against a signed JSON inventory
    of maximum counts.
- **Inputs:**
  - Python AST of these files:
    - `impl/common/l2070_server_plan/*.py`, its `steps/**` and `strategies/**`;
    - `impl/common/l7300_sql_versioned_transactions/steps/**` and `impl/common/l9700_sql_graph_query/steps/**`;
    - `api/l2070_batching/executor.py`;
    - `l2070_batching/{executor,conditional_step,compilation,plan_prerequisites,optimizer}.py`;
    - `impl/common/l2070_server_plan/steps/tests/test_conditional_compile_dialects.py`;
    - the fallback directories `l9750_graphql`, `l2080_lazy_orm`, `l2060_query_builder`, `l9900_auto_api`;
    - the callback scope: every `impl/**` and top-level `lNNNN_*/**` file, and `l2050_transaction/legacy_wrappers.py`.
    - SPQ-029 and SPQ-049 use the broad repository scope, tests included (:362-368).
  - `__init__.py` files are excluded. Paths with a `data_history` or `build` part are excluded. Many rules also skip
    paths with a `tests` part.
  - Whole-file source text: substring gates, and the strategy line count.
  - Signed JSON: `scripts/allowlists/serverplan_ordered_semantic_inventory.json`.
  - Python-embedded signed list: `l2070_batching/production_ir_allowlist.py`, which holds `ALLOWED_PRODUCTION_IR_TYPES`
    and `ALLOWLIST_SIGNATURE`. Its entries are imported at :143-148.
  - The repository public key, in `scripts/lint_lib/signing.py`.
- **Needs:**
  - `EXISTS doc, path_part, file_name, match`: every file and directory gate. Examples: `path in CORE_PLAN_FILES`,
    `is_relative_to(STEP_DIRS)`, `"tests" in parts`, `path.name == "executor.py" and "l9750_graphql" in parts`.
  - `EXISTS kind, field, child, text, attr_of, call_arg, within, within_stop, nearest, def_name, method_of, decorator,
    value, count, min, max`: the bulk of the rules. Examples:
    - SPQ-003, 004, 005, 006, 011, 012, 014-018, 021-027, 032-041, 045-048, 050 (non-annotation parts), 051, 052.
    - The oracle's `_call_name` (identifier, or `attr_of`'s attribute) and "first `return <int>` in a `level()` body"
      (`min` over node ids).
    - `_walk_without_nested_defs` is `within_stop` with a stop set {function_definition, lambda, class_definition}.
    - "≥2 self-calls" and "≥3 dispatching ifs" are `count` aggregates.
    - The SPQ-049 project threshold is a global `count : { candidate(_) } >= 2`.
    - SPQ-053's "k-th occurrence above the limit" is `count : { occ(o, key), o <= n } > limit`.
  - `NATIVE doc_text(d: number, s: loc)`: the whole file's text as a span.
    - Substring gates decide whether a rule runs at all, and several gates are not implied by the rule's own match:
      - `"PlanAnalysis" in source` for SPQ-028 (:4199);
      - the `EXECUTOR_RUNTIME_HELPERS` gate around all of SPQ-012/013/018 (:4187);
      - the build-call/execute token gate for SPQ-030/031, whose `*Intent`/`ServerPlan` path test the tokens do not
        cover (:636, :952);
      - lowercased `"unsupported"`/`"shape"` for SPQ-049 (:610, :4300);
      - `SOURCE_TRIGGERS` before any parse (:556).
    - SPQ-010 (:4171) reports at line 1501 when `len(source.splitlines()) > 1500`. `splitlines` also breaks on
      `\v \f \x1c-\x1e \x85    `. The CLI instead counts `"\n"` (:3789).
    - Native because these read the file's bytes. The module node's text is not guaranteed to cover leading and trailing
      trivia, and core's `text_rule` is a JSON matcher term, not a relation. With `doc_text`, `match("(?is).*shape.*", s)`
      and a 1500-line regex work as written.
  - `NATIVE parse_error(d: number)`: the file is not valid Python. The oracle silently skips files that raise
    `SyntaxError` (:4229-4232, :3973-3976). tree-sitter yields a partial tree, and MISSING nodes have no distinguishing
    kind, so a rule cannot tell from `kind(_, "ERROR")` alone.
  - `LIB ast_name(n: node, x: symbol)`: an identifier in a Python `ast.Name` position.
    - Body: `nameref` minus parameter names, import-statement names, `global`/`nonlocal` names and `except … as` names.
    - SPQ-024 (:2149) and SPQ-045 (:2541) flag Names but not import aliases. The same holds for SPQ-032 (:3108),
      SPQ-038 (:3309), SPQ-051 (:3712) and SPQ-050's shadowing (:3496-3510).
  - `LIB str_const(n: node, v: symbol)`: the Python AST's `Constant(str)` set.
    - It covers a `string` with no interpolation, a `concatenated_string` as one value, and each literal fragment of an
      f-string.
    - Body: `str`/`value` over those kinds, with `match_all` over `string_content` for fragments.
    - Used by: SPQ-049 fail_closed (:803), SPQ-024 word split (:2156), SPQ-042 constants (:3405, :3418), SPQ-047 CTE
      heads (:2866), and annotation strings (:3560).
  - `LIB py_if(i: node)`, `if_test(i, t)`, `if_body(i, s)`, `if_orelse(i, n)`: Python `If` nodes.
    - Covers `if_statement` and each `elif_clause`. orelse holds everything in the later elif/else clauses of the chain.
    - Python makes an `elif` a nested `If` in `orelse`; tree-sitter makes it a sibling clause.
    - Body: clauses of the same `if_statement` ordered by `child` index.
    - Used by:
      - SPQ-030/031: branch containment and the `x is None` test (:2394-2425);
      - SPQ-049 `type_dispatch` (:852);
      - SPQ-035 (:3179), SPQ-047 `_finish_batched_step` (:2933), SPQ-025 (:2201);
      - conditional facts: always-raises (:99-104) and backend controls (:426-431, which also take IfExp and Match
        subjects).
  - `LIB py_assign(st: node, t: node)`, `py_assign_value(st: node, v: node)`: Python `Assign.targets` as a flat list.
    - tree-sitter right-nests `a = b = v`; this flattens it to targets [a, b] and value v.
    - `a, b =` is a Tuple target and `[a, b] =` a List target; AnnAssign may have no value.
    - Used by: SPQ-007 (:1436), SPQ-008 Tuple-only `(x, _)` (:1474), SPQ-016 (:1844), SPQ-028 (:2338-2364), SPQ-042
      (:3405), SPQ-050 (:3661), SPQ-051 single-target rule (:3755), SPQ-052 `len(targets) == 1` (:2629).
  - `LIB call_path(e: node, i: number, s: symbol)`, `dotted_expr(e: node, s: symbol)`: syntactic dotted paths.
    - `call_path` is the oracle's `_call_path`: a Name/Attribute chain that passes through Calls, so `a.b().c` is
      (a, b, c); anything else is the empty path. Recursive over `attr_of` and `field(call, "function")`; index from
      the tail.
    - `dotted_expr` is the conditional facts' `_dotted_name`: the same chain as a dotted string.
    - Used by: tail, receiver `path[-2]`, "any part" tests (:769, :952-972); SPQ-033, 036, 037 (:3154, :3213-3270);
      SPQ-052 (:2636); conditional facts (`calls`, `StepExecutionRole.SQL`, `step.require_sql_physical_conditional`:
      :54-62, :445-450, :489).
  - `LIB base_tail(c: node, s: symbol)`: the oracle's `_base_names`. Each base contributes its own Name, its Attribute's
    last part, or a Subscript's value, all unresolved (:706-717). Used for the `ServerPlanStep`, `Protocol`,
    `PhysicalRegionCodegen` and `PipelineLifecycleEffect` tests. `class_base` resolves names, which the oracle does not.
  - `LIB body_def(b: node, f: node)`: f is a direct statement of class or module body b, looking through
    `decorated_definition`.
    - The oracle uses `cls.body` and `tree.body` (:702, :1193, :1428).
    - `method_of`/`method` are broader: they also take defs nested under an `if` or `try` in the class body.
  - `LIB const_expr_stmt(s: node)`, `body_stmt_count(f: node, k: number)`: expression statements whose expression is a
    literal constant (docstrings, `...`, numbers).
    - Used by: SPQ-001 thin-wrapper test (`<= 3`, :1158), `_is_abstract_method` (:1141-1155), SPQ-002 "first real
      statement" (:1228).
  - `LIB always_raises(b: node, exc: symbol)`: a statement block unconditionally raises `exc`
    (conditional_architecture_facts.py:99-118).
    - Positive recursion over statement index: a `raise exc` at i; or an if-with-else (or a match) whose branches all
      raise; or i is not a return and i+1 raises.
    - "Both branches raise, else continue" is written as a disjunction, so no negation goes through the recursion.
  - `LIB cmp_chain(c: node, k: number, op: symbol, r: node)`: every (operator, comparator) pair of a possibly chained
    comparison. SPQ-035 zips `ops`/`comparators` (:3184-3195). `cmp` drops chained comparisons.
  - `LIB local_fn(d: number, x: symbol, f: node)`, `ncall(f: node, g: node)`: SPQ-020's file-local name-based call
    graph (:1929-1981).
    - `local_fn` takes the last def of each name anywhere in the file (`max` over node id).
    - `ncall` links a call whose `_call_name` is x to `local_fn` x. The oracle's `seen` set makes this plain
      reachability.
    - `call_target` from calls.rr resolves scope-correctly, which the oracle does not.
  - `LIB` annotation resolution for SPQ-050 (:3467-3588), four relations:
    - `ann_import(s, x, origin)`: raw imports owned by scope s (not inside a nested def/class/lambda). Relative imports
      keep their dots: `"." * level + module + "." + name`. `import a.b` gives (a, a); `import a.b as c` gives (c, a.b).
    - `ann_imp(s, x, o)`: s's own imports override the inherited name set. Defs and classes inherit from
      `nearest(s, p, fscope)`.
    - `ann_shadow(s, x)`: Name stores and deletes, the root of an attribute store (`x.y = …` shadows x), def and class
      names, parameters, `except … as` names, `global` anywhere under a module scope and `nonlocal` anywhere under a
      function scope. Inherited shadows minus the scope's own import keys.
    - `ann_part(a, part, trusted)`: recursive over the annotation.
      - A Name/Attribute chain gives `import_target.suffix`, or the raw name when there is no import.
      - A non-Name chain root gives its suffix attributes as untrusted parts, then recurses into the root.
      - A string is parsed and recursed into; an unparsable string is itself a part.
      - A trusted `typing.Literal` keeps the origin only. A trusted `Annotated[T, …]` keeps the origin and T.
      - Anything else recurses into its children.
    - trusted = exactly one origin and not shadowed: `count : { ann_imp(s, x, _) } = 1`, `!ann_shadow(s, x)`.
    - The `names`/`imports` libraries cannot be reused here: they resolve relative imports, move bindings through
      `global`, and give no result for a shadowed name. Here a shadowed name still resolves, only untrusted.
  - `NATIVE parsed(s, "annotation", r)` for any string inside an annotation expression, not only `string_annotation`
    positions. The oracle runs `ast.parse(mode="eval")` on every str constant it visits under the annotation (:3560,
    annotation_ast.py:10). `EXISTS` if `parsed` is already total over strings.
  - `LIB` conditional-architecture facts (conditional_architecture_facts.py):
    - `ca_module(d, m)`: the path from the first part in {impl, l2070_batching, scripts}, else the file stem only
      (:155-166).
    - `ca_alias(d, local, origin)`: the last import wins, file-wide, functions included; `from . import x` is skipped
      and `from .m import Y` gives `m.Y` (:183-197).
    - `ca_base(c, q)`: the base's head replaced through the alias (:213-221).
    - `ca_cond(q)`: the transitive closure from `l2070_batching.conditional_step.ConditionalStep` (:512-524).
    - Inherited from the oracle: a nested function inside a method registers as a method of the class, by name
      (:247-266).
  - `LANG` a finding located at (path, line) with no node.
    - SPQ-029 reports at line 1 of a canonical owner when it is missing. If the file is absent, the finding goes to a
      fixed path: `l2070_batching/conditional_step.py`, `…/steps/sql_conditional.py`, `…/steps/_conditional_compile.py`
      or `…/sql_backend_executor.py` (:539-617).
    - SPQ-047 reports at line 1 of `dependent_read_cte.py` and `compound_sequence.py` when a required name or call is
      absent (:2918-2943).
    - SPQ-053 reports at line 1 of the JSON inventory when its signature is invalid (:4444-4452).
    - The candidate needed declared "virtual diagnostic locations" for this.
    - Example:
      ```
      .violation missing_owner(at: path_line, what: symbol)
      missing_owner(@path_line("impl/common/l2070_server_plan/sql_backend_executor.py", 1), "lowering") :- arch_present(), !lowering_ok().
      ```
      Rules plus aggregates cannot say it because `at` must be a node or span, and an absent file has neither.
  - `NATIVE` signed JSON record inventory: an extension of `signed_list`.
    - The file holds `purpose`, `schema_version`, `entries` (objects) and `signature` (:2650-2744).
    - The signed message is the JSON of `{allowlist: sorted(entries), linter: purpose, version: 1}`, where each entry
      is rendered `category|path|class|function|max` (:2678, signing.py `compute_allowlist_message`). The signature is
      Ed25519 under the repository key.
    - Schema errors make the whole inventory invalid, so every limit becomes 0: a category outside the set,
      `max_occurrences` < 1 or a bool, or a duplicate (path, class, function).
    - Needs: entry rows (the rule splits them with `match` captures) and an `invalid(path)` fact the check can report,
      rather than a refusal.
  - `NATIVE` signed Python-embedded list: `ALLOWED_PRODUCTION_IR_TYPES` and `ALLOWLIST_SIGNATURE` constants in a `.py`
    file.
    - The entries can already be read from the AST (`EXISTS`), but the signature check over the sorted entries needs a
      reader.
    - Only the CLI verifies it: it exits 1 on failure (:4521). The plugin ignores it.
  - `POLICY`:
    - The plugin is hard-fail: every finding fails.
    - The CLI is a signed per-code ratchet with `--strict`. There SPQ-010 is a count of files (`set_count`, :3792), not
      findings, and SPQ-029 is never computed (:3942-4114).
    - Signature failure is a finding (SPQ-053) in the plugin but a run abort in the CLI.
    - Any extractor error in the project input aborts the whole check with a `RuntimeError` (:4335-4343), while a
      `SyntaxError` silently skips that file.
- **Hard parts:**
  - Multiplicity and duplicates:
    - SPQ-012 reports a helper method twice, once as a function and once as a `PlanExecutor` method (:1600, :1619).
    - SPQ-049 reports a file with ≥2 candidates twice per candidate ("locally" and "across planner surfaces").
    - The conditional facts record `self.f = cb` in `__init__` and each state write twice, from `_record_assignment` and
      `on_Attribute` (:356-367, :409-423).
    - SPQ-007 counts a class nested in a method under both the outer method and its own class.
    - Under the loose findings contract these collapse to one key per (code, path, line), which the oracle's counts do
      not.
  - Last-wins dictionaries: SPQ-001's method map (:1173), `local_fn`, conditional `methods`/`functions`/`rows_by_name`
    and `aliases`, and SPQ-042's constants are all keyed by name, and the last definition overwrites earlier ones.
  - `_is_abstract_method` (:1141): a body of only `...` is not abstract, because constant statements are filtered out
    first. Only `pass` (plus docstrings) or an `abstractmethod` decorator counts. `return True` is an int return of 1
    for `level()` and `return_ints`; `return -4` is not a constant.
  - SPQ-053 ranks in `ast.NodeVisitor` order. Decorators and return annotations are visited after the body and counted
    under the function's own name; class bases are counted under the class.
  - SPQ-053's class and function are the innermost of each, independently: a class nested in function f gives
    (D, f). Node-id order differs from visitor order only for decorators and return annotations.
  - Conditional `compile_calls`: a `.compile()` receiver counts only if it was assigned from a branch source earlier in
    traversal order (:443-459). That is a node-id comparison, not flow.
  - The `backend_selection` quirk: any `.backend_type`/`.is_sql_backend`/`.dialect` attribute plus any If, IfExp or
    Match anywhere in the function, not necessarily the enclosing one (:122-128).
  - SPQ-050:
    - Import bindings ignore statement order.
    - Comprehension targets shadow in the enclosing scope.
    - An untrusted `Literal["Connection"]` is still parsed and matched.
    - A class inside a function resets `function_local` to false.
    - Service matching is substring matching on the resolved dotted part, so a raw `ConnectionId` matches unless a
      trusted import resolves it to one of three exports.
  - Argument counting (SPQ-020 zero-argument `.resolve()`, :1999): tree-sitter comments are named children of an
    argument list. A `str_const` must not include docstrings unless the oracle does; SPQ-024 does include them.
  - Message text (sorted colour lists, :941, :3049) needs string aggregation. The loose contract does not key on it,
    so it is left out.

### serverplan_target_architecture (hard-fail, 648 lines)
- **Computes:** a zero gate over three kinds of finding.
  - Per-node findings: SPTA004 legacy strategy class names; SPTA009 backend-named batching/transport
    classes and defs under generic planning paths; SPTA006 executor parameters annotated with `ServerPlanStep`;
    SPTA005 step imports into `executor.py`; SPTA012 prohibited import prefixes per contract file; SPTA007
    `step_tags(...)` in an if/elif/ifexp test, match subject or guard; SPTA008 `getattr(_, "_stmt"…)` and
    `isinstance(_, Sql*/…Grammar/…Stmt)` in strategies; SPTA010 receipt-count assignments not read from
    `receipt.`/`execution_receipt.`.
  - Project-level contract findings, many at files that may not exist: SPTA001 required top-level symbols,
    SPTA013 class fields / frozen-slots dataclass / StrEnum members / closed-union variants, SPTA002/003/011
    specific method signatures.
  - Behavioural probes (SPTB001-009, SPTA014) that *execute* repository code.
- **Inputs:** Python AST of `_SCAN_DIRS` plus the four contract files (`lint_serverplan_target_architecture.py:192-200`,
  `:540-543`), tests excluded. The required-shape tables are hard-coded (`:71-205`). Runtime: imports and runs
  `scripts/serverplan_target_shape_probes.py` (`:546-571`), which imports `l2070_batching`,
  `impl.common.l2065_plan_core` and others (`serverplan_target_shape_probes.py:8-37`).
- **Needs:**
  - `EXISTS` `stmt`/`defstmt` (top-level statements, decorators), `def_name`, `nearest(_, _, scope)`,
    `nameref` (for `_names` = every `ast.Name` id under an expression, `:229-232`), `decorator`, `call_arg`,
    `field`/`text` for the raw `node.module` without dots (`:324`), `doc` path equality and prefixes.
  - `EXISTS shadowed`/`site_latest` (python/names) for `unions[name]`: the last top-level assignment wins (`:389-393`).
  - `EXISTS count` aggregates for the SPTA013 "missing any" tests, one finding per class (`:466-468`, `:482-486`).
  - `LANG` path-anchored findings: a violation whose `at` is a repository path (and line), not a node, so
    that it can name a file that does not exist. SPTA001 reports required symbols "missing from {path}" when the
    file is absent (`:456-459`). SPTA002/003/011 report at fixed paths with line 1 (`:494-536`). `.violation` takes
    `at: loc` today, and a node needs a document.
    ```
    .violation missing_contract(at: path, symbol: symbol) "required target contract is missing from {at}"
    missing_contract(p, s) :- required_symbol(p, s), !top_symbol(p, s).
    ```
  - `LANG` string-join aggregate (sorted), for messages only: "missing fields: a, b" (`:468`, `:484`,
    `:493`). Under the loose key, `count > 0` is enough for parity, so this is cosmetic.
    `m = join ", " f : { req_field(p, c, f), !has_field(p, c, f) }`.
  - `NATIVE probe_result(code: symbol, probe: symbol, message: symbol)` (or the general form: a reader for an
    external check's JSON output). The SPTB probes construct objects and call methods of the data layer
    (`serverplan_target_shape_probes.py:84-115`). That is Python execution, which no Datalog over the tape can
    reproduce. SPTA014 (required probe IDs not registered, `:549-557`) is static. The IDs come from
    `probe_id: str = "…"` class fields of the classes listed in `PROBES` (`serverplan_target_shape_probes.py:44`,
    `:411-421`), and the rule can read them with pattern atoms.
- **Hard parts:** the oracle's notion of "top level" differs per construct. A class counts as top level when no
  class or function encloses it, so a class under a module-level `if` counts (`:260`). A function or assignment
  needs `len(ancestors) == 1`, so the same `if` hides it (`:303`, `:389`). `_frozen_slots` decides on the
  **first** decorator that is a call to `dataclass` (Name or attribute), and needs both `frozen=True` and
  `slots=True` as literal `True` (`:235-244`). `elif` is a nested `ast.If`, so each elif test is checked and
  reported at the elif line. Comprehension `if`s and `while` tests are not checked. SPTA005 tests
  `node.module`, which for `from .steps import X` is `"steps"` (no dots), so a single-dot relative import of
  `steps` passes (`:326`). In `ast.walk`, a string annotation is a Constant, so names in string annotations
  never count. A class dict keyed by name means a duplicate top-level class name keeps the last definition.

### versioned_merged_identity (hard-fail, 383 lines)
- **Computes:** every call whose callee name (Name id or attribute attr) is `SqlVersionedMergedCte`, or an
  alias imported for it, in the direct `*.py` children of two directories. Each call is keyed by caller
  `"<file name>:<Class.func.nested>"`. VMI002: caller not in the 12 expected callers. VMI003: expected caller
  whose `read_projection=` keyword value does not resolve to a call of one of three projection builders.
  VMI001: expected callers with no constructor at all.
- **Inputs:** Python AST of `impl/common/l2070_server_plan/steps/*.py` and `impl/common/l9700_sql_graph_query/*.py`
  (non-recursive glob, `check_versioned_merged_identity.py:29-32`, `:289`, `:356`). Hard-coded
  `EXPECTED_CALLERS` and `ALLOWED_PROJECTION_BUILDERS` (`:33-72`).
- **Needs:**
  - `EXISTS` `.scope include "impl/common/l2070_server_plan/steps/*.py"` (non-recursive glob).
  - `EXISTS qualname` + `doc` + `file_name` + `strlen`/`substr` for the caller key: qualname of the nearest
    enclosing def or class with the module prefix removed. Classes and functions at every depth count
    (`:143-163`). A call at module level has caller `"file.py:"`.
  - `EXISTS import_alias` for constructor aliases: `from <any module> import SqlVersionedMergedCte as Y`, any
    module, anywhere in the file (`:134-141`).
  - `LIB latest_asg_before(c: node, x: symbol, rhs: node)`: the last assignment to x in c's innermost function,
    *completed* (its end offset) before c starts. Tuple/list targets bind each name only when the value is a
    Call (`:193-201`). Body: `rasg` / a tuple-target rule, `nearest(…, fdef)`, `max` over end offsets.
    `on_Call` snapshots the current function's dict (`:232`), and the dict is written on `on_exit_Assign`
    (`:203-217`). Module-level assignments are never recorded (`:194`).
  - `LIB catalog_proj(c: node, e: node)`: recursive. e is a call of an allowed builder (by callee name), or a
    name whose `latest_asg_before(c, x, r)` is a catalog projection, **always relative to the constructor call
    c** (`:109-115`). Cycles fail, as with the `seen` set.
  - `LANG` path-anchored findings (as in serverplan_target_architecture). VMI001 is reported at the directory
    `impl/common/l2070_server_plan/steps` (the default path), line 0, when the expected caller has no
    constructor (`:337-347`, `:257`). No node or document exists for it.
- **Hard parts:** aliases are collected during traversal, so an alias import that appears *after* a call does
  not make that call a constructor (`constructor_names` is mutated in place, `:141`, `:224`). Each name in a
  resolution chain is looked up in the snapshot taken at the constructor call, not at the chain's own
  assignment. In `p = catalog(); q = p; p = other(); Cte(read_projection=q)`, the result is *not* catalog. An
  assignment inside a class body nested in a function writes into the function's dict (classes push no
  dict). The caller key uses only the file name, so same-named files in the two roots share callers.

### architecture_rule_docs (hard-fail, 334 lines)
- **Computes:** a set of `ARD001` "drift" failures between the architecture catalog and its documentation:
  - DL_SUITE linters that have no classification, and stale classifications;
  - architectural classifications that have no `doc_path`;
  - rule IDs (`\b(?:SPQ|SPT|STEP|HSP)-\d{3}\b`) found in four linter scripts but missing from the catalog;
  - required sections missing from three index docs;
  - history examples whose commit is not a full SHA, or whose commit or `commit:path` does not resolve in Git;
  - per rule doc: the file is missing, it lacks one of ten catalog texts or thirteen section names, it contains
    a forbidden scaffold phrase, it has fewer than 180 words, or its remediation text has fewer than 12 unique
    words.

  All failures are reported at `docs/design/ARCHITECTURAL_LINTERS.md`, with line = the failure's ordinal.
- **Inputs:** the evaluated values of `scripts/lint_lib/architectural_rules.py` (`ARCHITECTURE_RULES`,
  `LINTER_CLASSIFICATIONS`, `ARCHITECTURAL_NOTICE`) and `scripts/lint_all.py` (`DL_SUITE`); the raw text of
  `scripts/lint_serverplan_quality.py`, `lint_serverplan_state_tripwires.py`, `lint_serverplan_steps.py` and
  `lint_history_serverplan_projection.py`; markdown under `docs/design/` (three index docs plus every rule's
  `doc_path`); Git objects through `git cat-file -e` (`lint_architecture_rule_docs.py:219-247`). The plugin only
  triggers on `scripts/lint_lib/architectural_rules.py` (`:263-267`).
- **Needs:**
  - `NATIVE py_const(path: symbol, name: symbol, …)` evaluation, or a generated catalog artifact read with
    `.input arch_rule(IO=file, filename="…json")`. `ARCHITECTURE_RULES` is built at import time by loops over
    dicts, f-strings, `HISTORY_EXAMPLES[_idx % len(...)]`, `dataclasses.replace` and if-chains on lowercased
    titles (`architectural_rules.py:1244-1330`, `:3683-3876`). Datalog over the AST could only mirror that by
    copying the logic into the rule, which rules must not carry. `LINTER_CLASSIFICATIONS` and `DL_SUITE`
    (`lint_all.py:687`) are literal constructor calls, so pattern atoms could read them, but the rules derived
    from them are computed.
  - `NATIVE file_text(path: symbol, s: symbol)`: the contents of a non-Python file in the tree. core has only
    `file(path)`. Section, catalog-text and forbidden-phrase tests are `contains` over the markdown
    (`:162-205`). The four scripts could instead be read as Python documents in scope (`match_all` over module
    text), but only if they are in the policy's scope.
  - `NATIVE @count_matches(re: symbol, s: symbol): number`, or `LANG` an occurrence column for `match_all` on a
    symbol. The word count is the number of `[A-Za-z0-9_]+` *occurrences* (`:206-207`). `match_all` on a symbol
    binds only symbols, so equal words collapse under set semantics. The unique-word count (`:211`) is the
    collapsed form plus lower-casing: `NATIVE @lower(s)`, unless `@convert(s, "lowerCase", "")` is defined to keep
    separators.
    `n = @count_matches("[A-Za-z0-9_]+", t), n < 180`.
  - `NATIVE git_commit(sha: symbol)` and `git_blob_at(sha: symbol, path: symbol)`: object existence in the Git
    store, history included (`:219-247`). core's `file` covers only the scanned tree.
  - `LANG` path-anchored findings (as above): every failure sits at `docs/design/ARCHITECTURAL_LINTERS.md`,
    and a missing rule doc is itself a finding (`:179-181`).
  - `EXISTS count` for the line ordinal (`:281-291`): `l = 1 + count : { fail(k2), k2 < k }` over a composite
    sort key. Better, under the loose key, to drop the ordinal and key failures by message.
- **Hard parts:** the history check dedups on `(commit, path)` before testing (`:232-237`), so two rules that
  share an example report once, under the first rule's ID in dict order. Index-section tests skip the
  gap-closure doc, which has its own section list (`:163-172`). `_rule` rewrites `how_to_address` to
  `"For {title}: {address}"` unless the title already appears in it (`architectural_rules.py:1255`). The
  catalog-text tests therefore depend on computed strings, and the dependency is real.

## Summary table

Needs that are the same thing under different names are merged, and the other names are listed. Abbreviations:
MA = matching_architecture, IDE = impl_direct_execution, RRS = render_raw_strings, ROW = rowid_comparisons,
SEC = security_patterns, SPD = serverplan_dispatch_architecture, SPQ = serverplan_quality,
SPTA = serverplan_target_architecture, VMI = versioned_merged_identity, ARD = architecture_rule_docs.

| tag | signature or construct | checks that need it |
|---|---|---|
| LANG | Path-anchored finding: `at` is (repository path, line) with no node, so it can name an absent file, a directory or a non-Python file. Examples: absent owners, required contracts, signed-list files. | MA, SPD, SPQ, SPTA, VMI, ARD |
| LIB | `ast_parent(n, p)` / `unparen(n, m)`: the CPython-AST view of the tree. It skips `parenthesized_expression`, `argument_list` and similar wrappers that tree-sitter adds and `ast` does not have. | MA, RRS, SEC, SPD, SPQ |
| NATIVE | `py_str_part(lit, i, v, at, in_spec)`: CPython's `Constant(str)` parts, decoded. It merges implicit concatenation, splits each f-string literal run at interpolations, and covers format-spec parts. Bytes give no row. On top of it, LIB `str_const(n, v)`, `static_str(n, s)` and `fstr_literal(n, s)`. (Also named: `value` on f-string pieces.) | MA, RRS, SPQ (SEC/SPTA use the plain-literal case) |
| LIB | `ast_name(n, x)` / `name_or_attr(n, x)`: an identifier in an `ast.Name` position. That is `nameref` minus parameters, import names, `global`/`nonlocal` and `except … as` names; `name_or_attr` adds attribute attrs. | MA, SPQ, SPTA (`_names`) |
| NATIVE | Raw text of a file. `doc_text(d, s: span)` covers a parsed document's bytes (substring gates, line counts, line regexes). `file_text(path, s)` covers non-Python files (markdown). Over these, LIB `source_line(d, l, s)` uses `match_all` to give physical lines. | SPQ, SEC, MA (gate via module text), ARD |
| LIB | `py_assign(st, t)` / `assign_target` / `assign_value`: CPython `Assign.targets` flattened from tree-sitter's right-nested `a = b = v`, Tuple vs List targets, and an AnnAssign with an optional value. Also LIB `ast_stmt(n, s)`, the nearest CPython statement (an `expression_statement` around an assignment counts as the Assign). | MA, SPQ, SEC, VMI (tuple targets), SPTA |
| LIB | `latest_asg_before(c, x, rhs)` / `latest_store(n, st)`: the last assignment to x in the same function before point c (`max` over offsets). Covers last-wins name maps keyed by traversal order. | VMI, SPD, ROW (annotation map, file-wide variant), SPQ (last-def-wins maps) |
| NATIVE | `py_ast_order(n, i)`: pre-order index in CPython `_fields` order. Decorators and return annotations come after the body, and defaults after all parameters. Needed only where the oracle's "first"/"latest" follows `ast` visit order and start offsets differ. | ROW, MA, SPQ (SPQ-053) |
| NATIVE | `op_tok(n, s, start)`: anonymous operator tokens with offsets. On top of it, LIB `cmp_pair(c, i, op, l, r)` / `cmp_chain`: every pair of a chained comparison (`cmp` covers single operators only). | ROW, SPQ, SPD (whole-chain tests) |
| LIB | `base_simple(c, x)` / `base_tail(c, s)`: unresolved base names (Name id, Attribute attr, Subscript value). `class_base` resolves, the oracles do not. | ROW, SPQ |
| LIB | `type_name(e, t)` / `ann_name(n, s)` / `type_names(n, x)`: syntactic type names of an annotation or expression. Each oracle's variant differs: last attr only, dotted name, or `A\|B` flattened. One family, each with its own rule. | ROW, MA, SPD |
| LIB | `call_path(e, i, s)` / `dotted_expr(e, s)`: a Name/Attribute chain through calls, as a list or a dotted string. | SPQ, SPD (`backend_type` chains) |
| LIB | `ast_constant(n)`: CPython `ast.Constant`. Strings without interpolation, numbers, True/False/None, Ellipsis; not negative numbers or f-strings. | SEC, MA (MA027), SPTA (`True` keyword tests) |
| LIB | `doc_string(f, s)` / `const_expr_stmt(s)` / `body_stmt_count(f, k)`: docstrings and constant-expression statements of a body. | RRS, SPQ |
| LIB | `py_if(i)`, `if_test`, `if_body`, `if_orelse`: a Python `If` including each `elif` as a nested If. | SPQ (SPTA's elif reporting is the same shape) |
| LIB | `class_field(c, name, ann, st)` / `body_def(b, f)`: direct class-body annotated fields and defs, through decorators. `method_of` also takes defs under `if`/`try`. | MA, SPQ, SPTA |
| LIB | `rowid_type(d, x)`: per-file recursive closure of RowId subclasses. `rowid_expr(e, d)`: the RowId-operand heuristic. | ROW |
| LIB | `catalog_proj(c, e)`: projection resolution through call-time assignments, recursive. | VMI |
| LIB | `executor_owner(n)`: value-provenance fixpoint. `backend_source`, `dialect_truth`, `step_type`. | SPD |
| LIB | `match_delegate`, `ast_tuple_elt`: matching-specific helpers. | MA |
| LIB | `always_raises`, `local_fn`/`ncall`, `ann_import`/`ann_imp`/`ann_shadow`/`ann_part`, `ca_module`/`ca_alias`/`ca_base`/`ca_cond`: serverplan_quality-specific helpers. | SPQ |
| NATIVE | `parsed(s, "annotation", r)` for any string under an annotation, not only Ruff positions. | SPQ |
| NATIVE | `parse_error(d)`: the file is not valid Python, so the oracle skips it. | SPQ |
| NATIVE | Signed records beyond `signed_list`: a signed JSON inventory with structured entries, a schema check and an `invalid(path)` fact; signature constants embedded in a `.py` file. | SPQ |
| NATIVE | `@count_matches(re, s)` (or a LANG occurrence column on `match_all` over a symbol) plus `@lower(s)`: word counts with duplicates. | ARD |
| NATIVE | `git_commit(sha)`, `git_blob_at(sha, path)`: Git object existence, history included. | ARD |
| NATIVE | Evaluated Python constants (`py_const`), or a reader for a generated catalog artifact: catalogs built by loops, f-strings and `replace` at import time. | ARD |
| NATIVE | `probe_result(code, probe, message)`: an external runner's output for behavioural probes that execute repository code. | SPTA |
| LANG | String-join aggregate, sorted (`join ", " f : { … }`). Message text only; not needed under the loose key. | SPTA, SPQ |
| POLICY | Signature failure, or a must-be-empty list being non-empty, reported as a finding rather than a run refusal. | SPD, SPQ (SPQ-053) |
| POLICY | Budget grouped by severity bucket (`errors`/`warnings`) plus per-code severity. | SEC |
| POLICY | Extraction or read failure as a finding (SEC000), vs a SyntaxError silently skipping the file, vs an extractor error aborting the check. | SEC, SPQ |
| POLICY | Scope extended by a runtime option (`--application-root`), and a code subset selected by an option (`--phase`). | MA |
| POLICY | Two enforcement paths with different semantics: a hard-fail plugin vs a signed per-code ratchet CLI, with different counting (SPQ-010 counts files) and different code sets (SPQ-029 only in the plugin). Also rowid's CLI ratchet includes tests while its plugin does not. | SPQ, ROW |
